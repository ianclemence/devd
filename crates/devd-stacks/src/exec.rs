//! One way to run an external program for the stack engine: resolved
//! executable, argv (never a shell), streamed and redacted output, a log file,
//! a timeout and cancellation that kills the whole process tree.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;

use devd_core::{DevdError, Result};

use crate::tools::which_in;

/// What to run.
pub struct RunSpec {
    /// Program followed by arguments.
    pub argv: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Extra environment, applied over the inherited one.
    pub env: Vec<(String, String)>,
    /// Directories searched before `PATH`, and prepended to the child's `PATH`.
    pub extra_path: Vec<PathBuf>,
    /// Give up after this long.
    pub timeout: Option<Duration>,
}

/// How a run ended.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Exit code, when the process exited by itself.
    pub code: Option<i32>,
    /// Exited with status 0.
    pub ok: bool,
    /// Stopped because the caller cancelled.
    pub cancelled: bool,
    /// Stopped because the timeout elapsed.
    pub timed_out: bool,
    /// Wall-clock time.
    pub duration_ms: u128,
    /// The last lines of output, secrets removed.
    pub tail: Vec<String>,
}

const TAIL_LINES: usize = 200;

/// Environment variables that must never be inherited by project commands.
///
/// Kept for documentation and for callers that want to name the old denylist;
/// the runner clears the environment and applies
/// [`devd_core::process::CHILD_ENV_ALLOWLIST`] instead, which is the stronger
/// rule.
pub const STRIPPED_ENV: &[&str] = &["DEVD_TOKEN", "DEVD_HTTP_TOKEN"];

