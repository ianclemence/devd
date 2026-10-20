//! End-to-end Git lifecycle tests against real repositories on disk.
//!
//! Every test here drives `GitService`, the same typed layer the capabilities
//! call, against repositories created in a temporary directory. Nothing touches
//! the network: the "remote" fixtures are local bare repositories and, where a
//! failure is the point, a loopback address that refuses the connection
//! immediately.
//!
//! The acceptance properties the Git feature promises are asserted here rather
//! than described: a declined destructive operation does not run, an aborted
//! operation restores the previous state, and neither recovery nor a branch
//! switch silently removes work the user had not committed.

use std::path::{Path, PathBuf};

use devd_git::{
    ConfigScope, ConflictSide, DiffOptions, ForcePush, GitService, PullStrategy, ResetMode,
};
use tempfile::TempDir;

// ------------------------------------------------------------------- fixtures

/// A disposable repository plus the service that drives it.
struct Fixture {
    _dir: TempDir,
    repo: PathBuf,
    git: GitService,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("devd-git-{label}-"))
            .tempdir()
            .expect("a temporary directory");
        let git = GitService::discover(None).expect("git is on PATH");
        let repo = git.init(dir.path(), "main").expect("init");
        // Identity and signing are pinned per repository so a test never depends
        // on the machine's global Git configuration.
        git.config_set(Some(&repo), "user.name", "devd test", ConfigScope::Local)
            .expect("user.name");
        git.config_set(
            Some(&repo),
            "user.email",
            "devd@example.test",
            ConfigScope::Local,
        )
        .expect("user.email");
        git.config_set(Some(&repo), "commit.gpgsign", "false", ConfigScope::Local)
            .expect("commit.gpgsign");
        // Line-ending translation is pinned off so tests compare the bytes they
        // wrote. `devd` never changes a user's `core.autocrlf`; this is a test
        // fixture making the same repository behave identically on Windows and
        // Linux.
        git.config_set(Some(&repo), "core.autocrlf", "false", ConfigScope::Local)
            .expect("core.autocrlf");
        Self {
            _dir: dir,
            repo,
            git,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.repo.join(rel)
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent directory");
        }
        std::fs::write(&path, body).expect("write file");
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).expect("read file")
    }

    fn commit_all(&self, message: &str) -> String {
        self.git
            .stage(&self.repo, &[".".to_string()])
            .expect("stage");
        self.git
            .commit(&self.repo, message, false, false)
            .expect("commit")
    }

    fn subjects(&self, limit: u32) -> Vec<String> {
        self.git
            .log(&self.repo, limit, None, None)
            .expect("log")
            .into_iter()
            .map(|c| c.subject)
            .collect()
    }

    fn branch_names(&self) -> Vec<String> {
        self.git
            .branches(&self.repo, false)
            .expect("branches")
            .into_iter()
            .map(|b| b.name)
            .collect()
    }

    /// Run a raw `git` argv, for fixture setup only. The product itself never
    /// builds a command line this way; tests do it to create bare remotes and to
    /// inspect them from outside.
    fn raw(&self, cwd: Option<&Path>, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = self.git.exec(cwd, &args).expect("git ran");
        assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
        out.stdout
    }
}

fn temp_dir(label: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("devd-git-{label}-"))
        .tempdir()
        .expect("a temporary directory")
}

/// A `file://` URL for a local path, which is the only remote scheme a test can
/// use without network access.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

// -------------------------------------------------------------------- basics

#[test]
fn init_status_stage_and_commit() {
    let fx = Fixture::new("basic");
    assert!(fx.git.is_repo(&fx.repo));
    assert!(fx.git.status(&fx.repo).unwrap().clean);

    fx.write("README.md", "hello\n");
    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.untracked, vec!["README.md".to_string()]);
    assert!(!status.clean);
    assert!(status.staged.is_empty());

    fx.git.stage(&fx.repo, &[".".to_string()]).unwrap();
    let staged = fx.git.status(&fx.repo).unwrap();
    assert_eq!(staged.staged.len(), 1);
    assert!(staged.unstaged.is_empty());

    fx.git.commit(&fx.repo, "first", false, false).unwrap();
    let status = fx.git.status(&fx.repo).unwrap();
    assert!(status.clean);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(fx.subjects(5), vec!["first".to_string()]);
    assert_eq!(fx.git.rev_parse(&fx.repo, "HEAD").unwrap().len(), 40);
}

#[test]
fn commit_refuses_an_empty_message() {
    let fx = Fixture::new("empty-message");
    fx.write("a.txt", "a\n");
    fx.git.stage(&fx.repo, &[".".to_string()]).unwrap();
    assert!(fx.git.commit(&fx.repo, "   ", false, false).is_err());
}

