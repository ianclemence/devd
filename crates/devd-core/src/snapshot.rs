//! Undo for the operations that destroy something.
//!
//! An approval says a person agreed. It does not bring anything back. Before a
//! file is overwritten, edited or deleted, or a database is dropped, what is
//! about to be lost is saved here, with a record of what it was, who changed it
//! and which capability did. `restore` puts it back, and saves what it replaced
//! first, so an undo can itself be undone.
//!
//! This is a safety net with edges, and they are stated rather than hidden:
//! a file over [`MAX_FILE_BYTES`], a directory over [`MAX_DIR_FILES`] files or
//! [`MAX_DIR_BYTES`], and anything outside the filesystem (a deployed release,
//! a remote host, a sent message) is not snapshotted, and the result of the
//! operation says that no undo exists. Snapshots are kept for a bounded time
//! (see [`prune`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::capability::Ctx;
use crate::paths::Paths;
use crate::{util, DevdError, Result};

/// Largest single file saved.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Most files saved from one directory.
pub const MAX_DIR_FILES: usize = 5_000;
/// Most bytes saved from one directory.
pub const MAX_DIR_BYTES: u64 = 128 * 1024 * 1024;

/// What a snapshot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// One file.
    File,
    /// A directory, as a zip.
    Directory,
    /// A database dump.
    Database,
}

/// One saved state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Identifier.
    pub id: String,
    /// What it holds.
    pub kind: Kind,
    /// The path, or `engine:database` for a database.
    pub subject: String,
    /// The capability that was about to destroy it.
    pub capability: String,
    /// Who invoked that capability.
    pub actor: String,
    /// The run it belonged to, when it did.
    pub run_id: Option<String>,
    /// When it was taken, RFC 3339.
    pub created_at: String,
    /// Size of the saved payload.
    pub bytes: u64,
    /// Anything the restorer needs (a database's engine and port, for one).
    pub meta: Value,
}

fn dir(paths: &Paths) -> PathBuf {
    paths.snapshots().join("undo")
}

fn meta_path(paths: &Paths, id: &str) -> PathBuf {
    dir(paths).join(format!("{id}.json"))
}

/// Where a snapshot's payload lives.
pub fn payload_path(paths: &Paths, id: &str) -> PathBuf {
    dir(paths).join(format!("{id}.bin"))
}

fn write_meta(paths: &Paths, snap: &Snapshot) -> Result<()> {
    let text =
        serde_json::to_string_pretty(snap).map_err(|e| DevdError::internal(e.to_string()))?;
    util::atomic_write(&meta_path(paths, &snap.id), &text)
}

fn new_snapshot(ctx: &Ctx, kind: Kind, subject: &str, capability: &str, bytes: u64, meta: Value) -> Snapshot {
    Snapshot {
        id: util::new_id(),
        kind,
        subject: subject.to_string(),
        capability: capability.to_string(),
        actor: ctx.actor.label(),
        run_id: ctx.run.as_ref().map(|r| r.run_id.clone()),
        created_at: util::now(),
        bytes,
        meta,
    }
}

