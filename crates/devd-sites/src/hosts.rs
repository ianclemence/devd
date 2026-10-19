//! The hosts file: making a local hostname resolve.
//!
//! `devd` owns one clearly marked block of the system hosts file and nothing
//! else. Everything outside the markers is preserved byte for byte, including
//! the line endings already in the file, because the hosts file is shared with
//! whatever else the machine uses it for and mangling it would be a serious
//! failure for a development tool.
//!
//! Three decisions are worth stating outright:
//!
//! * **Writing is a separate capability from creating a site.** `site.create` is
//!   a `write`-tier operation; editing the hosts file is `admin`, because it
//!   changes how the whole machine resolves names. Splitting them means creating
//!   a site does not require an elevated approval, and syncing hosts says what
//!   it is doing and needs one.
//! * **The file is snapshotted before it is written.** The previous contents are
//!   copied into the layout's `snapshots` directory and recorded in the state
//!   store, so "put it back" is a real answer rather than an apology.
//! * **The path can be overridden with `DEVD_HOSTS_FILE`.** That is what makes
//!   this testable without an administrator, and it is documented rather than
//!   hidden: pointing devd at a scratch file is how the behaviour is verified.
//!
//! On Windows the real file lives under `System32` and writing it needs
//! elevation. `devd` does not try to elevate itself, does not ask for a password
//! and does not pretend a failed write succeeded: it reports the operating
//! system's own refusal and says what to do instead.

use std::path::{Path, PathBuf};

use devd_core::capability::Ctx;
use devd_core::state::{Snapshot, Site};
use devd_core::util;
use devd_core::{DevdError, Result};

/// Environment variable that redirects the hosts file. Used by the tests and by
/// anyone who wants to see what `devd` would write before it writes it.
pub const ENV_HOSTS_FILE: &str = "DEVD_HOSTS_FILE";

/// First line of the block `devd` owns.
pub const BEGIN: &str = "# >>> devd managed block - generated from `devd site list`, do not edit >>>";

/// Last line of the block `devd` owns.
pub const END: &str = "# <<< devd managed block <<<";

/// One entry to write.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    /// Address the name resolves to.
    pub address: String,
    /// Hostname.
    pub host: String,
}

impl Entry {
    /// The line this entry becomes.
    pub fn line(&self) -> String {
        format!("{}\t{}", self.address, self.host)
    }
}

/// The hosts file this installation manages.
pub fn path() -> PathBuf {
    if let Ok(custom) = std::env::var(ENV_HOSTS_FILE) {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    default_path()
}

fn default_path() -> PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        Path::new(&root).join("System32").join("drivers").join("etc").join("hosts")
    }
    #[cfg(unix)]
    {
        PathBuf::from("/etc/hosts")
    }
    #[cfg(not(any(windows, unix)))]
    {
        PathBuf::from("hosts")
    }
}

/// Every site that should resolve to the loopback address.
///
/// `localhost` and its subdomains are left out: they already resolve, and
/// writing them into the hosts file would be a change the user has to approve
/// for no benefit.
pub fn entries_for(sites: &[Site]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = sites
        .iter()
        .filter(|s| !crate::resolves_without_hosts(&s.host))
        .map(|s| Entry {
            address: "127.0.0.1".to_string(),
            host: s.host.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.host.cmp(&b.host));
    entries.dedup();
    entries
}

/// The entries currently inside the managed block.
///
/// Used by the plan and by the application's Sites panel to compare what is on
/// disk with what the state store says should be there.
pub fn current_entries() -> Result<Vec<Entry>> {
    let file = path();
    let Some(text) = util::read_to_string_opt(&file)? else {
        return Ok(Vec::new());
    };
    Ok(parse_block(&text))
}

/// Parse the managed block out of a hosts file.
pub fn parse_block(text: &str) -> Vec<Entry> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == BEGIN {
            inside = true;
            continue;
        }
        if trimmed == END {
            inside = false;
            continue;
        }
        if !inside {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(address), Some(host)) = (parts.next(), parts.next()) else {
            continue;
        };
        if address.is_empty() || host.is_empty() {
            continue;
        }
        out.push(Entry {
            address: address.to_string(),
            host: host.to_string(),
        });
    }
    out
}

/// Whether `host` is already managed.
pub fn is_managed(text: &str, host: &str) -> bool {
    parse_block(text).iter().any(|e| e.host == host)
}

