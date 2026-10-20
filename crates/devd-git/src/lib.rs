//! Typed Git service.
//!
//! # Why the `git` executable and not a library
//!
//! `devd` shells out to the user's own Git. That is a deliberate choice:
//!
//! * The operations the product promises (rebase, bisect, worktrees, submodules,
//!   LFS, credential helpers, hooks) are only fully faithful when Git itself
//!   performs them. A partial reimplementation would silently diverge from the
//!   user's expectations.
//! * The user's existing configuration - aliases, hooks, credential manager,
//!   SSH agent, `.gitattributes`, line-ending handling - keeps working.
//!
//! The cost of that choice is that argument construction must be disciplined:
//! **every invocation passes an argument array, never a shell string**, and every
//! caller-supplied name is validated as a Git ref before it reaches the command
//! line. Nothing in this module can be reached by a general-purpose
//! "run a command" capability.

pub mod caps;

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use devd_core::error::{DevdError, Result};
use devd_core::util::{self, CommandOutput};

/// A file's change state in the working tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChange {
    /// Two-character porcelain status (`M`, `A`, `D`, `R`, `C`, `?`, `U`...).
    pub status: String,
    /// Repository-relative path.
    pub path: String,
    /// Previous path for renames and copies.
    pub original_path: Option<String>,
}

/// Working tree summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    /// Current branch, or `None` when detached.
    pub branch: Option<String>,
    /// Upstream tracking reference, when configured.
    pub upstream: Option<String>,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
    /// Whether the upstream branch has been deleted on the remote.
    pub upstream_gone: bool,
    /// Staged changes.
    pub staged: Vec<FileChange>,
    /// Unstaged changes to tracked files.
    pub unstaged: Vec<FileChange>,
    /// Untracked files.
    pub untracked: Vec<String>,
    /// Files with merge conflicts.
    pub conflicted: Vec<String>,
    /// Whether there is nothing to report.
    pub clean: bool,
    /// In-progress operation, e.g. `merge`, `rebase`, `cherry-pick`, `bisect`.
    pub in_progress: Option<String>,
    /// True when nothing has been committed yet.
    pub unborn: bool,
}

impl RepoStatus {
    /// Every path that differs from `HEAD`.
    pub fn changed_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self.staged.iter().chain(self.unstaged.iter()) {
            if !out.contains(&c.path) {
                out.push(c.path.clone());
            }
        }
        for p in &self.untracked {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }
}

/// One commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// Full hash.
    pub hash: String,
    /// Abbreviated hash.
    pub short: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author timestamp, Unix seconds.
    pub timestamp: i64,
    /// Subject line.
    pub subject: String,
}

/// One branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Short name, e.g. `main` or `origin/main`.
    pub name: String,
    /// Whether this is the current branch.
    pub current: bool,
    /// Upstream tracking reference.
    pub upstream: Option<String>,
    /// Tip commit subject.
    pub subject: String,
}

/// One remote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInfo {
    /// Remote name.
    pub name: String,
    /// Whether the name is a Git command option (rejected by callers).
    pub fetch_url: String,
    /// Push URL, when it differs.
    pub push_url: Option<String>,
}

/// One tag.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    /// Tag name.
    pub name: String,
    /// Tagged commit hash.
    pub target: String,
    /// Annotation message, when annotated.
    pub message: Option<String>,
}

/// One stash entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashInfo {
    /// Stash reference, e.g. `stash@{0}`.
    pub reference: String,
    /// Stash subject.
    pub subject: String,
}

/// One linked worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Absolute path.
    pub path: String,
    /// Checked-out commit.
    pub head: String,
    /// Branch, when the worktree is on one.
    pub branch: Option<String>,
    /// Whether this is the main worktree.
    pub main: bool,
    /// Whether the worktree is locked.
    pub locked: bool,
}

/// Dry-run result for a fetch or pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPreview {
    /// Commits that would be brought in.
    pub incoming: Vec<CommitInfo>,
    /// Commits that would be published.
    pub outgoing: Vec<CommitInfo>,
    /// Whether local and remote have diverged and need a merge or rebase.
    pub diverged: bool,
}

/// Options for producing a diff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffOptions {
    /// Diff the index against `HEAD` rather than the working tree.
    pub staged: bool,
    /// Limit to a path.
    pub path: Option<String>,
    /// Compare against this revision instead of the index/working tree.
    pub rev: Option<String>,
    /// Number of context lines.
    pub context: Option<u32>,
    /// Produce a `--stat` summary instead of a patch.
    pub stat: bool,
}

/// Which reset mode to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ResetMode {
    /// Move `HEAD` only; keep the index and working tree.
    Soft,
    /// Move `HEAD` and reset the index; keep the working tree.
    Mixed,
    /// Move `HEAD`, reset the index and discard working tree changes.
    Hard,
}

impl ResetMode {
    /// The `--soft`/`--mixed`/`--hard` flag.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
}

/// Where a local branch stands relative to its upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Divergence {
    /// Nothing to do.
    UpToDate,
    /// Local is ahead only.
    Ahead,
    /// Local is behind only, and can fast-forward.
    Behind,
    /// Both sides have commits.
    Diverged,
    /// No upstream is configured.
    NoUpstream,
}

/// The Git executable wrapper.
pub struct GitService {
    git: PathBuf,
    version: String,
}

impl GitService {
    /// Locate `git`.
    ///
    /// `explicit` overrides `PATH` discovery, which is how the user pins a
    /// specific installation.
    pub fn discover(explicit: Option<&str>) -> Result<Self> {
        let git = match explicit {
            Some(p) if !p.trim().is_empty() => {
                let path = PathBuf::from(p);
                if !path.is_file() {
                    return Err(DevdError::not_found(format!(
                        "configured git executable does not exist: {}",
                        path.display()
                    )));
                }
                path
            }
            _ => util::which("git").ok_or_else(|| {
                DevdError::not_found(
                    "`git` was not found on PATH. Install Git, or set git_executable in settings.",
                )
            })?,
        };

        let out = util::run(
            &git,
            &["--version".to_string()],
            None,
            &env(),
            Some(std::time::Duration::from_secs(15)),
        )?;
        if !out.ok() {
            return Err(DevdError::invalid(format!(
                "`{}` is not a working git executable",
                git.display()
            )));
        }
        let version = out
            .stdout
            .trim()
            .strip_prefix("git version ")
            .unwrap_or(out.stdout.trim())
            .to_string();

        Ok(Self { git, version })
    }

    /// Path to the executable in use.
    pub fn git_path(&self) -> &Path {
        &self.git
    }

    /// Reported Git version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Run a Git command in `cwd`, inheriting the hardened environment.
    pub fn exec(&self, cwd: Option<&Path>, args: &[String]) -> Result<CommandOutput> {
        util::run(
            &self.git,
            args,
            cwd,
            &env(),
            Some(std::time::Duration::from_secs(600)),
        )
    }

    /// Run and require success, returning stdout.
    pub fn exec_out(&self, cwd: &Path, args: &[String]) -> Result<String> {
        let out = self.exec(Some(cwd), args)?;
        if !out.ok() {
            return Err(command_failed(args, &out));
        }
        Ok(out.stdout)
    }

    // ------------------------------------------------------------- discovery

