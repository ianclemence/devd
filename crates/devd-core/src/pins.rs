//! The versions a project pins for everyone who works on it.
//!
//! A person can choose a PHP version for a site in two different ways, and they mean
//! different things:
//!
//! * **Isolated** is a choice for this machine. devd keeps it in its own state and writes
//!   nothing into the project, so trying a newer PHP leaves the repository clean.
//! * **Pinned** is a choice for the project. It is written into a file that is committed, so
//!   a colleague, a CI job and an agent all start from the same version.
//!
//! The order is isolated, then pinned, then the machine's default. This module is the
//! *pinned* half: reading the files and writing the one devd owns.
//!
//! Files read, in the order they win:
//!
//! | tool | files |
//! | --- | --- |
//! | PHP  | `.devdrc` (`php=8.4`), `.php-version` |
//! | Node | `.devdrc` (`node=22`), `.nvmrc`, `.node-version` |
//!
//! `.devdrc` is `key=value` lines and `#` comments. A pin is looked for in the project folder
//! and upward, like `.nvmrc`, but never past the folder that holds `.git`, so one project's
//! pin cannot leak into a project that happens to be checked out beside it.

use std::path::{Path, PathBuf};

use crate::{DevdError, Result};

/// The file devd writes and owns.
pub const FILE: &str = ".devdrc";

/// One pinned version and where it was read from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Pin {
    /// The version as written (`8.4`, `22`), reduced to what a version manager accepts.
    pub version: String,
    /// The file name it came from, for saying so on screen.
    pub file: String,
}

/// What a project pins.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Pins {
    pub php: Option<Pin>,
    pub node: Option<Pin>,
}

/// A PHP series: the first two parts of a version (`8.4.27` is `8.4`). `None` for anything
/// that is not a version, so a stray word in a file cannot become a site's PHP.
pub fn php_series(raw: &str) -> Option<String> {
    let v = raw.trim().trim_start_matches(['v', 'V']).trim_start_matches("php").trim();
    let mut parts = v.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 2 && s.chars().all(|c| c.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    // Anything after the minor part must itself be numeric (a patch level), not `8.4-rc1`.
    if parts.any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// A Node version as a version manager writes it: `22`, `v22.1.0`, `lts/*`. Kept as written
/// minus the leading `v`, because devd names it and does not interpret it.
fn node_version(raw: &str) -> Option<String> {
    let v = raw.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() || v.len() > 24 || !v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '*' | '-' | '_')) {
        return None;
    }
    Some(v.to_string())
}