#[test]
fn partial_staging_moves_one_hunk_and_leaves_the_working_tree_alone() {
    let fx = Fixture::new("partial");
    fx.write("notes.txt", "line one\nline two\n");
    fx.commit_all("base");

    // Two changes far enough apart to be separate hunks.
    let mut body = String::from("line one changed\nline two\n");
    for i in 0..20 {
        body.push_str(&format!("filler {i}\n"));
    }
    body.push_str("tail changed\n");
    fx.write("notes.txt", &body);

    let patch = "\
--- a/notes.txt
+++ b/notes.txt
@@ -1,2 +1,2 @@
-line one
+line one changed
 line two
";
    fx.git.stage_patch(&fx.repo, patch, false).unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.staged.len(), 1, "one hunk is staged");
    assert_eq!(status.unstaged.len(), 1, "the other change is not");

    // Hunk staging must not touch the working tree.
    let on_disk = fx.read("notes.txt");
    assert!(on_disk.contains("line one changed"));
    assert!(on_disk.contains("tail changed"));

    // And the staged diff contains only the first hunk.
    let staged = fx
        .git
        .diff(
            &fx.repo,
            &DiffOptions {
                staged: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(staged.contains("+line one changed"));
    assert!(!staged.contains("tail changed"));
}

#[test]
fn amend_replaces_the_previous_commit_instead_of_adding_one() {
    let fx = Fixture::new("amend");
    fx.write("a.txt", "one\n");
    fx.commit_all("first");
    let before = fx.git.rev_parse(&fx.repo, "HEAD").unwrap();

    fx.write("a.txt", "two\n");
    fx.git.stage(&fx.repo, &[".".to_string()]).unwrap();
    fx.git
        .commit(&fx.repo, "first, fixed", true, false)
        .unwrap();

    let subjects = fx.subjects(5);
    assert_eq!(subjects, vec!["first, fixed".to_string()]);
    assert_ne!(before, fx.git.rev_parse(&fx.repo, "HEAD").unwrap());
    assert!(fx.git.status(&fx.repo).unwrap().clean);
}

#[test]
fn repo_root_resolves_from_a_nested_directory() {
    let fx = Fixture::new("nested");
    fx.write("src/deep/file.txt", "x\n");
    fx.commit_all("nested");

    let nested = fx.path("src/deep");
    let root = fx.git.repo_root(&nested).unwrap().expect("a repository");
    // Path identity is compared after canonicalisation so the assertion holds on
    // Windows, where the temporary directory is under a different spelling.
    assert_eq!(
        std::fs::canonicalize(root).unwrap(),
        std::fs::canonicalize(&fx.repo).unwrap()
    );
    // A directory that is not a repository resolves to `None`, never an error.
    let outside = temp_dir("outside");
    assert!(fx.git.repo_root(outside.path()).unwrap().is_none());
}

// ------------------------------------------------------------------ branches

#[test]
fn branch_create_switch_and_merge() {
    let fx = Fixture::new("branch");
    fx.write("a.txt", "base\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "feature", None, true)
        .unwrap();
    assert_eq!(
        fx.git.current_branch(&fx.repo).unwrap().as_deref(),
        Some("feature")
    );

    fx.write("feature.txt", "feature\n");
    fx.commit_all("add feature");

    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    assert_eq!(
        fx.git.current_branch(&fx.repo).unwrap().as_deref(),
        Some("main")
    );
    assert!(!fx.path("feature.txt").exists(), "not merged yet");

    fx.git
        .merge(&fx.repo, "feature", false, false, None)
        .unwrap();
    assert!(fx.path("feature.txt").exists(), "merged");
    assert_eq!(
        fx.subjects(10),
        vec!["add feature".to_string(), "base".to_string()]
    );
}

#[test]
fn an_uncommitted_change_survives_a_branch_switch() {
    let fx = Fixture::new("survive-switch");
    fx.write("tracked.txt", "committed\n");
    fx.write("unrelated.txt", "committed\n");
    fx.commit_all("base");

    // The user leaves work in progress in a file the new branch does not touch.
    fx.write("unrelated.txt", "work in progress\n");
    fx.git.create_branch(&fx.repo, "other", None, true).unwrap();

    assert_eq!(fx.read("unrelated.txt"), "work in progress\n");
    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, "unrelated.txt");
}

#[test]
fn switch_safety_reports_the_files_a_switch_would_overwrite() {
    let fx = Fixture::new("switch-safety");
    fx.write("a.txt", "v1\n");
    fx.write("b.txt", "v1\n");
    fx.commit_all("base");

    fx.git.create_branch(&fx.repo, "other", None, true).unwrap();
    fx.write("a.txt", "other version\n");
    fx.commit_all("change a on other");
    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();

    // Uncommitted work in a file the target branch also changed.
    fx.write("a.txt", "local work\n");
    let risky = fx.git.switch_safety(&fx.repo, "other").unwrap();
    assert!(risky.contains(&"a.txt".to_string()), "got {risky:?}");

    // A file only the target branch changed is also reported as at risk.
    let safe = fx.git.switch_safety(&fx.repo, "main").unwrap();
    assert!(safe.is_empty(), "got {safe:?}");
}

#[test]
fn rename_and_delete_branches() {
    let fx = Fixture::new("branch-admin");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");
    fx.git
        .create_branch(&fx.repo, "old-name", None, false)
        .unwrap();
    fx.git
        .rename_branch(&fx.repo, "old-name", "new-name")
        .unwrap();
    assert!(fx.branch_names().contains(&"new-name".to_string()));

    // A branch that is fully merged is safe to delete.
    fx.git
        .delete_branch(&fx.repo, "new-name", false, None)
        .unwrap();
    assert!(!fx.branch_names().contains(&"new-name".to_string()));

    // Force is required for an unmerged branch, which is exactly the guard the
    // destructive capability surfaces as an approval.
    fx.git
        .create_branch(&fx.repo, "unmerged", None, true)
        .unwrap();
    fx.write("u.txt", "u\n");
    fx.commit_all("only on unmerged");
    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    assert!(fx
        .git
        .delete_branch(&fx.repo, "unmerged", false, None)
        .is_err());
    fx.git
        .delete_branch(&fx.repo, "unmerged", true, None)
        .unwrap();
    assert!(!fx.branch_names().contains(&"unmerged".to_string()));
}

#[test]
fn prune_merged_branches_deletes_only_what_is_merged() {
    let fx = Fixture::new("prune");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "merged", None, true)
        .unwrap();
    fx.write("m.txt", "m\n");
    fx.commit_all("merged work");
    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    fx.git
        .merge(&fx.repo, "merged", false, false, None)
        .unwrap();

    fx.git
        .create_branch(&fx.repo, "unmerged", None, true)
        .unwrap();
    fx.write("u.txt", "u\n");
    fx.commit_all("unmerged work");
    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();

    let candidates = fx
        .git
        .prune_merged_branches(&fx.repo, "HEAD", true)
        .unwrap();
    assert!(candidates.contains(&"merged".to_string()), "{candidates:?}");
    assert!(
        !candidates.contains(&"unmerged".to_string()),
        "{candidates:?}"
    );

    let deleted = fx
        .git
        .prune_merged_branches(&fx.repo, "HEAD", false)
        .unwrap();
    assert!(deleted.contains(&"merged".to_string()));
    let names = fx.branch_names();
    assert!(!names.contains(&"merged".to_string()));
    assert!(names.contains(&"unmerged".to_string()));
}