/// Produce the new contents: everything outside the block, then the block.
///
/// The separator and the trailing newline of the original file are preserved, so
/// a file written by a Windows installer keeps its CRLF endings.
pub fn merge(existing: &str, entries: &[Entry]) -> String {
    let eol = if existing.contains("\r\n") {
        "\r\n"
    } else if existing.is_empty() {
        if cfg!(windows) {
            "\r\n"
        } else {
            "\n"
        }
    } else {
        "\n"
    };

    let mut kept: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in existing.split('\n') {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed == BEGIN {
            inside = true;
            continue;
        }
        if trimmed == END {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        kept.push(line);
    }
    // Drop trailing blank lines so repeated syncs do not grow the file.
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }

    let mut out = kept.join(eol);
    if !out.is_empty() {
        out.push_str(eol);
    }
    if !entries.is_empty() {
        out.push_str(eol);
        out.push_str(BEGIN);
        out.push_str(eol);
        for entry in entries {
            out.push_str(&entry.line());
            out.push_str(eol);
        }
        out.push_str(END);
        out.push_str(eol);
    }
    out
}

/// What a sync would do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncPlan {
    /// The hosts file.
    pub file: String,
    /// Entries that will be present afterwards.
    pub entries: Vec<Entry>,
    /// Entries in the file now that are not in the state store.
    pub removing: Vec<Entry>,
    /// Entries in the state store that are not in the file yet.
    pub adding: Vec<Entry>,
    /// Whether the file would change at all.
    pub changes: bool,
}

/// Compare the file with the state store without writing anything.
pub fn plan(ctx: &Ctx) -> Result<SyncPlan> {
    let sites = ctx.store.list_sites()?;
    let wanted = entries_for(&sites);
    let current = current_entries()?;
    let removing: Vec<Entry> = current
        .iter()
        .filter(|e| !wanted.contains(e))
        .cloned()
        .collect();
    let adding: Vec<Entry> = wanted
        .iter()
        .filter(|e| !current.contains(e))
        .cloned()
        .collect();
    let changes = !(removing.is_empty() && adding.is_empty());
    Ok(SyncPlan {
        file: path().display().to_string(),
        entries: wanted,
        removing,
        adding,
        changes,
    })
}

/// Snapshot the file, then write the managed block.
///
/// The snapshot is written before the new contents, so an interrupted sync
/// always leaves a copy of the previous state that can be restored by hand.
pub fn sync(ctx: &Ctx) -> Result<serde_json::Value> {
    let file = path();
    let sites = ctx.store.list_sites()?;
    let entries = entries_for(&sites);
    let before = util::read_to_string_opt(&file)?.unwrap_or_default();
    let after = merge(&before, &entries);

    let snapshot = if file.exists() && before != after {
        Some(snapshot_file(ctx, &file, &before, &entries)?)
    } else {
        None
    };

    if before != after {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        util::atomic_write(&file, &after).map_err(|e| match e {
            DevdError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
                DevdError::denied(format!(
                    "{} cannot be modified by this user. On Windows the hosts file needs an \
                     elevated shell, and on Linux it needs root; run the same command from an \
                     administrator shell, or set {ENV_HOSTS_FILE} to a file you can write.",
                    file.display()
                ))
            }
            other => other,
        })?;
    }

    ctx.events.emit_with(
        "hosts.synced",
        format!("{} hostname(s) point at 127.0.0.1", entries.len()),
        serde_json::json!({
            "file": file.display().to_string(),
            "entries": entries.len(),
            "changed": before != after,
        }),
    );

    Ok(serde_json::json!({
        "file": file.display().to_string(),
        "entries": entries,
        "changed": before != after,
        "snapshot": snapshot,
    }))
}

