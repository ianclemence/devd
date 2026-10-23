//! The `.env` file: read it with secrets masked, change it without losing comments or order, and
//! compare it with another environment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use devd_core::{util, DevdError, Result};

/// Words in a name that say its value is a secret.
const SECRET: &[&str] = &["PASSWORD", "SECRET", "TOKEN", "KEY", "PASS", "CREDENTIAL", "PRIVATE", "DSN", "SALT"];

/// Whether a variable's value should be hidden.
pub fn is_secret(name: &str) -> bool {
    let up = name.to_ascii_uppercase();
    SECRET.iter().any(|w| up.contains(w))
}

/// The value as shown: hidden for secrets, and only the length is given away.
pub fn masked(name: &str, value: &str) -> String {
    if value.is_empty() || !is_secret(name) {
        value.to_string()
    } else {
        format!("[hidden, {} characters]", value.chars().count())
    }
}

/// One line of the file.
#[derive(Debug, Clone, PartialEq)]
enum Line {
    /// A comment, a blank line or anything that is not an assignment, kept as written.
    Other(String),
    /// `NAME=value`, with the value unquoted.
    Var { name: String, value: String, raw: String },
}

fn parse_value(raw: &str) -> String {
    let v = raw.trim();
    if let Some(inner) = v.strip_prefix('"') {
        if let Some(end) = inner.rfind('"') {
            return inner[..end].replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\");
        }
    }
    if let Some(inner) = v.strip_prefix('\'') {
        if let Some(end) = inner.rfind('\'') {
            return inner[..end].to_string();
        }
    }
    // An unquoted value ends at a ` #` comment.
    match v.find(" #") {
        Some(i) => v[..i].trim_end().to_string(),
        None => v.to_string(),
    }
}

fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(|l| {
            let t = l.trim_start();
            let t = t.strip_prefix("export ").unwrap_or(t);
            if t.starts_with('#') || !t.contains('=') {
                return Line::Other(l.to_string());
            }
            let (name, value) = t.split_once('=').unwrap_or((t, ""));
            let name = name.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
                return Line::Other(l.to_string());
            }
            Line::Var { name: name.to_string(), value: parse_value(value), raw: l.to_string() }
        })
        .collect()
}

/// The variables of an env file's text, in order of appearance (the last assignment of a name wins).
pub fn variables(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for l in parse(text) {
        if let Line::Var { name, value, .. } = l {
            out.insert(name, value);
        }
    }
    out
}

fn quote(value: &str) -> String {
    let plain = !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:@,+=%".contains(c));
    if value.is_empty() || plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
    }
}

/// A valid variable name.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The file's text with these variables set: an existing line is replaced where it is, a new one is appended.
pub fn with_set(text: &str, set: &BTreeMap<String, String>) -> Result<String> {
    for (k, v) in set {
        if !valid_name(k) {
            return Err(DevdError::invalid(format!("`{k}` is not a variable name (letters, digits and underscores, not starting with a digit)")));
        }
        if v.contains('\0') {
            return Err(DevdError::invalid(format!("the value of {k} contains a NUL character")));
        }
    }
    let mut remaining = set.clone();
    let mut out: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for l in parse(text) {
        match l {
            Line::Var { name, raw, .. } => {
                if let Some(v) = remaining.get(&name).cloned() {
                    if seen.contains(&name) {
                        continue; // a repeated assignment of a name being set is dropped
                    }
                    seen.push(name.clone());
                    out.push(format!("{name}={}", quote(&v)));
                } else {
                    out.push(raw);
                }
            }
            Line::Other(s) => out.push(s),
        }
    }
    for n in &seen {
        remaining.remove(n);
    }
    for (k, v) in remaining {
        out.push(format!("{k}={}", quote(&v)));
    }
    let mut s = out.join("\n");
    s.push('\n');
    Ok(s)
}

