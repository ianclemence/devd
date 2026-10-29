//! What changed in a project while a run was working, whether or not a recorded
//! call said so.
//!
//! Evidence is recorded where a capability returns. A program an agent started,
//! an editor the person had open, a formatter a test run triggered: none of
//! those pass through the registry as a file write, yet they changed the
//! project. A snapshot of the project's tracked and untracked files before the
//! run, compared with one after, makes those changes visible. It is a
//! comparison of what exists, so it cannot say who made a change; the evidence
//! it produces says exactly that.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::{util, DevdError, Result};

/// Most files a snapshot records. A larger tree is truncated and says so.
pub const MAX_FILES: usize = 20_000;

/// Directories never walked when git is not available to say what is ignored.
const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "vendor",
    ".venv",
    "dist",
    "build",
    ".next",
    "__pycache__",
];

/// One file as the snapshot saw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Size in bytes.
    pub len: u64,
    /// Modification time, nanoseconds since the epoch.
    pub mtime: u128,
}

/// The files of a project at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    /// The checked-out commit, when the project is a git repository.
    pub head: Option<String>,
    /// Files by path relative to the project, with `/` separators.
    pub files: BTreeMap<String, Entry>,
    /// Whether the tree had more files than [`MAX_FILES`].
    pub truncated: bool,
}

/// The difference between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Delta {
    /// Files that exist now and did not.
    pub added: Vec<String>,
    /// Files whose size or modification time differs.
    pub modified: Vec<String>,
    /// Files that existed and no longer do.
    pub removed: Vec<String>,
    /// The commit before and after, when they differ.
    pub head_moved: Option<(Option<String>, Option<String>)>,
}

impl Delta {
    /// Whether anything changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.modified.is_empty()
            && self.removed.is_empty()
            && self.head_moved.is_none()
    }

    /// One sentence for the timeline.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("{} added", self.added.len()));
        }
        if !self.modified.is_empty() {
            parts.push(format!("{} modified", self.modified.len()));
        }
        if !self.removed.is_empty() {
            parts.push(format!("{} removed", self.removed.len()));
        }
        if self.head_moved.is_some() {
            parts.push("the checked-out commit moved".to_string());
        }
        format!(
            "observed in the project while the run worked: {}",
            parts.join(", ")
        )
    }
}

/// Record the files of the project at `root`.
///
/// Git decides what counts when the project is a repository: tracked files and
/// untracked files that are not ignored. Elsewhere a bounded walk skips the
/// usual dependency and build directories.
pub fn take(root: &Path) -> Snapshot {
    let head = git(root, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string());
    let listed = git(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
    let mut names: Vec<String> = match listed {
        Some(out) => out
            .split('\0')
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect(),
        None => walk(root),
    };
    names.sort();
    names.dedup();
    let truncated = names.len() > MAX_FILES;
    names.truncate(MAX_FILES);

    let mut files = BTreeMap::new();
    for name in names {
        if let Ok(meta) = std::fs::metadata(root.join(&name)) {
            if meta.is_file() {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                files.insert(
                    name,
                    Entry {
                        len: meta.len(),
                        mtime,
                    },
                );
            }
        }
    }
    Snapshot {
        head,
        files,
        truncated,
    }
}

/// What differs between two snapshots of the same project.
pub fn diff(before: &Snapshot, after: &Snapshot) -> Delta {
    let mut delta = Delta::default();
    for (name, now) in &after.files {
        match before.files.get(name) {
            None => delta.added.push(name.clone()),
            Some(then) if then != now => delta.modified.push(name.clone()),
            _ => {}
        }
    }
    for name in before.files.keys() {
        if !after.files.contains_key(name) {
            delta.removed.push(name.clone());
        }
    }
    if before.head != after.head {
        delta.head_moved = Some((before.head.clone(), after.head.clone()));
    }
    delta
}

/// Where the baseline for a run is kept, so a restart does not lose it.
pub fn baseline_path(root: &Path, run_id: &str) -> PathBuf {
    root.join("artifacts")
        .join(run_id)
        .join("observed-before.json")
}

/// Store a baseline.
pub fn save(path: &Path, snapshot: &Snapshot) -> Result<()> {
    let text =
        serde_json::to_string(snapshot).map_err(|e| DevdError::internal(e.to_string()))?;
    util::atomic_write(path, &text)
}

/// Read a stored baseline.
pub fn load(path: &Path) -> Option<Snapshot> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let git = util::which("git")?;
    let out = util::run(
        &git,
        &args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
        Some(root),
        &[],
        Some(std::time::Duration::from_secs(20)),
    )
    .ok()?;
    (out.code == 0).then_some(out.stdout)
}

fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_dir()
                || !SKIP.contains(&e.file_name().to_string_lossy().as_ref())
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        if out.len() > MAX_FILES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_edited_and_deleted_file_are_each_seen() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.txt"), "same").unwrap();
        std::fs::write(dir.path().join("edit.txt"), "one").unwrap();
        std::fs::write(dir.path().join("gone.txt"), "bye").unwrap();
        let before = take(dir.path());
        assert_eq!(before.files.len(), 3);

        std::fs::write(dir.path().join("edit.txt"), "two, longer").unwrap();
        std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
        let after = take(dir.path());

        let delta = diff(&before, &after);
        assert_eq!(delta.added, vec!["new.txt"]);
        assert_eq!(delta.modified, vec!["edit.txt"]);
        assert_eq!(delta.removed, vec!["gone.txt"]);
        assert!(!delta.is_empty());
        assert!(delta.summary().contains("1 added, 1 modified, 1 removed"));
    }

    #[test]
    fn an_untouched_project_has_an_empty_delta() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let first = take(dir.path());
        let second = take(dir.path());
        assert!(diff(&first, &second).is_empty());
    }

    #[test]
    fn dependency_and_build_directories_are_not_walked_without_git() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules/x")).unwrap();
        std::fs::write(dir.path().join("node_modules/x/i.js"), "x").unwrap();
        std::fs::write(dir.path().join("main.js"), "y").unwrap();
        let snap = take(dir.path());
        assert!(snap.files.contains_key("main.js"));
        assert!(!snap.files.keys().any(|k| k.contains("node_modules")));
    }

    #[test]
    fn a_baseline_survives_being_written_and_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let snap = take(dir.path());
        let file = baseline_path(dir.path(), "run1");
        save(&file, &snap).unwrap();
        assert_eq!(load(&file), Some(snap));
        assert_eq!(load(&dir.path().join("none.json")), None);
    }
}
