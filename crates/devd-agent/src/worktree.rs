//! A conversation's own worktree.
//!
//! Two pieces of work on one checkout tread on each other: a stash, a branch switch, a half-finished
//! edit. A conversation may own a git worktree of its project instead — a real branch, a real
//! directory, the same repository — so what it changes is its own until someone merges it.
//!
//! The project stays the project: it is what the sidebar groups under and what the run records. The
//! worktree is only where this conversation's tools run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use devd_core::error::{DevdError, Result};
use devd_core::util;

/// How long one git command may take before it is killed.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Where a conversation's worktree goes, under its project.
///
/// It sits in the project's own `.devd` folder, which devd already keeps out of git, so a worktree
/// never shows up in the status of the project it belongs to.
pub fn dir_for(project: &Path, short: &str) -> PathBuf {
    project
        .join(".devd")
        .join("worktrees")
        .join(format!("{short}-chat"))
}

/// The branch a conversation's worktree is on.
pub fn branch_for(short: &str) -> String {
    format!("devd/{short}")
}

/// Make the worktree a conversation works in, or find the one it already has.
///
/// A repository with no commit has nothing to branch from, so it is refused with what to do rather
/// than by committing the person's work for them.
pub fn create(project: &Path, short: &str) -> Result<(PathBuf, String)> {
    // Git refuses Windows' verbatim prefix, which is what a resolved folder carries.
    let project = &util::strip_verbatim(project);
    if !project.join(".git").exists() {
        return Err(DevdError::invalid(
            "this project is not a git repository, so there is no worktree to give a conversation",
        ));
    }
    if !has_head(project) {
        return Err(DevdError::invalid(
            "this repository has no commit yet; make one, then start a worktree",
        ));
    }
    let dir = dir_for(project, short);
    let branch = branch_for(short);
    exclude_devd(project);
    if dir.is_dir() {
        return Ok((dir, branch));
    }
    let added = git(project, &["worktree", "add", "-b", &branch, &path_arg(&dir), "HEAD"])?;
    if !added.ok() {
        // The branch may already exist from an earlier conversation or attempt.
        let again = git(project, &["worktree", "add", &path_arg(&dir), &branch])?;
        if !again.ok() {
            return Err(DevdError::conflict(format!(
                "could not make a worktree of this project: {}",
                util::truncate(added.stderr.trim(), 300)
            )));
        }
    }
    Ok((dir, branch))
}

/// The branch a worktree is on, for a folder that already exists.
pub fn branch_of(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    out.ok()
        .then(|| out.stdout.trim().to_string())
        .filter(|b| !b.is_empty() && b != "HEAD")
}

/// Whether a folder is a git checkout devd could give a conversation.
pub fn is_worktree(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn has_head(project: &Path) -> bool {
    git(project, &["rev-parse", "--verify", "HEAD"])
        .map(|o| o.ok())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) -> Result<devd_core::util::CommandOutput> {
    let mut all = vec!["-C".to_string(), path_arg(dir)];
    all.extend(args.iter().map(|a| a.to_string()));
    util::run(Path::new("git"), &all, None, &[], Some(GIT_TIMEOUT))
}

/// A path as one argument, whatever it has in it.
fn path_arg(path: &Path) -> String {
    path.display().to_string()
}

/// Keep `.devd/` out of the project's own status, the way the team's worktrees are kept out.
fn exclude_devd(project: &Path) {
    let ex = project.join(".git").join("info").join("exclude");
    let current = std::fs::read_to_string(&ex).unwrap_or_default();
    if current.lines().any(|l| l.trim() == "/.devd/") {
        return;
    }
    let _ = std::fs::create_dir_all(ex.parent().unwrap_or(project));
    let _ = std::fs::write(
        &ex,
        format!(
            "{current}{}/.devd/\n",
            if current.ends_with('\n') || current.is_empty() {
                ""
            } else {
                "\n"
            }
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worktree_lives_under_the_project_and_is_on_its_own_branch() {
        let project = Path::new("D:\\work\\shop");
        let dir = dir_for(project, "1a2b3c4d");
        assert!(dir.starts_with(project.join(".devd").join("worktrees")));
        assert_eq!(dir.file_name().unwrap(), "1a2b3c4d-chat");
        assert_eq!(branch_for("1a2b3c4d"), "devd/1a2b3c4d");
    }

    #[test]
    fn a_folder_that_is_not_a_repository_is_not_a_worktree() {
        assert!(!is_worktree(Path::new(std::env::temp_dir().to_str().unwrap())));
    }
}