    /// Absolute path to the repository containing `path`, if any.
    pub fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        if !path.exists() {
            return Ok(None);
        }
        let out = self.exec(Some(path), &["rev-parse".into(), "--show-toplevel".into()])?;
        if !out.ok() {
            return Ok(None);
        }
        let root = out.stdout.trim();
        if root.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(root)))
        }
    }

    /// Whether `path` is inside a work tree.
    pub fn is_repo(&self, path: &Path) -> bool {
        matches!(self.repo_root(path), Ok(Some(_)))
    }

    /// The `.git` directory for a repository.
    pub fn git_dir(&self, repo: &Path) -> Result<PathBuf> {
        let out = self.exec_out(repo, &["rev-parse".into(), "--git-dir".into()])?;
        let p = PathBuf::from(out.trim());
        Ok(if p.is_absolute() { p } else { repo.join(p) })
    }

    // ---------------------------------------------------------------- status

    /// Full working tree status.
    pub fn status(&self, repo: &Path) -> Result<RepoStatus> {
        let raw = self.exec_out(
            repo,
            &[
                "status".into(),
                "--porcelain=v1".into(),
                "-z".into(),
                "--branch".into(),
                "--untracked-files=all".into(),
            ],
        )?;
        let mut status = RepoStatus {
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            staged: Vec::new(),
            unstaged: Vec::new(),
            untracked: Vec::new(),
            conflicted: Vec::new(),
            clean: true,
            in_progress: None,
            unborn: false,
        };

        let mut fields = raw.split('\0').filter(|s| !s.is_empty()).peekable();
        if let Some(head) = fields.next() {
            if let Some(rest) = head.strip_prefix("## ") {
                parse_branch_header(rest, &mut status);
            } else {
                parse_change(head, None, &mut status);
            }
        }
        while let Some(entry) = fields.next() {
            let status_chars = entry.chars().take(2).collect::<String>();
            let needs_original = status_chars.contains('R') || status_chars.contains('C');
            let original = if needs_original {
                fields.next().map(|s| s.to_string())
            } else {
                None
            };
            parse_change(entry, original, &mut status);
        }

        status.clean = status.staged.is_empty()
            && status.unstaged.is_empty()
            && status.untracked.is_empty()
            && status.conflicted.is_empty();
        status.in_progress = self.in_progress(repo)?;
        Ok(status)
    }

    /// Detect an operation Git has left half-finished.
    pub fn in_progress(&self, repo: &Path) -> Result<Option<String>> {
        let dir = self.git_dir(repo)?;
        let markers = [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
            ("BISECT_LOG", "bisect"),
        ];
        for (file, label) in markers {
            if dir.join(file).exists() {
                return Ok(Some(label.to_string()));
            }
        }
        Ok(None)
    }

    /// The current branch name, or `None` when detached.
    pub fn current_branch(&self, repo: &Path) -> Result<Option<String>> {
        let out = self.exec_out(
            repo,
            &["rev-parse".into(), "--abbrev-ref".into(), "HEAD".into()],
        )?;
        let name = out.trim().to_string();
        if name == "HEAD" || name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// How far a branch has diverged from its upstream.
    pub fn divergence(&self, repo: &Path) -> Result<Divergence> {
        let st = self.status(repo)?;
        if st.upstream.is_none() {
            return Ok(Divergence::NoUpstream);
        }
        match (st.ahead, st.behind) {
            (0, 0) => Ok(Divergence::UpToDate),
            (_, 0) => Ok(Divergence::Ahead),
            (0, _) => Ok(Divergence::Behind),
            _ => Ok(Divergence::Diverged),
        }
    }

    // ------------------------------------------------------------------ diff

    /// Produce a diff according to `opts`.
    pub fn diff(&self, repo: &Path, opts: &DiffOptions) -> Result<String> {
        let mut args = vec!["diff".to_string(), "--no-color".to_string()];
        if opts.stat {
            args.push("--stat".into());
        }
        if let Some(c) = opts.context {
            args.push(format!("--unified={c}"));
        }
        if opts.staged {
            args.push("--cached".into());
        }
        if let Some(rev) = &opts.rev {
            validate_rev(rev)?;
            args.push(rev.clone());
        }
        if let Some(p) = &opts.path {
            validate_relpath(p)?;
            args.push("--".into());
            args.push(p.clone());
        }
        self.exec_out(repo, &args)
    }

    /// Diff of a single commit against its parent.
    pub fn show(&self, repo: &Path, rev: &str) -> Result<String> {
        validate_rev(rev)?;
        self.exec_out(repo, &["show".into(), "--no-color".into(), rev.to_string()])
    }

    /// Per-line authorship for a file.
    pub fn blame(&self, repo: &Path, path: &str) -> Result<String> {
        validate_relpath(path)?;
        self.exec_out(
            repo,
            &[
                "blame".into(),
                "--date=iso".into(),
                "--".into(),
                path.to_string(),
            ],
        )
    }

    /// Name the branches that already contain a commit.
    pub fn containing_branches(&self, repo: &Path, rev: &str) -> Result<Vec<String>> {
        validate_rev(rev)?;
        let out = self.exec_out(
            repo,
            &[
                "branch".into(),
                "--contains".into(),
                rev.to_string(),
                "--format=%(refname:short)".into(),
            ],
        )?;
        Ok(out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    // ---------------------------------------------------------------- staging

    /// Stage paths.
    pub fn stage(&self, repo: &Path, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(DevdError::invalid("no paths given"));
        }
        for p in paths {
            validate_relpath(p)?;
        }
        let mut args = vec!["add".to_string(), "--".to_string()];
        args.extend(paths.iter().cloned());
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Unstage paths, leaving the working tree untouched.
    pub fn unstage(&self, repo: &Path, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(DevdError::invalid("no paths given"));
        }
        for p in paths {
            validate_relpath(p)?;
        }
        let mut args = vec![
            "restore".to_string(),
            "--staged".to_string(),
            "--".to_string(),
        ];
        args.extend(paths.iter().cloned());
        let out = self.exec(Some(repo), &args)?;
        if out.ok() {
            return Ok(());
        }
        // Repositories with no commits yet cannot use `restore --staged`.
        let mut fallback = vec![
            "rm".into(),
            "--cached".into(),
            "-r".into(),
            "--quiet".into(),
            "--".into(),
        ];
        fallback.extend(paths.iter().cloned());
        let out = self.exec(Some(repo), &fallback)?;
        if out.ok() {
            Ok(())
        } else {
            Err(command_failed(&fallback, &out))
        }
    }

    /// Apply a unified diff to the index, which is how a single hunk is staged.
    pub fn stage_patch(&self, repo: &Path, patch: &str, reverse: bool) -> Result<()> {
        if patch.trim().is_empty() {
            return Err(DevdError::invalid("patch is empty"));
        }
        let dir = self.git_dir(repo)?;
        let patch_file = dir.join(format!("devd-stage-{}.patch", std::process::id()));
        std::fs::write(&patch_file, patch)?;
        let mut args = vec![
            "apply".to_string(),
            "--cached".to_string(),
            "--whitespace=nowarn".to_string(),
        ];
        if reverse {
            args.push("--reverse".into());
        }
        args.push(patch_file.to_string_lossy().to_string());
        let out = self.exec(Some(repo), &args);
        let _ = std::fs::remove_file(&patch_file);
        let out = out?;
        if out.ok() {
            Ok(())
        } else {
            Err(command_failed(&args, &out))
        }
    }

    /// Discard working tree changes for tracked paths.
    pub fn discard(&self, repo: &Path, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(DevdError::invalid("no paths given"));
        }
        for p in paths {
            validate_relpath(p)?;
        }
        let mut args = vec![
            "restore".to_string(),
            "--worktree".to_string(),
            "--".to_string(),
        ];
        args.extend(paths.iter().cloned());
        self.exec_out(repo, &args)?;
        Ok(())
    }

    // ------------------------------------------------------------------ commit

    /// Create a commit. Returns the new commit hash.
    pub fn commit(
        &self,
        repo: &Path,
        message: &str,
        amend: bool,
        allow_empty: bool,
    ) -> Result<String> {
        if message.trim().is_empty() {
            return Err(DevdError::invalid("commit message may not be empty"));
        }
        if message.contains('\0') {
            return Err(DevdError::invalid("commit message may not contain NUL"));
        }
        let mut args = vec!["commit".to_string(), "-m".to_string(), message.to_string()];
        if amend {
            args.push("--amend".into());
        }
        if allow_empty {
            args.push("--allow-empty".into());
        }
        // A GUI or agent must never be blocked by an interactive editor or
        // signing prompt it cannot answer.
        args.push("--no-edit".into());
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            return Err(command_failed(&args, &out));
        }
        let head = self.rev_parse(repo, "HEAD")?;
        Ok(head)
    }

    /// Resolve a revision to a full hash.
    pub fn rev_parse(&self, repo: &Path, rev: &str) -> Result<String> {
        validate_rev(rev)?;
        let out = self.exec_out(
            repo,
            &[
                "rev-parse".into(),
                "--verify".into(),
                format!("{rev}^{{commit}}"),
            ],
        )?;
        Ok(out.trim().to_string())
    }

    // ------------------------------------------------------------------- log

    /// Recent commits, newest first.
    pub fn log(
        &self,
        repo: &Path,
        limit: u32,
        rev: Option<&str>,
        path: Option<&str>,
    ) -> Result<Vec<CommitInfo>> {
        let mut args = vec![
            "log".to_string(),
            format!("--max-count={limit}"),
            "--pretty=format:%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%s%x1e".to_string(),
        ];
        if let Some(r) = rev {
            validate_rev(r)?;
            args.push(r.to_string());
        }
        if let Some(p) = path {
            validate_relpath(p)?;
            args.push("--".into());
            args.push(p.to_string());
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            // An unborn branch has no history; that is not an error.
            if out.stderr.contains("does not have any commits")
                || out.stderr.contains("unknown revision")
            {
                return Ok(Vec::new());
            }
            return Err(command_failed(&args, &out));
        }
        Ok(parse_log(&out.stdout))
    }

    /// Search history.
    pub fn search_commits(
        &self,
        repo: &Path,
        query: &str,
        author: Option<&str>,
        path: Option<&str>,
        limit: u32,
    ) -> Result<Vec<CommitInfo>> {
        let mut args = vec![
            "log".to_string(),
            format!("--max-count={limit}"),
            "--pretty=format:%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%s%x1e".to_string(),
            "--regexp-ignore-case".to_string(),
            format!("--grep={query}"),
        ];
        if let Some(a) = author {
            if a.starts_with('-') {
                return Err(DevdError::invalid("author may not start with '-'"));
            }
            args.push(format!("--author={a}"));
        }
        if let Some(p) = path {
            validate_relpath(p)?;
            args.push("--".into());
            args.push(p.to_string());
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            return Ok(Vec::new());
        }
        Ok(parse_log(&out.stdout))
    }

    // --------------------------------------------------------------- branches

    /// Local branches, optionally including remotes.
    pub fn branches(&self, repo: &Path, include_remotes: bool) -> Result<Vec<BranchInfo>> {
        let mut args = vec![
            "for-each-ref".to_string(),
            // `for-each-ref` has its own format language, where `%1f` is the
            // 0x1f separator byte. It does not understand `%x1f`.
            "--format=%(refname:short)%1f%(upstream:short)%1f%(subject)%1f%(HEAD)".to_string(),
        ];
        if include_remotes {
            args.push("refs/heads".into());
            args.push("refs/remotes".into());
        } else {
            args.push("refs/heads".into());
        }
        let out = self.exec_out(repo, &args)?;
        let mut branches = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() < 4 {
                continue;
            }
            branches.push(BranchInfo {
                name: parts[0].to_string(),
                upstream: if parts[1].is_empty() {
                    None
                } else {
                    Some(parts[1].to_string())
                },
                subject: parts[2].to_string(),
                current: parts[3].trim() == "*",
            });
        }
        Ok(branches)
    }

    /// Create a branch at `start_point` (defaults to the current `HEAD`).
    ///
    /// `checkout` selects creation *and* switch (`git branch -b` semantics),
    /// versus creating the branch and leaving the current branch alone.
    pub fn create_branch(
        &self,
        repo: &Path,
        name: &str,
        start_point: Option<&str>,
        checkout: bool,
    ) -> Result<()> {
        validate_ref(name)?;
        if let Some(sp) = start_point {
            validate_rev(sp)?;
        }
        // There is no `git branch -b`: creating a branch and switching to it is
        // `git switch -c`, and plain creation is `git branch`.
        let sub = if checkout { "switch" } else { "branch" };
        let mut args = vec![sub.to_string()];
        if checkout {
            args.push("-c".to_string());
        }
        args.push(name.to_string());
        if let Some(sp) = start_point {
            args.push(sp.to_string());
        }
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Rename a branch.
    pub fn rename_branch(&self, repo: &Path, from: &str, to: &str) -> Result<()> {
        validate_ref(from)?;
        validate_ref(to)?;
        self.exec_out(
            repo,
            &[
                "branch".into(),
                "-m".into(),
                from.to_string(),
                to.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Delete a branch. `force` is required for unmerged commits.
    pub fn delete_branch(
        &self,
        repo: &Path,
        name: &str,
        force: bool,
        remote: Option<&str>,
    ) -> Result<()> {
        validate_ref(name)?;
        let flag = if force { "-D" } else { "-d" };
        let mut args = vec!["branch".to_string(), flag.to_string(), name.to_string()];
        if let Some(r) = remote {
            validate_ref(r)?;
            args = vec![
                "push".into(),
                r.to_string(),
                "--delete".into(),
                name.to_string(),
            ];
        }
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Switch to an existing branch.
    ///
    /// Refuses when it would overwrite uncommitted work, unless `force` is set.
    pub fn switch_branch(&self, repo: &Path, name: &str, create: bool, force: bool) -> Result<()> {
        validate_ref(name)?;
        let mut args = vec!["switch".to_string()];
        if create {
            args.push("-c".into());
        }
        if force {
            args.push("--discard-changes".into());
        }
        args.push(name.to_string());
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Check out a commit without creating a branch.
    pub fn checkout_detached(&self, repo: &Path, rev: &str) -> Result<()> {
        validate_rev(rev)?;
        self.exec_out(repo, &["switch".into(), "--detach".into(), rev.to_string()])?;
        Ok(())
    }

    /// Whether switching to `target` would clobber local modifications.
    ///
    /// Only files that differ between the current commit and `target` can be
    /// overwritten by a switch, so the answer is the intersection of those files
    /// with the ones the user has changed locally. The range is two-dot on
    /// purpose: `target...HEAD` compares the merge base with `HEAD`, which is
    /// usually empty and would report a dangerous switch as safe.
    pub fn switch_safety(&self, repo: &Path, target: &str) -> Result<Vec<String>> {
        validate_ref(target)?;
        let status = self.status(repo)?;
        let mut risky: Vec<String> = Vec::new();
        if status.staged.is_empty() && status.unstaged.is_empty() {
            return Ok(risky);
        }
        let diff = self.diff(
            repo,
            &DiffOptions {
                rev: Some(format!("HEAD..{target}")),
                ..Default::default()
            },
        )?;
        let changed = status.changed_paths();
        for line in diff.lines() {
            // Trailing whitespace is trimmed because Git's output can carry a
            // carriage return on platforms that translate line endings.
            let line = line.trim_end();
            let candidate = line
                .strip_prefix("+++ b/")
                .or_else(|| line.strip_prefix("--- a/"));
            if let Some(path) = candidate {
                if changed.iter().any(|c| c == path) {
                    risky.push(path.to_string());
                }
            }
        }
        risky.sort();
        risky.dedup();
        Ok(risky)
    }

    // ------------------------------------------------------- merge and rebase

    /// Merge a branch into the current one.
    pub fn merge(
        &self,
        repo: &Path,
        branch: &str,
        no_ff: bool,
        squash: bool,
        message: Option<&str>,
    ) -> Result<String> {
        validate_ref(branch)?;
        let mut args = vec!["merge".to_string(), "--no-edit".to_string()];
        if no_ff {
            args.push("--no-ff".into());
        }
        if squash {
            args.push("--squash".into());
        }
        if let Some(m) = message {
            args.push("-m".into());
            args.push(m.to_string());
        }
        args.push(branch.to_string());
        let out = self.exec(Some(repo), &args)?;
        // A conflicted merge exits non-zero but is a state, not a failure.
        if !out.ok() {
            let status = self.status(repo)?;
            if status.conflicted.is_empty() && status.in_progress.is_none() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// Rebase the current branch onto `onto`.
    ///
    /// `onto` is a validated revision, or the literal `--root` to replay the
    /// whole history (which is what a full-history autosquash needs). An
    /// interactive rebase is only ever driven by the built-in no-op sequence
    /// editor, so it cannot hang waiting for a human that is not there.
    pub fn rebase(
        &self,
        repo: &Path,
        onto: &str,
        autosquash: bool,
        interactive: bool,
    ) -> Result<String> {
        let root = onto == "--root";
        if !root {
            validate_ref(onto)?;
        }
        let mut args = vec!["rebase".to_string()];
        // `--autosquash` is only meaningful with `-i`, so asking for one asks
        // for the other rather than failing later.
        if interactive || autosquash {
            // GIT_SEQUENCE_EDITOR is set to `true` so an interactive rebase with
            // no edits behaves like a non-interactive one instead of hanging.
            args.push("-i".into());
        }
        if autosquash {
            args.push("--autosquash".into());
        }
        if root {
            args.push("--root".into());
        } else {
            args.push(onto.to_string());
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            let status = self.status(repo)?;
            if status.conflicted.is_empty() && status.in_progress.is_none() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// Squash the last `count` commits into one with a new message.
    ///
    /// When `count` covers the whole branch the result must be a single root
    /// commit, which `reset --soft` cannot produce on its own, so the root is
    /// amended instead. Either way the newest tree is preserved.
    pub fn squash_last(&self, repo: &Path, count: u32, message: &str) -> Result<String> {
        if count < 2 {
            return Err(DevdError::invalid("squash requires at least 2 commits"));
        }
        if message.trim().is_empty() {
            return Err(DevdError::invalid("squash message may not be empty"));
        }
        let total: u32 = self
            .exec_out(repo, &["rev-list".into(), "--count".into(), "HEAD".into()])?
            .trim()
            .parse()
            .unwrap_or(0);
        if total < count {
            return Err(DevdError::invalid(format!(
                "cannot squash {count} commits: the branch has {total}"
            )));
        }

        if total == count {
            let roots = self.exec_out(
                repo,
                &["rev-list".into(), "--max-parents=0".into(), "HEAD".into()],
            )?;
            let root = roots.lines().last().unwrap_or("").trim().to_string();
            if root.is_empty() {
                return Err(DevdError::internal("could not find the root commit"));
            }
            self.exec_out(repo, &["reset".into(), "--soft".into(), root])?;
            let args = vec![
                "commit".to_string(),
                "--amend".to_string(),
                "-m".to_string(),
                message.to_string(),
            ];
            let out = self.exec(Some(repo), &args)?;
            if !out.ok() {
                return Err(command_failed(&args, &out));
            }
            return self.rev_parse(repo, "HEAD");
        }

        let target = format!("HEAD~{count}");
        self.exec_out(repo, &["reset".into(), "--soft".into(), target])?;
        self.commit(repo, message, false, false)
    }

    /// Continue an interrupted merge, rebase, cherry-pick or revert.
    pub fn continue_op(&self, repo: &Path, op: &str) -> Result<String> {
        let (sub, arg) = match op {
            "rebase" => ("rebase", "--continue"),
            "cherry-pick" => ("cherry-pick", "--continue"),
            "revert" => ("revert", "--continue"),
            "merge" => ("commit", "--no-edit"),
            other => return Err(DevdError::invalid(format!("cannot continue `{other}`"))),
        };
        let args = vec![sub.to_string(), arg.to_string()];
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            // `--continue` on a clean rebase exits non-zero in some versions.
            let status = self.status(repo)?;
            if status.in_progress.is_some() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// Abort an interrupted operation, restoring the previous state.
    pub fn abort_op(&self, repo: &Path, op: &str) -> Result<String> {
        let sub = match op {
            "rebase" | "merge" | "cherry-pick" | "revert" => op,
            other => return Err(DevdError::invalid(format!("cannot abort `{other}`"))),
        };
        let args = vec![sub.to_string(), "--abort".to_string()];
        self.exec_out(repo, &args)
    }

    /// List conflicted paths with their stage-2 and stage-3 content markers.
    pub fn conflicts(&self, repo: &Path) -> Result<Vec<String>> {
        let out = self.exec_out(
            repo,
            &[
                "diff".into(),
                "--name-only".into(),
                "--diff-filter=U".into(),
            ],
        )?;
        Ok(out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// Read a conflicted file with conflict markers intact.
    pub fn conflicted_file(&self, repo: &Path, path: &str) -> Result<String> {
        validate_relpath(path)?;
        let full = repo.join(path);
        std::fs::read_to_string(&full)
            .map_err(|e| DevdError::invalid(format!("cannot read `{path}`: {e}")))
    }

    /// Resolve a conflict by taking one side, then stage the result.
    pub fn resolve_conflict(&self, repo: &Path, path: &str, side: ConflictSide) -> Result<()> {
        validate_relpath(path)?;
        let flag = match side {
            ConflictSide::Ours => "--ours",
            ConflictSide::Theirs => "--theirs",
        };
        self.exec_out(
            repo,
            &[
                "checkout".into(),
                flag.into(),
                "--".into(),
                path.to_string(),
            ],
        )?;
        self.stage(repo, &[path.to_string()])
    }

    // ----------------------------------------------------------------- remotes

    /// Configured remotes.
    pub fn remotes(&self, repo: &Path) -> Result<Vec<RemoteInfo>> {
        let out = self.exec_out(repo, &["remote".into(), "-v".into()])?;
        let mut remotes: Vec<RemoteInfo> = Vec::new();
        for line in out.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 3 {
                continue;
            }
            let (name, url, kind) = (cols[0], cols[1], cols[2]);
            match remotes.iter_mut().find(|r| r.name == name) {
                Some(existing) => {
                    if kind == "(push)" {
                        existing.push_url = Some(url.to_string());
                    }
                }
                None => remotes.push(RemoteInfo {
                    name: name.to_string(),
                    fetch_url: redact_url(url),
                    push_url: if kind == "(push)" {
                        Some(redact_url(url))
                    } else {
                        None
                    },
                }),
            }
        }
        Ok(remotes)
    }

    /// Add a remote.
    pub fn remote_add(&self, repo: &Path, name: &str, url: &str) -> Result<()> {
        validate_ref(name)?;
        validate_remote_url(url)?;
        self.exec_out(
            repo,
            &[
                "remote".into(),
                "add".into(),
                name.to_string(),
                url.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Remove a remote.
    pub fn remote_remove(&self, repo: &Path, name: &str) -> Result<()> {
        validate_ref(name)?;
        self.exec_out(repo, &["remote".into(), "remove".into(), name.to_string()])?;
        Ok(())
    }

    /// Rename a remote.
    pub fn remote_rename(&self, repo: &Path, from: &str, to: &str) -> Result<()> {
        validate_ref(from)?;
        validate_ref(to)?;
        self.exec_out(
            repo,
            &[
                "remote".into(),
                "rename".into(),
                from.to_string(),
                to.to_string(),
            ],
        )?;
        Ok(())
    }

    /// Fetch from a remote, optionally pruning deleted remote branches.
    ///
    /// A `remote` of `None` fetches every remote.
    pub fn fetch(&self, repo: &Path, remote: Option<&str>, prune: bool) -> Result<String> {
        let mut args = vec!["fetch".to_string()];
        if prune {
            args.push("--prune".into());
        }
        match remote {
            Some(r) => {
                validate_ref(r)?;
                args.push(r.to_string());
            }
            None => args.push("--all".into()),
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            return Err(command_failed(&args, &out));
        }
        Ok(out.stderr)
    }

    /// Commits that would arrive from the upstream on a pull.
    pub fn incoming(
        &self,
        repo: &Path,
        remote: &str,
        branch: &str,
        limit: u32,
    ) -> Result<Vec<CommitInfo>> {
        let range = format!("HEAD..{remote}/{branch}");
        self.log(repo, limit, Some(&range), None)
    }

    /// Commits a push would publish.
    pub fn outgoing(
        &self,
        repo: &Path,
        remote: &str,
        branch: &str,
        limit: u32,
    ) -> Result<Vec<CommitInfo>> {
        let range = format!("{remote}/{branch}..HEAD");
        self.log(repo, limit, Some(&range), None)
    }

    /// Preview a synchronisation without changing anything.
    pub fn sync_preview(
        &self,
        repo: &Path,
        remote: &str,
        branch: &str,
        limit: u32,
    ) -> Result<SyncPreview> {
        let incoming = self
            .incoming(repo, remote, branch, limit)
            .unwrap_or_default();
        let outgoing = self
            .outgoing(repo, remote, branch, limit)
            .unwrap_or_default();
        Ok(SyncPreview {
            diverged: !incoming.is_empty() && !outgoing.is_empty(),
            incoming,
            outgoing,
        })
    }

    /// Pull using an explicit strategy.
    pub fn pull(
        &self,
        repo: &Path,
        remote: Option<&str>,
        branch: Option<&str>,
        strategy: PullStrategy,
    ) -> Result<String> {
        let mut args = vec!["pull".to_string()];
        match strategy {
            PullStrategy::Merge => {}
            PullStrategy::Rebase => args.push("--rebase".into()),
            PullStrategy::FastForwardOnly => args.push("--ff-only".into()),
        }
        args.push("--no-edit".into());
        if let Some(r) = remote {
            validate_ref(r)?;
            args.push(r.to_string());
            if let Some(b) = branch {
                validate_ref(b)?;
                args.push(b.to_string());
            }
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            let status = self.status(repo)?;
            if status.conflicted.is_empty() && status.in_progress.is_none() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// Push the current branch, optionally setting upstream.
    pub fn push(
        &self,
        repo: &Path,
        remote: &str,
        branch: Option<&str>,
        set_upstream: bool,
        force: ForcePush,
        tags: bool,
    ) -> Result<String> {
        validate_ref(remote)?;
        let mut args = vec!["push".to_string()];
        match force {
            ForcePush::None => {}
            ForcePush::WithLease => args.push("--force-with-lease".into()),
            ForcePush::Force => args.push("--force".into()),
        }
        if set_upstream {
            args.push("--set-upstream".into());
        }
        if tags {
            args.push("--tags".into());
        }
        args.push(remote.to_string());
        if let Some(b) = branch {
            validate_ref(b)?;
            args.push(b.to_string());
        }
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            return Err(command_failed(&args, &out));
        }
        Ok(out.stderr)
    }

    /// Set the upstream for a branch.
    pub fn set_upstream(&self, repo: &Path, branch: &str, upstream: &str) -> Result<()> {
        validate_ref(branch)?;
        validate_ref(upstream)?;
        self.exec_out(
            repo,
            &[
                "branch".into(),
                format!("--set-upstream-to={upstream}"),
                branch.to_string(),
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------ tags

    /// All tags, newest first.
    pub fn tags(&self, repo: &Path) -> Result<Vec<TagInfo>> {
        let out = self.exec_out(
            repo,
            &[
                "for-each-ref".into(),
                "--sort=-creatordate".into(),
                "--format=%(refname:short)%1f%(objectname:short)%1f%(contents:subject)%1f%(objecttype)".into(),
                "refs/tags".into(),
            ],
        )?;
        let mut tags = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() < 4 {
                continue;
            }
            tags.push(TagInfo {
                name: parts[0].to_string(),
                target: parts[1].to_string(),
                message: if parts[3] == "tag" && !parts[2].is_empty() {
                    Some(parts[2].to_string())
                } else {
                    None
                },
            });
        }
        Ok(tags)
    }

    /// Create a tag.
    pub fn create_tag(
        &self,
        repo: &Path,
        name: &str,
        message: Option<&str>,
        rev: Option<&str>,
        force: bool,
    ) -> Result<()> {
        validate_ref(name)?;
        let mut args = vec!["tag".to_string()];
        if let Some(m) = message {
            args.push("-a".into());
            args.push("-m".into());
            args.push(m.to_string());
        }
        if force {
            args.push("-f".into());
        }
        args.push(name.to_string());
        if let Some(r) = rev {
            validate_rev(r)?;
            args.push(r.to_string());
        }
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Delete a tag locally and optionally on a remote.
    pub fn delete_tag(&self, repo: &Path, name: &str, remote: Option<&str>) -> Result<()> {
        validate_ref(name)?;
        self.exec_out(repo, &["tag".into(), "-d".into(), name.to_string()])?;
        if let Some(r) = remote {
            validate_ref(r)?;
            self.exec_out(
                repo,
                &[
                    "push".into(),
                    r.to_string(),
                    "--delete".into(),
                    name.to_string(),
                ],
            )?;
        }
        Ok(())
    }

    // --------------------------------------------------------- history surgery

    /// Reset the branch, with the mode deciding how much is destroyed.
    pub fn reset(&self, repo: &Path, mode: ResetMode, rev: &str) -> Result<String> {
        validate_rev(rev)?;
        self.exec_out(repo, &["reset".into(), mode.flag().into(), rev.to_string()])
    }

    /// Revert a commit by creating its inverse.
    pub fn revert(&self, repo: &Path, rev: &str, no_commit: bool) -> Result<String> {
        validate_rev(rev)?;
        let mut args = vec!["revert".to_string(), "--no-edit".to_string()];
        if no_commit {
            args.push("--no-commit".into());
        }
        args.push(rev.to_string());
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            let status = self.status(repo)?;
            if status.conflicted.is_empty() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// Cherry-pick a commit or range.
    pub fn cherry_pick(&self, repo: &Path, rev: &str, no_commit: bool) -> Result<String> {
        validate_rev(rev)?;
        let mut args = vec!["cherry-pick".to_string()];
        if no_commit {
            args.push("--no-commit".into());
        }
        args.push(rev.to_string());
        let out = self.exec(Some(repo), &args)?;
        if !out.ok() {
            let status = self.status(repo)?;
            if status.conflicted.is_empty() && status.in_progress.is_none() {
                return Err(command_failed(&args, &out));
            }
        }
        Ok(out.stdout)
    }

    /// The reflog, which is what makes recovery from a bad reset possible.
    pub fn reflog(&self, repo: &Path, limit: u32) -> Result<Vec<String>> {
        let out = self.exec_out(
            repo,
            &[
                "reflog".into(),
                format!("--max-count={limit}"),
                "--date=iso".into(),
            ],
        )?;
        Ok(out.lines().map(|l| l.to_string()).collect())
    }

    /// Start a bisect run.
    pub fn bisect_start(&self, repo: &Path, bad: &str, good: &str) -> Result<String> {
        validate_rev(bad)?;
        validate_rev(good)?;
        self.exec_out(repo, &["bisect".into(), "start".into()])?;
        self.exec_out(repo, &["bisect".into(), "bad".into(), bad.to_string()])?;
        let out = self.exec_out(repo, &["bisect".into(), "good".into(), good.to_string()])?;
        Ok(out)
    }

    /// Mark the current bisect step.
    pub fn bisect_mark(&self, repo: &Path, verdict: BisectVerdict) -> Result<String> {
        let arg = match verdict {
            BisectVerdict::Good => "good",
            BisectVerdict::Bad => "bad",
            BisectVerdict::Skip => "skip",
        };
        self.exec_out(repo, &["bisect".into(), arg.to_string()])
    }

    /// End a bisect, returning to the original branch.
    pub fn bisect_reset(&self, repo: &Path) -> Result<String> {
        self.exec_out(repo, &["bisect".into(), "reset".into()])
    }

    // ----------------------------------------------------------------- stash

    /// List stash entries.
    pub fn stash_list(&self, repo: &Path) -> Result<Vec<StashInfo>> {
        // `git stash list` uses the log format language, where a separator is
        // `%x1f`. `%1f` is only understood by `for-each-ref` and would be
        // emitted literally here, silently collapsing every entry into one field.
        let out = self.exec_out(
            repo,
            &["stash".into(), "list".into(), "--format=%gd%x1f%gs".into()],
        )?;
        let mut out_entries = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() < 2 {
                continue;
            }
            out_entries.push(StashInfo {
                reference: parts[0].to_string(),
                subject: parts[1].to_string(),
            });
        }
        Ok(out_entries)
    }

    /// Create a stash entry.
    pub fn stash_push(
        &self,
        repo: &Path,
        message: Option<&str>,
        include_untracked: bool,
        keep_index: bool,
    ) -> Result<String> {
        let mut args = vec!["stash".to_string(), "push".to_string()];
        if include_untracked {
            args.push("--include-untracked".into());
        }
        if keep_index {
            args.push("--keep-index".into());
        }
        if let Some(m) = message {
            args.push("-m".into());
            args.push(m.to_string());
        }
        self.exec_out(repo, &args)
    }

    /// Apply a stash, optionally dropping it.
    pub fn stash_apply(
        &self,
        repo: &Path,
        reference: Option<&str>,
        pop: bool,
        index: bool,
    ) -> Result<String> {
        let mut args = vec![
            "stash".to_string(),
            if pop { "pop".into() } else { "apply".into() },
        ];
        if index {
            args.push("--index".into());
        }
        if let Some(r) = reference {
            validate_stash_ref(r)?;
            args.push(r.to_string());
        }
        self.exec_out(repo, &args)
    }

    /// Drop a stash entry.
    pub fn stash_drop(&self, repo: &Path, reference: &str) -> Result<String> {
        validate_stash_ref(reference)?;
        self.exec_out(
            repo,
            &["stash".into(), "drop".into(), reference.to_string()],
        )
    }

    /// Show a stash's patch.
    pub fn stash_show(&self, repo: &Path, reference: &str) -> Result<String> {
        validate_stash_ref(reference)?;
        self.exec_out(
            repo,
            &[
                "stash".into(),
                "show".into(),
                "-p".into(),
                reference.to_string(),
            ],
        )
    }

    // -------------------------------------------------------------- worktrees

    /// Linked worktrees.
    pub fn worktrees(&self, repo: &Path) -> Result<Vec<WorktreeInfo>> {
        let out = self.exec_out(
            repo,
            &["worktree".into(), "list".into(), "--porcelain".into()],
        )?;
        let mut items: Vec<WorktreeInfo> = Vec::new();
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                if let Some(last) = items.last() {
                    if last.path.is_empty() {
                        // unreachable, kept for clarity
                    }
                }
                items.push(WorktreeInfo {
                    path: p.to_string(),
                    head: String::new(),
                    branch: None,
                    main: items.is_empty(),
                    locked: false,
                });
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                if let Some(last) = items.last_mut() {
                    last.head = h.to_string();
                }
            } else if let Some(b) = line.strip_prefix("branch ") {
                if let Some(last) = items.last_mut() {
                    last.branch = Some(b.trim_start_matches("refs/heads/").to_string());
                }
            } else if line.starts_with("locked") {
                if let Some(last) = items.last_mut() {
                    last.locked = true;
                }
            } else if line.starts_with("detached") {
                if let Some(last) = items.last_mut() {
                    last.branch = None;
                }
            }
        }
        Ok(items)
    }

    /// Add a worktree for a branch.
    pub fn worktree_add(
        &self,
        repo: &Path,
        path: &Path,
        branch: &str,
        create_branch: bool,
    ) -> Result<()> {
        validate_ref(branch)?;
        let mut args = vec!["worktree".to_string(), "add".to_string()];
        if create_branch {
            args.push("-b".into());
            args.push(branch.to_string());
            args.push(path.to_string_lossy().to_string());
        } else {
            args.push(path.to_string_lossy().to_string());
            args.push(branch.to_string());
        }
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Remove a worktree. `force` discards its uncommitted changes.
    pub fn worktree_remove(&self, repo: &Path, path: &Path, force: bool) -> Result<()> {
        let mut args = vec!["worktree".to_string(), "remove".to_string()];
        if force {
            args.push("--force".into());
        }
        args.push(path.to_string_lossy().to_string());
        self.exec_out(repo, &args)?;
        Ok(())
    }

    /// Prune worktree metadata for directories that no longer exist.
    pub fn worktree_prune(&self, repo: &Path) -> Result<()> {
        self.exec_out(repo, &["worktree".into(), "prune".into()])?;
        Ok(())
    }

    // ------------------------------------------------------------ repos & cfg

    /// Initialise a repository.
    pub fn init(&self, path: &Path, initial_branch: &str) -> Result<PathBuf> {
        validate_ref(initial_branch)?;
        std::fs::create_dir_all(path)?;
        self.exec_out(
            path,
            &["init".into(), format!("--initial-branch={initial_branch}")],
        )?;
        self.repo_root(path)?
            .ok_or_else(|| DevdError::internal("init did not produce a repository"))
    }

    /// Clone a repository. `depth` of `None` performs a full clone.
    pub fn clone(
        &self,
        url: &str,
        destination: &Path,
        depth: Option<u32>,
        branch: Option<&str>,
        recurse_submodules: bool,
    ) -> Result<()> {
        validate_remote_url(url)?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut args = vec!["clone".to_string(), "--".to_string()];
        if let Some(d) = depth {
            args.insert(1, format!("--depth={d}"));
        }
        if let Some(b) = branch {
            validate_ref(b)?;
            args.insert(1, format!("--branch={b}"));
        }
        if recurse_submodules {
            args.insert(1, "--recurse-submodules".into());
        }
        args.push(url.to_string());
        args.push(destination.to_string_lossy().to_string());
        let out = self.exec(None, &args)?;
        if !out.ok() {
            return Err(command_failed(&args, &out));
        }
        Ok(())
    }

    /// Read a Git configuration value. `scope` selects the file to read.
    pub fn config_get(
        &self,
        repo: Option<&Path>,
        key: &str,
        scope: ConfigScope,
    ) -> Result<Option<String>> {
        validate_config_key(key)?;
        let mut args = vec!["config".to_string(), scope.flag().to_string()];
        args.push("--get".into());
        args.push(key.to_string());
        let out = self.exec(repo, &args)?;
        if out.code == 1 {
            return Ok(None);
        }
        if !out.ok() {
            return Err(command_failed(&args, &out));
        }
        Ok(Some(out.stdout.trim().to_string()))
    }

    /// Write a Git configuration value.
    pub fn config_set(
        &self,
        repo: Option<&Path>,
        key: &str,
        value: &str,
        scope: ConfigScope,
    ) -> Result<()> {
        validate_config_key(key)?;
        if config_key_holds_credentials(key) {
            return Err(DevdError::denied(format!(
                "`{key}` exists to store a credential. devd will not write it, because a token in \
                 `.git/config` is a token in plain text on disk, in every snapshot, and in the audit \
                 trail. Use a Git credential helper, an SSH key, or the OS credential store instead."
            )));
        }
        if value.starts_with("--") {
            return Err(DevdError::invalid("config value may not start with '--'"));
        }
        if config_disables_verification(key, value) {
            return Err(DevdError::denied(format!(
                "`{key}` set to `{value}` would disable certificate verification for Git \
                 transports. devd never weakens transport security; fix the certificate or \
                 trust its issuer instead."
            )));
        }
        let args = vec![
            "config".to_string(),
            scope.flag().to_string(),
            key.to_string(),
            value.to_string(),
        ];
        self.exec_out(repo.unwrap_or_else(|| Path::new(".")), &args)?;
        Ok(())
    }

    /// Append patterns to `.gitignore`.
    pub fn ignore_add(&self, repo: &Path, patterns: &[String]) -> Result<()> {
        if patterns.is_empty() {
            return Err(DevdError::invalid("no patterns given"));
        }
        let path = repo.join(".gitignore");
        let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        let mut added = Vec::new();
        for p in patterns {
            if p.contains('\n') || p.trim().is_empty() {
                return Err(DevdError::invalid(
                    "ignore patterns must be non-empty single lines",
                ));
            }
            if !existing.lines().any(|l| l.trim() == p.trim()) {
                existing.push_str(p);
                existing.push('\n');
                added.push(p.clone());
            }
        }
        util::atomic_write(&path, &existing)?;
        Ok(())
    }

    /// Delete merged local branches.
    pub fn prune_merged_branches(
        &self,
        repo: &Path,
        base: &str,
        dry_run: bool,
    ) -> Result<Vec<String>> {
        validate_ref(base)?;
        let out = self.exec_out(
            repo,
            &[
                "branch".into(),
                "--merged".into(),
                base.to_string(),
                "--format=%(refname:short)".into(),
            ],
        )?;
        let current = self.current_branch(repo)?;
        let candidates: Vec<String> = out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && Some(l) != current.as_ref() && l != base)
            .collect();
        if dry_run {
            return Ok(candidates);
        }
        let mut deleted = Vec::new();
        for b in candidates {
            if self.delete_branch(repo, &b, false, None).is_ok() {
                deleted.push(b);
            }
        }
        Ok(deleted)
    }
}

/// Which side of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConflictSide {
    /// The current branch's version.
    Ours,
    /// The incoming version.
    Theirs,
}

/// How a pull should integrate remote work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PullStrategy {
    /// Create a merge commit when needed.
    Merge,
    /// Replay local commits on top.
    Rebase,
    /// Refuse unless the update fast-forwards.
    FastForwardOnly,
}

/// How much force to apply to a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ForcePush {
    /// An ordinary push.
    None,
    /// Force, but refuse to clobber commits we have not seen.
    WithLease,
    /// Force unconditionally. Destructive.
    Force,
}

/// A bisect verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BisectVerdict {
    /// The commit is good.
    Good,
    /// The commit is bad.
    Bad,
    /// Cannot test this commit.
    Skip,
}

/// Which Git configuration file to read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConfigScope {
    /// The repository's own configuration.
    Local,
    /// The user's global configuration.
    Global,
    /// Machine-wide configuration.
    System,
}

impl ConfigScope {
    /// The matching `git config` flag.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Local => "--local",
            Self::Global => "--global",
            Self::System => "--system",
        }
    }
}

/// Environment applied to every Git invocation.
///
/// * `GIT_TERMINAL_PROMPT=0` - never hang waiting for credentials a GUI cannot
///   supply; fail with an actionable error instead.
/// * `LC_ALL=C` - English messages, so error classification is stable.
/// * `GIT_PAGER`/`PAGER` - no pager in a captured pipe.
/// * `GIT_SEQUENCE_EDITOR`/`GIT_EDITOR` - an interactive rebase with no edits
///   behaves like a non-interactive one instead of blocking forever.
/// * `GIT_OPTIONAL_LOCKS=0` - avoid fighting the user's own Git for index locks.
fn env() -> Vec<(String, String)> {
    vec![
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_PAGER".into(), "cat".into()),
        ("PAGER".into(), "cat".into()),
        ("LC_ALL".into(), "C".into()),
        ("GIT_EDITOR".into(), "true".into()),
        ("GIT_SEQUENCE_EDITOR".into(), "true".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ("GIT_ADVICE".into(), "0".into()),
    ]
}

fn parse_log(raw: &str) -> Vec<CommitInfo> {
    let mut commits = Vec::new();
    for record in raw.split('\u{1e}') {
        let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
        if record.is_empty() {
            continue;
        }
        let parts: Vec<&str> = record.split('\u{1f}').collect();
        if parts.len() < 6 {
            continue;
        }
        commits.push(CommitInfo {
            hash: parts[0].to_string(),
            short: parts[1].to_string(),
            author: parts[2].to_string(),
            email: parts[3].to_string(),
            timestamp: parts[4].parse().unwrap_or(0),
            subject: parts[5].to_string(),
        });
    }
    commits
}

fn parse_branch_header(rest: &str, status: &mut RepoStatus) {
    // A branch header fully describes the tracking state, so fields it does not
    // mention are cleared rather than left over from an earlier parse.
    status.upstream = None;
    status.upstream_gone = false;
    status.ahead = 0;
    status.behind = 0;

    if rest.starts_with("No commits yet on ") {
        status.unborn = true;
        status.branch = Some(rest.trim_start_matches("No commits yet on ").to_string());
        return;
    }
    if rest.starts_with("HEAD (no branch)") {
        status.branch = None;
        return;
    }
    let (names, tracking) = match rest.split_once(" [") {
        Some((n, t)) => (n, Some(t.trim_end_matches(']'))),
        None => (rest, None),
    };
    let (local, upstream) = match names.split_once("...") {
        Some((l, u)) => (l, Some(u)),
        None => (names, None),
    };
    status.branch = Some(local.to_string());
    if let Some(u) = upstream {
        if !u.is_empty() {
            status.upstream = Some(u.to_string());
        }
    }
    if let Some(t) = tracking {
        if t.contains("gone") {
            status.upstream_gone = true;
        }
        for part in t.split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                status.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                status.behind = n.parse().unwrap_or(0);
            }
        }
    }
}

fn parse_change(entry: &str, original: Option<String>, status: &mut RepoStatus) {
    if entry.len() < 3 {
        return;
    }
    let mut chars = entry.chars();
    let x = chars.next().unwrap_or(' ');
    let y = chars.next().unwrap_or(' ');
    let rest = &entry[2..];
    let path = rest.trim_start().to_string();
    let codes = format!("{x}{y}");

    if codes == "??" {
        status.untracked.push(path);
        return;
    }
    if codes == "!!" {
        return;
    }
    if is_conflicted(x, y) {
        status.conflicted.push(path.clone());
    }
    if x != ' ' && x != '?' {
        status.staged.push(FileChange {
            status: x.to_string(),
            path: path.clone(),
            original_path: original.clone(),
        });
    }
    if y != ' ' && y != '?' {
        status.unstaged.push(FileChange {
            status: y.to_string(),
            path,
            original_path: original,
        });
    }
}

fn is_conflicted(x: char, y: char) -> bool {
    matches!(
        format!("{x}{y}").as_str(),
        "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU"
    )
}

/// A `devd doctor` check for the Git backend.
///
/// Lives here, beside the service it checks, so that the CLI, the application's
/// System panel and the MCP server all report the same thing from one
/// implementation instead of three that can disagree.
pub fn doctor_check(configured: Option<&str>) -> devd_core::Check {
    match GitService::discover(configured) {
        Ok(git) => devd_core::Check::ok(
            "git",
            format!("{} at {}", git.version(), git.git_path().display()),
        ),
        Err(e) => devd_core::Check::fail("git", e.to_string()),
    }
}

fn command_failed(args: &[String], out: &CommandOutput) -> DevdError {
    let program = args.first().cloned().unwrap_or_else(|| "git".to_string());
    // Git writes a good number of its failures to *stdout*: "nothing to
    // commit, working tree clean", "Your branch is up to date", the advice that
    // accompanies a rejected push. Reporting an empty message because the words
    // landed in the other stream turns a real, explainable cause into a
    // mystery, which is worse than useless when the reader is an agent deciding
    // what to do next.
    let detail = if out.stderr.trim().is_empty() {
        &out.stdout
    } else {
        &out.stderr
    };
    let detail = if detail.trim().is_empty() {
        format!(
            "`git {program}` exited with code {} and produced no output",
            out.code
        )
    } else {
        util::truncate(&redact_url(detail), 2000)
    };
    DevdError::Command {
        program: format!("git {program}"),
        code: out.code,
        stderr: detail,
    }
}

/// Remove credentials embedded in a URL so they never reach logs or agent
/// context. Applied to every error message this module produces.
pub fn redact_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find("://") {
        out.push_str(&rest[..idx + 3]);
        let after = &rest[idx + 3..];
        let end = after
            .find(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '\n')
            .unwrap_or(after.len());
        let candidate = &after[..end];
        match candidate.find('@') {
            Some(at) => {
                out.push_str("[redacted]");
                out.push_str(&candidate[at..]);
            }
            None => out.push_str(candidate),
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Validate a branch, tag or remote name.
///
/// Rejects anything that could be parsed as a command-line option or that Git
/// itself forbids, which is what stops `--upload-pack=...`-style argument
/// injection from a model-generated branch name.
pub fn validate_ref(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(DevdError::invalid("name may not be empty"));
    }
    if name.len() > 255 {
        return Err(DevdError::invalid("name is too long"));
    }
    if name.starts_with('-') {
        return Err(DevdError::invalid(
            "name may not start with '-' (it would be read as an option)",
        ));
    }
    if name.starts_with('/') || name.ends_with('/') || name.ends_with('.') {
        return Err(DevdError::invalid("invalid leading or trailing character"));
    }
    if name.contains("..") {
        return Err(DevdError::invalid("name may not contain '..'"));
    }
    if name.contains("@{") || name == "@" {
        return Err(DevdError::invalid("name may not contain '@{'"));
    }
    if name.ends_with(".lock") {
        return Err(DevdError::invalid("name may not end with '.lock'"));
    }
    for bad in [' ', '~', '^', ':', '?', '*', '[', '\\', '\u{7f}'] {
        if name.contains(bad) {
            return Err(DevdError::invalid(format!("name may not contain {bad:?}")));
        }
    }
    if name.chars().any(|c| (c as u32) < 0x20) {
        return Err(DevdError::invalid(
            "name may not contain control characters",
        ));
    }
    Ok(())
}

/// Validate a revision expression used as an argument.
pub fn validate_rev(rev: &str) -> Result<()> {
    if rev.trim().is_empty() {
        return Err(DevdError::invalid("revision may not be empty"));
    }
    if rev.starts_with('-') {
        return Err(DevdError::invalid(
            "revision may not start with '-' (it would be read as an option)",
        ));
    }
    if rev.contains("--") && !rev.contains("...") {
        return Err(DevdError::invalid("revision may not contain '--'"));
    }
    if rev.chars().any(|c| (c as u32) < 0x20) {
        return Err(DevdError::invalid(
            "revision may not contain control characters",
        ));
    }
    Ok(())
}

/// Validate a repository-relative path.
///
/// Rejects absolute paths, drive letters, `..` traversal and leading dashes so a
/// path can never escape the work tree or become an option.
pub fn validate_relpath(path: &str) -> Result<()> {
    if path.trim().is_empty() {
        return Err(DevdError::invalid("path may not be empty"));
    }
    if path.starts_with('-') {
        return Err(DevdError::invalid("path may not start with '-'"));
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(DevdError::invalid("path must be repository-relative"));
    }
    if normalized.len() >= 2 && normalized.as_bytes()[1] == b':' {
        return Err(DevdError::invalid("path must not contain a drive letter"));
    }
    if normalized.split('/').any(|seg| seg == "..") {
        return Err(DevdError::invalid("path may not contain '..'"));
    }
    if path.chars().any(|c| (c as u32) < 0x20) {
        return Err(DevdError::invalid(
            "path may not contain control characters",
        ));
    }
    Ok(())
}

/// Validate a stash reference such as `stash@{2}`.
pub fn validate_stash_ref(reference: &str) -> Result<()> {
    if !reference.starts_with("stash@{") || !reference.ends_with('}') {
        return Err(DevdError::invalid(
            "expected a stash reference like stash@{0}",
        ));
    }
    let inner = &reference[7..reference.len() - 1];
    if !inner.chars().all(|c| c.is_ascii_digit()) {
        return Err(DevdError::invalid("stash index must be numeric"));
    }
    Ok(())
}

/// Validate a Git config key such as `user.email`.
pub fn validate_config_key(key: &str) -> Result<()> {
    if key.is_empty() || key.starts_with('-') {
        return Err(DevdError::invalid("invalid config key"));
    }
    let ok = key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if !ok || !key.contains('.') {
        return Err(DevdError::invalid(format!(
            "config key `{key}` must look like `section.name`"
        )));
    }
    Ok(())
}

/// Whether a Git config key exists to hold a credential.
///
/// `devd` refuses to write these. A token in `.git/config` is a token in plain
/// text on disk, in every snapshot, and in the audit trail, which is precisely
/// what the product promises never to create.
fn config_key_holds_credentials(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if lower.contains("extraheader") {
        return true;
    }
    let tail = lower.rsplit('.').next().unwrap_or("");
    matches!(
        tail,
        "password" | "passwd" | "token" | "secret" | "apikey" | "api_key" | "credential"
    )
}

/// Whether a configuration write would turn certificate verification off.
///
/// `devd` never weakens transport security. The key is matched by suffix so that
/// a per-URL form such as `http.https://host/.sslVerify` is caught as well as
/// the global `http.sslVerify`.
fn config_disables_verification(key: &str, value: &str) -> bool {
    let key = key.to_ascii_lowercase();
    if !key.ends_with("sslverify") {
        return false;
    }
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "no" | "off" | "0"
    )
}

/// Validate a remote URL.
///
/// Only transport schemes `devd` understands are accepted; plain `http://` is
/// rejected because it would send credentials and code in the clear; and a URL
/// that embeds a password is rejected outright.
pub fn validate_remote_url(url: &str) -> Result<()> {
    if url.trim().is_empty() {
        return Err(DevdError::invalid("remote URL may not be empty"));
    }
    if url.starts_with('-') {
        return Err(DevdError::invalid("remote URL may not start with '-'"));
    }
    if url.chars().any(|c| (c as u32) < 0x20) {
        return Err(DevdError::invalid(
            "remote URL may not contain control characters",
        ));
    }
    // A password in a remote URL is a password written to `.git/config` in plain
    // text, echoed into shell history, and printed by `git remote -v`. Credential
    // helpers and SSH keys are the supported mechanisms.
    if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if let Some((userinfo, _host)) = authority.rsplit_once('@') {
            if userinfo.contains(':') {
                return Err(DevdError::invalid(
                    "remote URLs may not embed a password; configure a Git credential helper or use an SSH key instead",
                ));
            }
        }
    }
    let schemes = ["https://", "ssh://", "git://", "file://"];
    if schemes.iter().any(|s| url.starts_with(s)) {
        return Ok(());
    }
    if url.starts_with("http://") {
        return Err(DevdError::invalid(
            "plain `http://` remotes are refused; use `https://` or `ssh://`",
        ));
    }
    // scp-style: user@host:path
    if let Some((before, _after)) = url.split_once(':') {
        if before.contains('@') && !before.contains('/') {
            return Ok(());
        }
    }
    // Absolute local path.
    if Path::new(url).is_absolute() {
        return Ok(());
    }
    Err(DevdError::invalid(format!(
        "unsupported remote URL `{url}`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_validation_blocks_option_injection() {
        assert!(validate_ref("feature/auth").is_ok());
        assert!(validate_ref("main").is_ok());
        assert!(validate_ref("--upload-pack=touch /tmp/pwned").is_err());
        assert!(validate_ref("-x").is_err());
        assert!(validate_ref("bad..ref").is_err());
        assert!(validate_ref("bad name").is_err());
        assert!(validate_ref("bad~1").is_err());
        assert!(validate_ref("bad:ref").is_err());
        assert!(validate_ref("branch.lock").is_err());
        assert!(validate_ref("").is_err());
    }

    #[test]
    fn relpath_validation_blocks_traversal() {
        assert!(validate_relpath("src/main.rs").is_ok());
        assert!(validate_relpath("a/b/c.txt").is_ok());
        assert!(validate_relpath("../secrets").is_err());
        assert!(validate_relpath("a/../../b").is_err());
        assert!(validate_relpath("/etc/passwd").is_err());
        assert!(validate_relpath("C:/Windows/system32").is_err());
        assert!(validate_relpath("-oops").is_err());
    }

    #[test]
    fn remote_url_validation_rejects_cleartext_and_options() {
        assert!(validate_remote_url("https://github.com/a/b.git").is_ok());
        assert!(validate_remote_url("git@github.com:a/b.git").is_ok());
        assert!(validate_remote_url("ssh://git@host/a.git").is_ok());
        assert!(validate_remote_url("http://host/a.git").is_err());
        assert!(validate_remote_url("--upload-pack=evil").is_err());
        assert!(validate_remote_url("ftp://host/a.git").is_err());
    }

    #[test]
    fn remote_urls_may_not_embed_a_password() {
        // A password in the URL would be written to `.git/config` in plain text.
        assert!(validate_remote_url("https://user:ghp_secret@github.com/a/b.git").is_err());
        assert!(validate_remote_url("ssh://git:pw@host/a.git").is_err());
        // A bare username carries no secret and stays allowed, because that is
        // how SSH and many HTTPS hosts address a repository.
        assert!(validate_remote_url("https://user@github.com/a/b.git").is_ok());
        assert!(validate_remote_url("ssh://git@host/a.git").is_ok());
    }

    #[test]
    fn credential_config_keys_are_refused() {
        assert!(config_key_holds_credentials("user.password"));
        assert!(config_key_holds_credentials("http.extraHeader"));
        assert!(config_key_holds_credentials("my.token"));
        assert!(!config_key_holds_credentials("user.email"));
        assert!(!config_key_holds_credentials("core.autocrlf"));
    }

    #[test]
    fn certificate_verification_cannot_be_turned_off() {
        assert!(config_disables_verification("http.sslVerify", "false"));
        assert!(config_disables_verification("http.sslVerify", "no"));
        assert!(config_disables_verification("http.sslVerify", "0"));
        // A per-URL form is caught by the same suffix rule.
        assert!(config_disables_verification(
            "http.https://host/.sslVerify",
            "off"
        ));
        // Turning it back on, or an unrelated key, is allowed.
        assert!(!config_disables_verification("http.sslVerify", "true"));
        assert!(!config_disables_verification("http.proxy", "false"));
        assert!(!config_disables_verification("core.autocrlf", "false"));
    }

    #[test]
    fn a_failure_with_no_stderr_reports_what_git_said_on_stdout() {
        // "nothing to commit, working tree clean" arrives on stdout with exit
        // code 1. Reporting an empty message would hide the actual reason.
        let out = CommandOutput {
            code: 1,
            stdout: "nothing to commit, working tree clean\n".to_string(),
            stderr: String::new(),
            duration_ms: 0,
        };
        let err = command_failed(
            &["commit".to_string(), "-m".to_string(), "x".to_string()],
            &out,
        );
        let text = err.to_string();
        assert!(text.contains("nothing to commit"), "{text}");

        // Genuine silence still says something useful rather than nothing.
        let quiet = CommandOutput {
            code: 128,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
        };
        let err = command_failed(&["push".to_string()], &quiet);
        assert!(err.to_string().contains("128"), "{err}");
    }

    #[test]
    fn a_failure_still_redacts_credentials_from_either_stream() {
        let out = CommandOutput {
            code: 128,
            stdout: "fatal: could not read from https://user:ghp_secret@github.com/a/b.git"
                .to_string(),
            stderr: String::new(),
            duration_ms: 0,
        };
        let err = command_failed(&["fetch".to_string()], &out);
        assert!(!err.to_string().contains("ghp_secret"), "{err}");
    }

    #[test]
    fn urls_are_redacted_in_messages() {
        let msg = "fatal: could not read from https://user:ghp_secret@github.com/a/b.git";
        let red = redact_url(msg);
        assert!(!red.contains("ghp_secret"), "{red}");
        assert!(red.contains("https://[redacted]@github.com"));
    }

    #[test]
    fn config_key_validation() {
        assert!(validate_config_key("user.email").is_ok());
        assert!(validate_config_key("core.autocrlf").is_ok());
        assert!(validate_config_key("noDot").is_err());
        assert!(validate_config_key("--global").is_err());
    }

    #[test]
    fn stash_ref_validation() {
        assert!(validate_stash_ref("stash@{0}").is_ok());
        assert!(validate_stash_ref("stash@{12}").is_ok());
        assert!(validate_stash_ref("stash@{x}").is_err());
        assert!(validate_stash_ref("HEAD").is_err());
    }

    #[test]
    fn branch_header_parsing() {
        let mut s = RepoStatus {
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            upstream_gone: false,
            staged: vec![],
            unstaged: vec![],
            untracked: vec![],
            conflicted: vec![],
            clean: true,
            in_progress: None,
            unborn: false,
        };
        parse_branch_header("main...origin/main [ahead 2, behind 3]", &mut s);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!(s.ahead, 2);
        assert_eq!(s.behind, 3);

        parse_branch_header("feature", &mut s);
        assert_eq!(s.branch.as_deref(), Some("feature"));
        assert!(s.upstream.is_none());
    }
}