/// Remove secrets from a line of output before it is stored, logged or shown.
pub fn redact_line(line: &str) -> String {
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let pats = PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)\b([A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_?KEY|PRIVATE_?KEY|CREDENTIAL|PASSPHRASE)[A-Z0-9_]*)\s*[=:]\s*\S+").unwrap(),
                "$1=[redacted]",
            ),
            (Regex::new(r"(?i)(authorization:\s*(?:bearer|basic)\s+)\S+").unwrap(), "$1[redacted]"),
            (Regex::new(r"([a-z][a-z0-9+.-]*://[^/\s:@]+:)[^@\s/]+(@)").unwrap(), "$1[redacted]$2"),
            (Regex::new(r"\b(ghp_|gho_|ghs_|github_pat_|sk-[a-z]*-?|xox[abprs]-|AKIA)[A-Za-z0-9_\-]{12,}").unwrap(), "[redacted-token]"),
        ]
    });
    let mut out = line.to_string();
    for (re, rep) in pats {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

/// Redact an argv for display and history.
pub fn redact_argv(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut hide_next = false;
    for a in argv {
        if hide_next {
            out.push("[redacted]".to_string());
            hide_next = false;
            continue;
        }
        let lower = a.to_ascii_lowercase();
        if lower.starts_with("--")
            && ["password", "token", "secret", "key"]
                .iter()
                .any(|m| lower.contains(m))
            && !a.contains('=')
        {
            hide_next = true;
        }
        out.push(redact_line(a));
    }
    out
}

/// Resolve a program to something `Command` can spawn.
///
/// Returns the executable and any leading arguments. Relative programs such as
/// `./mvnw` and `vendor/bin/pint` are resolved against the working directory,
/// and Windows batch shims go through `cmd.exe` with the argv still separate.
pub fn resolve_program(
    program: &str,
    cwd: &Path,
    extra: &[PathBuf],
) -> Result<(PathBuf, Vec<String>)> {
    let path = if program.contains('/') || program.contains('\\') {
        let p = cwd.join(program.trim_start_matches("./"));
        let found = if p.is_file() {
            Some(p)
        } else if cfg!(windows) {
            ["cmd", "bat", "exe"]
                .iter()
                .map(|e| p.with_extension(e))
                .find(|c| c.is_file())
        } else {
            None
        };
        found.ok_or_else(|| {
            DevdError::not_found(format!("`{program}` does not exist in {}", cwd.display()))
        })?
    } else {
        which_in(program, extra)
            .map(|(p, _)| p)
            .ok_or_else(|| DevdError::not_found(format!("`{program}` was not found on PATH")))?
    };
    let is_batch = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        .unwrap_or(false);
    if is_batch {
        Ok((
            PathBuf::from("cmd.exe"),
            vec!["/C".into(), path.display().to_string()],
        ))
    } else {
        Ok((path, Vec::new()))
    }
}

fn child_path(extra: &[PathBuf]) -> Option<std::ffi::OsString> {
    if extra.is_empty() {
        return None;
    }
    let mut dirs: Vec<PathBuf> = extra.to_vec();
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    std::env::join_paths(dirs).ok()
}

/// Add arguments to a command. When the program is a `.cmd`/`.bat` script run through
/// `cmd.exe`, an argument holding one of cmd's metacharacters is quoted: `composer ... ^4.0`
/// would otherwise reach Composer as `4.0`, an exact version, because `^` is cmd's escape.
fn push_args(cmd: &mut Command, through_cmd: bool, args: &[String]) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        for a in args {
            if through_cmd
                && a.chars().any(|c| "^&|<>()".contains(c))
                && !a.contains(['"', '%', '!'])
            {
                cmd.raw_arg(format!("\"{a}\""));
            } else {
                cmd.arg(a);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = through_cmd;
        cmd.args(args);
    }
}

/// Run to completion, streaming each (redacted) line to `on_line`.
pub fn run_streaming(
    spec: &RunSpec,
    cancel: &AtomicBool,
    log: Option<&Path>,
    on_line: &mut dyn FnMut(&str),
) -> Result<RunResult> {
    let Some(program) = spec.argv.first() else {
        return Err(DevdError::invalid("empty command"));
    };
    let (exe, lead) = resolve_program(program, &spec.cwd, &spec.extra_path)?;
    let mut cmd = Command::new(&exe);
    cmd.args(&lead);
    push_args(&mut cmd, !lead.is_empty(), &spec.argv[1..]);
    cmd.current_dir(&spec.cwd);
    // An allowlist, not a denylist: the daemon's credentials are not in the
    // child unless the caller passed them explicitly in `spec.env`.
    cmd.env_clear();
    devd_core::process::apply_child_environment(&mut cmd);
    cmd.env("CI", "1")
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0");
    if let Some(p) = child_path(&spec.extra_path) {
        cmd.env("PATH", p);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0200);
    }

    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| {
        DevdError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to start `{program}`: {e}"),
        ))
    })?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
                let _ = tx.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(|l| l.ok()) {
                let _ = tx.send(line);
            }
        }));
    }
    drop(tx);

    let mut logfile = match log {
        Some(p) => {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)?,
            )
        }
        None => None,
    };
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut handle = |line: String,
                      tail: &mut std::collections::VecDeque<String>,
                      on_line: &mut dyn FnMut(&str)| {
        let clean = redact_line(&line);
        if let Some(f) = logfile.as_mut() {
            let _ = writeln!(f, "{clean}");
        }
        on_line(&clean);
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(clean);
    };

    let pid = child.id();
    let (mut cancelled, mut timed_out) = (false, false);
    let status = loop {
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(line) => handle(line, &mut tail, on_line),
            Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if cancel.load(Ordering::SeqCst) && !cancelled {
            cancelled = true;
            devd_core::process::kill_tree(pid);
        }
        if let Some(t) = spec.timeout {
            if start.elapsed() > t && !timed_out {
                timed_out = true;
                devd_core::process::kill_tree(pid);
            }
        }
        if let Some(st) = child.try_wait()? {
            break st;
        }
    };
    for r in readers {
        let _ = r.join();
    }
    while let Ok(line) = rx.try_recv() {
        handle(line, &mut tail, on_line);
    }
    let code = if cancelled || timed_out {
        None
    } else {
        status.code()
    };
    Ok(RunResult {
        code,
        ok: !cancelled && !timed_out && status.success(),
        cancelled,
        timed_out,
        duration_ms: start.elapsed().as_millis(),
        tail: tail.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_removed_from_output() {
        assert_eq!(
            redact_line("DB_PASSWORD=hunter2 ok"),
            "DB_PASSWORD=[redacted] ok"
        );
        assert_eq!(
            redact_line("export API_KEY: abc123"),
            "export API_KEY=[redacted]"
        );
        assert_eq!(
            redact_line("postgres://app:s3cret@localhost/db"),
            "postgres://app:[redacted]@localhost/db"
        );
        assert!(
            redact_line("token ghp_abcdefghijklmnopqrstuvwxyz0123").contains("[redacted-token]")
        );
        assert_eq!(redact_line("nothing to hide"), "nothing to hide");
    }

    #[test]
    fn argv_secrets_are_hidden() {
        let a = redact_argv(&[
            "tool".into(),
            "--password".into(),
            "x".into(),
            "--name=y".into(),
        ]);
        assert_eq!(a, vec!["tool", "--password", "[redacted]", "--name=y"]);
    }

    #[cfg(unix)]
    #[test]
    fn streams_output_and_reports_exit_codes() {
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let r = run_streaming(
            &RunSpec {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    "echo one; echo two >&2; exit 3".into(),
                ],
                cwd: std::env::temp_dir(),
                env: vec![],
                extra_path: vec![],
                timeout: None,
            },
            &cancel,
            None,
            &mut |l| seen.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(r.code, Some(3));
        assert!(!r.ok);
        seen.sort();
        assert_eq!(seen, vec!["one", "two"]);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_the_process_tree() {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.store(true, Ordering::SeqCst);
        });
        let r = run_streaming(
            &RunSpec {
                argv: vec!["sh".into(), "-c".into(), "sleep 30 & sleep 30".into()],
                cwd: std::env::temp_dir(),
                env: vec![],
                extra_path: vec![],
                timeout: None,
            },
            &cancel,
            None,
            &mut |_| {},
        )
        .unwrap();
        assert!(r.cancelled && !r.ok);
        assert!(r.duration_ms < 5000);
    }

    #[test]
    fn missing_programs_are_reported_not_spawned() {
        let e =
            resolve_program("definitely-not-a-real-program-xyz", Path::new("."), &[]).unwrap_err();
        assert_eq!(e.code(), "not_found");
    }
}

/// Cancellation flags for runs that are in progress, by run id.
pub mod runs {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    fn table() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
        static T: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
        T.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Register a run and get its cancellation flag.
    pub fn register(id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut t) = table().lock() {
            t.insert(id.to_string(), flag.clone());
        }
        flag
    }

    /// Forget a finished run.
    pub fn finish(id: &str) {
        if let Ok(mut t) = table().lock() {
            t.remove(id);
        }
    }

    /// Ask a run to stop. Returns whether it was in progress.
    pub fn cancel(id: &str) -> bool {
        match table().lock().ok().and_then(|t| t.get(id).cloned()) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// Ids of runs in progress.
    pub fn active() -> Vec<String> {
        table()
            .lock()
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default()
    }
}
