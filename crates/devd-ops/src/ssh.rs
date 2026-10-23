//! SSH keys in `~/.ssh`: list them, make one, show a public key, take in a private key you already have.
//!
//! A private key is never returned by anything here. `ssh.public` shows the half that is meant to be shared
//! (what you paste into GitHub or Ploi), and a new key has no passphrase unless you add one afterwards.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::{util, DevdError, Result};

fn home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).ok_or_else(|| DevdError::internal("this machine has no home folder"))
}

/// `~/.ssh`.
pub fn dir() -> Result<PathBuf> {
    Ok(home()?.join(".ssh"))
}

/// A key's file name: letters, digits, dots, dashes and underscores, not starting with a dot.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with('.') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !name.ends_with(".pub")
}

/// The `ssh-keygen` program: on PATH, or the one Git for Windows brings.
fn keygen() -> Result<PathBuf> {
    let candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(if cfg!(windows) { "ssh-keygen.exe" } else { "ssh-keygen" })).collect())
        .unwrap_or_default();
    candidates
        .into_iter()
        .chain([PathBuf::from("C:\\Program Files\\Git\\usr\\bin\\ssh-keygen.exe"), PathBuf::from("/usr/bin/ssh-keygen")])
        .find(|p| p.is_file())
        .ok_or_else(|| DevdError::not_found("ssh-keygen was not found; install Git for Windows or OpenSSH"))
}

fn run_keygen(args: &[String]) -> Result<util::CommandOutput> {
    util::run(&keygen()?, args, None, &[], Some(Duration::from_secs(30)))
}

/// The keys that have a public half in `~/.ssh`.
pub fn keys() -> Result<Vec<Value>> {
    let d = dir()?;
    let mut out = Vec::new();
    for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(name) = n.strip_suffix(".pub") else { continue };
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        let mut parts = text.split_whitespace();
        let kind = parts.next().unwrap_or("").to_string();
        let comment = parts.skip(1).collect::<Vec<_>>().join(" ");
        let fingerprint = run_keygen(&["-lf".to_string(), e.path().display().to_string()]).ok().filter(|o| o.code == 0).map(|o| o.stdout.trim().to_string());
        out.push(json!({ "name": name, "type": kind, "comment": comment, "fingerprint": fingerprint, "has_private_key": d.join(name).is_file(), "public_file": e.path().display().to_string() }));
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(out)
}

/// The public key, the half that is meant to be shared.
pub fn public(name: &str) -> Result<String> {
    if !valid_name(name) {
        return Err(DevdError::invalid("that is not a key name"));
    }
    std::fs::read_to_string(dir()?.join(format!("{name}.pub"))).map(|t| t.trim().to_string()).map_err(|_| DevdError::not_found(format!("there is no public key named `{name}` in ~/.ssh")))
}

/// Make an ed25519 key with no passphrase. Refuses to touch a key that exists.
pub fn generate(name: &str, comment: Option<&str>) -> Result<String> {
    if !valid_name(name) {
        return Err(DevdError::invalid("a key name is letters, digits, dots, dashes and underscores, up to 64 characters"));
    }
    let d = dir()?;
    std::fs::create_dir_all(&d)?;
    let file = d.join(name);
    if file.exists() || d.join(format!("{name}.pub")).exists() {
        return Err(DevdError::conflict(format!("~/.ssh already has a key named `{name}`; choose another name")));
    }
    let comment = comment.filter(|c| !c.is_empty() && c.len() <= 100 && !c.contains(['\n', '\r', '"'])).unwrap_or("devd");
    let out = run_keygen(&["-t".into(), "ed25519".into(), "-f".into(), file.display().to_string(), "-N".into(), String::new(), "-C".into(), comment.to_string(), "-q".into()])?;
    if out.code != 0 {
        return Err(DevdError::unsupported(format!("ssh-keygen failed: {}", out.stderr.trim())));
    }
    public(name)
}

/// Take in a private key you already have: copy it into `~/.ssh` under a name, lock its permissions, and write
/// the matching public key when it has no passphrase. The original is left where it is.
pub fn adopt(from: &Path, name: &str) -> Result<Value> {
    if !valid_name(name) {
        return Err(DevdError::invalid("a key name is letters, digits, dots, dashes and underscores, up to 64 characters"));
    }
    let head = std::fs::read_to_string(from).map_err(|e| DevdError::not_found(format!("cannot read {}: {e}", from.display())))?;
    let first = head.lines().next().unwrap_or("");
    if !(first.starts_with("-----BEGIN") && first.contains("PRIVATE KEY")) || head.len() > 20_000 {
        return Err(DevdError::invalid("that file is not an SSH private key"));
    }
    let d = dir()?;
    std::fs::create_dir_all(&d)?;
    let dest = d.join(name);
    if dest.exists() {
        return Err(DevdError::conflict(format!("~/.ssh already has a file named `{name}`; choose another name")));
    }
    std::fs::copy(from, &dest)?;
    lock(&dest);
    let mut public_written = false;
    if let Ok(o) = run_keygen(&["-y".into(), "-f".into(), dest.display().to_string()]) {
        if o.code == 0 && !o.stdout.trim().is_empty() {
            std::fs::write(d.join(format!("{name}.pub")), format!("{}\n", o.stdout.trim()))?;
            public_written = true;
        }
    }
    Ok(json!({ "name": name, "private_key": dest.display().to_string(), "public_key_written": public_written, "note": if public_written { "The original file was not touched." } else { "The key has a passphrase, so its public half could not be derived here; `ssh-keygen -y -f` does it when you give the passphrase." } }))
}

/// Make a private key readable by its owner only (ssh refuses a key others can read).
fn lock(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_default();
        if !user.is_empty() {
            let _ = std::process::Command::new("icacls").arg(path).args(["/inheritance:r", "/grant:r"]).arg(format!("{user}:(R,W)")).output();
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_cannot_climb_out_of_the_ssh_folder() {
        assert!(valid_name("id_ed25519") && valid_name("ploi-prod.key") && valid_name("github"));
        for bad in ["", ".hidden", "../x", "a/b", "a\\b", "x.pub", "with space", &"k".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_file_that_is_not_a_private_key_is_refused_and_nothing_is_copied() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notes.txt");
        std::fs::write(&f, "just some text\n").unwrap();
        assert!(adopt(&f, "taken").unwrap_err().to_string().contains("not an SSH private key"));
        assert!(adopt(&d.path().join("missing"), "taken").is_err());
        assert!(adopt(&f, "../escape").is_err());
    }
}