/// Save what is at `path` before the operation `capability` changes it.
///
/// Returns `None` when there is nothing to save (the path does not exist yet) or
/// when it is beyond what is saved; the second case is reported through
/// [`Skipped`] by [`before_change`]. The operation must never fail because its
/// safety net could not be made, so callers use [`before_change`].
fn save_path(ctx: &Ctx, path: &Path, capability: &str) -> Result<Option<Snapshot>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    std::fs::create_dir_all(dir(&ctx.paths))?;
    if meta.is_file() {
        if meta.len() > MAX_FILE_BYTES {
            return Err(DevdError::invalid(format!(
                "{} is {} bytes, over the {} that are saved",
                path.display(),
                meta.len(),
                MAX_FILE_BYTES
            )));
        }
        let snap = new_snapshot(ctx, Kind::File, &path.to_string_lossy(), capability, meta.len(), json!({}));
        std::fs::copy(path, payload_path(&ctx.paths, &snap.id))?;
        write_meta(&ctx.paths, &snap)?;
        return Ok(Some(snap));
    }
    if meta.is_dir() {
        let id = util::new_id();
        let payload = payload_path(&ctx.paths, &id);
        let file = std::fs::File::create(&payload)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let (mut files, mut total) = (0usize, 0u64);
        for entry in walkdir::WalkDir::new(path).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let rel = entry.path().strip_prefix(path).unwrap_or(entry.path());
            let name = rel.to_string_lossy().replace('\\', "/");
            if name.is_empty() {
                continue;
            }
            if entry.file_type().is_dir() {
                zip.add_directory(format!("{name}/"), options)
                    .map_err(|e| DevdError::internal(e.to_string()))?;
            } else if entry.file_type().is_file() {
                files += 1;
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                if files > MAX_DIR_FILES || total > MAX_DIR_BYTES {
                    drop(zip);
                    let _ = std::fs::remove_file(&payload);
                    return Err(DevdError::invalid(format!(
                        "{} has more than {} files or {} bytes, beyond what is saved",
                        path.display(),
                        MAX_DIR_FILES,
                        MAX_DIR_BYTES
                    )));
                }
                zip.start_file(&name, options)
                    .map_err(|e| DevdError::internal(e.to_string()))?;
                let mut source = std::fs::File::open(entry.path())?;
                let mut buf = Vec::new();
                source.read_to_end(&mut buf)?;
                zip.write_all(&buf)?;
            }
        }
        zip.finish().map_err(|e| DevdError::internal(e.to_string()))?;
        let bytes = std::fs::metadata(&payload).map(|m| m.len()).unwrap_or(0);
        let mut snap = new_snapshot(ctx, Kind::Directory, &path.to_string_lossy(), capability, bytes, json!({ "files": files }));
        snap.id = id;
        write_meta(&ctx.paths, &snap)?;
        return Ok(Some(snap));
    }
    Ok(None)
}

/// What [`before_change`] did, for the operation's result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Undo {
    /// The snapshot to restore, when one was taken.
    pub snapshot: Option<String>,
    /// Why there is none, when there is none and something existed to save.
    pub note: Option<String>,
}

impl Undo {
    /// Add the undo fields to a capability's result. Nothing is added when there
    /// was nothing to save, so an ordinary result stays as it was.
    pub fn attach(self, mut result: Value) -> Value {
        if let Some(object) = result.as_object_mut() {
            if let Some(id) = self.snapshot {
                object.insert("undo".into(), json!(id));
            }
            if let Some(note) = self.note {
                object.insert("undo_note".into(), json!(note));
            }
        }
        result
    }
}

/// Save `path` before `capability` changes or removes it.
///
/// Never an error: a change that was approved is not blocked by a safety net
/// that could not be made. Instead the result says there is no undo, and why.
pub fn before_change(ctx: &Ctx, path: &Path, capability: &str) -> Undo {
    match save_path(ctx, path, capability) {
        Ok(Some(snap)) => Undo {
            snapshot: Some(snap.id),
            note: None,
        },
        Ok(None) => Undo {
            snapshot: None,
            note: None,
        },
        Err(e) => Undo {
            snapshot: None,
            note: Some(format!("no undo is available: {e}")),
        },
    }
}

/// Record a payload someone else wrote into the snapshot area (a database dump).
pub fn record_payload(
    ctx: &Ctx,
    kind: Kind,
    subject: &str,
    capability: &str,
    id: &str,
    meta: Value,
) -> Result<Snapshot> {
    let payload = payload_path(&ctx.paths, id);
    let bytes = std::fs::metadata(&payload)
        .map_err(|_| DevdError::not_found("the snapshot payload was not written"))?
        .len();
    let mut snap = new_snapshot(ctx, kind, subject, capability, bytes, meta);
    snap.id = id.to_string();
    write_meta(&ctx.paths, &snap)?;
    Ok(snap)
}

/// A fresh identifier and the payload path for a snapshot a caller will write itself.
pub fn allocate(ctx: &Ctx) -> Result<(String, PathBuf)> {
    std::fs::create_dir_all(dir(&ctx.paths))?;
    let id = util::new_id();
    let path = payload_path(&ctx.paths, &id);
    Ok((id, path))
}