fn snapshot_file(ctx: &Ctx, file: &Path, contents: &str, entries: &[Entry]) -> Result<String> {
    let dir = ctx.paths.snapshots();
    std::fs::create_dir_all(&dir)?;
    let id = util::new_id();
    let stem = file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "hosts".to_string());
    let target = dir.join(format!("{stem}-{id}.txt"));
    util::atomic_write(&target, contents)?;
    let bytes = std::fs::metadata(&target).map(|m| m.len() as i64).unwrap_or(0);
    let checksum = util::sha256_file(&target)?;
    ctx.store.create_snapshot(&Snapshot {
        id: id.clone(),
        scope: "hosts".to_string(),
        path: target.display().to_string(),
        bytes,
        checksum,
        created_at: util::now(),
        description: format!(
            "hosts file before syncing {} entr{}",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" }
        ),
    })?;
    Ok(target.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(host: &str) -> Site {
        Site {
            id: util::new_id(),
            name: host.split('.').next().unwrap_or("site").to_string(),
            host: host.to_string(),
            root: format!("C:/www/{host}"),
            web_server: "nginx".to_string(),
            php: None,
            tls_mode: "internal".to_string(),
            db_name: None,
            created_at: util::now(),
        }
    }

    #[test]
    fn a_block_is_appended_to_an_existing_file() {
        let existing = "127.0.0.1 localhost\n::1 localhost\n";
        let out = merge(existing, &entries_for(&[site("blog.test")]));
        assert!(out.starts_with("127.0.0.1 localhost\n::1 localhost\n"), "{out}");
        assert!(out.contains(BEGIN), "{out}");
        assert!(out.contains("127.0.0.1\tblog.test"), "{out}");
        assert!(out.trim_end().ends_with(END), "{out}");
    }

    #[test]
    fn syncing_twice_does_not_grow_the_file() {
        let existing = "127.0.0.1 localhost\n";
        let sites = [site("blog.test")];
        let once = merge(existing, &entries_for(&sites));
        let twice = merge(&once, &entries_for(&sites));
        assert_eq!(once, twice, "a sync must be idempotent");
    }

    #[test]
    fn an_entry_that_is_no_longer_wanted_disappears() {
        let existing = "127.0.0.1 localhost\n";
        let with_site = merge(existing, &entries_for(&[site("blog.test"), site("shop.test")]));
        let without = merge(&with_site, &entries_for(&[site("shop.test")]));
        assert!(!without.contains("blog.test"), "{without}");
        assert!(without.contains("shop.test"), "{without}");
        // The block itself goes away once nothing is managed.
        let cleared = merge(&without, &[]);
        assert!(!cleared.contains(BEGIN), "{cleared}");
        assert!(cleared.contains("127.0.0.1 localhost"), "{cleared}");
    }

    #[test]
    fn nothing_outside_the_markers_is_touched() {
        let existing = "# a comment\n127.0.0.1 localhost\n10.0.0.5 build.internal\n";
        let out = merge(existing, &entries_for(&[site("blog.test")]));
        let cleared = merge(&out, &[]);
        assert_eq!(cleared, existing, "removing the block restores the original");
    }

    #[test]
    fn crlf_endings_are_preserved() {
        let existing = "127.0.0.1 localhost\r\n::1 localhost\r\n";
        let out = merge(existing, &entries_for(&[site("blog.test")]));
        assert!(out.contains("\r\n"), "the file's own line endings must survive");
        assert!(!out.contains("\n\n"), "no mixed endings: {out:?}");
        assert!(parse_block(&out).iter().any(|e| e.host == "blog.test"));
    }

    #[test]
    fn the_block_is_parsed_back_out() {
        let text = merge("127.0.0.1 localhost\n", &entries_for(&[site("a.test"), site("b.test")]));
        let entries = parse_block(&text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].host, "a.test");
        assert_eq!(entries[0].address, "127.0.0.1");
        assert!(is_managed(&text, "b.test"));
        assert!(!is_managed(&text, "c.test"));
    }

    #[test]
    fn entries_are_deduplicated_and_sorted() {
        let entries = entries_for(&[site("b.test"), site("a.test"), site("a.test")]);
        assert_eq!(
            entries.iter().map(|e| e.host.as_str()).collect::<Vec<_>>(),
            vec!["a.test", "b.test"]
        );
    }

    #[test]
    fn the_override_environment_variable_decides_the_path() {
        let previous = std::env::var(ENV_HOSTS_FILE).ok();
        std::env::set_var(ENV_HOSTS_FILE, "D:\\scratch\\hosts-fixture");
        assert_eq!(path(), PathBuf::from("D:\\scratch\\hosts-fixture"));
        std::env::remove_var(ENV_HOSTS_FILE);
        assert!(path().is_absolute());
        match previous {
            Some(value) => std::env::set_var(ENV_HOSTS_FILE, value),
            None => std::env::remove_var(ENV_HOSTS_FILE),
        }
    }
}