/// The file's text without these variables.
pub fn without(text: &str, names: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for l in parse(text) {
        match l {
            Line::Var { name, raw, .. } => {
                if !names.contains(&name) {
                    out.push(raw);
                }
            }
            Line::Other(s) => out.push(s),
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// How two environments differ.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Diff {
    /// Set here, missing there.
    pub only_here: Vec<String>,
    /// Set there, missing here.
    pub only_there: Vec<String>,
    /// Set in both with different values. Secrets say only that they differ.
    pub different: Vec<Changed>,
    /// How many variables are the same in both.
    pub same: usize,
    /// Things worth a second look: debug mode on in production, a local URL there, and so on.
    pub warnings: Vec<String>,
}

/// One variable whose value differs.
#[derive(Debug, Clone, Serialize)]
pub struct Changed {
    /// The variable.
    pub name: String,
    /// Here, masked.
    pub here: String,
    /// There, masked.
    pub there: String,
}

/// Compare this environment with another.
pub fn diff(here: &BTreeMap<String, String>, there: &BTreeMap<String, String>) -> Diff {
    let mut d = Diff::default();
    for (k, v) in here {
        match there.get(k) {
            None => d.only_here.push(k.clone()),
            Some(o) if o == v => d.same += 1,
            Some(o) => d.different.push(Changed {
                name: k.clone(),
                here: if is_secret(k) { "[differs]".to_string() } else { v.clone() },
                there: if is_secret(k) { "[differs]".to_string() } else { o.clone() },
            }),
        }
    }
    for k in there.keys() {
        if !here.contains_key(k) {
            d.only_there.push(k.clone());
        }
    }
    let there_env = there.get("APP_ENV").map(String::as_str).unwrap_or("");
    if there.get("APP_DEBUG").is_some_and(|v| v.eq_ignore_ascii_case("true")) && there_env == "production" {
        d.warnings.push("APP_DEBUG is true where APP_ENV is production: error pages will show source and secrets".to_string());
    }
    if there.get("APP_URL").is_some_and(|u| u.contains(".test") || u.contains("localhost") || u.contains("127.0.0.1")) {
        d.warnings.push("APP_URL there points at a local address".to_string());
    }
    if there.get("APP_KEY").is_none_or(String::is_empty) {
        d.warnings.push("APP_KEY is not set there".to_string());
    }
    d
}

/// The `.env` of a project folder.
pub fn path_of(root: &Path) -> PathBuf {
    root.join(".env")
}

/// Write the file, keeping the previous one beside it as `.env.devd-backup-<time>` (the last five are kept).
pub fn write_with_backup(root: &Path, text: &str) -> Result<Option<PathBuf>> {
    let path = path_of(root);
    let mut backup = None;
    if path.is_file() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let b = root.join(format!(".env.devd-backup-{stamp}"));
        std::fs::copy(&path, &b)?;
        backup = Some(b);
        let mut old: Vec<PathBuf> = std::fs::read_dir(root)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".env.devd-backup-")))
            .collect();
        old.sort();
        while old.len() > 5 {
            let _ = std::fs::remove_file(old.remove(0));
        }
    }
    util::atomic_write(&path, text)?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# app\nAPP_NAME=\"My App\"\nAPP_ENV=local\nAPP_KEY=base64:abc=\nDB_PASSWORD='p#ss'\nexport MAIL_HOST=127.0.0.1 # local\n\nEMPTY=\n";

    #[test]
    fn values_are_read_with_quotes_comments_and_export() {
        let v = variables(SAMPLE);
        assert_eq!(v["APP_NAME"], "My App");
        assert_eq!(v["DB_PASSWORD"], "p#ss");
        assert_eq!(v["MAIL_HOST"], "127.0.0.1");
        assert_eq!(v["EMPTY"], "");
        assert_eq!(v.len(), 6);
    }

    #[test]
    fn secrets_are_masked_and_the_rest_shown() {
        assert_eq!(masked("APP_ENV", "local"), "local");
        assert_eq!(masked("DB_PASSWORD", "hunter2"), "[hidden, 7 characters]");
        assert_eq!(masked("STRIPE_SECRET", ""), "");
    }

    #[test]
    fn setting_keeps_comments_order_and_other_lines() {
        let set = BTreeMap::from([("APP_ENV".to_string(), "production".to_string()), ("NEW_ONE".to_string(), "two words".to_string())]);
        let out = with_set(SAMPLE, &set).unwrap();
        assert!(out.starts_with("# app\nAPP_NAME=\"My App\"\nAPP_ENV=production\n"), "{out}");
        assert!(out.contains("export MAIL_HOST=127.0.0.1 # local"), "untouched lines stay as written: {out}");
        assert!(out.ends_with("NEW_ONE=\"two words\"\n"), "{out}");
        assert_eq!(variables(&out)["APP_ENV"], "production");
        assert!(with_set(SAMPLE, &BTreeMap::from([("1BAD".to_string(), "x".to_string())])).is_err());
    }

    #[test]
    fn unsetting_removes_only_the_named_lines() {
        let out = without(SAMPLE, &["APP_KEY".to_string(), "EMPTY".to_string()]);
        assert!(!out.contains("APP_KEY") && !out.contains("EMPTY"));
        assert!(out.contains("# app") && out.contains("APP_ENV=local"));
    }

    #[test]
    fn a_diff_names_what_differs_without_showing_secrets_and_warns_about_production() {
        let here = variables("APP_ENV=local\nAPP_KEY=a\nONLY_HERE=1\nSAME=1\nDB_PASSWORD=x\n");
        let there = variables("APP_ENV=production\nAPP_DEBUG=true\nAPP_URL=http://shop.test\nAPP_KEY=b\nONLY_THERE=1\nSAME=1\nDB_PASSWORD=y\n");
        let d = diff(&here, &there);
        assert_eq!(d.only_here, vec!["ONLY_HERE"]);
        assert!(d.only_there.contains(&"ONLY_THERE".to_string()));
        assert_eq!(d.same, 1);
        let key = d.different.iter().find(|c| c.name == "APP_KEY").unwrap();
        assert_eq!(key.here, "[differs]");
        let env = d.different.iter().find(|c| c.name == "APP_ENV").unwrap();
        assert_eq!((env.here.as_str(), env.there.as_str()), ("local", "production"));
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
    }

    #[test]
    fn writing_keeps_a_backup_and_prunes_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "A=1\n").unwrap();
        let b = write_with_backup(dir.path(), "A=2\n").unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(b).unwrap(), "A=1\n");
        assert_eq!(std::fs::read_to_string(dir.path().join(".env")).unwrap(), "A=2\n");
        for i in 0..8 {
            std::fs::write(dir.path().join(format!(".env.devd-backup-2000010{i}-000000")), "x").unwrap();
        }
        write_with_backup(dir.path(), "A=3\n").unwrap();
        let n = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".env.devd-backup-")).count();
        assert_eq!(n, 5);
    }
}
