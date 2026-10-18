//! Git capabilities.
//!
//! Every Git operation `devd` offers is declared here, exactly once, and is
//! therefore reachable - with identical permissions, auditing and approval rules
//! - from the CLI, the TUI, the desktop GUI, the HTTP API and the MCP server.
//!
//! Three rules shape this module:
//!
//! 1. **No shell.** Arguments are typed structs that become argument arrays. A
//!    branch named `--upload-pack=...` fails validation rather than becoming an
//!    option.
//! 2. **Tier reflects risk.** Inspection is [`Tier::Read`]; ordinary edits are
//!    [`Tier::Write`]; operations that rewrite history, discard work or touch a
//!    remote are [`Tier::Destructive`] or [`Tier::Remote`] and therefore demand
//!    an approval a human had to grant.
//! 3. **Force push is its own capability.** It is not a flag on `git.push`, so
//!    there is no way to reach a forced update without passing through the
//!    approval gate that its tier enforces.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
use devd_core::state::Repo;
use devd_core::util;
use devd_core::{DevdError, Registry, Result};

use crate::{
    BisectVerdict, ConfigScope, ConflictSide, DiffOptions, ForcePush, GitService, PullStrategy,
    ResetMode,
};

// -------------------------------------------------------------------- arguments

macro_rules! args {
    ($( $(#[$m:meta])* $name:ident { $( $f:ident : $t:ty ),* $(,)? } )*) => {
        $(
            $(#[$m])*
            #[derive(Debug, Clone, Deserialize, JsonSchema)]
            pub struct $name {
                $( pub $f: $t, )*
            }
        )*
    };
}

args! {
    /// No arguments.
    NoArgs {}
    /// Identify a repository by any path inside its working tree.
    RepoArgs { repo: String }
    /// A repository plus one or more repository-relative paths.
    RepoPathsArgs { repo: String, paths: Vec<String> }
    /// Options for producing a diff.
    DiffArgs {
        repo: String,
        staged: Option<bool>,
        path: Option<String>,
        rev: Option<String>,
        stat: Option<bool>,
        context: Option<u32>,
    }
    /// Options for creating a commit.
    CommitArgs {
        repo: String,
        message: String,
        amend: Option<bool>,
        allow_empty: Option<bool>,
    }
    /// Options for reading history.
    LogArgs { repo: String, limit: Option<u32>, rev: Option<String>, path: Option<String> }
    /// A repository and a single revision.
    RevArgs { repo: String, rev: String }
    /// A repository and a file.
    BlameArgs { repo: String, path: String }
    /// A history search.
    SearchArgs {
        repo: String,
        query: String,
        author: Option<String>,
        path: Option<String>,
        limit: Option<u32>,
    }
    /// A patch to apply to the index, used for hunk-level staging.
    StagePatchArgs { repo: String, patch: String, reverse: Option<bool> }
    /// Options for creating a branch.
    BranchCreateArgs { repo: String, name: String, start_point: Option<String>, checkout: Option<bool> }
    /// Options for listing branches.
    BranchListArgs { repo: String, all: Option<bool> }
    /// Options for deleting a branch.
    BranchDeleteArgs { repo: String, name: String, force: Option<bool>, remote: Option<String> }
    /// Options for renaming a branch.
    BranchRenameArgs { repo: String, from: String, to: String }
    /// Options for setting a branch upstream.
    BranchUpstreamArgs { repo: String, branch: String, upstream: String }
    /// Options for switching branches.
    SwitchArgs { repo: String, name: String, create: Option<bool>, force: Option<bool> }
    /// A repository and the branch we intend to switch to.
    SwitchCheckArgs { repo: String, target: String }
    /// Options for a merge.
    MergeArgs {
        repo: String,
        branch: String,
        no_ff: Option<bool>,
        squash: Option<bool>,
        message: Option<String>,
    }
    /// Options for a rebase.
    RebaseArgs { repo: String, onto: String, autosquash: Option<bool>, interactive: Option<bool> }
    /// A repository and a mode.
    ModeArgs { repo: String, mode: ResetMode, rev: String }
    /// Options for reverting a commit.
    RevertArgs { repo: String, rev: String, no_commit: Option<bool> }
    /// A repository and the operation to continue or abort.
    OpArgs { repo: String, op: String }
    /// A repository and how to resolve a conflict.
    ResolveArgs { repo: String, path: String, side: ConflictSide }
    /// Options for adding a remote.
    RemoteAddArgs { repo: String, name: String, url: String }
    /// A repository and a remote name.
    RemoteNameArgs { repo: String, name: String }
    /// Options for renaming a remote.
    RemoteRenameArgs { repo: String, from: String, to: String }
    /// Options for fetching.
    FetchArgs { repo: String, remote: Option<String>, prune: Option<bool> }
    /// Options for previewing or describing synchronisation.
    SyncArgs { repo: String, remote: Option<String>, branch: Option<String> }
    /// Options for pulling.
    PullArgs {
        repo: String,
        remote: Option<String>,
        branch: Option<String>,
        strategy: Option<PullStrategy>,
    }
    /// Options for pushing.
    PushArgs { repo: String, remote: Option<String>, branch: Option<String>, set_upstream: Option<bool>, tags: Option<bool> }
    /// Options for creating a tag.
    TagCreateArgs { repo: String, name: String, message: Option<String>, rev: Option<String>, force: Option<bool> }
    /// A repository and a tag name.
    TagDeleteArgs { repo: String, name: String, remote: Option<String> }
    /// Options for listing the reflog.
    ReflogArgs { repo: String, limit: Option<u32> }
    /// Options for squashing recent commits.
    SquashArgs { repo: String, count: u32, message: String }
    /// Options for starting a bisect.
    BisectStartArgs { repo: String, bad: String, good: String }
    /// A repository and a bisect verdict.
    BisectMarkArgs { repo: String, verdict: BisectVerdict }
    /// Options for creating a stash.
    StashPushArgs { repo: String, message: Option<String>, include_untracked: Option<bool>, keep_index: Option<bool> }
    /// Options for applying or popping a stash.
    StashRefArgs { repo: String, reference: Option<String>, pop: Option<bool>, index: Option<bool> }
    /// A repository and a stash entry.
    StashDropArgs { repo: String, reference: String }
    /// Options for adding a worktree.
    WorktreeAddArgs { repo: String, path: String, branch: String, create_branch: Option<bool> }
    /// Options for removing a worktree.
    WorktreeRemoveArgs { repo: String, path: String, force: Option<bool> }
    /// Options for initialising a repository.
    InitArgs { path: String, initial_branch: Option<String> }
    /// Options for cloning a repository.
    CloneArgs { url: String, destination: String, depth: Option<u32>, branch: Option<String>, recurse_submodules: Option<bool> }
    /// Options for reading Git configuration.
    ConfigGetArgs { repo: Option<String>, key: String, scope: Option<ConfigScope> }
    /// Options for writing Git configuration.
    ConfigSetArgs { repo: Option<String>, key: String, value: String, scope: Option<ConfigScope> }
    /// Patterns to add to `.gitignore`.
    IgnoreArgs { repo: String, patterns: Vec<String> }
    /// Options for pruning merged branches.
    PruneArgs { repo: String, base: Option<String>, dry_run: Option<bool> }
}

// -------------------------------------------------------------------- plumbing

/// Register every Git capability, binding them to `git`.
pub fn register(reg: &mut Registry, git: Arc<GitService>) -> Result<()> {
    // The Git executable is required for every capability in this module.
    // Failing here means `devd` reports a clear "install Git" error at start-up
    // rather than a confusing command-not-found deep inside an operation.
    let _ = git.git_path();

    register_read(reg, &git)?;
    register_write(reg, &git)?;
    register_exec(reg, &git)?;
    register_destructive(reg, &git)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add<A, P, Q>(
    reg: &mut Registry,
    id: &str,
    summary: &str,
    description: &str,
    group: &str,
    tier: Tier,
    effect: Effect,
    agent_safe: bool,
    dry_run: bool,
    idempotent: bool,
    plan: P,
    apply: Q,
) -> Result<()>
where
    A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
    P: Fn(&Ctx, &A) -> Result<Plan> + Send + Sync + 'static,
    Q: Fn(&Ctx, &A) -> Result<Outcome> + Send + Sync + 'static,
{
    let cap = Capability::builder(id, summary)
        .description(description)
        .group(group)
        .tier(tier)
        .effect(effect)
        .agent_safe(agent_safe)
        .dry_run(dry_run)
        .idempotent(idempotent)
        .args_schema(
            serde_json::to_value(schemars::schema_for!(A))
                .map_err(|e| DevdError::internal(format!("cannot build schema for {id}: {e}")))?,
        )
        .handler(
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                plan(ctx, &typed)
            },
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                apply(ctx, &typed)
            },
        )
        .build()?;
    reg.register(cap)
}

macro_rules! git_cap {
    (
        $reg:expr, $git:expr, $id:literal, $group:literal, $summary:literal, $doc:literal,
        $tier:expr, $effect:expr, $agent:expr, $dry:expr, $idem:expr,
        $args:ty, $plan:expr, $apply:expr
    ) => {{
        let plan_service = ::std::sync::Arc::clone($git);
        let apply_service = ::std::sync::Arc::clone($git);
        add::<$args, _, _>(
            $reg,
            $id,
            $summary,
            $doc,
            $group,
            $tier,
            $effect,
            $agent,
            $dry,
            $idem,
            move |ctx: &Ctx, a: &$args| $plan(ctx, a, plan_service.as_ref()),
            move |ctx: &Ctx, a: &$args| $apply(ctx, a, apply_service.as_ref()),
        )?;
    }};
}

/// Resolve any path inside a work tree to the repository root.
fn repo_of(git: &GitService, path: &str) -> Result<PathBuf> {
    let candidate = PathBuf::from(path);
    if !candidate.exists() {
        return Err(DevdError::not_found(format!(
            "path does not exist: {}",
            candidate.display()
        )));
    }
    git.repo_root(&candidate)?
        .ok_or_else(|| DevdError::invalid(format!("not a Git repository: {}", candidate.display())))
}

/// Record that `devd` has seen this repository. Best effort: a failure to index
/// a repository must never fail the Git operation the user asked for.
fn track_repo(ctx: &Ctx, git: &GitService, repo: &Path) {
    let path = repo.to_string_lossy().to_string();
    let name = repo
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let remote_url = git
        .remotes(repo)
        .ok()
        .and_then(|list| list.into_iter().find(|r| r.name == "origin"))
        .map(|r| r.fetch_url);
    let default_branch = git.current_branch(repo).ok().flatten();
    let id = ctx
        .store
        .get_repo_by_path(&path)
        .ok()
        .flatten()
        .map(|r| r.id)
        .unwrap_or_else(util::new_id);
    let entry = Repo {
        id,
        path,
        name,
        remote_url,
        default_branch,
        last_seen_at: util::now(),
    };
    let _ = ctx.store.upsert_repo(&entry);
}

fn json_status(status: &crate::RepoStatus) -> Value {
    serde_json::to_value(status).unwrap_or(Value::Null)
}

/// A plan that performs no work, for pure inspection.
fn read_plan(id: &str, what: impl Into<String>) -> Plan {
    Plan::new(id, what).effect("none")
}

// ------------------------------------------------------------------ read tier

fn register_read(reg: &mut Registry, git: &Arc<GitService>) -> Result<()> {
    git_cap!(reg, git, "git.status", "git.repo",
        "Show working tree status",
        "Reports the current branch, upstream tracking, ahead/behind counts, staged and unstaged changes, untracked files, conflicts and any interrupted Git operation.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.status", format!("read status of {}", a.repo))),
        |ctx: &Ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            track_repo(ctx, git, &repo);
            let status = git.status(&repo)?;
            Ok(Outcome::with_data(
                format!(
                    "{}: {} staged, {} unstaged, {} untracked, {} conflicted{}{}",
                    status.branch.as_deref().unwrap_or("(detached)"),
                    status.staged.len(),
                    status.unstaged.len(),
                    status.untracked.len(),
                    status.conflicted.len(),
                    if status.ahead > 0 { format!(", {} ahead", status.ahead) } else { String::new() },
                    if status.behind > 0 { format!(", {} behind", status.behind) } else { String::new() },
                ),
                json_status(&status),
            ))
        });

    git_cap!(reg, git, "git.repo.open", "git.repo",
        "Open a repository and describe it",
        "Resolves the repository root and returns branch, upstream, remotes and in-progress state. Read-only: nothing is recorded, so use `git.repo.track` to index it.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.repo.open", format!("open {}", a.repo))),
        |ctx: &Ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let tracked = ctx.store.get_repo_by_path(&repo.to_string_lossy())?.is_some();
            let status = git.status(&repo)?;
            let remotes = git.remotes(&repo)?;
            let worktrees = git.worktrees(&repo)?;
            Ok(Outcome::with_data(
                format!("opened {}", repo.display()),
                serde_json::json!({
                    "root": repo.to_string_lossy(),
                    "branch": status.branch,
                    "upstream": status.upstream,
                    "ahead": status.ahead,
                    "behind": status.behind,
                    "clean": status.clean,
                    "in_progress": status.in_progress,
                    "remotes": remotes,
                    "worktrees": worktrees.len(),
                    "tracked": tracked,
                }),
            ))
        });

    git_cap!(reg, git, "git.repo.list", "git.repo",
        "List repositories devd has seen",
        "Returns every repository recorded in devd's index, most recently used first.",
        Tier::Read, Effect::Read, true, false, true, NoArgs,
        |_ctx, _a: &NoArgs, _git| Ok(read_plan("git.repo.list", "list indexed repositories")),
        |ctx: &Ctx, _a: &NoArgs, _git| {
            let repos = ctx.store.list_repos()?;
            Ok(Outcome::with_data(
                format!("{} repositories", repos.len()),
                serde_json::to_value(&repos).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.diff", "git.changes",
        "Show a diff",
        "Produces a patch for the working tree, the index, a single path, or between arbitrary revisions.",
        Tier::Read, Effect::Read, true, false, true, DiffArgs,
        |_ctx, a: &DiffArgs, _git| {
            let target = if a.staged.unwrap_or(false) { "staged changes" } else { "unstaged changes" };
            Ok(read_plan("git.diff", format!("diff {target} in {}", a.repo)))
        },
        |_ctx, a: &DiffArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let text = git.diff(
                &repo,
                &DiffOptions {
                    staged: a.staged.unwrap_or(false),
                    path: a.path.clone(),
                    rev: a.rev.clone(),
                    context: a.context,
                    stat: a.stat.unwrap_or(false),
                },
            )?;
            Ok(Outcome::with_data(
                format!("{} bytes of diff", text.len()),
                serde_json::json!({ "patch": text }),
            ))
        });

    git_cap!(reg, git, "git.show", "git.history",
        "Show a commit with its patch",
        "Returns commit metadata and the full patch for one revision.",
        Tier::Read, Effect::Read, true, false, true, RevArgs,
        |_ctx, a: &RevArgs, _git| Ok(read_plan("git.show", format!("show {}", a.rev))),
        |_ctx, a: &RevArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let text = git.show(&repo, &a.rev)?;
            Ok(Outcome::with_data(
                format!("{} bytes", text.len()),
                serde_json::json!({ "patch": text }),
            ))
        });

    git_cap!(reg, git, "git.blame", "git.history",
        "Blame a file",
        "Shows which commit and author last touched each line of a file.",
        Tier::Read, Effect::Read, true, false, true, BlameArgs,
        |_ctx, a: &BlameArgs, _git| Ok(read_plan("git.blame", format!("blame {}", a.path))),
        |_ctx, a: &BlameArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let text = git.blame(&repo, &a.path)?;
            Ok(Outcome::with_data(
                format!("blame for {}", a.path),
                serde_json::json!({ "blame": text }),
            ))
        });

    git_cap!(reg, git, "git.search", "git.history",
        "Search commit history",
        "Searches commit messages, optionally restricted to an author or a path.",
        Tier::Read, Effect::Read, true, false, true, SearchArgs,
        |_ctx, a: &SearchArgs, _git| Ok(read_plan("git.search", format!("search for {:?}", a.query))),
        |_ctx, a: &SearchArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let hits = git.search_commits(
                &repo,
                &a.query,
                a.author.as_deref(),
                a.path.as_deref(),
                a.limit.unwrap_or(50).min(500),
            )?;
            Ok(Outcome::with_data(
                format!("{} matching commits", hits.len()),
                serde_json::to_value(&hits).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.log", "git.history",
        "Read commit history",
        "Returns recent commits, optionally for one revision range or path.",
        Tier::Read, Effect::Read, true, false, true, LogArgs,
        |_ctx, a: &LogArgs, _git| Ok(read_plan("git.log", format!("read history of {}", a.repo))),
        |_ctx, a: &LogArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let commits = git.log(
                &repo,
                a.limit.unwrap_or(30).min(500),
                a.rev.as_deref(),
                a.path.as_deref(),
            )?;
            Ok(Outcome::with_data(
                format!("{} commits", commits.len()),
                serde_json::to_value(&commits).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.branch.list", "git.branches",
        "List branches",
        "Lists local branches, optionally including remote-tracking branches, with upstream and tip subject.",
        Tier::Read, Effect::Read, true, false, true, BranchListArgs,
        |_ctx, a: &BranchListArgs, _git| Ok(read_plan("git.branch.list", format!("list branches in {}", a.repo))),
        |_ctx, a: &BranchListArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let branches = git.branches(&repo, a.all.unwrap_or(false))?;
            let current = branches.iter().find(|b| b.current).map(|b| b.name.clone());
            Ok(Outcome::with_data(
                format!("{} branches (on {})", branches.len(), current.as_deref().unwrap_or("detached HEAD")),
                serde_json::to_value(&branches).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.remote.list", "git.remotes",
        "List remotes",
        "Lists configured remotes. Credentials embedded in URLs are redacted.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.remote.list", format!("list remotes of {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remotes = git.remotes(&repo)?;
            Ok(Outcome::with_data(
                format!("{} remotes", remotes.len()),
                serde_json::to_value(&remotes).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.tag.list", "git.tags",
        "List tags",
        "Lists tags, newest first, noting which are annotated.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.tag.list", format!("list tags of {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let tags = git.tags(&repo)?;
            Ok(Outcome::with_data(
                format!("{} tags", tags.len()),
                serde_json::to_value(&tags).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.stash.list", "git.stashes",
        "List stashes",
        "Lists stash entries with their subjects.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.stash.list", format!("list stashes of {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let stashes = git.stash_list(&repo)?;
            Ok(Outcome::with_data(
                format!("{} stash entries", stashes.len()),
                serde_json::to_value(&stashes).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.stash.show", "git.stashes",
        "Show a stash as a patch",
        "Returns the diff stored in a stash entry without applying it.",
        Tier::Read, Effect::Read, true, false, true, StashRefArgs,
        |_ctx, _a: &StashRefArgs, _git| Ok(read_plan("git.stash.show", "show stash")),
        |_ctx, a: &StashRefArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let reference = a.reference.clone().unwrap_or_else(|| "stash@{0}".to_string());
            let text = git.stash_show(&repo, &reference)?;
            Ok(Outcome::with_data(
                format!("{} bytes", text.len()),
                serde_json::json!({ "patch": text }),
            ))
        });

    git_cap!(reg, git, "git.worktree.list", "git.worktrees",
        "List worktrees",
        "Lists linked worktrees with their head, branch and lock state.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.worktree.list", format!("list worktrees of {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let worktrees = git.worktrees(&repo)?;
            Ok(Outcome::with_data(
                format!("{} worktrees", worktrees.len()),
                serde_json::to_value(&worktrees).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.reflog", "git.recovery",
        "Read the reflog",
        "Returns recent reflog entries, which is how a bad reset or rebase is recovered.",
        Tier::Read, Effect::Read, true, false, true, ReflogArgs,
        |_ctx, a: &ReflogArgs, _git| Ok(read_plan("git.reflog", format!("read reflog of {}", a.repo))),
        |_ctx, a: &ReflogArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let entries = git.reflog(&repo, a.limit.unwrap_or(50).min(500))?;
            Ok(Outcome::with_data(
                format!("{} reflog entries", entries.len()),
                serde_json::json!({ "entries": entries }),
            ))
        });

    git_cap!(reg, git, "git.sync.preview", "git.remotes",
        "Preview incoming and outgoing commits",
        "Shows what a fetch would bring in and what a push would publish, without contacting the remote for a write.",
        Tier::Read, Effect::Read, true, false, true, SyncArgs,
        |_ctx, a: &SyncArgs, _git| Ok(read_plan("git.sync.preview", format!("preview sync of {}", a.repo))),
        |_ctx, a: &SyncArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remote = a.remote.clone().unwrap_or_else(|| "origin".to_string());
            let branch = match a.branch.clone() {
                Some(b) => b,
                None => git
                    .current_branch(&repo)?
                    .ok_or_else(|| DevdError::invalid("detached HEAD: specify a branch"))?,
            };
            let preview = git.sync_preview(&repo, &remote, &branch, 50)?;
            Ok(Outcome::with_data(
                format!(
                    "{} incoming, {} outgoing{}",
                    preview.incoming.len(),
                    preview.outgoing.len(),
                    if preview.diverged { ", diverged" } else { "" }
                ),
                serde_json::to_value(&preview).unwrap_or(Value::Null),
            ))
        });

    git_cap!(reg, git, "git.switch.check", "git.branches",
        "Check whether a branch switch would lose work",
        "Identifies files modified locally that the target branch also changes, which is what makes a switch destructive.",
        Tier::Read, Effect::Read, true, false, true, SwitchCheckArgs,
        |_ctx, a: &SwitchCheckArgs, _git| Ok(read_plan("git.switch.check", format!("check switching to {}", a.target))),
        |_ctx, a: &SwitchCheckArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let risky = git.switch_safety(&repo, &a.target)?;
            Ok(Outcome::with_data(
                if risky.is_empty() {
                    format!("switching to {} would not overwrite local changes", a.target)
                } else {
                    format!("switching to {} would overwrite {} modified file(s)", a.target, risky.len())
                },
                serde_json::json!({ "at_risk": risky, "safe": risky.is_empty() }),
            ))
        });

    git_cap!(reg, git, "git.config.get", "git.config",
        "Read Git configuration",
        "Reads a configuration value and reports which file it came from, so the user can tell whether it is local, global or system.",
        Tier::Read, Effect::Read, true, false, true, ConfigGetArgs,
        |_ctx, a: &ConfigGetArgs, _git| Ok(read_plan("git.config.get", format!("read {}", a.key))),
        |_ctx, a: &ConfigGetArgs, git: &GitService| {
            let scope = a.scope.unwrap_or(match a.repo {
                Some(_) => ConfigScope::Local,
                None => ConfigScope::Global,
            });
            let repo = match &a.repo {
                Some(p) => Some(repo_of(git, p)?),
                None => None,
            };
            let value = git.config_get(repo.as_deref(), &a.key, scope)?;
            Ok(Outcome::with_data(
                match &value {
                    Some(v) => format!("{} = {v}", a.key),
                    None => format!("{} is not set at {} scope", a.key, scope_flag(scope)),
                },
                serde_json::json!({ "key": a.key, "value": value, "scope": scope_flag(scope) }),
            ))
        });

    git_cap!(reg, git, "git.conflicts.list", "git.merge",
        "List conflicted files",
        "Returns the paths Git could not merge automatically.",
        Tier::Read, Effect::Read, true, false, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(read_plan("git.conflicts.list", format!("list conflicts in {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let files = git.conflicts(&repo)?;
            Ok(Outcome::with_data(
                format!("{} conflicted files", files.len()),
                serde_json::json!({ "conflicted": files }),
            ))
        });

    git_cap!(reg, git, "git.conflicts.show", "git.merge",
        "Show a conflicted file",
        "Returns a conflicted file with its conflict markers intact, so a human or agent can decide how to resolve it.",
        Tier::Read, Effect::Read, true, false, true, BlameArgs,
        |_ctx, a: &BlameArgs, _git| Ok(read_plan("git.conflicts.show", format!("show conflict in {}", a.path))),
        |_ctx, a: &BlameArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let text = git.conflicted_file(&repo, &a.path)?;
            Ok(Outcome::with_data(
                format!("{} bytes", text.len()),
                serde_json::json!({ "path": a.path, "content": text }),
            ))
        });

    Ok(())
}

fn scope_flag(scope: ConfigScope) -> &'static str {
    match scope {
        ConfigScope::Local => "local",
        ConfigScope::Global => "global",
        ConfigScope::System => "system",
    }
}

// ----------------------------------------------------------------- write tier

fn register_write(reg: &mut Registry, git: &Arc<GitService>) -> Result<()> {
    git_cap!(reg, git, "git.stage", "git.changes",
        "Stage paths",
        "Adds paths to the index. This is what a commit will record.",
        Tier::Write, Effect::Write, true, true, false, RepoPathsArgs,
        |_ctx, a: &RepoPathsArgs, _git| {
            Ok(Plan::new("git.stage", format!("stage {} path(s)", a.paths.len()))
                .step(format!("git add -- {}", a.paths.join(" "))))
        },
        |_ctx, a: &RepoPathsArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.stage(&repo, &a.paths)?;
            Ok(Outcome::with_data(
                format!("staged {} path(s)", a.paths.len()),
                serde_json::json!({ "paths": a.paths }),
            ))
        });

    git_cap!(reg, git, "git.unstage", "git.changes",
        "Unstage paths",
        "Removes paths from the index while leaving the working tree untouched.",
        Tier::Write, Effect::Write, true, true, false, RepoPathsArgs,
        |_ctx, a: &RepoPathsArgs, _git| {
            Ok(Plan::new("git.unstage", format!("unstage {} path(s)", a.paths.len()))
                .step(format!("git restore --staged -- {}", a.paths.join(" "))))
        },
        |_ctx, a: &RepoPathsArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.unstage(&repo, &a.paths)?;
            Ok(Outcome::with_data(
                format!("unstaged {} path(s)", a.paths.len()),
                serde_json::json!({ "paths": a.paths }),
            ))
        });

    git_cap!(reg, git, "git.stage.patch", "git.changes",
        "Stage a patch (hunk or line level)",
        "Applies a unified diff to the index, which is how an individual hunk or line is staged without staging the rest of the file.",
        Tier::Write, Effect::Write, true, true, false, StagePatchArgs,
        |_ctx, a: &StagePatchArgs, _git| {
            Ok(Plan::new("git.stage.patch", format!("apply a {} byte patch to the index", a.patch.len()))
                .step("git apply --cached <patch>"))
        },
        |_ctx, a: &StagePatchArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.stage_patch(&repo, &a.patch, a.reverse.unwrap_or(false))?;
            Ok(Outcome::ok("applied patch to the index"))
        });

    git_cap!(reg, git, "git.commit", "git.commits",
        "Create or amend a commit",
        "Records the staged changes. Refuses an empty message, and never opens an interactive editor or a signing prompt.",
        Tier::Write, Effect::Write, true, true, false, CommitArgs,
        |ctx: &Ctx, a: &CommitArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let status = git.status(&repo)?;
            let verb = if a.amend.unwrap_or(false) { "amend HEAD with" } else { "create a commit from" };
            let mut plan = Plan::new("git.commit", format!("{verb} {} staged change(s)", status.staged.len()))
                .step(format!("message: {:?}", first_line(&a.message)));
            if status.staged.is_empty() && !a.amend.unwrap_or(false) && !a.allow_empty.unwrap_or(false)
            {
                plan = plan.step("warning: nothing is staged, so this will fail");
            }
            if ctx.actor.is_agent() {
                plan = plan.step("committing as the agent; the message and diff are shown before applying");
            }
            Ok(plan)
        },
        |_ctx, a: &CommitArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let before = git.rev_parse(&repo, "HEAD").ok();
            let hash = git.commit(
                &repo,
                &a.message,
                a.amend.unwrap_or(false),
                a.allow_empty.unwrap_or(false),
            )?;
            Ok(Outcome::with_data(
                format!("committed {}", &hash[..7.min(hash.len())]),
                serde_json::json!({ "commit": hash, "previous": before }),
            ))
        });

    git_cap!(reg, git, "git.branch.create", "git.branches",
        "Create a branch",
        "Creates a branch, optionally at a given start point and optionally switching to it.",
        Tier::Write, Effect::Write, true, true, true, BranchCreateArgs,
        |_ctx, a: &BranchCreateArgs, _git| {
            Ok(Plan::new("git.branch.create", format!("create branch {}", a.name))
                .step(match &a.start_point {
                    Some(s) => format!("base it on {s}"),
                    None => "base it on the current HEAD".to_string(),
                })
                .step(if a.checkout.unwrap_or(false) { "switch to it" } else { "leave the current branch checked out" }))
        },
        |ctx: &Ctx, a: &BranchCreateArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            // Creating a branch and leaving it unchecked out must not be
            // blocked by a dirty tree; switching to it can be.
            if a.checkout.unwrap_or(false) {
                let risky = git.switch_safety(&repo, &a.name)?;
                if !risky.is_empty() && ctx.settings.git_protect_uncommitted {
                    return Err(DevdError::conflict(format!(
                        "switching to the new branch would overwrite local changes in: {}. \
                         Commit or stash them first, or set git_protect_uncommitted = false.",
                        risky.join(", ")
                    )));
                }
            }
            git.create_branch(&repo, &a.name, a.start_point.as_deref(), a.checkout.unwrap_or(false))?;
            Ok(Outcome::with_data(
                format!("created branch {}", a.name),
                serde_json::json!({ "branch": a.name }),
            ))
        });

    git_cap!(reg, git, "git.branch.rename", "git.branches",
        "Rename a branch",
        "Renames a local branch.",
        Tier::Write, Effect::Write, true, true, true, BranchRenameArgs,
        |_ctx, a: &BranchRenameArgs, _git| {
            Ok(Plan::new("git.branch.rename", format!("rename {} to {}", a.from, a.to)))
        },
        |_ctx, a: &BranchRenameArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.rename_branch(&repo, &a.from, &a.to)?;
            Ok(Outcome::ok(format!("renamed {} to {}", a.from, a.to)))
        });

    git_cap!(reg, git, "git.branch.upstream", "git.remotes",
        "Set a branch upstream",
        "Configures the remote-tracking branch a local branch follows.",
        Tier::Write, Effect::Config, true, true, true, BranchUpstreamArgs,
        |_ctx, a: &BranchUpstreamArgs, _git| {
            Ok(Plan::new("git.branch.upstream", format!("track {} with {}", a.branch, a.upstream)))
        },
        |_ctx, a: &BranchUpstreamArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.set_upstream(&repo, &a.branch, &a.upstream)?;
            Ok(Outcome::ok(format!("{} now tracks {}", a.branch, a.upstream)))
        });

    git_cap!(reg, git, "git.tag.create", "git.tags",
        "Create a tag",
        "Creates an annotated or lightweight tag. Overwriting an existing tag requires the force flag.",
        Tier::Write, Effect::Write, true, true, false, TagCreateArgs,
        |_ctx, a: &TagCreateArgs, _git| {
            let mut plan = Plan::new(
                "git.tag.create",
                format!("create {} tag {}", if a.message.is_some() { "annotated" } else { "lightweight" }, a.name),
            );
            if a.rev.is_some() {
                plan = plan.step(format!("target {}", a.rev.clone().unwrap_or_default()));
            }
            if a.force.unwrap_or(false) {
                plan = Plan { ..plan }.step("force: an existing tag with this name will be moved");
            }
            Ok(plan)
        },
        |_ctx, a: &TagCreateArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.create_tag(&repo, &a.name, a.message.as_deref(), a.rev.as_deref(), a.force.unwrap_or(false))?;
            Ok(Outcome::ok(format!("created tag {}", a.name)))
        });

    git_cap!(reg, git, "git.remote.add", "git.remotes",
        "Add a remote",
        "Adds a remote. Plain http:// URLs are refused because they would carry credentials and code in the clear.",
        Tier::Write, Effect::Config, true, true, true, RemoteAddArgs,
        |_ctx, a: &RemoteAddArgs, _git| {
            Ok(Plan::new("git.remote.add", format!("add remote {} -> {}", a.name, crate::redact_url(&a.url))))
        },
        |_ctx, a: &RemoteAddArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.remote_add(&repo, &a.name, &a.url)?;
            Ok(Outcome::ok(format!("added remote {}", a.name)))
        });

    git_cap!(reg, git, "git.remote.rename", "git.remotes",
        "Rename a remote",
        "Renames a configured remote.",
        Tier::Write, Effect::Config, true, true, true, RemoteRenameArgs,
        |_ctx, a: &RemoteRenameArgs, _git| {
            Ok(Plan::new("git.remote.rename", format!("rename remote {} to {}", a.from, a.to)))
        },
        |_ctx, a: &RemoteRenameArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.remote_rename(&repo, &a.from, &a.to)?;
            Ok(Outcome::ok(format!("renamed remote {} to {}", a.from, a.to)))
        });

    git_cap!(reg, git, "git.stash.push", "git.stashes",
        "Create a stash",
        "Shelves working tree changes so the tree can be clean.",
        Tier::Write, Effect::Write, true, true, false, StashPushArgs,
        |_ctx, a: &StashPushArgs, _git| {
            Ok(Plan::new("git.stash.push", "shelve local changes")
                .step(if a.include_untracked.unwrap_or(false) { "including untracked files" } else { "tracked files only" }))
        },
        |_ctx, a: &StashPushArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.stash_push(
                &repo,
                a.message.as_deref(),
                a.include_untracked.unwrap_or(false),
                a.keep_index.unwrap_or(false),
            )?;
            Ok(Outcome::with_data(
                out.trim().to_string(),
                serde_json::json!({ "output": out }),
            ))
        });

    git_cap!(reg, git, "git.stash.apply", "git.stashes",
        "Apply a stash",
        "Restores a stash's changes. Set pop to also drop the entry once applied.",
        Tier::Write, Effect::Write, true, true, false, StashRefArgs,
        |_ctx, a: &StashRefArgs, _git| {
            let reference = a.reference.clone().unwrap_or_else(|| "stash@{0}".to_string());
            Ok(Plan::new(
                "git.stash.apply",
                format!("{} {reference}", if a.pop.unwrap_or(false) { "pop" } else { "apply" }),
            ))
        },
        |_ctx, a: &StashRefArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.stash_apply(&repo, a.reference.as_deref(), a.pop.unwrap_or(false), a.index.unwrap_or(false))?;
            Ok(Outcome::with_data(out.trim().to_string(), serde_json::json!({ "output": out })))
        });

    git_cap!(reg, git, "git.worktree.add", "git.worktrees",
        "Add a worktree",
        "Creates a second working tree for a branch. This is the isolation mechanism the agent prefers for substantial tasks, because it cannot disturb the user's current checkout.",
        Tier::Write, Effect::Write, true, true, true, WorktreeAddArgs,
        |_ctx, a: &WorktreeAddArgs, _git| {
            Ok(Plan::new("git.worktree.add", format!("add a worktree at {} for {}", a.path, a.branch))
                .step(if a.create_branch.unwrap_or(false) {
                    format!("create branch {} there", a.branch)
                } else {
                    format!("check out existing branch {}", a.branch)
                })
                .effect("no effect on the current working tree"))
        },
        |_ctx, a: &WorktreeAddArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let path = PathBuf::from(&a.path);
            git.worktree_add(&repo, &path, &a.branch, a.create_branch.unwrap_or(false))?;
            Ok(Outcome::with_data(
                format!("added worktree at {}", path.display()),
                serde_json::json!({ "path": a.path, "branch": a.branch }),
            ))
        });

    git_cap!(reg, git, "git.ignore.add", "git.repo",
        "Add patterns to .gitignore",
        "Appends patterns to `.gitignore`, skipping ones already present.",
        Tier::Write, Effect::Write, true, true, true, IgnoreArgs,
        |_ctx, a: &IgnoreArgs, _git| {
            Ok(Plan::new("git.ignore.add", format!("add {} pattern(s) to .gitignore", a.patterns.len()))
                .step(a.patterns.join(", ")))
        },
        |_ctx, a: &IgnoreArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.ignore_add(&repo, &a.patterns)?;
            Ok(Outcome::with_data(
                format!("updated .gitignore with {} pattern(s)", a.patterns.len()),
                serde_json::json!({ "patterns": a.patterns }),
            ))
        });

    git_cap!(reg, git, "git.config.set", "git.config",
        "Write Git configuration",
        "Writes a configuration value at local, global or system scope. The scope is always explicit, so nothing is changed implicitly.",
        Tier::Write, Effect::Config, false, true, false, ConfigSetArgs,
        |_ctx, a: &ConfigSetArgs, _git| {
            let scope = a.scope.unwrap_or(match a.repo {
                Some(_) => ConfigScope::Local,
                None => ConfigScope::Global,
            });
            Ok(Plan::new("git.config.set", format!("set {} at {} scope", a.key, scope_flag(scope)))
                .step(format!("{} = {}", a.key, crate::redact_url(&a.value))))
        },
        |_ctx, a: &ConfigSetArgs, git: &GitService| {
            let scope = a.scope.unwrap_or(match a.repo {
                Some(_) => ConfigScope::Local,
                None => ConfigScope::Global,
            });
            let repo = match &a.repo {
                Some(p) => Some(repo_of(git, p)?),
                None => None,
            };
            git.config_set(repo.as_deref(), &a.key, &a.value, scope)?;
            Ok(Outcome::ok(format!("set {} at {} scope", a.key, scope_flag(scope))))
        });

    git_cap!(reg, git, "git.repo.init", "git.repo",
        "Initialise a repository",
        "Creates a new repository with an explicit initial branch name, so the result does not depend on the machine's default.",
        Tier::Write, Effect::Write, true, true, true, InitArgs,
        |ctx: &Ctx, a: &InitArgs, _git| {
            let branch = a.initial_branch.clone().unwrap_or_else(|| ctx.settings.git_default_branch.clone());
            let mut plan = Plan::new("git.repo.init", format!("initialise a repository at {}", a.path))
                .step(format!("initial branch: {branch}"));
            if Path::new(&a.path).join(".git").exists() {
                plan = plan.step("note: a repository already exists here");
            }
            Ok(plan)
        },
        |ctx: &Ctx, a: &InitArgs, git: &GitService| {
            let branch = a.initial_branch.clone().unwrap_or_else(|| ctx.settings.git_default_branch.clone());
            let root = git.init(Path::new(&a.path), &branch)?;
            track_repo(ctx, git, &root);
            Ok(Outcome::with_data(
                format!("initialised {} on {branch}", root.display()),
                serde_json::json!({ "root": root.to_string_lossy(), "branch": branch }),
            ))
        });

    git_cap!(reg, git, "git.repo.track", "git.repo",
        "Track a repository in devd",
        "Adds a repository to devd's index so it appears in the GUI and CLI listings without being discovered by scanning.",
        Tier::Write, Effect::Write, true, true, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(Plan::new("git.repo.track", format!("index {}", a.repo))),
        |ctx: &Ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            track_repo(ctx, git, &repo);
            Ok(Outcome::ok(format!("tracking {}", repo.display())))
        });

    Ok(())
}

