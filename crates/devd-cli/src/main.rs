//! The `devd` command-line interface.
//!
//! The CLI is deliberately thin. It resolves a layout, opens the state store,
//! builds the shared capability registry and dispatches. Every Git operation
//! below therefore goes through exactly the same permission, approval and audit
//! path as the desktop GUI and the MCP server, which is what makes the security
//! properties of the product checkable rather than merely documented.

use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};

use devd_core::config::Settings;
use devd_core::paths::Paths;
use devd_core::{DevdError, Result};

mod app;
mod gitcat;
mod web;
mod window;

use app::{exit_code_for, git_check, merge_args, App};

#[derive(Parser, Debug)]
#[command(
    name = "devd",
    version,
    about = "Agent-native local development environment manager",
    long_about = "devd manages local runtimes, sites, DNS and TLS, and gives humans and \
                  agents one capability API over all of it - including a first-class Git \
                  integration.\n\nEverything devd can do is declared once, with a permission \
                  tier. Sensitive operations ask a human to approve them, and a declined \
                  operation never runs, whichever interface it came from.",
    disable_help_subcommand = true
)]
struct Cli {
    /// Emit machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Show what an operation would do, and change nothing.
    #[arg(long, global = true)]
    plan: bool,
    /// Approve operations that require approval, without prompting.
    ///
    /// This is how a human approves something deliberately from a script. Agents
    /// never set it: the MCP server builds a different actor, and an agent actor
    /// cannot satisfy an approval requirement at all.
    #[arg(long, short = 'y', global = true)]
    yes: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// List the capabilities devd exposes.
    Capabilities {
        /// Restrict the listing to one discovery group, e.g. `git.branches`.
        #[arg(long)]
        group: Option<String>,
        /// Filter by free text.
        query: Option<String>,
    },
    /// Describe one capability, including its argument schema.
    Describe {
        /// Capability identifier, e.g. `git.reset`.
        id: String,
    },
    /// Invoke a capability directly.
    Call {
        /// Capability identifier.
        id: String,
        /// Arguments as a JSON object.
        #[arg(long)]
        args: Option<String>,
        /// A single argument as `key=value`. Repeatable.
        #[arg(long = "arg", value_name = "KEY=VALUE")]
        set: Vec<String>,
    },
    /// Git operations, named exactly as Git names them.
    Git {
        #[command(subcommand)]
        cmd: GitCommand,
    },
    /// Show recent audit entries.
    Audit {
        /// How many entries to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
    },
    /// Open the devd application: projects, Git and the capability registry.
    Ui {
        /// Port to listen on. Defaults to the last used port, then 8787 upward.
        #[arg(long)]
        port: Option<u16>,
        /// Start the interface without opening a window.
        #[arg(long)]
        no_open: bool,
    },
    /// Run the local interface without opening a window, for scripts and tests.
    Serve {
        /// Port to listen on. Defaults to the last used port, then 8787 upward.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Serve the Model Context Protocol on stdin/stdout, for agents.
    Mcp {
        /// Name reported to the audit trail for the calling client.
        #[arg(long, default_value = "mcp-client")]
        client: String,
        /// Print the tool list and exit, instead of serving.
        #[arg(long)]
        list: bool,
    },
    /// Check the environment for problems.
    Doctor,
}

#[derive(Subcommand, Debug)]
pub(crate) enum GitCommand {
    /// Show working tree status.
    Status {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show a diff.
    Diff {
        /// Diff the index against `HEAD` instead of the working tree.
        #[arg(long)]
        staged: bool,
        /// Show a diffstat instead of the patch.
        #[arg(long)]
        stat: bool,
        /// Diff against this revision.
        #[arg(long)]
        rev: Option<String>,
        /// Limit the diff to a path.
        path: Option<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Stage paths into the index.
    Stage {
        /// Paths to stage. `.` stages everything.
        paths: Vec<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove paths from the index, leaving the working tree alone.
    Unstage {
        paths: Vec<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Discard uncommitted changes in the given paths. This loses work.
    Discard {
        paths: Vec<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Record the staged changes as a commit.
    Commit {
        /// Commit message.
        #[arg(short = 'm', long)]
        message: String,
        /// Replace the previous commit instead of adding one.
        #[arg(long)]
        amend: bool,
        /// Commit even when nothing is staged.
        #[arg(long = "allow-empty")]
        allow_empty: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show commit history.
    Log {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Work with branches.
    Branch {
        #[command(subcommand)]
        cmd: BranchCommand,
    },
    /// Switch to another branch.
    Switch {
        name: String,
        /// Create the branch, then switch to it.
        #[arg(short = 'c', long)]
        create: bool,
        /// Switch even if it discards uncommitted work.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Update remote-tracking branches.
    Fetch {
        #[arg(long)]
        remote: Option<String>,
        /// Remove remote-tracking branches that no longer exist on the remote.
        #[arg(long)]
        prune: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Fetch and integrate.
    Pull {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// How to integrate. Defaults to fast-forward-only, which never creates
        /// a merge commit by surprise.
        #[arg(long, value_enum)]
        strategy: Option<StrategyArg>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Publish commits to a remote.
    Push {
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// Record the remote branch as this branch's upstream.
        #[arg(short = 'u', long)]
        set_upstream: bool,
        /// Also push tags.
        #[arg(long)]
        tags: bool,
        /// Force the update. Prefers --force-with-lease.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Clone a repository.
    Clone {
        url: String,
        /// Where to clone to. Defaults to the repository's name.
        destination: Option<String>,
        /// Create a shallow clone of this depth.
        #[arg(long)]
        depth: Option<u32>,
        /// Check out this branch.
        #[arg(long)]
        branch: Option<String>,
        /// Initialise submodules recursively.
        #[arg(long = "recurse-submodules")]
        recurse_submodules: bool,
    },
    /// Merge a branch into the current one.
    Merge {
        branch: String,
        /// Always create a merge commit.
        #[arg(long = "no-ff")]
        no_ff: bool,
        /// Stage the result without committing.
        #[arg(long)]
        squash: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Work with stashes.
    Stash {
        #[command(subcommand)]
        cmd: StashCommand,
    },
    /// Work with linked worktrees.
    Worktree {
        #[command(subcommand)]
        cmd: WorktreeCommand,
    },
    /// Work with remotes.
    Remote {
        #[command(subcommand)]
        cmd: RemoteCommand,
    },
    /// Work with tags.
    Tag {
        #[command(subcommand)]
        cmd: TagCommand,
    },
    /// List files Git could not merge.
    Conflicts {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Continue an interrupted merge, rebase, cherry-pick or revert.
    Continue {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Abort an interrupted operation, restoring the previous state.
    Abort {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Read the reflog.
    Reflog {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Get out of a bad state: abort what is in progress, or show how to go back.
    Undo {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Create a commit that undoes another commit.
    Revert {
        rev: String,
        /// Apply the reversal to the index and working tree without committing.
        #[arg(long = "no-commit")]
        no_commit: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Move the branch to a revision.
    Reset {
        rev: String,
        /// `soft` keeps everything staged, `mixed` unstages, `hard` discards.
        #[arg(long, value_enum, default_value_t = ResetArg::Mixed)]
        mode: ResetArg,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Apply the change from another commit.
    #[command(name = "cherry-pick")]
    CherryPick {
        rev: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Replay the current branch's commits onto another revision.
    Rebase {
        /// The revision to replay onto.
        #[arg(long)]
        onto: Option<String>,
        /// Replay the entire history instead of a range.
        #[arg(long)]
        root: bool,
        /// Squash `fixup!`/`squash!` commits into the commits they name.
        #[arg(long)]
        autosquash: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Combine the most recent commits into one.
    Squash {
        /// How many commits to combine, counting from the newest.
        #[arg(default_value_t = 2)]
        count: u32,
        /// The message for the combined commit.
        #[arg(short = 'm', long)]
        message: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Inspect or reset a bisect run.
    Bisect {
        #[command(subcommand)]
        cmd: BisectCommand,
    },
    /// Manage the repositories devd knows about.
    Repo {
        #[command(subcommand)]
        cmd: RepoCommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum BisectCommand {
    /// Start a bisect between a known-bad and a known-good revision.
    Start {
        bad: String,
        good: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Mark the current revision good, bad, or untestable.
    Mark {
        /// `good`, `bad` or `skip`.
        #[arg(value_enum)]
        verdict: VerdictArg,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Return to the branch and state the bisect started from.
    Reset {
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum RepoCommand {
    /// List repositories devd has indexed.
    List,
    /// Resolve a repository and describe it, without indexing it.
    Open {
        /// Any path inside the working tree.
        path: String,
    },
    /// Create a repository and index it.
    Init {
        path: String,
        /// The initial branch name.
        #[arg(long = "initial-branch")]
        initial_branch: Option<String>,
    },
    /// Add a repository to devd's index.
    Track {
        /// Any path inside the working tree.
        path: String,
    },
    /// Remove a repository from the index. Nothing on disk is touched.
    Untrack { path: String },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum VerdictArg {
    Good,
    Bad,
    Skip,
}

#[derive(Subcommand, Debug)]
pub(crate) enum BranchCommand {
    /// List branches.
    List {
        /// Include remote-tracking branches.
        #[arg(short = 'a', long)]
        all: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Create a branch.
    Create {
        name: String,
        /// Start the branch here instead of at `HEAD`.
        #[arg(long = "start-point")]
        start_point: Option<String>,
        /// Switch to the new branch.
        #[arg(long)]
        checkout: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete a branch.
    Delete {
        name: String,
        /// Delete even when the branch has unmerged commits.
        #[arg(short = 'D', long)]
        force: bool,
        /// Delete the branch on this remote as well.
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Rename a branch.
    Rename {
        from: String,
        to: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Check whether switching to a branch would overwrite local changes.
    Check {
        target: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete local branches already merged into a base branch.
    Prune {
        /// The branch to test against. Defaults to `HEAD`.
        #[arg(long)]
        base: Option<String>,
        /// Report the candidates without deleting anything.
        #[arg(long = "dry-run")]
        dry_run: bool,
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum StashCommand {
    /// List stash entries.
    List {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Shelve local changes.
    Push {
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Include untracked files.
        #[arg(short = 'u', long)]
        include_untracked: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Restore a stash without removing it.
    Apply {
        reference: Option<String>,
        /// Also restore what was staged.
        #[arg(long)]
        index: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Restore a stash and remove it.
    Pop {
        reference: Option<String>,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete a stash entry.
    Drop {
        reference: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Show a stash as a patch.
    Show {
        reference: Option<String>,
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum WorktreeCommand {
    /// List worktrees.
    List {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Add a worktree for a branch.
    Add {
        path: String,
        branch: String,
        /// Create the branch in the new worktree.
        #[arg(short = 'b', long)]
        create_branch: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove a worktree.
    Remove {
        path: String,
        /// Remove it even when it has uncommitted changes.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Drop bookkeeping for worktrees whose directories are gone.
    Prune {
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum RemoteCommand {
    /// List remotes.
    List {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Add a remote.
    Add {
        name: String,
        url: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Rename a remote.
    Rename {
        from: String,
        to: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove a remote and its remote-tracking branches.
    Remove {
        name: String,
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TagCommand {
    /// List tags.
    List {
        #[arg(long)]
        repo: Option<String>,
    },
    /// Create a tag.
    Create {
        name: String,
        #[arg(short = 'm', long)]
        message: Option<String>,
        /// Tag this revision instead of `HEAD`.
        #[arg(long)]
        rev: Option<String>,
        /// Move an existing tag.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete a tag.
    Delete {
        name: String,
        /// Also delete it from this remote.
        #[arg(long)]
        remote: Option<String>,
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum StrategyArg {
    /// Create a merge commit when histories have diverged.
    Merge,
    /// Replay local commits on top of the incoming ones.
    Rebase,
    /// Refuse to integrate unless it is a fast-forward.
    FastForwardOnly,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ResetArg {
    /// Move `HEAD` only; keep the index and working tree.
    Soft,
    /// Move `HEAD` and the index; keep the working tree.
    Mixed,
    /// Move `HEAD`, the index and the working tree. Destroys uncommitted work.
    Hard,
}

fn main() {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("devd: {e}");
            std::process::exit(exit_code_for(&e));
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    // `doctor` must work even when something it checks is broken, so it runs
    // before the application bootstrap that would fail on a missing Git.
    if let Command::Doctor = cli.command {
        return doctor(cli.json);
    }

    let app = App::bootstrap(cli.json, cli.plan, cli.yes)?;

    match cli.command {
        Command::Capabilities { group, query } => {
            list_capabilities(&app, group.as_deref(), query.as_deref())?;
            Ok(0)
        }
        Command::Describe { id } => {
            let described = app.registry.describe_json(&id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&described)
                    .map_err(|e| DevdError::internal(e.to_string()))?
            );
            Ok(0)
        }
        Command::Call { id, args, set } => {
            let value = merge_args(args, set)?;
            app.invoke(&id, value)?;
            Ok(0)
        }
        Command::Git { cmd } => {
            gitcat::run(&app, cmd)?;
            Ok(0)
        }
        Command::Audit { limit } => {
            let rows = app.store.list_audit(limit)?;
            if app.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows)
                        .map_err(|e| DevdError::internal(e.to_string()))?
                );
            } else if rows.is_empty() {
                println!("no audit entries yet");
            } else {
                for row in rows {
                    println!(
                        "{}  {:<14} {:<26} {:<12} {}",
                        row.at,
                        row.actor,
                        row.capability,
                        row.tier,
                        if row.ok { "ok" } else { "FAILED" }
                    );
                }
            }
            Ok(0)
        }
        Command::Doctor => unreachable!("handled before bootstrap"),
        Command::Mcp { client, list } => {
            if list {
                let tools = devd_mcp::tools();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&tools)
                        .map_err(|e| DevdError::internal(e.to_string()))?
                );
                return Ok(0);
            }
            // The Git probe is contributed here, as it is for the HTTP server,
            // because `devd-mcp` must not depend on one backend.
            let git_executable = app.settings.git_executable.clone();
            let extra: Vec<devd_mcp::ExtraChecks> =
                vec![Arc::new(move || vec![git_check(git_executable.as_deref())])];
            let server = devd_mcp::McpServer::new(app.registry.clone(), app.ctx(), client)
                .with_extra_checks(extra);
            // Nothing but protocol traffic may reach stdout.
            server.serve_stdio()?;
            Ok(0)
        }
        Command::Ui { port, no_open } => {
            web::serve(app, port, !no_open)?;
            Ok(0)
        }
        Command::Serve { port } => {
            let started = web::serve(app, port, false)?;
            if started.reused {
                println!("attached to the instance already on port {}", started.port);
            }
            Ok(0)
        }
    }
}

fn list_capabilities(app: &App, group: Option<&str>, query: Option<&str>) -> Result<()> {
    let mut caps = app.registry.list();
    if let Some(g) = group {
        caps.retain(|c| c.group == g);
    }
    if let Some(q) = query {
        caps = app.registry.search(q, group, 1000, 0);
    }
    caps.sort_by(|a, b| a.id.cmp(&b.id));

    if app.json {
        let described: Vec<_> = caps
            .iter()
            .map(|c| app.registry.describe_json(&c.id))
            .collect::<Result<Vec<_>>>()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&described)
                .map_err(|e| DevdError::internal(e.to_string()))?
        );
        return Ok(());
    }

    println!("{} capabilities", caps.len());
    let mut current = String::new();
    for cap in &caps {
        if cap.group != current {
            current = cap.group.clone();
            println!("\n{current}");
        }
        println!(
            "  {:<24} {:<12} {}{}",
            cap.id,
            cap.tier.as_str(),
            cap.summary,
            if cap.tier.requires_approval() {
                "  [needs approval]"
            } else {
                ""
            }
        );
    }
    println!(
        "\nGroups are the unit of discovery: an agent loads a group's tools on demand \
         rather than every capability at once."
    );
    Ok(())
}

/// Environment check. Independent of the capability registry so that it can
/// report the registry's own preconditions as broken rather than failing.
///
/// The checks themselves live in `devd-core`, and the Git check is appended
/// here, because that is the single split that lets `devd doctor` and the
/// application's System panel report identical results: the core contributes
/// what it can see, and each entry point adds what only it can reach.
fn doctor(json: bool) -> Result<i32> {
    let paths = Paths::discover()?;
    let settings = Settings::load(&paths.settings_file()).unwrap_or_default();

    let mut checks = devd_core::doctor::core_checks(&paths, &settings);
    checks.push(app::git_check(settings.git_executable.as_deref()));

    let failures = checks.iter().filter(|c| c.is_fail()).count();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": failures == 0,
                "failures": failures,
                "checks": checks,
            }))
            .map_err(|e| DevdError::internal(e.to_string()))?
        );
    } else {
        for check in &checks {
            println!(
                "{} {:<10} {}",
                match check.level {
                    devd_core::Level::Fail => "FAIL",
                    devd_core::Level::Info => "info",
                    devd_core::Level::Ok => "ok  ",
                },
                check.name,
                check.detail
            );
        }
        println!();
        if failures == 0 {
            println!("all checks passed");
        } else {
            println!("{failures} check(s) failed");
        }
    }

    Ok(if failures == 0 { 0 } else { 1 })
}
