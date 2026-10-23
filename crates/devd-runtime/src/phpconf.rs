//! Reading and changing an installed PHP's `php.ini` after it is generated.
//!
//! devd writes `php.ini` once, at install. Everything a person or an agent changes later - the
//! limits profile, a single value, an extension - goes through here, so the file stays one
//! devd can still read: the values live in one clearly marked block at the end (the last
//! value wins in PHP), and extensions are switched by their own `extension=` line.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use devd_core::{util, DevdError, Result};

const BEGIN: &str = "; >>> devd managed values (edit with `devd php configure`)";
const END: &str = "; <<< devd managed values";

/// The limits a profile sets. `local` is generous so a big import or a slow debugger session
/// is not cut off; `production` is what a typical Ploi server runs, so a limit that would bite
/// there bites here first.
pub fn profile_values(profile: &str) -> Result<BTreeMap<String, String>> {
    let pairs: &[(&str, &str)] = match profile {
        "local" => &[
            ("memory_limit", "2G"),
            ("upload_max_filesize", "2G"),
            ("post_max_size", "2G"),
            ("max_execution_time", "36000"),
        ],
        "production" => &[
            ("memory_limit", "128M"),
            ("upload_max_filesize", "5M"),
            ("post_max_size", "8M"),
            ("max_execution_time", "30"),
        ],
        other => return Err(DevdError::invalid(format!("`{other}` is not a profile; use `local` or `production`"))),
    };
    Ok(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

/// The values a caller may set one by one.
pub const SETTABLE: &[&str] = &["memory_limit", "upload_max_filesize", "post_max_size", "max_execution_time", "max_input_time", "date.timezone", "display_errors", "opcache.enable"];

fn split_block(text: &str) -> (String, BTreeMap<String, String>) {
    let mut outside = String::new();
    let mut values = BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == BEGIN {
            inside = true;
        } else if line.trim() == END {
            inside = false;
        } else if inside {
            if let Some((k, v)) = line.split_once('=') {
                values.insert(k.trim().to_string(), v.trim().to_string());
            }
        } else {
            outside.push_str(line);
            outside.push('\n');
        }
    }
    (outside, values)
}

/// The values in the managed block.
pub fn managed_values(ini: &Path) -> Result<BTreeMap<String, String>> {
    Ok(split_block(&std::fs::read_to_string(ini)?).1)
}

/// What PHP will actually use for these settings: the last assignment in the file wins, wherever it is
/// (devd's block, or a line the PHP build or a person put there). A setting that is not assigned is absent.
pub fn effective_values(ini: &Path, keys: &[&str]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for line in std::fs::read_to_string(ini)?.lines() {
        let t = line.trim();
        if t.starts_with(';') || t.starts_with('[') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let k = k.trim();
        if !keys.contains(&k) {
            continue;
        }
        let v = v.split(';').next().unwrap_or("").trim().trim_matches('"').to_string();
        if !v.is_empty() {
            out.insert(k.to_string(), v);
        }
    }
    Ok(out)
}

/// Write these values into the managed block, keeping the ones already there that are not named.
pub fn set_values(ini: &Path, set: &BTreeMap<String, String>) -> Result<()> {
    for k in set.keys() {
        if !SETTABLE.contains(&k.as_str()) {
            return Err(DevdError::invalid(format!("`{k}` cannot be set here; the settable values are {}", SETTABLE.join(", "))));
        }
    }
    let (mut outside, mut values) = split_block(&std::fs::read_to_string(ini)?);
    for (k, v) in set {
        if v.trim().is_empty() || v.contains(['\n', '\r', '"', ';']) {
            return Err(DevdError::invalid(format!("`{v}` is not a usable value for {k}")));
        }
        values.insert(k.clone(), v.trim().to_string());
    }
    while outside.ends_with("\n\n") {
        outside.pop();
    }
    let mut out = outside;
    out.push_str(&format!("\n{BEGIN}\n"));
    for (k, v) in &values {
        out.push_str(&format!("{k} = {v}\n"));
    }
    out.push_str(&format!("{END}\n"));
    util::atomic_write(ini, &out)
}

/// Apply a limits profile to an ini.
pub fn apply_profile(ini: &Path, profile: &str) -> Result<()> {
    set_values(ini, &profile_values(profile)?)
}

/// One extension, as the ini and the build's `ext` folder describe it.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionState {
    /// The extension's name, as `extension=` takes it.
    pub name: String,
    /// Whether the build ships it (a DLL in `ext`) or it is only named in the ini.
    pub shipped: bool,
    /// Whether the ini loads it.
    pub enabled: bool,
}

fn directive(line: &str) -> Option<(String, bool)> {
    let t = line.trim_start();
    let (commented, rest) = match t.strip_prefix(';') {
        Some(r) => (true, r.trim_start()),
        None => (false, t),
    };
    let name = rest.strip_prefix("extension=")?;
    let name = name.split([';', ' ', '\t']).next().unwrap_or("").trim().trim_matches('"');
    let name = name.strip_prefix("php_").unwrap_or(name);
    let name = name.strip_suffix(".dll").unwrap_or(name);
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    Some((name.to_lowercase(), !commented))
}