/// Snapshots, newest first.
pub fn list(paths: &Paths, limit: usize) -> Vec<Snapshot> {
    let mut all: Vec<Snapshot> = std::fs::read_dir(dir(paths))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str::<Snapshot>(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    all.truncate(limit);
    all
}

/// One snapshot.
pub fn get(paths: &Paths, id: &str) -> Result<Snapshot> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(DevdError::invalid("that is not a snapshot id"));
    }
    let text = std::fs::read_to_string(meta_path(paths, id))
        .map_err(|_| DevdError::not_found(format!("no snapshot `{id}`")))?;
    serde_json::from_str(&text).map_err(|e| DevdError::internal(format!("snapshot `{id}` is unreadable: {e}")))
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Restored {
    /// Where the content went.
    pub path: PathBuf,
    /// The snapshot that holds what the restore replaced, when something was replaced.
    pub replaced: Option<String>,
}

/// Put a file or directory snapshot back where it came from.
///
/// What is there now is saved first, so the restore can be undone. A directory
/// is restored only into a place that does not exist or is empty: merging a
/// saved tree into a live one would invent a state nobody had.
pub fn restore_path(ctx: &Ctx, id: &str) -> Result<Restored> {
    let snap = get(&ctx.paths, id)?;
    let target = PathBuf::from(&snap.subject);
    let payload = payload_path(&ctx.paths, id);
    if !payload.is_file() {
        return Err(DevdError::not_found("the saved content has been removed"));
    }
    match snap.kind {
        Kind::File => {
            let replaced = save_path(ctx, &target, "snapshot.restore").ok().flatten().map(|s| s.id);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = std::fs::read(&payload)?;
            let tmp = target.with_extension("devd-restore");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &target)?;
            Ok(Restored { path: target, replaced })
        }
        Kind::Directory => {
            if target.exists() {
                let empty = target.is_dir()
                    && std::fs::read_dir(&target).map(|mut d| d.next().is_none()).unwrap_or(false);
                if !empty {
                    return Err(DevdError::conflict(format!(
                        "{} exists and is not empty; move it aside before restoring over it",
                        target.display()
                    )));
                }
            }
            std::fs::create_dir_all(&target)?;
            let mut archive = zip::ZipArchive::new(std::fs::File::open(&payload)?)
                .map_err(|e| DevdError::internal(e.to_string()))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| DevdError::internal(e.to_string()))?;
                // `enclosed_name` refuses an entry that climbs out of the target.
                let Some(rel) = entry.enclosed_name() else { continue };
                let out = target.join(rel);
                if entry.is_dir() {
                    std::fs::create_dir_all(&out)?;
                } else {
                    if let Some(parent) = out.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut file = std::fs::File::create(&out)?;
                    std::io::copy(&mut entry, &mut file)?;
                }
            }
            Ok(Restored { path: target, replaced: None })
        }
        Kind::Database => Err(DevdError::invalid(
            "a database snapshot is restored with the database tools, not as files",
        )),
    }
}