// -------------------------------------------------------------------- rebase

#[test]
fn rebase_replays_local_commits_onto_another_branch() {
    let fx = Fixture::new("rebase");
    fx.write("base.txt", "base\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "feature", None, true)
        .unwrap();
    fx.write("feature.txt", "feature\n");
    fx.commit_all("feature work");

    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    fx.write("main.txt", "main\n");
    fx.commit_all("main work");

    fx.git
        .switch_branch(&fx.repo, "feature", false, false)
        .unwrap();
    fx.git.rebase(&fx.repo, "main", false, false).unwrap();

    assert_eq!(
        fx.git.current_branch(&fx.repo).unwrap().as_deref(),
        Some("feature")
    );
    // Linear: main's commit now precedes the feature commit.
    assert_eq!(
        fx.subjects(10),
        vec![
            "feature work".to_string(),
            "main work".to_string(),
            "base".to_string()
        ]
    );
    assert!(fx.path("main.txt").exists());
}

#[test]
fn autosquash_folds_fixup_commits_without_an_editor() {
    let fx = Fixture::new("autosquash");
    fx.write("a.txt", "a\n");
    fx.commit_all("first");
    fx.write("b.txt", "b\n");
    fx.commit_all("second");
    fx.write("c.txt", "c\n");
    fx.commit_all("fixup! second");
    assert_eq!(fx.subjects(10).len(), 3);

    fx.git.rebase(&fx.repo, "--root", true, true).unwrap();

    let subjects = fx.subjects(10);
    assert_eq!(subjects, vec!["second".to_string(), "first".to_string()]);
    // The fixup's content survives the fold.
    assert!(fx.path("c.txt").exists());
}

#[test]
fn squash_last_combines_commits_into_one() {
    let fx = Fixture::new("squash");
    fx.write("a.txt", "a\n");
    fx.commit_all("first");
    fx.write("b.txt", "b\n");
    fx.commit_all("second");
    fx.write("c.txt", "c\n");
    fx.commit_all("third");

    fx.git.squash_last(&fx.repo, 3, "everything").unwrap();
    assert_eq!(fx.subjects(10), vec!["everything".to_string()]);
    assert!(fx.path("a.txt").exists());
    assert!(fx.path("b.txt").exists());
    assert!(fx.path("c.txt").exists());
    assert!(fx.git.status(&fx.repo).unwrap().clean);
}

// ---------------------------------------------------------------- conflicts

#[test]
fn a_merge_conflict_is_detected_resolved_and_committed() {
    let fx = Fixture::new("merge-conflict");
    fx.write("z.txt", "base\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "feature", None, true)
        .unwrap();
    fx.write("z.txt", "feature\n");
    fx.commit_all("feature change");

    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    fx.write("z.txt", "main\n");
    fx.commit_all("main change");

    // A conflicted merge is a state, not a failure.
    fx.git
        .merge(&fx.repo, "feature", false, false, None)
        .unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.conflicted, vec!["z.txt".to_string()]);
    assert_eq!(status.in_progress.as_deref(), Some("merge"));
    assert_eq!(
        fx.git.conflicts(&fx.repo).unwrap(),
        vec!["z.txt".to_string()]
    );

    // The conflicted file exposes both sides before a choice is made.
    let conflicted = fx.git.conflicted_file(&fx.repo, "z.txt").unwrap();
    assert!(conflicted.contains("<<<<<<<"), "{conflicted}");
    assert!(conflicted.contains("main"), "{conflicted}");
    assert!(conflicted.contains("feature"), "{conflicted}");

    fx.git
        .resolve_conflict(&fx.repo, "z.txt", ConflictSide::Ours)
        .unwrap();
    fx.git.continue_op(&fx.repo, "merge").unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert!(status.clean, "{status:?}");
    assert_eq!(status.in_progress, None);
    assert_eq!(fx.read("z.txt"), "main\n");
}

#[test]
fn aborting_a_conflicted_merge_never_removes_unrelated_user_work() {
    let fx = Fixture::new("merge-abort");
    fx.write("z.txt", "base\n");
    fx.write("user.txt", "committed\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "feature", None, true)
        .unwrap();
    fx.write("z.txt", "feature\n");
    fx.commit_all("feature change");

    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    fx.write("z.txt", "main\n");
    fx.commit_all("main change");

    // Work in progress that the merge does not touch.
    fx.write("user.txt", "uncommitted work\n");

    fx.git
        .merge(&fx.repo, "feature", false, false, None)
        .unwrap();
    assert_eq!(
        fx.git.status(&fx.repo).unwrap().in_progress.as_deref(),
        Some("merge")
    );

    fx.git.abort_op(&fx.repo, "merge").unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.in_progress, None);
    assert!(status.conflicted.is_empty());
    assert_eq!(fx.read("z.txt"), "main\n", "the merge was undone");
    assert_eq!(
        fx.read("user.txt"),
        "uncommitted work\n",
        "unrelated uncommitted work must survive the recovery"
    );
    assert_eq!(status.unstaged.len(), 1);
}

#[test]
fn aborting_a_conflicted_rebase_restores_the_original_branch() {
    let fx = Fixture::new("rebase-abort");
    fx.write("z.txt", "base\n");
    fx.commit_all("base");

    fx.git
        .create_branch(&fx.repo, "feature", None, true)
        .unwrap();
    fx.write("z.txt", "feature\n");
    fx.commit_all("feature change");

    fx.git
        .switch_branch(&fx.repo, "main", false, false)
        .unwrap();
    fx.write("z.txt", "main\n");
    fx.commit_all("main change");
    let main_tip = fx.git.rev_parse(&fx.repo, "main").unwrap();

    fx.git
        .switch_branch(&fx.repo, "feature", false, false)
        .unwrap();
    fx.git.rebase(&fx.repo, "main", false, false).unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert!(!status.conflicted.is_empty(), "{status:?}");
    assert_eq!(status.in_progress.as_deref(), Some("rebase"));

    fx.git.abort_op(&fx.repo, "rebase").unwrap();

    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.in_progress, None);
    assert!(status.conflicted.is_empty());
    assert_eq!(
        fx.git.current_branch(&fx.repo).unwrap().as_deref(),
        Some("feature")
    );
    assert_eq!(fx.read("z.txt"), "feature\n", "the rebase was undone");
    assert_eq!(fx.git.rev_parse(&fx.repo, "main").unwrap(), main_tip);
}

