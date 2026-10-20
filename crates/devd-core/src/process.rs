//! Process supervision.
//!
//! `devd` owns the lifetime of everything it starts. The daemon spawns each
//! managed service with stdout and stderr redirected to a log file, records the
//! process id in the state store, and kills the whole **process tree** on stop -
//! otherwise a stopped nginx leaves orphaned `php-cgi` workers holding the
//! FastCGI socket, which is exactly the class of bug a batch-file launcher
//! cannot fix.
//!
//! **Windows note (honest limitation):** tree termination is performed with
//! `taskkill /T /F` rather than a Windows Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. A job object would additionally
//! guarantee that children die if `devd` itself is killed without running its
//! shutdown path. That is a known gap, not a solved problem; the current
//! approach covers every ordinary start/stop/restart path and daemon shutdown,
//! but not `TerminateProcess` on the daemon. See `docs/limitations.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::{DevdError, Result};
use crate::util;

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
    }

    pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    pub const STILL_ACTIVE: u32 = 259;
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
    /// When true, the process starts with an empty environment and sees only
    /// `envs`. Used by project manifests so the daemon's own credentials are
    /// not inherited.
    pub clear_env: bool,
    /// File to append stdout and stderr to.
    pub log_path: PathBuf,
    /// How many times this service has already been restarted.
    pub restarts: i64,
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
    /// Returns `(pid, run_id)`.
    pub fn start(&self, spec: SpawnSpec, run_id: String) -> Result<(u32, String)> {
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

        let managed = ManagedProcess {
            name: name.clone(),
            kind: spec.kind,
            child,
            pid,
            started_at: util::now(),
            run_id,
            log_path: spec.log_path,
            restarts: spec.restarts,
        };

        let mut guard = self
            .procs
            .lock()
            .map_err(|_| DevdError::internal("supervisor lock poisoned"))?;
        guard.insert(name, managed);

        Ok((pid, spec_run_id(&guard, &spec.name)))
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

        kill_tree(managed.pid);
        let code = match managed.child.wait() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        Ok(Some((code, managed.run_id)))
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

fn spec_run_id(_guard: &HashMap<String, ManagedProcess>, _name: &str) -> String {
    String::new()
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
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
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
    #[cfg(unix)]
    {
        let path = format!("/proc/{pid}/comm");
        let text = std::fs::read_to_string(path).ok()?;
        let name = text.trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
        None
    }
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
    #[cfg(unix)]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (pid, _run) = sup
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
        };
        let _ = (program, args);
        let (pid1, _) = sup.start(spec(), "r1".into()).unwrap();
        let (pid2, _) = sup.start(spec(), "r2".into()).unwrap();
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
}
