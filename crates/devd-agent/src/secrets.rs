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

/// A key as the vendor issued it, without what tends to arrive around it.
///
/// A key reaches devd by being copied: from a web page, a password manager, a
/// `.env` line, a `curl` example. What comes along is predictable, and each
/// piece turns a good key into a rejected one with nothing on screen to show
/// why:
///
/// * a byte-order mark, when the key file was saved from a Windows editor;
/// * zero-width characters and non-breaking spaces from a web page;
/// * quotes, from `KEY="sk-…"`;
/// * the name and equals sign, from pasting the whole `.env` line;
/// * `Bearer `, from pasting the header instead of the key;
/// * a second line, from a trailing newline or a comment under the key.
pub fn clean_key(raw: &str) -> String {
    let invisible = |c: char| matches!(c, '\u{feff}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{00a0}');
    let line = raw
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || invisible(c)))
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let mut key: String = line.chars().filter(|c| !invisible(*c)).collect();
    // `OPENAI_API_KEY=sk-…` or `export OPENAI_API_KEY=sk-…`.
    if let Some((name, value)) = key.split_once('=') {
        let name = name.trim().trim_start_matches("export ").trim();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            key = value.trim().to_string();
        }
    }
    let unquoted = key.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let key = unquoted.strip_prefix("Bearer ").or_else(|| unquoted.strip_prefix("bearer ")).unwrap_or(unquoted);
    key.trim().to_string()
}

/// Store a key with owner-only permissions.
pub fn save_key(paths: &Paths, id: &str, key: &str) -> Result<()> {
    if !valid_id(id) {
        return Err(DevdError::invalid("invalid provider id"));
    }
    let key = clean_key(key);
    let key = key.as_str();
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

/// The key for a provider, if there is one.
///
/// A key a person saved in devd for this provider comes first; the environment
/// variable the provider names is used when there is none. It used to be the
/// other way round, and the result was the most common "my key is not being
/// used" report: an old `OPENAI_API_KEY` left in the environment silently beat
/// the new key just pasted into the window, and the provider kept answering 401.
/// The key somebody typed for this provider, here, is the more specific
/// instruction. To make the environment decide, remove the saved key.
///
/// Either source is cleaned the same way (see [`clean_key`]).
pub fn read_key(paths: &Paths, p: &Provider) -> Option<String> {
    let saved = std::fs::read_to_string(key_path(paths, &p.id)).ok().map(|s| clean_key(&s)).filter(|s| !s.is_empty());
    saved.or_else(|| env_key(p))
}

/// The key in the environment variable the provider names, when it holds one.
fn env_key(p: &Provider) -> Option<String> {
    let var = p.key_env.as_deref()?;
    std::env::var(var).ok().map(|v| clean_key(&v)).filter(|v| !v.is_empty())
}

/// Where the key comes from, without reading it out. Agrees with [`read_key`].
pub fn key_source(paths: &Paths, p: &Provider) -> KeySource {
    let saved = std::fs::read_to_string(key_path(paths, &p.id)).map(|s| !clean_key(&s).is_empty()).unwrap_or(false);
    if saved {
        KeySource::File
    } else if env_key(p).is_some() {
        KeySource::Env
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

    fn provider(id: &str, key_env: Option<&str>) -> Provider {
        Provider {
            id: id.into(),
            kind: "openai".into(),
            label: id.into(),
            base_url: "https://x/v1".into(),
            key_env: key_env.map(str::to_string),
            default_model: None,
            is_default: false,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn a_key_is_recovered_from_what_it_was_pasted_with() {
        let key = "sk-live-abcdef1234567890";
        for pasted in [
            format!("\u{feff}{key}\r\n"),
            format!("  {key}  "),
            format!("\"{key}\""),
            format!("'{key}'"),
            format!("Bearer {key}"),
            format!("OPENAI_API_KEY={key}"),
            format!("export OPENAI_API_KEY=\"{key}\"\n"),
            format!("# my key\n{key}\nsecond line"),
            format!("{key}\u{200b}"),
            format!("\u{00a0}{key}"),
        ] {
            assert_eq!(clean_key(&pasted), key, "{pasted:?}");
        }
        // A key that happens to contain `=` is not mistaken for a `.env` line.
        assert_eq!(clean_key("abc=def=ghij"), "abc=def=ghij");
        assert_eq!(clean_key("\n\n"), "");
    }

    #[test]
    fn a_saved_key_wins_over_a_stale_environment_variable() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path());
        let var = "DEVD_TEST_STALE_PROVIDER_KEY";
        std::env::set_var(var, "old-key-from-the-environment");
        let p = provider("stale", Some(var));
        // With nothing saved, the environment is used.
        assert_eq!(read_key(&paths, &p).as_deref(), Some("old-key-from-the-environment"));
        assert_eq!(key_source(&paths, &p), KeySource::Env);
        // A key typed for this provider is the one that is sent.
        save_key(&paths, "stale", "the-key-just-pasted").unwrap();
        assert_eq!(read_key(&paths, &p).as_deref(), Some("the-key-just-pasted"));
        assert_eq!(key_source(&paths, &p), KeySource::File);
        // Removing it hands the decision back to the environment.
        delete_key(&paths, "stale");
        assert_eq!(key_source(&paths, &p), KeySource::Env);
        std::env::remove_var(var);
        assert_eq!(key_source(&paths, &p), KeySource::None);
    }

    #[test]
    fn a_key_file_saved_by_a_windows_editor_is_still_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path());
        let path = key_path(&paths, "bom");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "\u{feff}sk-from-notepad-123456\r\n").unwrap();
        assert_eq!(read_key(&paths, &provider("bom", None)).as_deref(), Some("sk-from-notepad-123456"));
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