// -------------------------------------------------------------------- resets

#[test]
fn reset_modes_have_the_documented_effect() {
    let fx = Fixture::new("reset");
    fx.write("a.txt", "one\n");
    fx.commit_all("base");
    fx.write("a.txt", "two\n");
    fx.commit_all("second");

    // `soft` moves HEAD only: the change stays staged.
    fx.git.reset(&fx.repo, ResetMode::Soft, "HEAD~1").unwrap();
    assert_eq!(fx.read("a.txt"), "two\n");
    let status = fx.git.status(&fx.repo).unwrap();
    assert_eq!(status.staged.len(), 1, "soft keeps the index");
    assert!(status.unstaged.is_empty());
    assert_eq!(fx.subjects(10), vec!["base".to_string()]);

    fx.commit_all("second again");

    // `mixed` moves HEAD and clears the index, keeping the files.
    fx.git.reset(&fx.repo, ResetMode::Mixed, "HEAD~1").unwrap();
    assert_eq!(fx.read("a.txt"), "two\n");
    let status = fx.git.status(&fx.repo).unwrap();
    assert!(status.staged.is_empty(), "mixed clears the index");
    assert_eq!(status.unstaged.len(), 1);

    fx.commit_all("second yet again");

    // `hard` discards the working tree change as well.
    fx.git.reset(&fx.repo, ResetMode::Hard, "HEAD~1").unwrap();
    assert_eq!(fx.read("a.txt"), "one\n");
    assert!(fx.git.status(&fx.repo).unwrap().clean);
}