// ------------------------------------------------------------------ exec tier

fn register_exec(reg: &mut Registry, git: &Arc<GitService>) -> Result<()> {
    git_cap!(reg, git, "git.switch", "git.branches",
        "Switch branch",
        "Switches the working tree to another branch, optionally creating it. Refuses by default when it would overwrite uncommitted work.",
        Tier::Exec, Effect::Write, true, true, false, SwitchArgs,
        |ctx: &Ctx, a: &SwitchArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let status = git.status(&repo)?;
            let mut plan = Plan::new(
                "git.switch",
                format!("switch {} to {}", status.branch.as_deref().unwrap_or("(detached)"), a.name),
            );
            if !status.clean {
                let risky = git.switch_safety(&repo, &a.name)?;
                if risky.is_empty() {
                    plan = plan.step("local changes do not overlap the target branch and will be carried over");
                } else {
                    plan.effects.push("local changes would be overwritten".to_string());
                    plan = plan
                        .step(format!("BLOCKED by default: these modified files also change on {}: {}", a.name, risky.join(", ")))
                        .destructive(true);
                }
            }
            if a.force.unwrap_or(false) {
                plan = plan
                    .step("force is set: uncommitted changes will be DISCARDED")
                    .destructive(true)
                    .approval(true);
            }
            let _ = ctx;
            Ok(plan)
        },
        |ctx: &Ctx, a: &SwitchArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let force = a.force.unwrap_or(false);
            if !force && ctx.settings.git_protect_uncommitted {
                let risky = git.switch_safety(&repo, &a.name)?;
                if !risky.is_empty() {
                    return Err(DevdError::conflict(format!(
                        "switching to {} would overwrite local changes in: {}. Commit or stash them first, \
                         or pass force = true to discard them deliberately.",
                        a.name,
                        risky.join(", ")
                    )));
                }
            }
            git.switch_branch(&repo, &a.name, a.create.unwrap_or(false), force)?;
            Ok(Outcome::ok(format!("switched to {}", a.name)))
        });

    git_cap!(reg, git, "git.fetch", "git.remotes",
        "Fetch from a remote",
        "Updates remote-tracking branches, optionally pruning ones deleted on the remote. Does not touch local branches.",
        Tier::Exec, Effect::Remote, true, true, true, FetchArgs,
        |_ctx, a: &FetchArgs, _git| {
            Ok(Plan::new(
                "git.fetch",
                format!("fetch {}", a.remote.clone().unwrap_or_else(|| "all remotes".into())),
            )
            .step(if a.prune.unwrap_or(false) { "prune deleted remote-tracking branches" } else { "leave stale remote-tracking branches" })
            .effect("no local branch is modified"))
        },
        |_ctx, a: &FetchArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.fetch(&repo, a.remote.as_deref(), a.prune.unwrap_or(false))?;
            Ok(Outcome::with_data(
                "fetch complete".to_string(),
                serde_json::json!({ "output": out }),
            ))
        });

    git_cap!(reg, git, "git.pull", "git.remotes",
        "Pull from a remote",
        "Fetches and integrates. The integration strategy is explicit: merge, rebase, or fast-forward-only.",
        Tier::Exec, Effect::Write, true, true, false, PullArgs,
        |_ctx, a: &PullArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let strategy = a.strategy.unwrap_or(PullStrategy::FastForwardOnly);
            let mut plan = Plan::new("git.pull", format!("pull with the {} strategy", strategy_name(strategy)));
            if let (Some(remote), Some(branch)) = (a.remote.clone(), a.branch.clone()) {
                if let Ok(preview) = git.sync_preview(&repo, &remote, &branch, 50) {
                    if preview.diverged {
                        plan = plan.step(format!(
                            "history has diverged: {} incoming and {} outgoing commits; \
                             fast-forward-only will refuse and merge or rebase will be needed",
                            preview.incoming.len(),
                            preview.outgoing.len()
                        ));
                    }
                }
            }
            Ok(plan)
        },
        |_ctx, a: &PullArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.pull(
                &repo,
                a.remote.as_deref(),
                a.branch.as_deref(),
                a.strategy.unwrap_or(PullStrategy::FastForwardOnly),
            )?;
            let status = git.status(&repo)?;
            let mut outcome = Outcome::with_data(
                "pull complete".to_string(),
                serde_json::json!({ "output": out, "conflicted": status.conflicted }),
            );
            if !status.conflicted.is_empty() {
                outcome = outcome.warn(format!(
                    "{} file(s) have merge conflicts and need resolution",
                    status.conflicted.len()
                ));
            }
            Ok(outcome)
        });

    git_cap!(reg, git, "git.push", "git.remotes",
        "Push a branch",
        "Publishes commits. Force is deliberately not available here; it has its own capability and its own approval gate.",
        Tier::Remote, Effect::Remote, true, true, false, PushArgs,
        |_ctx, a: &PushArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remote = a.remote.clone().unwrap_or_else(|| "origin".to_string());
            let branch = match a.branch.clone() {
                Some(b) => b,
                None => git.current_branch(&repo)?.unwrap_or_else(|| "HEAD".to_string()),
            };
            let mut plan = Plan::new("git.push", format!("push {branch} to {remote}"));
            if let Ok(outgoing) = git.outgoing(&repo, &remote, &branch, 100) {
                plan = plan.step(format!("{} commit(s) will be published", outgoing.len()));
            }
            if a.set_upstream.unwrap_or(false) {
                plan = plan.step(format!("set {remote}/{branch} as the upstream"));
            }
            if a.tags.unwrap_or(false) {
                plan = plan.step("also push tags");
            }
            Ok(plan)
        },
        |_ctx, a: &PushArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.push(
                &repo,
                a.remote.as_deref().unwrap_or("origin"),
                a.branch.as_deref(),
                a.set_upstream.unwrap_or(false),
                ForcePush::None,
                a.tags.unwrap_or(false),
            )?;
            Ok(Outcome::with_data("push complete", serde_json::json!({ "output": out })))
        });

    git_cap!(reg, git, "git.merge", "git.merge",
        "Merge a branch",
        "Merges a branch into the current one. A conflict is reported as a state to resolve, not as a crash.",
        Tier::Exec, Effect::Write, true, true, false, MergeArgs,
        |_ctx, a: &MergeArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let current = git.current_branch(&repo)?.unwrap_or_else(|| "(detached)".to_string());
            Ok(Plan::new("git.merge", format!("merge {} into {current}", a.branch))
                .step(if a.no_ff.unwrap_or(false) { "always create a merge commit" } else { "fast-forward when possible" })
                .step(if a.squash.unwrap_or(false) { "squash the incoming commits into the index" } else { "preserve incoming commit history" }))
        },
        |_ctx, a: &MergeArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.merge(
                &repo,
                &a.branch,
                a.no_ff.unwrap_or(false),
                a.squash.unwrap_or(false),
                a.message.as_deref(),
            )?;
            Ok(finish_operation(&repo, git, "merge", out))
        });

    git_cap!(reg, git, "git.rebase", "git.merge",
        "Rebase the current branch",
        "Replays local commits onto another base. Rewrites local commit identity, so conflicts are reported with everything needed to continue or abort.",
        Tier::Exec, Effect::Write, true, true, false, RebaseArgs,
        |_ctx, a: &RebaseArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let current = git.current_branch(&repo)?.unwrap_or_else(|| "(detached)".to_string());
            Ok(Plan::new("git.rebase", format!("rebase {current} onto {}", a.onto))
                .step("local commits are replayed with new hashes")
                .step(if a.autosquash.unwrap_or(false) { "autosquash fixup/squash commits" } else { "keep commits as they are" })
                .effect("the original commits remain reachable through the reflog"))
        },
        |_ctx, a: &RebaseArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.rebase(&repo, &a.onto, a.autosquash.unwrap_or(false), a.interactive.unwrap_or(false))?;
            Ok(finish_operation(&repo, git, "rebase", out))
        });

    git_cap!(reg, git, "git.cherry.pick", "git.history",
        "Cherry-pick a commit",
        "Applies the change introduced by a commit onto the current branch.",
        Tier::Exec, Effect::Write, true, true, false, RevArgs,
        |_ctx, a: &RevArgs, _git| {
            Ok(Plan::new("git.cherry.pick", format!("cherry-pick {}", a.rev))
                .step("a new commit with a new hash is created"))
        },
        |_ctx, a: &RevArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.cherry_pick(&repo, &a.rev, false)?;
            Ok(finish_operation(&repo, git, "cherry-pick", out))
        });

    git_cap!(reg, git, "git.revert", "git.history",
        "Revert a commit",
        "Creates a new commit that undoes the change, which is the safe way to undo work that is already shared.",
        Tier::Exec, Effect::Write, true, true, false, RevertArgs,
        |_ctx, a: &RevertArgs, _git| {
            Ok(Plan::new("git.revert", format!("revert {}", a.rev))
                .step("history is extended, never rewritten, so this is safe on shared branches"))
        },
        |_ctx, a: &RevertArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.revert(&repo, &a.rev, a.no_commit.unwrap_or(false))?;
            Ok(finish_operation(&repo, git, "revert", out))
        });

    git_cap!(reg, git, "git.op.continue", "git.merge",
        "Continue an interrupted operation",
        "Continues a merge, rebase, cherry-pick or revert that stopped for conflicts, after the conflicts have been resolved and staged.",
        Tier::Exec, Effect::Write, true, true, false, OpArgs,
        |_ctx, a: &OpArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remaining = git.conflicts(&repo)?;
            let mut plan = Plan::new("git.op.continue", format!("continue the {} in progress", a.op));
            if remaining.is_empty() {
                plan = plan.step("all conflicts are resolved");
            } else {
                plan = plan.step(format!("{} file(s) still conflicted: {}", remaining.len(), remaining.join(", ")));
            }
            Ok(plan)
        },
        |_ctx, a: &OpArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.continue_op(&repo, &a.op)?;
            Ok(finish_operation(&repo, git, &a.op, out))
        });

    git_cap!(reg, git, "git.op.abort", "git.merge",
        "Abort an interrupted operation",
        "Returns the repository to the state it was in before the merge, rebase, cherry-pick or revert started.",
        Tier::Exec, Effect::Write, true, true, false, OpArgs,
        |_ctx, a: &OpArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let status = git.status(&repo)?;
            let mut plan = Plan::new("git.op.abort", format!("abort the {} in progress", a.op))
                .step("the working tree returns to its pre-operation state")
                .step("changes made while resolving conflicts are discarded")
                .effect("this is the supported recovery path");
            if let Some(op) = &status.in_progress {
                plan = plan.step(format!("Git reports: {op}"));
            }
            Ok(plan)
        },
        |_ctx, a: &OpArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.abort_op(&repo, &a.op)?;
            let status = git.status(&repo)?;
            Ok(Outcome::with_data(
                format!("aborted {}", a.op),
                serde_json::json!({ "output": out, "status": status }),
            ))
        });

    git_cap!(reg, git, "git.clone", "git.repo",
        "Clone a repository",
        "Clones a remote repository, optionally shallow or with submodules. Hooks and project setup code are never executed as part of a clone.",
        Tier::Exec, Effect::Remote, true, true, false, CloneArgs,
        |_ctx, a: &CloneArgs, _git| {
            let mut plan = Plan::new("git.clone", format!("clone {} into {}", crate::redact_url(&a.url), a.destination))
                .step("no hooks, setup scripts or project code are executed");
            if let Some(d) = a.depth {
                plan = plan.step(format!("shallow clone, depth {d}"));
            }
            if a.recurse_submodules.unwrap_or(false) {
                plan = plan.step("initialise submodules recursively");
            }
            Ok(plan)
        },
        |ctx: &Ctx, a: &CloneArgs, git: &GitService| {
            let destination = PathBuf::from(&a.destination);
            if destination.exists() && destination.read_dir().map(|mut d| d.next().is_some()).unwrap_or(false) {
                return Err(DevdError::conflict(format!(
                    "destination {} already exists and is not empty",
                    destination.display()
                )));
            }
            git.clone(
                &a.url,
                &destination,
                a.depth,
                a.branch.as_deref(),
                a.recurse_submodules.unwrap_or(false),
            )?;
            track_repo(ctx, git, &destination);
            let status = git.status(&destination)?;
            Ok(Outcome::with_data(
                format!("cloned into {}", destination.display()),
                serde_json::json!({
                    "root": destination.to_string_lossy(),
                    "branch": status.branch,
                    "clean": status.clean,
                }),
            ))
        });

    Ok(())
}