/// Every extension the build ships or the ini names, and whether it loads.
pub fn list_extensions(install: &Path) -> Result<Vec<ExtensionState>> {
    let mut map: BTreeMap<String, ExtensionState> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(install.join("ext")) {
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_lowercase();
            if let Some(n) = f.strip_prefix("php_").and_then(|r| r.strip_suffix(".dll")) {
                map.insert(n.to_string(), ExtensionState { name: n.to_string(), shipped: true, enabled: false });
            }
        }
    }
    for line in std::fs::read_to_string(install.join("php.ini"))?.lines() {
        if let Some((name, on)) = directive(line) {
            let entry = map.entry(name.clone()).or_insert(ExtensionState { name, shipped: false, enabled: false });
            entry.enabled |= on;
        }
    }
    Ok(map.into_values().collect())
}

/// Switch an extension on or off in `php.ini`.
pub fn set_extension(install: &Path, name: &str, enable: bool) -> Result<()> {
    let name = name.trim().to_lowercase();
    let known = list_extensions(install)?;
    let state = known.iter().find(|e| e.name == name);
    if state.is_none() || !state.is_some_and(|s| s.shipped) && enable {
        return Err(DevdError::not_found(format!(
            "this PHP does not ship an extension named `{name}`; the ones it ships are {}",
            known.iter().filter(|e| e.shipped).map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    let ini = install.join("php.ini");
    let text = std::fs::read_to_string(&ini)?;
    let mut out = String::with_capacity(text.len() + 32);
    let mut touched = false;
    for line in text.lines() {
        match directive(line) {
            Some((n, _)) if n == name => {
                if !touched {
                    out.push_str(&if enable { format!("extension={name}\n") } else { format!(";extension={name}\n") });
                    touched = true;
                }
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if !touched && enable {
        out.push_str(&format!("extension={name}\n"));
    }
    util::atomic_write(&ini, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(tag: &str, ini: &str, dlls: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devd-phpconf-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        for d in dlls {
            std::fs::write(dir.join("ext").join(format!("php_{d}.dll")), "").unwrap();
        }
        std::fs::write(dir.join("php.ini"), ini).unwrap();
        dir
    }

    #[test]
    fn a_profile_is_written_once_and_replaced_not_stacked() {
        let dir = install("profile", "memory_limit = 128M\n", &[]);
        let ini = dir.join("php.ini");
        apply_profile(&ini, "local").unwrap();
        apply_profile(&ini, "production").unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert_eq!(text.matches(BEGIN).count(), 1, "{text}");
        let v = managed_values(&ini).unwrap();
        assert_eq!(v["upload_max_filesize"], "5M");
        assert_eq!(v["max_execution_time"], "30");
        assert!(text.starts_with("memory_limit = 128M"), "what was there before is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_single_value_keeps_the_rest_of_the_block() {
        let dir = install("single", "", &[]);
        let ini = dir.join("php.ini");
        apply_profile(&ini, "production").unwrap();
        set_values(&ini, &BTreeMap::from([("memory_limit".to_string(), "512M".to_string())])).unwrap();
        let v = managed_values(&ini).unwrap();
        assert_eq!(v["memory_limit"], "512M");
        assert_eq!(v["max_execution_time"], "30");
        assert!(set_values(&ini, &BTreeMap::from([("disable_functions".to_string(), "x".to_string())])).is_err());
        assert!(set_values(&ini, &BTreeMap::from([("memory_limit".to_string(), "1G\nevil=1".to_string())])).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_effective_value_is_the_last_assignment_wherever_it_is() {
        let dir = install("effective", "memory_limit = 128M\n; memory_limit = 1M\nupload_max_filesize = 2M ; the default\n[opcache]\nopcache.enable = 1\nmemory_limit = 512M\n", &[]);
        let ini = dir.join("php.ini");
        let v = effective_values(&ini, &["memory_limit", "upload_max_filesize", "post_max_size"]).unwrap();
        assert_eq!(v["memory_limit"], "512M", "the later line wins");
        assert_eq!(v["upload_max_filesize"], "2M", "a trailing comment is not part of the value");
        assert!(!v.contains_key("post_max_size"), "an unassigned setting is absent");
        apply_profile(&ini, "production").unwrap();
        assert_eq!(effective_values(&ini, &["memory_limit"]).unwrap()["memory_limit"], "128M", "devd's block comes last, so it wins");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extensions_are_listed_and_switched() {
        let dir = install("ext", "extension=curl\n;extension=gd\n", &["curl", "gd", "gmp"]);
        let state = |n: &str| list_extensions(&dir).unwrap().into_iter().find(|e| e.name == n).unwrap();
        assert!(state("curl").enabled && !state("gd").enabled && !state("gmp").enabled);
        set_extension(&dir, "gd", true).unwrap();
        set_extension(&dir, "curl", false).unwrap();
        set_extension(&dir, "gmp", true).unwrap();
        assert!(state("gd").enabled && !state("curl").enabled && state("gmp").enabled);
        assert!(set_extension(&dir, "nope", true).is_err());
        let text = std::fs::read_to_string(dir.join("php.ini")).unwrap();
        assert_eq!(text.matches("extension=gd").count(), 1, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