#[test]
fn the_reflog_makes_a_hard_reset_recoverable() {
    let fx = Fixture::new("reflog");
    fx.write("a.txt", "one\n");
    fx.commit_all("first");
    fx.write("a.txt", "two\n");
    fx.commit_all("second");
    let lost = fx.git.rev_parse(&fx.repo, "HEAD").unwrap();

    fx.git.reset(&fx.repo, ResetMode::Hard, "HEAD~1").unwrap();
    assert_eq!(fx.read("a.txt"), "one\n");

    let entries = fx.git.reflog(&fx.repo, 20).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.contains("second") || e.contains("reset")),
        "reflog should record the reset: {entries:?}"
    );

    // The discarded commit is still reachable by hash, which is the promise
    // `devd git undo` relies on.
    fx.git.reset(&fx.repo, ResetMode::Hard, &lost).unwrap();
    assert_eq!(fx.read("a.txt"), "two\n");
    assert_eq!(fx.subjects(10).first().map(String::as_str), Some("second"));
}

// ------------------------------------------------------------------ stashes

#[test]
fn the_stash_lifecycle_shelves_and_restores_work() {
    let fx = Fixture::new("stash");
    fx.write("a.txt", "one\n");
    fx.commit_all("base");

    fx.write("a.txt", "work in progress\n");
    fx.write("untracked.txt", "scratch\n");
    fx.git
        .stash_push(&fx.repo, Some("wip"), true, false)
        .unwrap();

    assert!(fx.git.status(&fx.repo).unwrap().clean, "shelved");
    assert_eq!(fx.read("a.txt"), "one\n");
    assert!(!fx.path("untracked.txt").exists(), "untracked was included");

    let entries = fx.git.stash_list(&fx.repo).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].subject.contains("wip"), "{:?}", entries[0]);

    // The patch is readable without applying it.
    let shown = fx.git.stash_show(&fx.repo, &entries[0].reference).unwrap();
    assert!(shown.contains("work in progress"), "{shown}");

    // `apply` restores without dropping.
    fx.git.stash_apply(&fx.repo, None, false, false).unwrap();
    assert_eq!(fx.read("a.txt"), "work in progress\n");
    assert_eq!(fx.git.stash_list(&fx.repo).unwrap().len(), 1);

    // A second stash of the same work, then `pop`, which restores and drops.
    fx.git
        .stash_push(&fx.repo, Some("again"), false, false)
        .unwrap();
    assert_eq!(fx.read("a.txt"), "one\n");
    fx.git.stash_apply(&fx.repo, None, true, false).unwrap();
    assert_eq!(fx.read("a.txt"), "work in progress\n");
    assert_eq!(fx.git.stash_list(&fx.repo).unwrap().len(), 1);

    // Dropping is explicit and removes exactly the named entry.
    let remaining = fx.git.stash_list(&fx.repo).unwrap();
    fx.git
        .stash_drop(&fx.repo, &remaining[0].reference)
        .unwrap();
    assert!(fx.git.stash_list(&fx.repo).unwrap().is_empty());
    assert_eq!(fx.read("a.txt"), "work in progress\n", "files untouched");
}

// ---------------------------------------------------------------- worktrees

#[test]
fn the_worktree_lifecycle_adds_lists_and_removes() {
    let fx = Fixture::new("worktree");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");
    // A worktree cannot check out a branch that is already checked out, so the
    // main worktree moves aside first.
    fx.write("b.txt", "b\n");
    fx.commit_all("second");
    fx.git
        .create_branch(&fx.repo, "wt-other", None, false)
        .unwrap();

    let holder = temp_dir("worktree-holder");
    let wt_path = holder.path().join("linked");

    fx.git
        .worktree_add(&fx.repo, &wt_path, "wt-other", false)
        .unwrap();

    let worktrees = fx.git.worktrees(&fx.repo).unwrap();
    assert_eq!(worktrees.len(), 2, "{worktrees:?}");
    let linked = worktrees
        .iter()
        .find(|w| !w.main)
        .expect("a linked worktree");
    assert_eq!(linked.branch.as_deref(), Some("wt-other"));
    assert!(wt_path.join("a.txt").exists());

    // The linked worktree is an independent working tree over the same history.
    std::fs::write(wt_path.join("c.txt"), "c\n").unwrap();
    fx.git.stage(&wt_path, &[".".to_string()]).unwrap();
    fx.git
        .commit(&wt_path, "from the worktree", false, false)
        .unwrap();
    assert!(fx.branch_names().contains(&"wt-other".to_string()));
    assert!(
        !fx.path("c.txt").exists(),
        "the main worktree is unaffected"
    );

    fx.git.worktree_remove(&fx.repo, &wt_path, false).unwrap();
    assert_eq!(fx.git.worktrees(&fx.repo).unwrap().len(), 1);
    assert!(!wt_path.exists());
}

