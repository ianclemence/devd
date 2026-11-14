//! What a step changed, and a way back.
//!
//! Before a step that can change files, the project's files are recorded; after it, again. Both records are
//! git trees in a history of devd's own (`snapshots/steps/<project>`), never in the project's repository, so
//! nothing an agent does here shows in the person's git. The difference is the list of files the step
//! changed, and undoing the step puts exactly those files back as they were: a file it changed is restored,
//! a file it made is removed, a file it deleted comes back. Files the step did not touch are left alone, so
//! undoing an agent's step never undoes the person's own work.

use std::path::{Path, PathBuf};
use std::process::Command;

use devd_core::{DevdError, Result};

/// One file a step changed: `A` added, `M` modified, `D` deleted, and its path in the project.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Change {
    /// `A`, `M` or `D`.
    pub status: String,
    /// The path within the project, with forward slashes.
    pub path: String,
}

/// The history devd keeps for one project folder.
fn history(snapshots: &Path, project: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    project.to_string_lossy().to_lowercase().replace('\\', "/").hash(&mut h);
    snapshots.join("steps").join(format!("{:016x}", h.finish()))
}

fn git(dir: &Path, project: &Path, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg(format!("--git-dir={}", dir.display()))
        .arg(format!("--work-tree={}", project.display()))
        .args(["-c", "core.autocrlf=false", "-c", "core.safecrlf=false", "-c", "core.longpaths=true"])
        .args(args)
        .current_dir(project);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| DevdError::internal(format!("git could not be started: {e}")))?;
    if !out.status.success() {
        return Err(DevdError::internal(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Record the project's files now. Returns the record's id, or None when the folder cannot be recorded (no
/// git on this machine, or not a folder). A record is never a reason a step fails.
pub fn take(snapshots: &Path, project: &Path) -> Option<String> {
    if !project.is_dir() {
        return None;
    }
    let dir = history(snapshots, project);
    if !dir.join("HEAD").is_file() {
        std::fs::create_dir_all(&dir).ok()?;
        let out = Command::new("git").args(["init", "--bare", "--quiet"]).arg(&dir).output().ok()?;
        if !out.status.success() {
            return None;
        }
    }
    // Ignored files (dependencies, builds) are what the project's own .gitignore says they are.
    git(&dir, project, &["add", "--all", "--force", "--ignore-errors", "--", ".", ":!*.devd/worktrees/*"]).ok()?;
    git(&dir, project, &["write-tree"]).ok()
}

/// The files that differ between two records.
pub fn changes(snapshots: &Path, project: &Path, before: &str, after: &str) -> Result<Vec<Change>> {
    if before == after {
        return Ok(Vec::new());
    }
    let dir = history(snapshots, project);
    let listed = git(&dir, project, &["diff-tree", "-r", "--no-renames", "--name-status", before, after])?;
    Ok(listed
        .lines()
        .filter_map(|line| {
            let (status, path) = line.split_once('\t')?;
            Some(Change { status: status.chars().next()?.to_string(), path: path.to_string() })
        })
        .collect())
}

/// Put the files a step changed back as they were before it. Returns what was put back.
pub fn undo(snapshots: &Path, project: &Path, before: &str, after: &str) -> Result<Vec<Change>> {
    let changed = changes(snapshots, project, before, after)?;
    let dir = history(snapshots, project);
    for c in &changed {
        let file = project.join(&c.path);
        match c.status.as_str() {
            // Made by the step: it goes.
            "A" => {
                if file.is_file() {
                    std::fs::remove_file(&file).map_err(|e| DevdError::internal(format!("could not remove {}: {e}", c.path)))?;
                }
            }
            // Changed or deleted by the step: its earlier content comes back.
            _ => {
                let content = Command::new("git")
                    .arg(format!("--git-dir={}", dir.display()))
                    .args(["cat-file", "blob"])
                    .arg(format!("{before}:{}", c.path))
                    .output()
                    .map_err(|e| DevdError::internal(format!("git could not be started: {e}")))?;
                if !content.status.success() {
                    return Err(DevdError::internal(format!("the earlier {} could not be read", c.path)));
                }
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&file, content.stdout).map_err(|e| DevdError::internal(format!("could not restore {}: {e}", c.path)))?;
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_present() -> bool {
        Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn a_steps_changes_are_listed_and_undone_and_nothing_else_is_touched() {
        if !git_present() {
            eprintln!("skipped: no git on this machine");
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        std::fs::write(p.join("kept.txt"), "the person's file\n").unwrap();
        std::fs::write(p.join("changed.txt"), "before\n").unwrap();
        std::fs::write(p.join("deleted.txt"), "will be deleted\n").unwrap();
        let before = take(home.path(), p).expect("recorded");

        // The step.
        std::fs::write(p.join("changed.txt"), "after\n").unwrap();
        std::fs::remove_file(p.join("deleted.txt")).unwrap();
        std::fs::create_dir_all(p.join("new")).unwrap();
        std::fs::write(p.join("new").join("made.txt"), "made by the step\n").unwrap();
        let after = take(home.path(), p).expect("recorded");

        let mut changed = changes(home.path(), p, &before, &after).unwrap();
        changed.sort_by(|a, b| a.path.cmp(&b.path));
        let listed: Vec<(&str, &str)> = changed.iter().map(|c| (c.status.as_str(), c.path.as_str())).collect();
        assert_eq!(listed, [("M", "changed.txt"), ("D", "deleted.txt"), ("A", "new/made.txt")]);

        // The person edits a file the step did not touch; undo must leave it.
        std::fs::write(p.join("kept.txt"), "the person's newer words\n").unwrap();
        undo(home.path(), p, &before, &after).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("changed.txt")).unwrap(), "before\n");
        assert_eq!(std::fs::read_to_string(p.join("deleted.txt")).unwrap(), "will be deleted\n");
        assert!(!p.join("new").join("made.txt").exists());
        assert_eq!(std::fs::read_to_string(p.join("kept.txt")).unwrap(), "the person's newer words\n");
        assert!(!p.join(".git").exists(), "the project's own git is never touched");
    }
}