fn strategy_name(strategy: PullStrategy) -> &'static str {
    match strategy {
        PullStrategy::Merge => "merge",
        PullStrategy::Rebase => "rebase",
        PullStrategy::FastForwardOnly => "fast-forward-only",
    }
}

/// Report the outcome of an operation that can stop for conflicts.
///
/// A conflicted merge or rebase is **not** a failure: it is a state the user has
/// to resolve. Reporting it as success-with-a-warning is the honest description,
/// and it is what keeps the agent from claiming a merge worked when it did not.
fn finish_operation(repo: &Path, git: &GitService, op: &str, output: String) -> Outcome {
    match git.status(repo) {
        Ok(status) => {
            if !status.conflicted.is_empty() {
                Outcome::with_data(
                    format!(
                        "{op} stopped with {} conflicted file(s); resolve them and then continue, or abort",
                        status.conflicted.len()
                    ),
                    serde_json::json!({
                        "output": output,
                        "conflicted": status.conflicted,
                        "in_progress": status.in_progress,
                    }),
                )
            } else if status.in_progress.is_some() {
                Outcome::with_data(
                    format!("{op} is still in progress"),
                    serde_json::json!({ "output": output, "in_progress": status.in_progress }),
                )
            } else {
                Outcome::with_data(
                    format!("{op} completed"),
                    serde_json::json!({ "output": output, "branch": status.branch }),
                )
            }
        }
        Err(_) => Outcome::with_data(format!("{op} completed"), serde_json::json!({ "output": output })),
    }
}