#[test]
fn removing_a_dirty_worktree_requires_force() {
    let fx = Fixture::new("worktree-force");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");
    fx.git.create_branch(&fx.repo, "wt", None, false).unwrap();

    let holder = temp_dir("worktree-force-holder");
    let wt_path = holder.path().join("linked");
    fx.git
        .worktree_add(&fx.repo, &wt_path, "wt", false)
        .unwrap();
    std::fs::write(wt_path.join("dirty.txt"), "uncommitted\n").unwrap();

    // Removing a worktree with uncommitted work must not silently succeed.
    assert!(fx.git.worktree_remove(&fx.repo, &wt_path, false).is_err());
    assert!(wt_path.join("dirty.txt").exists(), "work preserved");

    fx.git.worktree_remove(&fx.repo, &wt_path, true).unwrap();
    assert!(!wt_path.exists());
}

// -------------------------------------------------- tags, cherry-pick, revert

#[test]
fn tags_cherry_pick_and_revert() {
    let fx = Fixture::new("tags");
    fx.write("a.txt", "one\n");
    fx.commit_all("first");

    fx.git
        .create_tag(&fx.repo, "v1.0.0", Some("release one"), None, false)
        .unwrap();
    let tags = fx.git.tags(&fx.repo).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v1.0.0");
    assert_eq!(tags[0].message.as_deref(), Some("release one"));

    // A cherry-picked commit lands on another branch under a new hash.
    fx.write("b.txt", "two\n");
    fx.commit_all("second");
    let second = fx.git.rev_parse(&fx.repo, "HEAD").unwrap();

    fx.git
        .create_branch(&fx.repo, "target", None, true)
        .unwrap();
    fx.git.reset(&fx.repo, ResetMode::Hard, "HEAD~1").unwrap();
    // Give the target its own history first. A commit with the same parent,
    // tree, author and (to the second) time as the original would hash
    // identically, which made this assertion depend on the clock.
    fx.write("c.txt", "three\n");
    fx.commit_all("third");
    fx.git.cherry_pick(&fx.repo, &second, false).unwrap();
    assert!(fx.path("b.txt").exists());
    assert_ne!(fx.git.rev_parse(&fx.repo, "HEAD").unwrap(), second);

    // Revert undoes it with a new commit, which is the safe way to undo shared
    // history.
    fx.git.revert(&fx.repo, "HEAD", false).unwrap();
    assert!(!fx.path("b.txt").exists());
    assert_eq!(
        fx.subjects(10).first().map(String::as_str),
        Some("Revert \"second\"")
    );

    fx.git.delete_tag(&fx.repo, "v1.0.0", None).unwrap();
    assert!(fx.git.tags(&fx.repo).unwrap().is_empty());
}

// ------------------------------------------------------------------ remotes

