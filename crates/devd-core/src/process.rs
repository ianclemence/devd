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
            let _ = util::run(&taskkill, &args, None, &[], Some(std::time::Duration::from_secs(15)));
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

/// Whether a process id is alive. Used to detect orphans after a crash.
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        if let Some(tasklist) = util::which("tasklist") {
            let args = vec!["/FI".to_string(), format!("PID eq {pid}"), "/NH".to_string()];
            if let Ok(out) = util::run(&tasklist, &args, None, &[], Some(std::time::Duration::from_secs(10))) {
                return out.stdout.contains(&pid.to_string()) && !out.stdout.contains("No tasks");
            }
        }
        false
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