/// `key=value` lines of a `.devdrc`, comments and blanks dropped. The last value of a key wins.
fn parse_rc(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or("").trim();
            let (k, v) = line.split_once('=')?;
            let v = v.trim().trim_matches(['"', '\'']).trim();
            (!k.trim().is_empty() && !v.is_empty()).then(|| (k.trim().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

/// The first directory at or above `dir` that holds `name`, stopping after the one that holds
/// `.git` (and after a few levels, for a project that is not a repository).
fn find_up(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut cur = Some(dir);
    for _ in 0..6 {
        let d = cur?;
        let candidate = d.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if d.join(".git").exists() {
            return None;
        }
        cur = d.parent();
    }
    None
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Read what the project in `dir` pins.
pub fn read(dir: &Path) -> Pins {
    let rc = find_up(dir, FILE).and_then(|p| std::fs::read_to_string(&p).ok().map(|t| (p, parse_rc(&t))));
    let rc_value = |key: &str| -> Option<String> {
        rc.as_ref().and_then(|(_, kv)| kv.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
    };

    let php = rc_value("php")
        .and_then(|v| php_series(&v))
        .map(|version| Pin { version, file: FILE.to_string() })
        .or_else(|| {
            let p = find_up(dir, ".php-version")?;
            let version = php_series(std::fs::read_to_string(&p).ok()?.lines().next()?)?;
            Some(Pin { version, file: file_name(&p) })
        });

    let node = rc_value("node")
        .and_then(|v| node_version(&v))
        .map(|version| Pin { version, file: FILE.to_string() })
        .or_else(|| {
            [".nvmrc", ".node-version"].iter().find_map(|name| {
                let p = find_up(dir, name)?;
                let version = node_version(std::fs::read_to_string(&p).ok()?.lines().next()?)?;
                Some(Pin { version, file: file_name(&p) })
            })
        });

    Pins { php, node }
}

/// Set (`Some`) or remove (`None`) one tool's pin in the project's `.devdrc`, leaving every
/// other line and comment as it was. Returns the path written. The file is created in `dir`
/// when none exists above it; a file that exists higher up is the one edited, so a subfolder
/// does not grow a second, disagreeing pin.
pub fn set(dir: &Path, tool: &str, version: Option<&str>) -> Result<PathBuf> {
    let key = match tool {
        "php" | "node" => tool,
        other => return Err(DevdError::invalid(format!("`{other}` cannot be pinned; the tools are php and node"))),
    };
    let value = match (key, version.map(str::trim).filter(|v| !v.is_empty())) {
        (_, None) => None,
        ("php", Some(v)) => Some(php_series(v).ok_or_else(|| DevdError::invalid(format!("`{v}` is not a PHP version; write it like 8.4")))?),
        (_, Some(v)) => Some(node_version(v).ok_or_else(|| DevdError::invalid(format!("`{v}` is not a Node.js version; write it like 22")))?),
    };

    let path = find_up(dir, FILE).unwrap_or_else(|| dir.join(FILE));
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = with_key(&existing, key, value.as_deref());
    if updated.trim().is_empty() {
        // Nothing left worth keeping: a file of nothing is removed rather than committed.
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    } else if updated != existing {
        crate::util::atomic_write(&path, &updated)?;
    }
    Ok(path)
}

/// The text of a `.devdrc` with `key` set to `value`, or removed when `None`.
fn with_key(text: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    for line in text.lines() {
        let body = line.split('#').next().unwrap_or("");
        let is_key = body.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key));
        if !is_key {
            out.push(line.to_string());
        } else if let (Some(v), false) = (value, placed) {
            out.push(format!("{key}={v}"));
            placed = true;
        }
    }
    if let (Some(v), false) = (value, placed) {
        out.push(format!("{key}={v}"));
    }
    if out.is_empty() {
        return String::new();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("devd-pins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_series_is_the_first_two_parts_and_nothing_else_is_a_version() {
        assert_eq!(php_series("8.4").as_deref(), Some("8.4"));
        assert_eq!(php_series("8.4.27").as_deref(), Some("8.4"));
        assert_eq!(php_series("v8.3.1").as_deref(), Some("8.3"));
        assert_eq!(php_series("php8.2").as_deref(), Some("8.2"));
        for bad in ["", "8", "latest", "8.4-rc1", "8.x", "8..4", "123.4"] {
            assert_eq!(php_series(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn devdrc_pins_php_and_node_and_ignores_comments() {
        let d = dir("read");
        std::fs::write(d.join(FILE), "# the team's versions\nphp = 8.4  # production\nnode=\"22\"\n").unwrap();
        let p = read(&d);
        assert_eq!(p.php, Some(Pin { version: "8.4".into(), file: ".devdrc".into() }));
        assert_eq!(p.node, Some(Pin { version: "22".into(), file: ".devdrc".into() }));
    }

    #[test]
    fn devdrc_wins_over_the_other_tools_files_and_they_still_count_alone() {
        let d = dir("order");
        std::fs::write(d.join(".php-version"), "8.2.1\n").unwrap();
        std::fs::write(d.join(".nvmrc"), "v20.11.0\n").unwrap();
        let p = read(&d);
        assert_eq!(p.php.as_ref().map(|x| (x.version.as_str(), x.file.as_str())), Some(("8.2", ".php-version")));
        assert_eq!(p.node.as_ref().map(|x| (x.version.as_str(), x.file.as_str())), Some(("20.11.0", ".nvmrc")));
        std::fs::write(d.join(FILE), "php=8.4\nnode=22\n").unwrap();
        let p = read(&d);
        assert_eq!(p.php.unwrap().file, ".devdrc");
        assert_eq!(p.node.unwrap().version, "22");
    }

    #[test]
    fn a_pin_is_found_in_a_parent_but_never_past_the_repository() {
        let d = dir("up");
        let outer = d.join("outer");
        let repo = outer.join("repo");
        let sub = repo.join("app");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(outer.join(FILE), "php=7.4\n").unwrap();
        assert_eq!(read(&sub).php, None, "a pin outside the repository is another project's");
        std::fs::write(repo.join(FILE), "php=8.3\n").unwrap();
        assert_eq!(read(&sub).php.unwrap().version, "8.3", "the repository's own pin covers its subfolders");
    }

    #[test]
    fn setting_a_pin_keeps_every_other_line_and_removing_the_last_removes_the_file() {
        let d = dir("set");
        std::fs::write(d.join(FILE), "# versions\nnode=22\nphp=8.2 # old\n").unwrap();
        set(&d, "php", Some("8.4.9")).unwrap();
        assert_eq!(std::fs::read_to_string(d.join(FILE)).unwrap(), "# versions\nnode=22\nphp=8.4\n");
        set(&d, "php", None).unwrap();
        assert_eq!(std::fs::read_to_string(d.join(FILE)).unwrap(), "# versions\nnode=22\n");
        set(&d, "node", None).unwrap();
        // A comment is worth keeping, so the file stays until it is empty of everything.
        assert!(d.join(FILE).exists());
        std::fs::write(d.join(FILE), "php=8.4\n").unwrap();
        set(&d, "php", None).unwrap();
        assert!(!d.join(FILE).exists(), "an empty .devdrc is not left behind");
    }

    #[test]
    fn a_pin_that_is_not_a_version_is_refused_and_writes_nothing() {
        let d = dir("bad");
        assert!(set(&d, "php", Some("latest")).is_err());
        assert!(set(&d, "ruby", Some("3.3")).is_err());
        assert!(!d.join(FILE).exists());
    }
}
