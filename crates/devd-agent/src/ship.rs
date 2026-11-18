//! Shipping what a conversation did: what changed in its working tree, and the
//! pull request the branch becomes.
//!
//! The window shows the changes with the project's own Git surface — staging,
//! per-file diffs, commit, pull and push — because that surface already exists and
//! a second one would drift. What is here is the part that surface does not have:
//! a summary a conversation can show beside its title, and the one call that turns
//! a pushed branch into a review.

use std::path::{Path, PathBuf};
use std::time::Duration;

use devd_core::error::{DevdError, Result};
use devd_core::util;

/// How long one git or gh command may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Opening a pull request talks to the host, which can be slower than a local git command.
const HOST_TIMEOUT: Duration = Duration::from_secs(120);

/// What has changed in a working tree, in the shape the window and an agent both read.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// The branch it is on, when it is on one.
    pub branch: Option<String>,
    /// The published branch it tracks, when it tracks one.
    pub upstream: Option<String>,
    /// Commits it has that the upstream does not.
    pub ahead: i64,
    /// Commits the upstream has that it does not.
    pub behind: i64,
    /// One entry per changed path, in git's order.
    pub files: Vec<Change>,
}

/// One changed path, with git's two status letters.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// The path, relative to the working tree.
    pub path: String,
    /// The index letter (`A`, `M`, `D`, `R`, `?` …), or a space for unchanged.
    pub index: char,
    /// The working-tree letter, or a space for unchanged.
    pub work: char,
}

impl Change {
    /// The change in words: what a person reads instead of a letter.
    pub fn words(&self) -> &'static str {
        match (self.index, self.work) {
            ('?', _) => "new file",
            ('U', _) | (_, 'U') => "conflicted",
            ('A', _) => "added",
            ('D', _) | (_, 'D') => "deleted",
            ('R', _) => "renamed",
            ('C', _) => "copied",
            (_, 'M') | ('M', _) => "modified",
            _ => "changed",
        }
    }

    /// Whether the change is staged (in the index) rather than only in the working tree.
    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?'
    }
}

impl Summary {
    /// Whether the working tree has nothing to commit.
    pub fn clean(&self) -> bool {
        self.files.is_empty()
    }

    /// How many paths are staged, how many are not, and how many are new.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut staged = 0;
        let mut unstaged = 0;
        let mut untracked = 0;
        for f in &self.files {
            if f.index == '?' {
                untracked += 1;
            } else if f.staged() {
                staged += 1;
            }
            if f.index != '?' && f.work != ' ' {
                unstaged += 1;
            }
        }
        (staged, unstaged, untracked)
    }

    /// The one line the window shows beside a conversation.
    pub fn line(&self) -> String {
        let (staged, unstaged, untracked) = self.counts();
        let mut said = if self.clean() {
            "No uncommitted changes".to_string()
        } else {
            let n = self.files.len();
            format!("{n} changed file{}", if n == 1 { "" } else { "s" })
        };
        if staged + unstaged + untracked > 0 && !self.clean() {
            said.push_str(&format!(
                " ({staged} staged, {unstaged} not staged, {untracked} new)"
            ));
        }
        if self.ahead > 0 {
            said.push_str(&format!(", {} to push", self.ahead));
        }
        if self.behind > 0 {
            said.push_str(&format!(", {} to pull", self.behind));
        }
        said
    }
}

