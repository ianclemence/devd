//! Process supervision.
//!
//! `devd` owns the lifetime of everything it starts. The daemon spawns each
//! managed service with stdout and stderr redirected to a log file, records the
//! process id in the state store, and kills the whole **process tree** on stop -
//! otherwise a stopped nginx leaves orphaned `php-cgi` workers holding the
//! FastCGI socket, which is exactly the class of bug a batch-file launcher
//! cannot fix.
//!
//! **Windows isolation:** supervised work (a project task, a check, a one-shot
//! command) is placed in its own Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and stopping it terminates that job.
//! When the supervising process is killed without running its shutdown path,
//! the operating system closes the job handles and kills every process in the
//! job. Assignment happens immediately after spawn; if devd itself already runs
//! inside a restrictive job (some CI runners) the assignment can fail, and
//! `kill_tree` remains the fallback for that case.
//!
//! A service or a public link is different: [`SpawnSpec::persist`] is true
//! because it is meant to outlive the command that started it. Such a process
//! is **not** put in a kill-on-close job, since closing the job as a one-shot
//! `devd service start` exits would kill the service it had just started. Its
//! tree is stopped with `kill_tree` when the service is stopped, and it keeps
//! running if devd crashes, which is what makes the stack usable without a
//! background daemon. The child environment is allowlisted rather than
//! inherited, so the daemon's credentials do not leak into a project's
//! processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::{DevdError, Result};
use crate::util;

// devd targets Windows, Linux and macOS. Saying so here is better than a pid
// lookup that quietly answers "nothing" on a platform nobody has tried.
#[cfg(not(any(windows, unix)))]
compile_error!("devd supports Windows, Linux and macOS");

/// Windows process queries.
///
/// `tasklist` is the portable answer, but it starts a process per call - about
/// half a second each - which made a service list with three services take
/// several seconds. These are the same questions asked of the kernel directly.
#[cfg(windows)]
mod win {
    use core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        pub fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        pub fn QueryFullProcessImageNameW(
            handle: *mut c_void,
            flags: u32,
            buffer: *mut u16,
            size: *mut u32,
        ) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
        pub fn GetProcessTimes(
            handle: *mut c_void,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;

        // Job objects. A job is a kernel container for a process and every
        // descendant it creates; with KILL_ON_JOB_CLOSE the kernel terminates
        // the whole container when the last handle closes, including when devd
        // is killed without running its shutdown path.
        pub fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        pub fn SetInformationJobObject(
            job: *mut c_void,
            class: u32,
            info: *mut c_void,
            len: u32,
        ) -> i32;
        pub fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        pub fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
    }

    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    pub const STILL_ACTIVE: u32 = 259;

    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic: BasicLimitInformation,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    /// A job that kills everything in it when its last handle closes.
    pub fn create_kill_on_close_job() -> Option<usize> {
        // SAFETY: the calls take a null name/attributes, a pointer to a local
        // struct of the documented layout and that struct's size. The struct is
        // alive for the duration of the call.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info = ExtendedLimitInformation::default();
            info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ExtendedLimitInformation>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return None;
            }
            Some(job as usize)
        }
    }

    pub fn assign(job: usize, process: *mut c_void) -> bool {
        // SAFETY: both values are handles to kernel objects owned by this
        // process. Assignment does not take ownership of either.
        unsafe { AssignProcessToJobObject(job as *mut c_void, process) != 0 }
    }

    pub fn terminate_job(job: usize) {
        // SAFETY: the handle came from CreateJobObjectW and is still open.
        unsafe {
            TerminateJobObject(job as *mut c_void, 1);
        }
    }

    pub fn close_job(job: usize) {
        // SAFETY: the handle came from CreateJobObjectW and is closed once.
        unsafe {
            CloseHandle(job as *mut c_void);
        }
    }
}