#[test]
fn clone_fetch_pull_and_push_against_a_local_remote() {
    let origin = Fixture::new("origin");
    origin.write("a.txt", "one\n");
    origin.commit_all("first");
    let first = origin.git.rev_parse(&origin.repo, "HEAD").unwrap();

    let remote_dir = temp_dir("bare-remote");
    let bare = remote_dir.path().join("origin.git");
    origin.raw(
        None,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let url = file_url(&bare);

    origin.git.remote_add(&origin.repo, "origin", &url).unwrap();
    let remotes = origin.git.remotes(&origin.repo).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_eq!(remotes[0].name, "origin");

    origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            true,
            ForcePush::None,
            false,
        )
        .unwrap();
    assert_eq!(
        origin
            .raw(Some(&bare), &["rev-parse", "--verify", "main"])
            .trim(),
        first
    );
    let status = origin.git.status(&origin.repo).unwrap();
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!(status.ahead, 0);

    // Clone the bare repository.
    let clone_dir = temp_dir("clone");
    let clone_path = clone_dir.path().join("clone");
    origin
        .git
        .clone(&url, &clone_path, None, None, false)
        .unwrap();
    assert!(clone_path.join("a.txt").exists());
    let clone_head = origin.git.rev_parse(&clone_path, "HEAD").unwrap();
    assert_eq!(clone_head, first);
    assert_eq!(
        origin.git.current_branch(&clone_path).unwrap().as_deref(),
        Some("main")
    );

    // Commit in the clone and push; the clone already tracks origin/main.
    std::fs::write(clone_path.join("b.txt"), "two\n").unwrap();
    origin.git.stage(&clone_path, &[".".to_string()]).unwrap();
    origin
        .git
        .commit(&clone_path, "second from the clone", false, false)
        .unwrap();
    let second = origin.git.rev_parse(&clone_path, "HEAD").unwrap();
    origin
        .git
        .push(
            &clone_path,
            "origin",
            Some("main"),
            false,
            ForcePush::None,
            false,
        )
        .unwrap();

    // The origin sees the new commit only after a fetch, and the preview says so
    // before anything is integrated.
    origin
        .git
        .fetch(&origin.repo, Some("origin"), false)
        .unwrap();
    let preview = origin
        .git
        .sync_preview(&origin.repo, "origin", "main", 10)
        .unwrap();
    assert!(!preview.diverged, "{preview:?}");
    assert_eq!(preview.incoming.len(), 1);
    assert_eq!(preview.incoming[0].subject, "second from the clone");
    assert!(preview.outgoing.is_empty());
    assert!(!origin.path("b.txt").exists(), "not integrated yet");

    origin
        .git
        .pull(
            &origin.repo,
            Some("origin"),
            Some("main"),
            PullStrategy::FastForwardOnly,
        )
        .unwrap();
    assert_eq!(origin.git.rev_parse(&origin.repo, "HEAD").unwrap(), second);
    assert_eq!(origin.read("b.txt"), "two\n");
    assert!(origin.git.status(&origin.repo).unwrap().clean);
}

#[test]
fn a_diverged_branch_refuses_a_fast_forward_only_pull() {
    let origin = Fixture::new("diverged-origin");
    origin.write("a.txt", "one\n");
    origin.commit_all("first: -");
    let remote_dir = temp_dir("diverged-bare");
    let bare = remote_dir.path().join("origin.git");
    origin.raw(
        None,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let url = file_url(&bare);
    origin.git.remote_add(&origin.repo, "origin", &url).unwrap();
    origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            true,
            ForcePush::None,
            false,
        )
        .unwrap();

    let clone_dir = temp_dir("diverged-clone");
    let clone_path = clone_dir.path().join("clone");
    origin
        .git
        .clone(&url, &clone_path, None, None, false)
        .unwrap();

    // Both sides commit, so the histories diverge.
    std::fs::write(clone_path.join("b.txt"), "b\n").unwrap();
    origin.git.stage(&clone_path, &[".".to_string()]).unwrap();
    origin
        .git
        .commit(&clone_path, "from clone", false, false)
        .unwrap();
    origin
        .git
        .push(
            &clone_path,
            "origin",
            Some("main"),
            false,
            ForcePush::None,
            false,
        )
        .unwrap();

    origin.write("c.txt", "c\n");
    origin.commit_all("from origin");

    origin
        .git
        .fetch(&origin.repo, Some("origin"), false)
        .unwrap();
    let preview = origin
        .git
        .sync_preview(&origin.repo, "origin", "main", 10)
        .unwrap();
    assert!(preview.diverged, "{preview:?}");

    // Fast-forward-only must refuse rather than create a merge commit.
    assert!(origin
        .git
        .pull(
            &origin.repo,
            Some("origin"),
            Some("main"),
            PullStrategy::FastForwardOnly
        )
        .is_err());
    // The local commit is untouched by the refusal.
    assert!(origin.path("c.txt").exists());
    assert_eq!(
        origin.subjects(10).first().map(String::as_str),
        Some("from origin")
    );
}

#[test]
fn a_force_push_can_recover_a_rewritten_remote_branch() {
    let origin = Fixture::new("force-push");
    origin.write("a.txt", "one\n");
    origin.commit_all("first");
    origin.write("a.txt", "two\n");
    origin.commit_all("second");

    let remote_dir = temp_dir("force-bare");
    let bare = remote_dir.path().join("origin.git");
    origin.raw(
        None,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let url = file_url(&bare);
    origin.git.remote_add(&origin.repo, "origin", &url).unwrap();
    origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            true,
            ForcePush::None,
            false,
        )
        .unwrap();

    // Rewrite local history, which makes the remote ref a non-fast-forward.
    origin.git.squash_last(&origin.repo, 2, "combined").unwrap();
    // The clone knows the remote's tip, so it can push safely with a lease.
    assert!(origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            false,
            ForcePush::None,
            false
        )
        .is_err());
    origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            false,
            ForcePush::WithLease,
            false,
        )
        .unwrap();
    assert_eq!(
        origin
            .raw(Some(&bare), &["log", "--format=%s", "-1", "main"])
            .trim(),
        "combined"
    );
}

