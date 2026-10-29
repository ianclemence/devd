//! Housekeeping the resident process does so that nothing grows forever.
//!
//! Two things grow without bound on a machine that is left running: the log
//! files of services and jobs, and the audit table. Both get a policy here, with
//! a setting for each and a default that keeps a useful history.

use std::path::{Path, PathBuf};

use crate::capability::Ctx;
use crate::Result;

/// A log over this size is rotated.
pub const DEFAULT_MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
/// Rotated copies kept beside the live file.
pub const DEFAULT_KEEP: usize = 5;
/// Audit rows older than this many days are removed. `0` keeps everything.
pub const DEFAULT_AUDIT_DAYS: i64 = 365;
/// The download cache is trimmed to this size.
pub const DEFAULT_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// A cached download unused for this many days is removed.
pub const DEFAULT_CACHE_DAYS: u64 = 90;

/// What one rotation did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rotation {
    /// The live log.
    pub file: PathBuf,
    /// Its size before rotation.
    pub bytes: u64,
    /// Whether the live file was emptied. False when another process holds it in
    /// a way that forbids that; the copy then still exists and the next pass
    /// tries again.
    pub truncated: bool,
}

/// Rotate `file` if it is larger than `max_bytes`, keeping `keep` older copies.
///
/// The live file is copied to `<name>.1` and then emptied in place rather than
/// renamed. A service keeps its log open for as long as it runs, and on Windows
/// a file that is open cannot be renamed; copy-then-truncate works for an
/// appending writer on every platform. Lines written between the copy and the
/// truncation are lost, which is the known cost of rotating without the
/// service's cooperation.
pub fn rotate_file(file: &Path, max_bytes: u64, keep: usize) -> Result<Option<Rotation>> {
    let bytes = match std::fs::metadata(file) {
        Ok(meta) if meta.is_file() => meta.len(),
        _ => return Ok(None),
    };
    if bytes <= max_bytes || keep == 0 {
        return Ok(None);
    }
    let numbered = |n: usize| {
        let mut name = file.as_os_str().to_os_string();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    };
    // Oldest first, so nothing is overwritten before it has moved.
    let _ = std::fs::remove_file(numbered(keep));
    for n in (1..keep).rev() {
        let from = numbered(n);
        if from.is_file() {
            std::fs::rename(&from, numbered(n + 1))?;
        }
    }
    std::fs::copy(file, numbered(1))?;
    let truncated = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(file)
        .is_ok();
    Ok(Some(Rotation {
        file: file.to_path_buf(),
        bytes,
        truncated,
    }))
}

/// Rotate every live `.log` file under `dir`, to a bounded depth.
///
/// Rotated copies (`.log.1`, ...) are not themselves rotated.
pub fn rotate_logs(dir: &Path, max_bytes: u64, keep: usize) -> Vec<Rotation> {
    let mut done = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let is_log = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("log"));
        if entry.file_type().is_file() && is_log {
            if let Ok(Some(rotation)) = rotate_file(path, max_bytes, keep) {
                done.push(rotation);
            }
        }
    }
    done
}

/// One pass of housekeeping. Returns a line per thing it did.
pub fn run(ctx: &Ctx) -> Vec<String> {
    let mut did = Vec::new();
    let max = setting_u64(ctx, "logs.max_bytes", DEFAULT_MAX_LOG_BYTES);
    let keep = setting_u64(ctx, "logs.keep", DEFAULT_KEEP as u64) as usize;
    for root in [ctx.paths.logs(), ctx.paths.etc()] {
        for r in rotate_logs(&root, max, keep) {
            did.push(format!(
                "rotated {} ({} bytes{})",
                r.file.display(),
                r.bytes,
                if r.truncated { "" } else { ", still open" }
            ));
        }
    }
    let cache_bytes = setting_u64(ctx, "cache.max_bytes", DEFAULT_CACHE_BYTES);
    let cache_days = setting_u64(ctx, "cache.max_age_days", DEFAULT_CACHE_DAYS);
    if let Ok(gone) = crate::download::evict(
        &ctx.paths.cache(),
        cache_bytes,
        std::time::Duration::from_secs(cache_days * 86_400),
    ) {
        if gone.files > 0 {
            did.push(format!(
                "removed {} cached download(s), {} bytes",
                gone.files, gone.bytes
            ));
        }
    }
    let days = setting_u64(ctx, "audit.retention_days", DEFAULT_AUDIT_DAYS as u64) as i64;
    if days > 0 {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        if let Ok(removed) = ctx.store.prune_audit_before(&cutoff) {
            if removed > 0 {
                did.push(format!(
                    "removed {removed} audit rows older than {days} days"
                ));
            }
        }
    }
    did
}

fn setting_u64(ctx: &Ctx, key: &str, default: u64) -> u64 {
    ctx.store
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_log_is_copied_aside_and_emptied_and_old_copies_shift() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("nginx.log");
        std::fs::write(&log, "a".repeat(100)).unwrap();
        let first = rotate_file(&log, 50, 3).unwrap().unwrap();
        assert!(first.truncated);
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nginx.log.1"))
                .unwrap()
                .len(),
            100
        );

        std::fs::write(&log, "b".repeat(80)).unwrap();
        rotate_file(&log, 50, 3).unwrap().unwrap();
        assert!(std::fs::read_to_string(dir.path().join("nginx.log.1"))
            .unwrap()
            .starts_with('b'));
        assert!(std::fs::read_to_string(dir.path().join("nginx.log.2"))
            .unwrap()
            .starts_with('a'));
    }

    #[test]
    fn only_the_newest_copies_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("x.log");
        for round in 0..5 {
            std::fs::write(&log, format!("{round}").repeat(100)).unwrap();
            rotate_file(&log, 10, 2).unwrap();
        }
        assert!(dir.path().join("x.log.1").is_file());
        assert!(dir.path().join("x.log.2").is_file());
        assert!(!dir.path().join("x.log.3").exists());
    }

    #[test]
    fn a_small_log_and_a_missing_file_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("small.log");
        std::fs::write(&log, "tiny").unwrap();
        assert!(rotate_file(&log, 1024, 3).unwrap().is_none());
        assert!(rotate_file(&dir.path().join("none.log"), 1, 3)
            .unwrap()
            .is_none());
    }

    #[test]
    fn rotation_walks_the_tree_and_skips_rotated_copies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.log"), "x".repeat(40)).unwrap();
        std::fs::write(dir.path().join("a.log.1"), "x".repeat(400)).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x".repeat(400)).unwrap();
        let done = rotate_logs(dir.path(), 10, 2);
        assert_eq!(done.len(), 1);
        assert!(done[0].file.ends_with("a.log"));
    }
}