/// Names a project process may inherit when devd clears its environment.
///
/// The allowlist is deliberately about *names*, not values: a PATH, a home
/// directory, a locale and an installed runtime's own home are configuration,
/// while a token, password or API key is a credential. Credentials a project
/// needs belong in the project's own environment file, passed as `envs`, not in
/// the daemon's environment.
pub const CHILD_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SYSTEMROOT",
    "SystemDrive",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "USER",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "TZ",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "DEVD_HOME",
    "CI",
    "NO_COLOR",
    "FORCE_COLOR",
    "JAVA_HOME",
    "GOROOT",
    "GOPATH",
    "GOMODCACHE",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUST_BACKTRACE",
    "RUST_LOG",
    "NODE_PATH",
    "NVM_HOME",
    "NVM_SYMLINK",
    "VOLTA_HOME",
    "PNPM_HOME",
    "DENO_DIR",
    "BUN_INSTALL",
    "PYENV_ROOT",
    "PIPX_HOME",
    "POETRY_HOME",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "COMPOSER_HOME",
    "DOTNET_ROOT",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "FLUTTER_ROOT",
    "HOMEBREW_PREFIX",
    "SSH_AUTH_SOCK",
];

/// Copy the allowlisted variables that are set into a cleared environment.
pub fn apply_child_environment(cmd: &mut Command) {
    for name in CHILD_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(name) {
            cmd.env(name, value);
        }
    }
}

/// A running managed process.
struct ManagedProcess {
    name: String,
    kind: String,
    child: Child,
    pid: u32,
    started_at: String,
    run_id: String,
    log_path: PathBuf,
    restarts: i64,
    /// Windows Job Object handle, stored as an integer so the process stays
    /// `Send`. The job is killed when the handle closes.
    #[cfg(windows)]
    job: Option<usize>,
}

#[cfg(windows)]
impl Drop for ManagedProcess {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            win::close_job(job);
        }
    }
}

/// Public view of a supervised process.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    /// Service name.
    pub name: String,
    /// Service kind.
    pub kind: String,
    /// Operating-system process id.
    pub pid: u32,
    /// Whether the process is still alive.
    pub running: bool,
    /// Start timestamp.
    pub started_at: String,
    /// How many restarts the supervisor has performed for this service.
    pub restarts: i64,
    /// Where this process writes its output.
    pub log_path: String,
    /// Exit code, when the process has exited.
    pub exit_code: Option<i32>,
}

/// Specification for starting a process.
pub struct SpawnSpec {
    /// Service name, unique among running processes.
    pub name: String,
    /// Service kind.
    pub kind: String,
    /// Absolute path to the executable.
    pub program: PathBuf,
    /// Arguments, passed as an array - never a shell string.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
    /// Extra environment variables.
    pub envs: Vec<(String, String)>,
    /// When true, the process starts with a cleared environment and sees only
    /// the allowlisted system/runtime variables ([`CHILD_ENV_ALLOWLIST`]) plus
    /// `envs`. Used by project manifests and services so the daemon's own
    /// credentials are not inherited.
    pub clear_env: bool,
    /// File to append stdout and stderr to.
    pub log_path: PathBuf,
    /// How many times this service has already been restarted.
    pub restarts: i64,
    /// When true, the process is meant to outlive the command that started it:
    /// a service or a public link. On Windows it is not placed in a
    /// kill-on-close Job Object, because closing that job as a one-shot
    /// `devd service start` exits would kill the service it just started.
    /// Stopping it still terminates the whole tree. Supervised work - a
    /// project task or a check - is false and keeps the job.
    pub persist: bool,
}

/// Owns every process `devd` started.
#[derive(Default)]
pub struct Supervisor {
    procs: Mutex<HashMap<String, ManagedProcess>>,
}