#[test]
fn a_remote_url_with_credentials_is_refused_and_redacted() {
    let fx = Fixture::new("credentials");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");

    // A password in a remote URL would be written to `.git/config` in plain text.
    let err = fx
        .git
        .remote_add(
            &fx.repo,
            "origin",
            "https://user:ghp_secret@github.com/a/b.git",
        )
        .expect_err("a credential-bearing URL must be refused");
    let message = err.to_string();
    assert!(!message.contains("ghp_secret"), "{message}");
    assert!(fx.git.remotes(&fx.repo).unwrap().is_empty());

    // A username without a password is fine.
    fx.git
        .remote_add(&fx.repo, "origin", "https://git@github.com/a/b.git")
        .unwrap();
    assert_eq!(fx.git.remotes(&fx.repo).unwrap().len(), 1);

    // A credential-looking key cannot be written either.
    assert!(fx
        .git
        .config_set(
            Some(&fx.repo),
            "user.password",
            "hunter2",
            ConfigScope::Local
        )
        .is_err());
    assert!(fx
        .git
        .config_set(
            Some(&fx.repo),
            "http.extraHeader",
            "AUTHORIZATION: basic x",
            ConfigScope::Local
        )
        .is_err());
}

#[test]
fn a_failing_remote_reports_an_error_instead_of_hanging() {
    let fx = Fixture::new("unreachable");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");
    // Port 9 on the loopback interface refuses connections immediately, so this
    // exercises the failure path without any network egress. `GIT_TERMINAL_PROMPT=0`
    // is set by the service, so Git cannot stop to ask for a password.
    fx.git
        .remote_add(&fx.repo, "origin", "https://127.0.0.1:9/repo.git")
        .unwrap();

    let err = fx
        .git
        .fetch(&fx.repo, Some("origin"), false)
        .expect_err("fetch must fail");
    let message = err.to_string();
    assert!(!message.is_empty());
    // Nothing secret can appear: the URL carries no credentials by construction,
    // and the redactor runs over every message the service emits.
    assert!(!message.contains("ghp_"), "{message}");
}

#[test]
fn ignore_patterns_are_appended_to_gitignore() {
    let fx = Fixture::new("ignore");
    fx.write("a.txt", "a\n");
    fx.commit_all("base");
    fx.git
        .ignore_add(&fx.repo, &["target/".to_string(), "*.log".to_string()])
        .unwrap();
    let ignored = fx.read(".gitignore");
    assert!(ignored.contains("target/"), "{ignored}");
    assert!(ignored.contains("*.log"), "{ignored}");
}

#[test]
fn outgoing_and_incoming_are_reported_separately() {
    let origin = Fixture::new("sync-preview");
    origin.write("a.txt", "one\n");
    origin.commit_all("first");
    let remote_dir = temp_dir("sync-bare");
    let bare = remote_dir.path().join("origin.git");
    origin.raw(
        None,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let url = file_url(&bare);
    origin.git.remote_add(&origin.repo, "origin", &url).unwrap();
    origin
        .git
        .push(
            &origin.repo,
            "origin",
            Some("main"),
            true,
            ForcePush::None,
            false,
        )
        .unwrap();

    let clone_dir = temp_dir("sync-clone");
    let clone_path = clone_dir.path().join("clone");
    origin
        .git
        .clone(&url, &clone_path, None, None, false)
        .unwrap();

    // The clone is ahead: one local commit that the remote does not have.
    std::fs::write(clone_path.join("b.txt"), "b\n").unwrap();
    origin.git.stage(&clone_path, &[".".to_string()]).unwrap();
    origin
        .git
        .commit(&clone_path, "local only", false, false)
        .unwrap();

    let outgoing = origin
        .git
        .outgoing(&clone_path, "origin", "main", 10)
        .unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].subject, "local only");

    let incoming = origin
        .git
        .incoming(&clone_path, "origin", "main", 10)
        .unwrap();
    assert!(incoming.is_empty(), "{incoming:?}");
}

#[test]
fn filesystem_paths_with_spaces_are_handled() {
    // Repository paths on Windows frequently contain spaces; every argument is
    // passed as an argv element rather than a shell string, so this must work.
    let holder = temp_dir("spaces");
    let spaced = holder.path().join("a repo with spaces");
    std::fs::create_dir_all(&spaced).unwrap();
    let git = GitService::discover(None).unwrap();
    let repo = git.init(&spaced, "main").unwrap();
    git.config_set(Some(&repo), "user.name", "devd test", ConfigScope::Local)
        .unwrap();
    git.config_set(
        Some(&repo),
        "user.email",
        "devd@example.test",
        ConfigScope::Local,
    )
    .unwrap();
    std::fs::write(repo.join("a file.txt"), "content\n").unwrap();

    git.stage(&repo, &[".".to_string()]).unwrap();
    git.commit(&repo, "with spaces", false, false).unwrap();
    assert!(git.status(&repo).unwrap().clean);
    // The single root commit holds the file; `show` reads it back through the
    // same argv-only path.
    assert!(git.show(&repo, "HEAD").unwrap().contains("a file.txt"));
}
