//! Small shared helpers: name validation, atomic file writes, hashing, and
//! structured external-command execution.
//!
//! **Everything that runs an external program in `devd` goes through
//! [`run`].** It takes an argument array (never a shell string), so no user
//! input, repository content, branch name, or model-generated text can be
//! interpreted as shell syntax.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::{DevdError, Result};

/// Result of running an external program.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    /// Exit code, or `-1` when the process was killed on timeout.
    pub code: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
}

impl CommandOutput {
    /// Whether the process exited successfully.
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// Turn a non-zero exit into an error, keeping stderr for the message.
    pub fn require_ok(self, program: &str) -> Result<Self> {
        if self.ok() {
            Ok(self)
        } else {
            Err(DevdError::Command {
                program: program.to_string(),
                code: self.code,
                stderr: truncate(&self.stderr, 2000),
            })
        }
    }
}

/// Run a program with an argument array and no shell.
///
/// `envs` are applied on top of the inherited environment. A `timeout` of
/// `None` waits indefinitely.
pub fn run(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    envs: &[(String, String)],
    timeout: Option<Duration>,
) -> Result<CommandOutput> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| {
        DevdError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to spawn `{}`: {e}", program.display()),
        ))
    })?;

    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || read_all(&mut out_pipe));
    let err_handle = std::thread::spawn(move || read_all(&mut err_pipe));

    let code = loop {
        match child.try_wait()? {
            Some(status) => break status.code().unwrap_or(-1),
            None => {
                if let Some(limit) = timeout {
                    if start.elapsed() > limit {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    let duration_ms = start.elapsed().as_millis();

    if code == -1 {
        if let Some(limit) = timeout {
            return Err(DevdError::Timeout(format!(
                "`{}` did not finish within {}s",
                program.display(),
                limit.as_secs()
            )));
        }
    }

    Ok(CommandOutput {
        code,
        stdout,
        stderr,
        duration_ms,
    })
}

fn read_all(pipe: &mut Option<impl Read>) -> String {
    let mut buf = Vec::new();
    if let Some(p) = pipe.as_mut() {
        let _ = p.read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Total size of every file under `path`, in bytes.
///
/// Used by plans that say how much disk an operation will free, and by the
/// application's overview panel. A path that does not exist is zero bytes
/// rather than an error: "nothing there" is a legitimate answer.
pub fn directory_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

/// Locate an executable on `PATH` (appending `.exe` on Windows).
pub fn which(program: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|s| s.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };

    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        if cfg!(windows) && !program.contains('.') {
            for ext in &exts {
                let candidate = dir.join(format!("{program}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Validate a filesystem/DNS-safe label: lowercase letters, digits and inner
/// hyphens, 1-63 characters, never starting or ending with a hyphen.
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        return Err(DevdError::invalid(
            "name must be between 1 and 63 characters",
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(DevdError::invalid(
            "name may not start or end with a hyphen",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
    {
        return Err(DevdError::invalid(format!(
            "name `{name}` may only contain lowercase letters, digits, '-', '_' and '.'"
        )));
    }
    if name.contains("..") {
        return Err(DevdError::invalid("name may not contain '..'"));
    }
    Ok(())
}

/// Validate a hostname that will reach a configuration file, a certificate or
/// the hosts file.
///
/// Stricter than `validate_name` in one direction and looser in another: it
/// permits a fully qualified name with several labels, and it refuses anything
/// that is not a plain DNS name. That matters because a hostname is written into
/// nginx configuration, into an X.509 `subjectAltName` and into a system file -
/// a newline or a `;` in a name is a configuration injection, and validation is
/// the place to stop it, before any of those are written.
pub fn validate_hostname(host: &str) -> Result<()> {
    let host = host.trim();
    if host.is_empty() {
        return Err(DevdError::invalid("a hostname is required"));
    }
    if host.len() > 253 {
        return Err(DevdError::invalid(
            "hostname must be 253 characters or fewer",
        ));
    }
    if host.ends_with('.') {
        return Err(DevdError::invalid("hostname may not end with a dot"));
    }
    for label in host.split('.') {
        if label.is_empty() {
            return Err(DevdError::invalid(format!(
                "`{host}` has an empty label; check for a doubled dot"
            )));
        }
        if label.len() > 63 {
            return Err(DevdError::invalid(format!(
                "the label `{label}` in `{host}` is longer than 63 characters"
            )));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(DevdError::invalid(format!(
                "the label `{label}` in `{host}` may not start or end with a hyphen"
            )));
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(DevdError::invalid(format!(
                "`{host}` may only contain lowercase letters, digits, '-' and '.'"
            )));
        }
    }
    Ok(())
}

/// Write a file atomically: write a sibling temp file, then rename over the
/// target. Prevents half-written configuration if the process dies mid-write.
pub fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The temporary name is derived from the whole file name, not from its
    // extension: `Path::with_extension` replaces everything after the last dot,
    // which would turn `blog.test.crt` and `blog.test.key` into the same
    // temporary file.
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("devd-write");
    let tmp = path.with_file_name(format!(".{name}.tmp{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read a file to a string, returning `None` when it does not exist.
pub fn read_to_string_opt(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// SHA-256 of a byte slice, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Truncate a string for display, appending an ellipsis marker.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... [truncated {} bytes]", &s[..end], s.len() - end)
}

/// Current UTC timestamp, RFC 3339.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A new random identifier.
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A short random token, used for loopback API authentication.
pub fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Strip Windows' verbatim prefix from a canonical path.
///
/// `std::fs::canonicalize` returns `\\?\C:\...` on Windows. That form is useful
/// for long-path filesystem calls but hostile to child processes: `cmd.exe`
/// treats it as a UNC path and silently starts in `C:\Windows`, and it is ugly
/// to show a person. The prefix is removed for paths handed to processes and
/// shown in the interface; the lexically equivalent ordinary path resolves to
/// the same directory.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// Replace the user's home directory in a path with `~` for display.
pub fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
        let home_str = home.to_string_lossy().replace('\\', "/");
        if s.replace('\\', "/").starts_with(&home_str) {
            return s.replace('\\', "/").replacen(&home_str, "~", 1);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_rules() {
        assert!(validate_name("my-site").is_ok());
        assert!(validate_name("site2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("-bad").is_err());
        assert!(validate_name("bad-").is_err());
        assert!(validate_name("UPPER").is_err());
        assert!(validate_name("a/../b").is_err());
        assert!(validate_name(&"x".repeat(64)).is_err());
    }

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = std::env::temp_dir().join(format!("devd-util-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        atomic_write(&f, "one").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "one");
        atomic_write(&f, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostname_validation_rules() {
        assert!(validate_hostname("blog.test").is_ok());
        assert!(validate_hostname("a.b.c.test").is_ok());
        assert!(validate_hostname("my-app.test").is_ok());
        assert!(validate_hostname("localhost").is_ok());
        assert!(validate_hostname("").is_err());
        assert!(
            validate_hostname("Blog.test").is_err(),
            "uppercase is not a DNS name"
        );
        assert!(validate_hostname("blog..test").is_err());
        assert!(validate_hostname("-blog.test").is_err());
        assert!(validate_hostname("blog-.test").is_err());
        assert!(validate_hostname("blog.test.").is_err());
        assert!(validate_hostname(&format!("{}.test", "x".repeat(64))).is_err());
        // The reason this function exists: a name is written into a
        // configuration file and a certificate, so nothing that could end a line
        // or start a directive may pass.
        assert!(validate_hostname("blog.test\nserver { listen 1; }").is_err());
        assert!(validate_hostname("blog.test; deny all").is_err());
        assert!(validate_hostname("blog.test<script>").is_err());
        assert!(validate_hostname("blog\ttest").is_err());
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "héllo wörld";
        let t = truncate(s, 4);
        assert!(t.starts_with("hél"));
    }
}