/// Stop children from inheriting this process's own standard handles.
///
/// A service is started **detached** so that it outlives the command that started
/// it. On Windows such a child would otherwise inherit the console - or, worse,
/// the pipe a script is capturing `devd`'s output through - and a shell that
/// waits for that pipe to reach end-of-file then waits for nginx to exit, which
/// is never. The symptom is a command that has printed its result and appears to
/// hang; `devd stack up | Tee-Object out.txt` is the plain case, and an editor's
/// integrated terminal is the common one.
///
/// Every child `devd` starts is given its handles explicitly - a log file for a
/// service, a fresh pipe inside [`crate::util::run`] for a one-shot command - so
/// clearing the inherit flag on our own handles costs nothing. Nothing outside
/// this process is modified: handle flags live in this process's handle table.
#[cfg(windows)]
pub fn isolate_standard_handles() {
    use core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const STD_ERROR_HANDLE: u32 = (-12i32) as u32;
    const HANDLE_FLAG_INHERIT: u32 = 1;

    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: both calls take a handle value and bit flags. Nothing is
        // dereferenced, the handles are borrowed from the operating system for
        // the duration of the call, and a failure - a null or invalid handle,
        // which happens when a surface has no console at all - is ignored
        // because there is then nothing to inherit in the first place.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() && handle as isize != -1 {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

/// Nothing to do where the operating system gives a child only the descriptors
/// it was passed explicitly.
#[cfg(not(windows))]
pub fn isolate_standard_handles() {}

impl Supervisor {
    /// An idle supervisor.
    pub fn new() -> Self {
        Self {
            procs: Mutex::new(HashMap::new()),
        }
    }

    /// Start a process, replacing any previous process with the same name.
    ///
    /// Returns the process id. The caller names the run, so there is no run
    /// identifier to hand back.
    pub fn start(&self, spec: SpawnSpec, run_id: String) -> Result<u32> {
        self.stop(&spec.name)?;

        if let Some(parent) = spec.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&spec.log_path)?;
        let log_err = log.try_clone()?;

        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        if spec.clear_env {
            cmd.env_clear();
            apply_child_environment(&mut cmd);
        }
        for (k, v) in &spec.envs {
            cmd.env(k, v);
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: survive the daemon's
            // console and let us signal the group on shutdown.
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }

        // Give the child its own process group so `kill_tree` can signal the
        // whole tree (a dev server and the workers it forks) on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        // The service is about to be started detached; make sure it cannot hold
        // the shell that launched devd open. See `isolate_standard_handles`.
        isolate_standard_handles();

        let child = cmd.spawn().map_err(|e| {
            DevdError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to start `{}` ({}): {e}",
                    spec.name,
                    spec.program.display()
                ),
            ))
        })?;
        let pid = child.id();
        let name = spec.name.clone();

        // Put the child in its own job before anything it starts can escape it.
        // Assignment after spawn is a small window; the alternative is
        // CREATE_SUSPENDED, which `std::process` does not expose. In the window
        // the child has not run any of its own code yet on a normal start, and
        // the tree kill below still covers the ordinary stop path.
        #[cfg(windows)]
        let job = if spec.persist {
            // A service outlives the command that started it. A
            // kill-on-close job would terminate it the moment a one-shot
            // `devd service start` or `devd stack up` exits, which is the
            // opposite of what those commands promise.
            None
        } else {
            win::create_kill_on_close_job().and_then(|job| {
                use std::os::windows::io::AsRawHandle;
                if win::assign(job, child.as_raw_handle()) {
                    Some(job)
                } else {
                    win::close_job(job);
                    None
                }
            })
        };

        let managed = ManagedProcess {
            name: name.clone(),
            kind: spec.kind,
            child,
            pid,
            started_at: util::now(),
            run_id,
            log_path: spec.log_path,
            restarts: spec.restarts,
            #[cfg(windows)]
            job,
        };

        let mut guard = self
            .procs
            .lock()
            .map_err(|_| DevdError::internal("supervisor lock poisoned"))?;
        guard.insert(name, managed);

        Ok(pid)
    }

    /// Stop a process and its children. Succeeds when nothing was running.
    pub fn stop(&self, name: &str) -> Result<Option<(i32, String)>> {
        let mut guard = self
            .procs
            .lock()
            .map_err(|_| DevdError::internal("supervisor lock poisoned"))?;
        let Some(mut managed) = guard.remove(name) else {
            return Ok(None);
        };

        // The job is the reliable tree kill: terminate it first so nothing the
        // service forked can survive its parent. `kill_tree` remains the
        // fallback for the case where no job could be assigned.
        #[cfg(windows)]
        if let Some(job) = managed.job.take() {
            win::terminate_job(job);
            win::close_job(job);
            managed.job = None;
        }
        kill_tree(managed.pid);
        let code = match managed.child.wait() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        Ok(Some((code, managed.run_id.clone())))
    }

    /// Stop everything. Used on daemon shutdown.
    pub fn stop_all(&self) -> Result<Vec<String>> {
        let names: Vec<String> = {
            let guard = self
                .procs
                .lock()
                .map_err(|_| DevdError::internal("supervisor lock poisoned"))?;
            guard.keys().cloned().collect()
        };
        for name in &names {
            let _ = self.stop(name);
        }
        Ok(names)
    }

    /// Whether a named process is currently running.
    pub fn is_running(&self, name: &str) -> bool {
        let Ok(mut guard) = self.procs.lock() else {
            return false;
        };
        let Some(managed) = guard.get_mut(name) else {
            return false;
        };
        matches!(managed.child.try_wait(), Ok(None))
    }

    /// Snapshot of every supervised process, reaping exited children.
    pub fn list(&self) -> Vec<ProcessInfo> {
        let Ok(mut guard) = self.procs.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for managed in guard.values_mut() {
            let (running, exit_code) = match managed.child.try_wait() {
                Ok(None) => (true, None),
                Ok(Some(status)) => (false, status.code()),
                Err(_) => (false, None),
            };
            out.push(ProcessInfo {
                name: managed.name.clone(),
                kind: managed.kind.clone(),
                pid: managed.pid,
                running,
                started_at: managed.started_at.clone(),
                restarts: managed.restarts,
                log_path: managed.log_path.to_string_lossy().to_string(),
                exit_code,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Names that have exited since they were started.
    pub fn reaped(&self) -> Vec<(String, String, i32)> {
        let Ok(mut guard) = self.procs.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut done = Vec::new();
        for (name, managed) in guard.iter_mut() {
            if let Ok(Some(status)) = managed.child.try_wait() {
                done.push((
                    name.clone(),
                    managed.run_id.clone(),
                    status.code().unwrap_or(-1),
                ));
            }
        }
        for (name, run_id, code) in done {
            out.push((name.clone(), run_id, code));
            let _ = guard.remove(&name);
        }
        out
    }

    /// Read the tail of a service's log file.
    pub fn tail_log(&self, log_path: &Path, max_lines: usize) -> Result<Vec<String>> {
        let text = std::fs::read_to_string(log_path).unwrap_or_default();
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        if lines.len() > max_lines {
            lines.drain(0..lines.len() - max_lines);
        }
        Ok(lines)
    }
}

/// Terminate a process and all of its descendants.
///
/// Deliberately uses the platform tools rather than a shell string: `taskkill`
/// on Windows (which walks the child tree itself) and a process-group signal on
/// Linux.
pub fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        if let Some(taskkill) = util::which("taskkill") {
            let args = vec![
                "/PID".to_string(),
                pid.to_string(),
                "/T".to_string(),
                "/F".to_string(),
            ];
            let _ = util::run(
                &taskkill,
                &args,
                None,
                &[],
                Some(std::time::Duration::from_secs(15)),
            );
        }
    }
    #[cfg(unix)]
    {
        // Negative pid signals the whole process group.
        let pid_i = pid as i32;
        unsafe {
            libc_kill(-pid_i, 15);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        unsafe {
            libc_kill(-pid_i, 9);
        }
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

/// The supervisor shared by every capability in this process.
///
/// `devd` does not require a background daemon for services to run. A one-shot
/// `devd service start nginx` starts nginx detached and records its pid, so the
/// process outlives the command that started it. The shared supervisor matters
/// for the long-lived surfaces - the application window and `devd serve` - which
/// start services *and* keep the handles, so they can wait for a clean exit on
/// shutdown instead of killing by pid.
pub fn shared() -> std::sync::Arc<Supervisor> {
    static SHARED: std::sync::OnceLock<std::sync::Arc<Supervisor>> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| std::sync::Arc::new(Supervisor::new()))
        .clone()
}

/// Image name of a running process, without a path or extension.
///
/// Used to confirm that a pid read back from a pid file is still the process it
/// claims to be: process ids are reused, and killing a recycled pid would
/// terminate something that has nothing to do with `devd`.
pub fn process_image(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    #[cfg(windows)]
    {
        unsafe {
            let handle = win::OpenProcess(win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buf = vec![0u16; 32 * 1024];
            let mut len = buf.len() as u32;
            let ok = win::QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
            win::CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            let name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
            let stem = name.trim_end_matches(".exe");
            if stem.is_empty() {
                None
            } else {
                Some(stem.to_string())
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let text = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let name = text.trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }
    // macOS has no /proc; `ps` names the process.
    #[cfg(target_os = "macos")]
    {
        let ps = util::which("ps")?;
        let out = util::run(
            &ps,
            &[
                "-p".to_string(),
                pid.to_string(),
                "-o".to_string(),
                "comm=".to_string(),
            ],
            None,
            &[],
            Some(std::time::Duration::from_secs(5)),
        )
        .ok()?;
        let path = out.stdout.trim();
        let name = path.rsplit('/').next().unwrap_or(path);
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }
}

/// An opaque token for when a process started.
///
/// A process id is reused; the pair of an id and the moment that process began is
/// not. Comparing the token read now with the one recorded when devd launched or
/// claimed the process is how ownership is established without trusting a port,
/// an image name or a pid file alone. The token is only ever compared for
/// equality, so its unit differs by platform: Windows creation time, Linux
/// clock ticks since boot, macOS the `ps` start string.
pub fn process_start_token(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    #[cfg(windows)]
    {
        unsafe {
            let handle = win::OpenProcess(win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
            let ok = win::GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
            win::CloseHandle(handle);
            (ok != 0 && created != 0).then(|| created.to_string())
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        start_ticks_from_stat(&stat)
    }
    #[cfg(target_os = "macos")]
    {
        let ps = util::which("ps")?;
        let out = util::run(
            &ps,
            &["-p".to_string(), pid.to_string(), "-o".to_string(), "lstart=".to_string()],
            None,
            &[],
            Some(std::time::Duration::from_secs(5)),
        )
        .ok()?;
        let text = out.stdout.trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// The start time field of a `/proc/<pid>/stat` line.
///
/// The command name is in parentheses and may itself contain spaces and
/// parentheses, so the fields are counted from the last `)`.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn start_ticks_from_stat(stat: &str) -> Option<String> {
    let rest = stat.rsplit_once(')')?.1;
    // After the command: state is field 3, start time is field 22.
    rest.split_whitespace().nth(19).map(str::to_string)
}

/// Whether a process id is alive. Used to detect orphans after a crash.
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        // `tasklist` costs about half a second per call, which turned a status
        // page into a multi-second wait once there were a few services. The
        // process handle answers the same question in microseconds.
        unsafe {
            let handle = win::OpenProcess(win::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = win::GetExitCodeProcess(handle, &mut code);
            win::CloseHandle(handle);
            ok != 0 && code == win::STILL_ACTIVE
        }
    }
    // Signal 0 sends nothing and reports whether the process exists. It works
    // on every Unix, which /proc does not (macOS has none).
    #[cfg(unix)]
    {
        unsafe { libc_kill(pid as i32, 0) == 0 }
    }
}

/// A process id together with when that process started.
///
/// A bare pid in a file proves nothing after the operating system reuses the
/// number. The start time recorded beside it is what makes the file mean "the
/// process devd launched", and a record from before this was written (a bare
/// pid) is still read, with the weaker image check alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidRecord {
    /// Process id.
    pub pid: u32,
    /// Start token recorded when devd launched the process, when it was.
    pub start: Option<String>,
}

/// Write the pid file for a process devd has just launched.
pub fn write_pid_record(file: &std::path::Path, pid: u32) -> Result<()> {
    let text = match process_start_token(pid) {
        Some(start) => format!("{pid}
{start}
"),
        None => pid.to_string(),
    };
    util::atomic_write(file, &text)
}

/// Read a pid file, in either format.
pub fn read_pid_record(file: &std::path::Path) -> Option<PidRecord> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let start = lines
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string);
    Some(PidRecord { pid, start })
}

impl PidRecord {
    /// Whether this is still the process devd launched: alive, running the
    /// expected program, and (when recorded) started at the recorded time.
    pub fn is_owned(&self, expected_image: &str) -> bool {
        pid_alive(self.pid)
            && process_image(self.pid).is_some_and(|i| i.eq_ignore_ascii_case(expected_image))
            && self
                .start
                .as_deref()
                .is_none_or(|recorded| process_start_token(self.pid).as_deref() == Some(recorded))
    }
}

/// The process that holds a claim file: the id written in it, when that process is still running
/// and is a devd program. A claim whose process has gone, or whose id was reused by something else,
/// is stale and held by nobody.
pub fn live_devd_holder(file: &std::path::Path) -> Option<u32> {
    let pid = std::fs::read_to_string(file)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    (pid_alive(pid)
        && process_image(pid).is_some_and(|image| image.to_ascii_lowercase().contains("devd")))
    .then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_has_a_stable_start_token_and_a_dead_one_has_none() {
        let me = std::process::id();
        let first = process_start_token(me).expect("this process has a start time");
        assert_eq!(process_start_token(me), Some(first));
        assert_eq!(process_start_token(0), None);
        assert_eq!(process_start_token(u32::MAX - 7), None);
    }

    #[test]
    fn a_pid_record_needs_the_same_start_time_to_be_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("devd.pid");
        let me = std::process::id();
        let image = process_image(me).unwrap();
        write_pid_record(&file, me).unwrap();
        let record = read_pid_record(&file).unwrap();
        assert_eq!(record.pid, me);
        assert!(record.is_owned(&image));
        assert!(!record.is_owned("some-other-program"));

        // The same pid number with a different start time is a recycled pid.
        let recycled = PidRecord {
            pid: me,
            start: Some("0".into()),
        };
        assert!(!recycled.is_owned(&image));

        // A pid file written by an older devd is a bare number and still reads.
        std::fs::write(&file, me.to_string()).unwrap();
        let legacy = read_pid_record(&file).unwrap();
        assert_eq!(legacy.start, None);
        assert!(legacy.is_owned(&image));
        assert!(read_pid_record(&dir.path().join("missing")).is_none());
    }

    #[test]
    fn the_start_field_is_counted_from_the_last_parenthesis() {
        let stat = "42 (my (odd) name) S 1 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 200";
        assert_eq!(start_ticks_from_stat(stat).as_deref(), Some("987654"));
    }

    fn sleeper_program() -> (PathBuf, Vec<String>) {
        #[cfg(windows)]
        {
            (
                PathBuf::from("cmd.exe"),
                vec!["/c".into(), "ping -n 30 127.0.0.1 > NUL".into()],
            )
        }
        #[cfg(unix)]
        {
            (PathBuf::from("/bin/sleep"), vec!["30".to_string()])
        }
    }

    #[test]
    fn start_stop_lifecycle() {
        let dir = std::env::temp_dir().join(format!("devd-sup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sup = Supervisor::new();
        let (program, args) = sleeper_program();
        let pid = sup
            .start(
                SpawnSpec {
                    name: "sleeper".into(),
                    kind: "test".into(),
                    program,
                    args,
                    cwd: None,
                    envs: vec![],
                    clear_env: false,
                    log_path: dir.join("sleeper.log"),
                    restarts: 0,
                    persist: false,
                },
                "run-1".into(),
            )
            .unwrap();
        assert!(pid > 0);
        assert!(sup.is_running("sleeper"));
        assert_eq!(sup.list().len(), 1);

        let stopped = sup.stop("sleeper").unwrap();
        assert!(stopped.is_some());
        assert!(!sup.is_running("sleeper"));
        assert!(sup.list().is_empty());

        // Stopping something that is not running is not an error.
        assert!(sup.stop("sleeper").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn starting_twice_replaces_the_first_process() {
        let dir = std::env::temp_dir().join(format!("devd-sup2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sup = Supervisor::new();
        let (program, args) = sleeper_program();
        let spec = || SpawnSpec {
            name: "sleeper".into(),
            kind: "test".into(),
            program: sleeper_program().0,
            args: sleeper_program().1,
            cwd: None,
            envs: vec![],
            clear_env: false,
            log_path: dir.join("sleeper.log"),
            restarts: 0,
            persist: false,
        };
        let _ = (program, args);
        let pid1 = sup.start(spec(), "r1".into()).unwrap();
        let pid2 = sup.start(spec(), "r2".into()).unwrap();
        assert_ne!(pid1, pid2);
        assert_eq!(sup.list().len(), 1);
        sup.stop_all().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_tree_tolerates_unknown_pid() {
        kill_tree(0);
        kill_tree(999_999);
    }

    #[test]
    fn a_persistent_process_outlives_the_supervisor_that_started_it() {
        let dir = std::env::temp_dir().join(format!("devd-sup3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid = {
            let sup = Supervisor::new();
            let (program, args) = sleeper_program();
            sup.start(
                SpawnSpec {
                    name: "daemon".into(),
                    kind: "service".into(),
                    program,
                    args,
                    cwd: None,
                    envs: vec![],
                    clear_env: false,
                    log_path: dir.join("daemon.log"),
                    restarts: 0,
                    persist: true,
                },
                "run-1".into(),
            )
            .unwrap()
        };
        // The command that started it has gone. A service must still be there:
        // `devd stack up` starts the stack and exits, and the stack is useless
        // if the web server dies with the command that started it.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            pid_alive(pid),
            "a persistent process must survive the supervisor being dropped"
        );
        kill_tree(pid);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
