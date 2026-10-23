//! Logs: the Laravel log as entries, and the web server and PHP logs as tails.

use std::path::{Path, PathBuf};

use serde::Serialize;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

/// One entry of the Laravel log.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    /// `2026-03-04 10:15:00`.
    pub at: String,
    /// The environment channel, `local`, `production`, `testing`.
    pub env: String,
    /// `ERROR`, `WARNING`, `INFO`, ...
    pub level: String,
    /// The first line of the message, cut at 600 characters.
    pub message: String,
    /// The first `#0` frame of the stack trace, when there is one.
    pub origin: Option<String>,
    /// How many lines the entry spans.
    pub lines: usize,
}

fn header(line: &str) -> Option<(String, String, String, String)> {
    // [2026-03-04 10:15:00] local.ERROR: message
    let rest = line.strip_prefix('[')?;
    let (at, rest) = rest.split_once(']')?;
    if at.len() < 19 || !at.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let rest = rest.trim_start();
    let (chan, message) = rest.split_once(": ")?;
    let (env, level) = chan.split_once('.')?;
    if env.is_empty() || level.is_empty() || level.chars().any(|c| !c.is_ascii_alphabetic()) {
        return None;
    }
    Some((at[..19].to_string(), env.to_string(), level.to_ascii_uppercase(), message.to_string()))
}

/// Entries from the text of a Laravel log, oldest first.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for line in text.lines() {
        if let Some((at, env, level, message)) = header(line) {
            let mut m = message;
            if m.chars().count() > 600 {
                m = m.chars().take(600).collect::<String>() + "...";
            }
            out.push(Entry { at, env, level, message: m, origin: None, lines: 1 });
        } else if let Some(last) = out.last_mut() {
            last.lines += 1;
            if last.origin.is_none() && line.starts_with("#0 ") {
                last.origin = Some(line[3..].chars().take(300).collect());
            }
        }
    }
    out
}

/// The newest Laravel log file of a project (`laravel.log`, or the newest `laravel-YYYY-MM-DD.log`).
pub fn laravel_file(root: &Path) -> Option<PathBuf> {
    let dir = root.join("storage").join("logs");
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("laravel") && e.file_name().to_string_lossy().ends_with(".log"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort();
    files.pop().map(|(_, p)| p)
}

/// The last `limit` entries of a site's Laravel log, optionally only these levels or containing some text.
pub fn laravel(ctx: &Ctx, site: &str, limit: usize, levels: &[String], search: Option<&str>) -> Result<serde_json::Value> {
    let (_, root) = crate::site_root(ctx, site)?;
    let Some(file) = laravel_file(&root) else {
        return Ok(serde_json::json!({ "file": null, "entries": [], "note": "this project has no Laravel log yet (storage/logs/laravel.log)" }));
    };
    // The last megabyte holds far more than a screenful; reading all of a large log helps nobody.
    let text = crate::tail_bytes(&file, 1_000_000);
    let wanted: Vec<String> = levels.iter().map(|l| l.to_ascii_uppercase()).collect();
    let needle = search.map(str::to_ascii_lowercase);
    let mut entries: Vec<Entry> = parse(&text)
        .into_iter()
        .filter(|e| wanted.is_empty() || wanted.contains(&e.level))
        .filter(|e| needle.as_ref().is_none_or(|n| e.message.to_ascii_lowercase().contains(n)))
        .collect();
    let total = entries.len();
    if entries.len() > limit {
        entries.drain(0..entries.len() - limit);
    }
    entries.reverse();
    Ok(serde_json::json!({
        "file": file.display().to_string(),
        "size_bytes": std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0),
        "matching": total,
        "entries": entries,
    }))
}

/// Empty the Laravel log files of a project, keeping a copy of the newest as `.bak`.
pub fn clear_laravel(ctx: &Ctx, site: &str) -> Result<usize> {
    let (_, root) = crate::site_root(ctx, site)?;
    let dir = root.join("storage").join("logs");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).map_err(|e| DevdError::not_found(format!("no log folder at {}: {e}", dir.display())))?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("laravel") && name.ends_with(".log") {
            let _ = std::fs::copy(e.path(), e.path().with_extension("log.bak"));
            std::fs::write(e.path(), "")?;
            n += 1;
        }
    }
    Ok(n)
}

/// A kind of site log.
pub fn site_log_path(ctx: &Ctx, site: &str, kind: &str) -> Result<PathBuf> {
    let s = crate::site_named(ctx, site)?;
    let logs = ctx.paths.logs();
    Ok(match kind {
        "nginx_access" => logs.join(format!("nginx-{}.access.log", s.host)),
        "nginx_error" => logs.join(format!("nginx-{}.error.log", s.host)),
        "php" => {
            let series = s.php.clone().or_else(|| devd_core::config::Settings::load(&ctx.paths.settings_file()).ok().and_then(|x| x.primary_php)).unwrap_or_default();
            let version = ctx
                .store
                .list_runtimes()?
                .into_iter()
                .filter(|r| r.kind == "php" && devd_services::php_series(&r.version) == devd_services::php_series(&series))
                .map(|r| r.version)
                .max()
                .ok_or_else(|| DevdError::not_found("no PHP is installed for this site"))?;
            logs.join(format!("php-{version}.log"))
        }
        other => return Err(DevdError::invalid(format!("`{other}` is not a log kind; use nginx_access, nginx_error or php"))),
    })
}

/// The last `lines` lines of a site's web server or PHP log.
pub fn site_log(ctx: &Ctx, site: &str, kind: &str, lines: usize) -> Result<serde_json::Value> {
    let path = site_log_path(ctx, site, kind)?;
    let text = crate::tail_bytes(&path, 400_000);
    let mut all: Vec<&str> = text.lines().collect();
    let lines = lines.clamp(1, 1000);
    if all.len() > lines {
        all.drain(0..all.len() - lines);
    }
    Ok(serde_json::json!({ "file": path.display().to_string(), "exists": path.is_file(), "lines": all }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "[2026-03-04 10:15:00] local.ERROR: Undefined variable $x (View: a.blade.php) {\"exception\":\"[object]\"}\n#0 /app/vendor/x.php(10): foo()\n#1 {main}\n[2026-03-04 10:16:00] local.INFO: Hello\n[2026-03-04 10:17:00] production.warning: Careful\n";

    #[test]
    fn entries_are_read_with_their_stack() {
        let e = parse(LOG);
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].level.as_str(), e[0].env.as_str(), e[0].lines), ("ERROR", "local", 3));
        assert_eq!(e[0].origin.as_deref(), Some("/app/vendor/x.php(10): foo()"));
        assert_eq!(e[1].message, "Hello");
        assert_eq!(e[2].level, "WARNING");
    }

    #[test]
    fn lines_before_the_first_entry_are_ignored() {
        assert!(parse("garbage\nmore\n").is_empty());
        assert_eq!(parse("junk\n[2026-03-04 10:15:00] local.INFO: ok\n").len(), 1);
    }

    #[test]
    fn the_newest_log_file_is_chosen() {
        let d = tempfile::tempdir().unwrap();
        let logs = d.path().join("storage").join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("laravel-2026-03-01.log"), "old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(logs.join("laravel-2026-03-02.log"), "new").unwrap();
        std::fs::write(logs.join("other.log"), "x").unwrap();
        assert!(laravel_file(d.path()).unwrap().ends_with("laravel-2026-03-02.log"));
    }
}