// ----------------------------------------------------------- destructive tier

fn register_destructive(reg: &mut Registry, git: &Arc<GitService>) -> Result<()> {
    git_cap!(reg, git, "git.discard", "git.changes",
        "Discard working tree changes",
        "Throws away uncommitted modifications to tracked files. Uncommitted work is lost and is not recoverable from the repository.",
        Tier::Destructive, Effect::Write, true, true, false, RepoPathsArgs,
        |_ctx, a: &RepoPathsArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let status = git.status(&repo)?;
            let affected: Vec<String> = status
                .unstaged
                .iter()
                .filter(|c| a.paths.iter().any(|p| c.path == *p))
                .map(|c| c.path.clone())
                .collect();
            Ok(Plan::new(
                "git.discard",
                format!("discard uncommitted changes in {} path(s)", a.paths.len()),
            )
            .step(format!("files: {}", a.paths.join(", ")))
            .step(format!(
                "{} of these currently differ from HEAD",
                affected.len()
            ))
            .step("this content is NOT recoverable from Git; it was never committed")
            .destructive(true)
            .approval(true))
        },
        |_ctx, a: &RepoPathsArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.discard(&repo, &a.paths)?;
            Ok(Outcome::with_data(
                format!("discarded changes in {} path(s)", a.paths.len()),
                serde_json::json!({ "paths": a.paths }),
            ))
        });

    git_cap!(reg, git, "git.reset", "git.recovery",
        "Reset the branch",
        "Moves the branch to another revision. Soft keeps everything, mixed unstages, hard discards the working tree. The plan states exactly what each mode destroys.",
        Tier::Destructive, Effect::Write, true, true, false, ModeArgs,
        |_ctx, a: &ModeArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let head = git.rev_parse(&repo, "HEAD").ok();
            let mut plan = Plan::new("git.reset", format!("reset {:?} to {}", a.mode, a.rev));
            match a.mode {
                ResetMode::Soft => {
                    plan = plan.step("HEAD moves; the index and working tree are untouched");
                }
                ResetMode::Mixed => {
                    plan = plan.step("HEAD and the index move; the working tree is untouched");
                }
                ResetMode::Hard => {
                    plan = plan.step("HEAD, the index AND the working tree are reset");
                    plan = plan.step("uncommitted changes to tracked files are DISCARDED");
                }
            }
            if let Some(head) = head {
                if let Ok(lost) = git.log(&repo, 100, Some(&format!("{}..HEAD", a.rev)), None) {
                    if !lost.is_empty() {
                        plan = plan.step(format!(
                            "{} commit(s) would no longer be reachable from the branch (recoverable via the reflog)",
                            lost.len()
                        ));
                        plan.data = serde_json::json!({
                            "leaving_reach": lost.iter().map(|c| &c.short).collect::<Vec<_>>(),
                            "head": head,
                        });
                    }
                }
            }
            plan = plan.destructive(matches!(a.mode, ResetMode::Hard)).approval(true);
            Ok(plan)
        },
        |_ctx, a: &ModeArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let before = git.rev_parse(&repo, "HEAD").ok();
            let out = git.reset(&repo, a.mode, &a.rev)?;
            let after = git.rev_parse(&repo, "HEAD").ok();
            Ok(Outcome::with_data(
                format!("reset ({:?}) to {}", a.mode, a.rev),
                serde_json::json!({
                    "output": out,
                    "before": before,
                    "after": after,
                    "recover_with": "git.reset to the previous commit, or git.reflog to find it",
                }),
            ))
        });

    git_cap!(reg, git, "git.branch.delete", "git.branches",
        "Delete a branch",
        "Deletes a local or remote branch. A branch with unmerged commits needs the force flag, and the plan says how many commits would be lost.",
        Tier::Destructive, Effect::Write, true, true, false, BranchDeleteArgs,
        |_ctx, a: &BranchDeleteArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let mut plan = Plan::new("git.branch.delete", format!("delete branch {}", a.name));
            if let Some(remote) = &a.remote {
                plan = plan.step(format!("on remote {remote}, which affects everyone who shares it"));
            }
            if let Ok(unmerged) = git.log(&repo, 100, Some(&format!("HEAD..{}", a.name)), None) {
                if !unmerged.is_empty() {
                    plan = plan.step(format!(
                        "{} commit(s) are on {} but not on HEAD: they stay in the reflog but the branch name is gone",
                        unmerged.len(),
                        a.name
                    ));
                    plan.data = serde_json::json!({ "unmerged": unmerged.iter().map(|c| &c.short).collect::<Vec<_>>() });
                    if !a.force.unwrap_or(false) {
                        plan = plan.step("without force this will be refused, because the commits are not merged");
                    }
                }
            }
            Ok(plan.destructive(true).approval(true))
        },
        |_ctx, a: &BranchDeleteArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.delete_branch(&repo, &a.name, a.force.unwrap_or(false), a.remote.as_deref())?;
            Ok(Outcome::ok(format!("deleted branch {}", a.name)))
        });

    git_cap!(reg, git, "git.tag.delete", "git.tags",
        "Delete a tag",
        "Deletes a tag locally and optionally from a remote. Deleting a published tag breaks anyone who has pinned it.",
        Tier::Destructive, Effect::Write, true, true, false, TagDeleteArgs,
        |_ctx, a: &TagDeleteArgs, _git| {
            let mut plan = Plan::new("git.tag.delete", format!("delete tag {}", a.name));
            if let Some(remote) = &a.remote {
                plan = plan.step(format!("also delete it from {remote}, which is a rewrite of published history"));
            }
            Ok(plan.destructive(true).approval(true))
        },
        |_ctx, a: &TagDeleteArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.delete_tag(&repo, &a.name, a.remote.as_deref())?;
            Ok(Outcome::ok(format!("deleted tag {}", a.name)))
        });

    git_cap!(reg, git, "git.worktree.remove", "git.worktrees",
        "Remove a worktree",
        "Removes a linked worktree. Its uncommitted changes are lost unless they were committed.",
        Tier::Destructive, Effect::Write, true, true, false, WorktreeRemoveArgs,
        |_ctx, a: &WorktreeRemoveArgs, _git| {
            Ok(Plan::new("git.worktree.remove", format!("remove the worktree at {}", a.path))
                .step("uncommitted changes in that worktree are lost")
                .step(if a.force.unwrap_or(false) { "force is set: removal proceeds even if the worktree is dirty" } else { "a dirty worktree makes Git refuse" })
                .destructive(true)
                .approval(true))
        },
        |_ctx, a: &WorktreeRemoveArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.worktree_remove(&repo, Path::new(&a.path), a.force.unwrap_or(false))?;
            Ok(Outcome::ok(format!("removed worktree {}", a.path)))
        });

    git_cap!(reg, git, "git.worktree.prune", "git.worktrees",
        "Prune worktree metadata",
        "Removes bookkeeping for worktrees whose directories no longer exist.",
        Tier::Write, Effect::Config, true, true, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(Plan::new("git.worktree.prune", format!("prune stale worktrees of {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.worktree_prune(&repo)?;
            Ok(Outcome::ok("pruned stale worktree metadata"))
        });

    git_cap!(reg, git, "git.stash.drop", "git.stashes",
        "Drop a stash",
        "Deletes a stash entry. The changes it held become unreachable through the stash list.",
        Tier::Destructive, Effect::Write, true, true, false, StashDropArgs,
        |_ctx, a: &StashDropArgs, _git| {
            Ok(Plan::new("git.stash.drop", format!("drop {}", a.reference))
                .step("the entry disappears from the stash list")
                .step("its commits stay in the reflog until Git garbage-collects them")
                .destructive(true)
                .approval(true))
        },
        |_ctx, a: &StashDropArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.stash_drop(&repo, &a.reference)?;
            Ok(Outcome::with_data(out.trim().to_string(), serde_json::json!({ "output": out })))
        });

    git_cap!(reg, git, "git.remote.remove", "git.remotes",
        "Remove a remote",
        "Removes a remote and its remote-tracking branches. Nothing on the remote itself is touched.",
        Tier::Destructive, Effect::Config, true, true, false, RemoteNameArgs,
        |_ctx, a: &RemoteNameArgs, _git| {
            Ok(Plan::new("git.remote.remove", format!("remove remote {}", a.name))
                .step("remote-tracking branches for it are removed locally")
                .step("local branches are not affected")
                .approval(true))
        },
        |_ctx, a: &RemoteNameArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.remote_remove(&repo, &a.name)?;
            Ok(Outcome::ok(format!("removed remote {}", a.name)))
        });

    git_cap!(reg, git, "git.repo.prune", "git.repo",
        "Delete merged branches",
        "Finds local branches already merged into a base branch and deletes them. Deleting the wrong branch is recoverable only through the reflog.",
        Tier::Destructive, Effect::Write, true, true, false, PruneArgs,
        |_ctx, a: &PruneArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let base = a.base.clone().unwrap_or_else(|| "HEAD".to_string());
            let candidates = git.prune_merged_branches(&repo, &base, true)?;
            let mut plan = Plan::new("git.repo.prune", format!("delete {} merged branch(es)", candidates.len()))
                .step(format!("merged into {base}"));
            for branch in &candidates {
                plan = plan.step(format!("delete {branch}"));
            }
            plan.data = serde_json::json!({ "candidates": candidates });
            Ok(plan.approval(true))
        },
        |_ctx, a: &PruneArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let base = a.base.clone().unwrap_or_else(|| "HEAD".to_string());
            if a.dry_run.unwrap_or(false) {
                let candidates = git.prune_merged_branches(&repo, &base, true)?;
                return Ok(Outcome::with_data(
                    format!("{} branches would be deleted", candidates.len()),
                    serde_json::json!({ "candidates": candidates, "dry_run": true }),
                ));
            }
            let deleted = git.prune_merged_branches(&repo, &base, false)?;
            Ok(Outcome::with_data(
                format!("deleted {} merged branch(es)", deleted.len()),
                serde_json::json!({ "deleted": deleted }),
            ))
        });

    git_cap!(reg, git, "git.squash", "git.history",
        "Squash recent commits",
        "Combines the last N commits into one with a new message. The original commits remain in the reflog.",
        Tier::Destructive, Effect::Write, true, true, false, SquashArgs,
        |_ctx, a: &SquashArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let mut plan = Plan::new("git.squash", format!("squash the last {} commits", a.count))
                .step(format!("new message: {:?}", first_line(&a.message)))
                .step("history is rewritten; if these commits were pushed, the remote will need a force push");
            if let Ok(commits) = git.log(&repo, a.count, None, None) {
                plan.data = serde_json::json!({
                    "squashing": commits.iter().map(|c| (&c.short, &c.subject)).collect::<Vec<_>>()
                });
            }
            Ok(plan.destructive(true).approval(true))
        },
        |_ctx, a: &SquashArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let hash = git.squash_last(&repo, a.count, &a.message)?;
            Ok(Outcome::with_data(
                format!("squashed {} commits into {}", a.count, &hash[..7.min(hash.len())]),
                serde_json::json!({ "commit": hash }),
            ))
        });

    git_cap!(reg, git, "git.bisect.start", "git.recovery",
        "Start a bisect",
        "Begins a binary search for the commit that introduced a defect.",
        Tier::Exec, Effect::Write, true, true, false, BisectStartArgs,
        |_ctx, a: &BisectStartArgs, _git| {
            Ok(Plan::new("git.bisect.start", format!("bisect between good {} and bad {}", a.good, a.bad))
                .step("the working tree will be checked out at each step")
                .step("a dirty tree blocks this; commit or stash first"))
        },
        |_ctx, a: &BisectStartArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.bisect_start(&repo, &a.bad, &a.good)?;
            Ok(Outcome::with_data("bisect started", serde_json::json!({ "output": out })))
        });

    git_cap!(reg, git, "git.bisect.mark", "git.recovery",
        "Mark the current bisect step",
        "Records whether the commit currently checked out is good, bad, or untestable.",
        Tier::Exec, Effect::Write, true, true, false, BisectMarkArgs,
        |_ctx, a: &BisectMarkArgs, _git| {
            Ok(Plan::new("git.bisect.mark", format!("mark the current commit as {:?}", a.verdict)))
        },
        |_ctx, a: &BisectMarkArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.bisect_mark(&repo, a.verdict)?;
            Ok(Outcome::with_data(
                if out.contains("is the first bad commit") { "bisect complete" } else { "bisect advanced" },
                serde_json::json!({ "output": out }),
            ))
        });

    git_cap!(reg, git, "git.bisect.reset", "git.recovery",
        "End a bisect",
        "Stops the bisect and returns to the branch that was checked out before it started.",
        Tier::Write, Effect::Write, true, true, false, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(Plan::new("git.bisect.reset", format!("end the bisect in {}", a.repo))),
        |_ctx, a: &RepoArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let out = git.bisect_reset(&repo)?;
            Ok(Outcome::with_data("bisect ended", serde_json::json!({ "output": out })))
        });

    git_cap!(reg, git, "git.conflicts.resolve", "git.merge",
        "Resolve a conflict by choosing a side",
        "Replaces the conflicted file with one side and stages it. The other side's content for that file is discarded.",
        Tier::Destructive, Effect::Write, true, true, false, ResolveArgs,
        |_ctx, a: &ResolveArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let conflicted = git.conflicts(&repo)?;
            let mut plan = Plan::new(
                "git.conflicts.resolve",
                format!("resolve {} by taking {:?}", a.path, a.side),
            )
            .step("the other side's version of this file is discarded")
            .step("the file is then staged");
            if !conflicted.iter().any(|c| c == &a.path) {
                plan = plan.step("note: this path is not currently conflicted");
            }
            Ok(plan.destructive(true))
        },
        |_ctx, a: &ResolveArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            git.resolve_conflict(&repo, &a.path, a.side)?;
            Ok(Outcome::ok(format!("resolved {} by taking {:?}", a.path, a.side)))
        });

    // Force push lives in its own capability so that no code path can reach a
    // forced update without passing the approval gate its tier enforces.
    git_cap!(reg, git, "git.push.force", "git.remotes",
        "Force push a branch",
        "Overwrites the remote branch. Prefers --force-with-lease, which refuses if someone else pushed commits you have not seen; --force is available and is genuinely dangerous.",
        Tier::Remote, Effect::Remote, true, true, false, PushArgs,
        |_ctx, a: &PushArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remote = a.remote.clone().unwrap_or_else(|| "origin".to_string());
            let branch = match a.branch.clone() {
                Some(b) => b,
                None => git.current_branch(&repo)?.unwrap_or_else(|| "HEAD".to_string()),
            };
            let mut plan = Plan::new("git.push.force", format!("FORCE push {branch} to {remote}"))
                .step("commits that exist only on the remote will become unreachable for everyone")
                .step("--force-with-lease is used, so this is refused if the remote moved since the last fetch")
                .destructive(true)
                .approval(true);
            if let Ok(safety) = git.switch_safety(&repo, &format!("{remote}/{branch}")) {
                if !safety.is_empty() {
                    plan = plan.step(format!("local files differ from the remote tip: {}", safety.join(", ")));
                }
            }
            Ok(plan)
        },
        |_ctx, a: &PushArgs, git: &GitService| {
            let repo = repo_of(git, &a.repo)?;
            let remote = a.remote.clone().unwrap_or_else(|| "origin".to_string());
            let branch = match a.branch.clone() {
                Some(b) => b,
                None => git.current_branch(&repo)?.unwrap_or_else(|| "HEAD".to_string()),
            };
            let out = git.push(
                &repo,
                &remote,
                Some(&branch),
                a.set_upstream.unwrap_or(false),
                ForcePush::WithLease,
                a.tags.unwrap_or(false),
            )?;
            Ok(Outcome::with_data(
                format!("force pushed {branch} to {remote}"),
                serde_json::json!({ "output": out }),
            ))
        });

    // A repository that has not been opened yet can be forgotten from the index.
    git_cap!(reg, git, "git.repo.untrack", "git.repo",
        "Forget a repository in devd",
        "Removes a repository from devd's index. Nothing on disk is touched.",
        Tier::Write, Effect::Config, true, true, true, RepoArgs,
        |_ctx, a: &RepoArgs, _git| Ok(Plan::new("git.repo.untrack", format!("forget {}", a.repo))),
        |ctx: &Ctx, a: &RepoArgs, _git| {
            let removed = ctx.store.delete_repo(&a.repo)?;
            Ok(Outcome::ok(if removed {
                format!("forgot {}", a.repo)
            } else {
                format!("{} was not in the index", a.repo)
            }))
        });

    Ok(())
}

fn first_line(message: &str) -> String {
    util::truncate(message.lines().next().unwrap_or("").trim(), 72)
}