/// What has changed in `dir`, and where its branch stands against its upstream.
pub fn changes(dir: &Path) -> Result<Summary> {
    let dir = util::strip_verbatim(dir);
    let status = git(&dir, &["-c", "core.quotepath=false", "status", "--porcelain"])?;
    let mut summary = Summary {
        branch: branch_of(&dir),
        ..Default::default()
    };
    for line in status.stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let mut letters = line.chars();
        let index = letters.next().unwrap_or(' ');
        let work = letters.next().unwrap_or(' ');
        let rest = line[2..].trim_start();
        // A rename reads `R  old -> new`; the new path is what a person acts on.
        let path = rest.split_once(" -> ").map(|(_, to)| to).unwrap_or(rest);
        summary.files.push(Change {
            path: path.trim_matches('"').to_string(),
            index,
            work,
        });
    }
    if let Some(up) = upstream_of(&dir) {
        summary.upstream = Some(up.clone());
        if let Ok(out) = git(&dir, &["rev-list", "--left-right", "--count", &format!("{up}...HEAD")]) {
            let mut parts = out.stdout.split_whitespace();
            summary.behind = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            summary.ahead = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    Ok(summary)
}

/// The branch the folder is on, when it is on one.
pub fn branch_of(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()?;
    out.ok()
        .then(|| out.stdout.trim().to_string())
        .filter(|b| !b.is_empty() && b != "HEAD")
}

/// The published branch this one tracks, when it tracks one.
pub fn upstream_of(dir: &Path) -> Option<String> {
    let out = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok()?;
    out.ok()
        .then(|| out.stdout.trim().to_string())
        .filter(|u| !u.is_empty())
}

/// Open the pull request a pushed branch becomes, through the GitHub CLI.
///
/// The branch must already be published: a review of commits nobody else has is not a review. With
/// no title, `gh` fills both title and body from the commits themselves, which is what a person
/// would write anyway and stays true as the branch changes.
pub fn open_pr(dir: &Path, title: Option<&str>, body: Option<&str>) -> Result<String> {
    let dir = util::strip_verbatim(dir);
    if branch_of(&dir).is_none() {
        return Err(DevdError::invalid(
            "this folder is not on a branch, so there is nothing to open a pull request from",
        ));
    }
    if upstream_of(&dir).is_none() {
        return Err(DevdError::invalid(
            "push the branch first: a pull request is a review of commits the host can see",
        ));
    }
    let mut args = vec!["pr".to_string(), "create".to_string()];
    match (title, body) {
        (Some(t), b) => {
            args.push("--title".into());
            args.push(t.to_string());
            args.push("--body".into());
            args.push(b.unwrap_or("").to_string());
        }
        (None, _) => args.push("--fill".into()),
    }
    let out = util::run(
        Path::new("gh"),
        &args,
        Some(&dir),
        &[],
        Some(HOST_TIMEOUT),
    )
    .map_err(|e| {
        DevdError::internal(format!(
            "the GitHub CLI (gh) could not run: {e}. Install it and sign in with `gh auth login`, \
             or open the pull request on the host."
        ))
    })?;
    if !out.ok() {
        return Err(DevdError::conflict(format!(
            "`gh pr create` failed: {}",
            util::truncate(out.stderr.trim(), 600)
        )));
    }
    let url = out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("https://"))
        .last()
        .unwrap_or_default()
        .to_string();
    if url.is_empty() {
        return Err(DevdError::conflict(
            "the pull request was created but the host did not print its address",
        ));
    }
    Ok(url)
}

fn git(dir: &Path, args: &[&str]) -> Result<devd_core::util::CommandOutput> {
    let mut all = vec!["-C".to_string(), dir.display().to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    util::run(Path::new("git"), &all, None, &[], Some(GIT_TIMEOUT))
}

/// The folder itself, with Windows' verbatim prefix removed, for callers that keep it.
pub fn folder(dir: &Path) -> PathBuf {
    util::strip_verbatim(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_reads_like_a_sentence_and_says_what_is_where() {
        let mut s = Summary {
            branch: Some("devd/ab12".into()),
            ahead: 2,
            ..Default::default()
        };
        assert_eq!(s.line(), "No uncommitted changes, 2 to push");
        s.files = vec![
            Change { path: "src/a.rs".into(), index: 'M', work: ' ' },
            Change { path: "src/b.rs".into(), index: ' ', work: 'M' },
            Change { path: "new.rs".into(), index: '?', work: '?' },
        ];
        assert_eq!(
            s.line(),
            "3 changed files (1 staged, 1 not staged, 1 new), 2 to push"
        );
        assert_eq!((s.counts()), (1, 1, 1));
        assert!(!s.clean());
        assert_eq!(s.files[2].words(), "new file");
        assert!(s.files[0].staged() && !s.files[1].staged());
    }

    #[test]
    fn a_conflict_is_named_as_one() {
        let c = Change { path: "a.rs".into(), index: 'U', work: 'U' };
        assert_eq!(c.words(), "conflicted");
    }
}