/// Remove snapshots older than `max_age`, then the oldest until the total fits.
pub fn prune(paths: &Paths, max_age: std::time::Duration, max_bytes: u64) -> usize {
    let mut all = list(paths, usize::MAX);
    let now = chrono::Utc::now();
    let mut removed = 0;
    let mut drop_one = |s: &Snapshot| {
        let _ = std::fs::remove_file(payload_path(paths, &s.id));
        if std::fs::remove_file(meta_path(paths, &s.id)).is_ok() {
            removed += 1;
        }
    };
    all.retain(|s| {
        let age = chrono::DateTime::parse_from_rfc3339(&s.created_at)
            .map(|t| now.signed_duration_since(t).to_std().unwrap_or_default())
            .unwrap_or_default();
        if age > max_age {
            drop_one(s);
            false
        } else {
            true
        }
    });
    let mut total: u64 = all.iter().map(|s| s.bytes).sum();
    // `list` is newest first, so the oldest are at the end.
    while total > max_bytes {
        let Some(oldest) = all.pop() else { break };
        total = total.saturating_sub(oldest.bytes);
        drop_one(&oldest);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Actor, Surface};
    use crate::config::Settings;
    use crate::events::EventBus;
    use crate::state::Store;
    use std::sync::Arc;

    fn ctx() -> (Ctx, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.migrate().unwrap();
        (
            Ctx {
                store,
                paths: Arc::new(paths),
                settings: Arc::new(Settings::default()),
                events: EventBus::new(),
                actor: Actor::human(Surface::Cli),
                run: None,
                dry_run: false,
            },
            dir,
        )
    }

    #[test]
    fn an_overwritten_file_comes_back_and_the_restore_can_itself_be_undone() {
        let (ctx, dir) = ctx();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "original").unwrap();

        let undo = before_change(&ctx, &file, "fs.write");
        let id = undo.snapshot.clone().expect("a snapshot was taken");
        std::fs::write(&file, "overwritten").unwrap();

        let restored = restore_path(&ctx, &id).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        // What the restore replaced was saved, so "undo the undo" exists.
        let replaced = restored.replaced.expect("the overwritten text was saved");
        restore_path(&ctx, &replaced).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "overwritten");
    }

    #[test]
    fn a_deleted_directory_is_restored_whole_into_an_empty_place_only() {
        let (ctx, dir) = ctx();
        let tree = dir.path().join("site");
        std::fs::create_dir_all(tree.join("css")).unwrap();
        std::fs::write(tree.join("index.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(tree.join("css/app.css"), "body{}").unwrap();

        let id = before_change(&ctx, &tree, "fs.delete").snapshot.unwrap();
        std::fs::remove_dir_all(&tree).unwrap();

        restore_path(&ctx, &id).unwrap();
        assert_eq!(std::fs::read_to_string(tree.join("index.html")).unwrap(), "<h1>hi</h1>");
        assert_eq!(std::fs::read_to_string(tree.join("css/app.css")).unwrap(), "body{}");

        // A second restore into the now-occupied place is refused, not merged.
        let err = restore_path(&ctx, &id).unwrap_err();
        assert_eq!(err.code(), "conflict");
    }

    #[test]
    fn nothing_to_save_is_not_an_error_and_a_too_big_file_is_reported_not_hidden() {
        let (ctx, dir) = ctx();
        let missing = before_change(&ctx, &dir.path().join("new.txt"), "fs.write");
        assert_eq!(missing, Undo { snapshot: None, note: None });

        let big = dir.path().join("big.bin");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        let undo = before_change(&ctx, &big, "fs.delete");
        assert!(undo.snapshot.is_none());
        assert!(undo.note.as_deref().is_some_and(|n| n.contains("no undo")), "{undo:?}");
    }

    #[test]
    fn snapshots_list_newest_first_and_ids_cannot_escape_the_directory() {
        let (ctx, dir) = ctx();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "1").unwrap();
        let first = before_change(&ctx, &file, "fs.write").snapshot.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = before_change(&ctx, &file, "fs.edit").snapshot.unwrap();
        let listed = list(&ctx.paths, 10);
        assert_eq!(listed[0].id, second);
        assert_eq!(listed[1].id, first);
        assert_eq!(listed[0].capability, "fs.edit");
        assert!(get(&ctx.paths, "../../state").is_err());
    }

    #[test]
    fn old_snapshots_are_pruned_by_age_and_then_by_total_size() {
        let (ctx, dir) = ctx();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x".repeat(100)).unwrap();
        let old = before_change(&ctx, &file, "fs.write").snapshot.unwrap();
        // Age it by rewriting its record.
        let mut snap = get(&ctx.paths, &old).unwrap();
        snap.created_at = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        write_meta(&ctx.paths, &snap).unwrap();
        let fresh_a = before_change(&ctx, &file, "fs.write").snapshot.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let fresh_b = before_change(&ctx, &file, "fs.write").snapshot.unwrap();

        let removed = prune(&ctx.paths, std::time::Duration::from_secs(30 * 86_400), u64::MAX);
        assert_eq!(removed, 1);
        assert!(get(&ctx.paths, &old).is_err());

        // Over the size cap the oldest of what is left goes first.
        let removed = prune(&ctx.paths, std::time::Duration::from_secs(365 * 86_400), 100);
        assert_eq!(removed, 1);
        assert!(get(&ctx.paths, &fresh_a).is_err());
        assert!(get(&ctx.paths, &fresh_b).is_ok());
    }
}
