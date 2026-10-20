//! Where provider API keys live.
//!
//! Keys are never written to the database, the settings file, the audit trail,
//! logs, API responses or the model's context. A provider either names an
//! environment variable (nothing is stored) or has a key file that only the
//! current user can read.

use std::path::PathBuf;

use devd_core::{DevdError, Paths, Provider, Result};

/// Where a key comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// The environment variable the provider names.
    Env,
    /// A key stored by devd.
    File,
    /// No key.
    None,
}

fn key_path(paths: &Paths, id: &str) -> PathBuf {
    paths.root().join("secrets").join("providers").join(format!("{id}.key"))
}

/// Provider ids are file names; keep them boring.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !id.starts_with('-')
}

/// Store a key with owner-only permissions.
pub fn save_key(paths: &Paths, id: &str, key: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(DevdError::invalid("invalid provider id"));
    }
    let key = key.trim();
    if key.len() < 8 || key.len() > 512 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(DevdError::invalid("that does not look like an API key (8-512 characters, no spaces or line breaks)"));
    }
    let path = key_path(paths, id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        restrict(dir, true);
    }
    std::fs::write(&path, key)?;
    restrict(&path, false);
    Ok(())
}

#[cfg(unix)]
fn restrict(p: &std::path::Path, dir: bool) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(if dir { 0o700 } else { 0o600 }));
}

#[cfg(not(unix))]
fn restrict(_p: &std::path::Path, _dir: bool) {
    // The key lives under the user's own profile directory; Windows ACLs there
    // already exclude other users.
}

/// Delete a stored key.
pub fn delete_key(paths: &Paths, id: &str) {
    if valid_id(id) {
        let _ = std::fs::remove_file(key_path(paths, id));
    }
}

/// The key for a provider, if there is one. Environment wins over the file so a
/// deployment can override what a person typed.
pub fn read_key(paths: &Paths, p: &Provider) -> Option<String> {
    if let Some(var) = &p.key_env {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    std::fs::read_to_string(key_path(paths, &p.id)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Where the key comes from, without reading it out.
pub fn key_source(paths: &Paths, p: &Provider) -> KeySource {
    if p.key_env.as_ref().map(|v| std::env::var(v).map(|x| !x.trim().is_empty()).unwrap_or(false)).unwrap_or(false) {
        KeySource::Env
    } else if key_path(paths, &p.id).is_file() {
        KeySource::File
    } else {
        KeySource::None
    }
}

/// A form of a key safe to show: only the last four characters.
pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 12 {
        return "••••".into();
    }
    format!("••••{}", key.chars().skip(n - 4).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stored_privately_and_masked() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        save_key(&paths, "anthropic", "sk-ant-api03-abcdefghijklmnop").unwrap();
        let p = Provider { id: "anthropic".into(), kind: "anthropic".into(), label: "A".into(), base_url: "x".into(), key_env: None, default_model: None, is_default: false, enabled: true, created_at: "".into(), updated_at: "".into() };
        assert_eq!(read_key(&paths, &p).as_deref(), Some("sk-ant-api03-abcdefghijklmnop"));
        assert_eq!(key_source(&paths, &p), KeySource::File);
        assert_eq!(mask("sk-ant-api03-abcdefghijklmnop"), "••••mnop");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_path(&paths, "anthropic")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        delete_key(&paths, "anthropic");
        assert_eq!(key_source(&paths, &p), KeySource::None);
    }

    #[test]
    fn bad_keys_and_ids_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        assert!(save_key(&paths, "../evil", "sk-ant-api03-abcdefghijklmnop").is_err());
        assert!(save_key(&paths, "ok", "short").is_err());
        assert!(save_key(&paths, "ok", "has a space in it 12345").is_err());
        assert!(save_key(&paths, "ok", "line\nbreak-1234567890").is_err());
    }
}
