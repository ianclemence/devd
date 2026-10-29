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

mod aicat;
mod app;
mod daemon;
mod devcat;
mod gitcat;
mod runtimecat;
mod setup;
mod stackcat;
mod web;
mod window;

use app::{exit_code_for, git_check, merge_args, App};

#[derive(Parser, Debug)]
#[command(
    name = "devd",
    version,
    about = "Agent-native local development environment manager",
    long_about = "devd manages local runtimes, services, sites, hosts and TLS, and gives humans and \
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
    /// Install and manage runtimes: PHP, Node and Go.
    Runtime {
        #[command(subcommand)]
        cmd: RuntimeCommand,
    },
    /// Install, start and stop services: nginx, the PHP listener and MariaDB.
    Service {
        #[command(subcommand)]
        cmd: ServiceCommand,
    },
    /// Projects served over a local hostname.
    Site {
        #[command(subcommand)]
        cmd: SiteCommand,
    },
    /// PHP limits, the worker pool and extensions.
    Php {
        #[command(subcommand)]
        cmd: PhpCommand,
    },
    /// The managed block in the system hosts file.
    Hosts {
        #[command(subcommand)]
        cmd: HostsCommand,
    },
    /// The generated web server configuration.
    Web {
        #[command(subcommand)]
        cmd: WebCommand,
    },
    /// Start or stop every service together.
    Stack {
        #[command(subcommand)]
        cmd: StackCommand,
    },
    /// The local certificate authority and site certificates.
    Tls {
        #[command(subcommand)]
        cmd: TlsCommand,
    },
    /// Databases on devd's MariaDB, or PostgreSQL with `--engine postgres`.
    Db {
        /// `mariadb` (the default) or `postgres`.
        #[arg(long, global = true)]
        engine: Option<String>,
        #[command(subcommand)]
        cmd: DbCommand,
    },
    /// Show recent audit entries.
    Audit {
        /// How many entries to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: u32,
    },
    /// Prepare this installation: runtimes, services and a first site.
    ///
    /// Safe to run again; steps that are already done are skipped. Without
    /// `--yes` it stops before anything that needs approval, which today is
    /// trusting the local certificate authority.
    ///
    /// PHP and the web server are always installed. Pick the rest with `--services`: a comma-separated
    /// list from mariadb, redis, memcached, mailpit, or `none`. The default is `mariadb`.
    Setup {
        /// The services to install besides PHP and the web server.
        #[arg(long, value_delimiter = ',')]
        services: Option<Vec<String>>,
        /// The PHP series to install, e.g. `8.4`. The default is the latest stable.
        #[arg(long)]
        php: Option<String>,
        /// The PHP limits profile: `local` (generous) or `production` (a typical Ploi server).
        #[arg(long)]
        profile: Option<String>,
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
    /// Create a new project from a stack (guided when run in a terminal).
    New(NewArgs),
    /// Browse the supported stacks, their options and the compatibility matrix.
    Stacks {
        #[command(subcommand)]
        cmd: StacksCommand,
    },
    /// Detect, install, remove and run the tools and services projects need.
    Tools {
        #[command(subcommand)]
        cmd: ToolsCommand,
    },
    /// Understand, import and work on projects: detect, tasks, run, exec.
    Project {
        #[command(subcommand)]
        cmd: ProjectCommand,
    },
    /// Talk to devd's built-in agent, which can use every capability devd has.
    Chat(ChatArgs),
    /// List, delete and clear conversations.
    Chats {
        #[command(subcommand)]
        cmd: ChatsCommand,
    },
    /// The agent team: describe a goal, review the plan, follow the work, see the evidence.
    Team {
        #[command(subcommand)]
        cmd: TeamCommand,
    },
    /// Run templates: a goal written once, with blanks, that the team can be started on again.
    Template {
        #[command(subcommand)]
        cmd: TemplateCommand,
    },
    /// Schedules: team runs devd starts on a clock, while devd is running.
    Schedule {
        #[command(subcommand)]
        cmd: ScheduleCommand,
    },
    /// Skills agents load: playbooks, techniques and principles.
    Skills {
        #[command(subcommand)]
        cmd: SkillsCommand,
    },
    /// Model providers and API keys for the built-in agent.
    Provider {
        #[command(subcommand)]
        cmd: ProviderCommand,
    },
    /// The model catalogue, from the vendors' own documentation and APIs.
    Models {
        #[command(subcommand)]
        cmd: ModelsCommand,
    },
    /// Messages agents left for you.
    Inbox {
        #[command(subcommand)]
        cmd: InboxCommand,
    },
    /// Operations agents asked you to approve.
    Approvals {
        #[command(subcommand)]
        cmd: ApprovalsCommand,
    },
    /// Open the terminal dashboard.
    Tui,
    /// Serve the Model Context Protocol on stdin/stdout, for agents.
    Mcp {
        /// Name reported to the audit trail for the calling client.
        #[arg(long, default_value = "mcp-client")]
        client: String,
        /// Print the tool list and exit, instead of serving.
        #[arg(long)]
        list: bool,
    },
    /// The resident process that runs scheduled work and serves the window, the dashboard and
    /// the command line: `run`, `start`, `stop` or `status`.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCommand,
    },
    /// Update devd from its signed releases: `check`, `apply`; `keygen` and `sign` are for
    /// whoever publishes releases.
    Update {
        #[command(subcommand)]
        cmd: UpdateCommand,
    },
    /// Start the stack when you sign in: `status`, `enable` or `disable`.
    Autostart {
        #[command(subcommand)]
        cmd: AutostartCommand,
    },
    /// Make `php` and `composer` work from any terminal: `status`, `enable` or `disable`.
    #[command(name = "path")]
    PathSetup {
        #[command(subcommand)]
        cmd: PathCommand,
    },
    /// Install Composer, checked against the digest getcomposer.org publishes.
    Composer {
        #[command(subcommand)]
        cmd: ComposerCommand,
    },
    /// Check the environment for problems.
    Doctor,
    /// Rebuild a damaged state database from what can still be read. Close devd first; the damaged files are kept.
    RepairState,
}

#[derive(clap::Args, Debug)]
pub(crate) struct NewArgs {
    /// Stack id, e.g. `laravel` or `nextjs` (see `devd stacks list`).
    pub stack: Option<String>,
    /// Project name; also the directory name.
    pub name: Option<String>,
    /// Directory to create the project in. Defaults to the projects root.
    #[arg(long)]
    pub dest: Option<String>,
    /// Framework version id, e.g. `12` for Laravel 12.
    #[arg(long = "stack-version")]
    pub stack_version: Option<String>,
    /// Package manager, e.g. `pnpm`.
    #[arg(long)]
    pub pm: Option<String>,
    /// A stack option as `key=value`, e.g. `-o database=pgsql`. Repeatable.
    #[arg(short = 'o', long = "option", value_name = "KEY=VALUE")]
    pub option: Vec<String>,
    /// Do not initialise a git repository.
    #[arg(long)]
    pub no_git: bool,
    /// Do not install dependencies.
    #[arg(long)]
    pub no_install: bool,
    /// Extra argument appended to the scaffolding command. Repeatable.
    #[arg(long = "extra", allow_hyphen_values = true)]
    pub extra: Vec<String>,
    /// Ask the guided questions even though a stack and name were given.
    #[arg(long)]
    pub interactive: bool,
}

#[derive(Subcommand, Debug)]
pub(crate) enum StacksCommand {
    /// List the supported stacks.
    List {
        /// web, frontend, backend, mobile or language.
        #[arg(long)]
        category: Option<String>,
    },
    /// Show one stack: versions, options, tasks, services, documentation.
    Show { id: String },
    /// Print the JSON Schemas of requests, plans, reports and project context.
    Schema,
    /// Print the compatibility matrix.
    Matrix {
        /// Emit the Markdown used for docs/stacks.md.
        #[arg(long)]
        markdown: bool,
    },
    /// Show recent initialisations, tasks and tool changes.
    Runs { subject: Option<String> },
    /// Cancel a running operation by its run id.
    Cancel { run_id: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ToolsCommand {
    /// List every tool and service with a live check.
    List {
        #[arg(long)]
        category: Option<String>,
    },
    /// Check whether this machine satisfies a stack.
    Check {
        stack: String,
        #[arg(long = "stack-version")]
        version: Option<String>,
        #[arg(long)]
        pm: Option<String>,
    },
    /// Show exactly what installing or removing a tool would do.
    Plan {
        tool: String,
        #[arg(long)]
        remove: bool,
    },
    /// Install a tool (asks for approval).
    Install { tool: String },
    /// Remove a tool (asks for approval; data is kept).
    Remove { tool: String },
    /// Show a service's state and health.
    Status { tool: String },
    /// Start a service and wait until it answers.
    Start { tool: String },
    /// Stop a service.
    Stop { tool: String },
    /// Restart a service.
    Restart { tool: String },
    /// Recent service log lines.
    Logs { tool: String },
    /// Connected Android devices and emulators (adb).
    Devices,
    /// Databases and services already on this machine: what is installed, what answers, what needs them.
    Services,
    /// Decide how devd treats a service found on this machine (changes nothing on the machine).
    Decide {
        tool: String,
        /// adopt, observe, external or ignore.
        mode: Option<String>,
        /// Port of your own instance (for external).
        #[arg(long)]
        port: Option<u16>,
        /// Forget the decision.
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ProjectCommand {
    /// Inspect a directory: stack, package manager, runtimes, tasks, services, docs.
    Detect {
        #[arg(default_value = ".")]
        path: String,
        /// Treat the project as this stack instead of detecting it.
        #[arg(long)]
        stack: Option<String>,
    },
    /// Open a terminal in a project, with its pinned runtimes and devd first on PATH.
    Terminal {
        #[arg(default_value = ".")]
        path: String,
    },
    /// Register an existing project without changing it.
    Import {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        stack: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List projects devd knows.
    List,
    /// Look around a folder: the directories inside it, and which look like projects.
    Browse {
        #[arg(default_value = ".")]
        path: String,
        /// Include dot folders and hidden files.
        #[arg(long)]
        hidden: bool,
    },
    /// Unregister a project (files are kept).
    Forget { path: String },
    /// List a project's tasks.
    Tasks {
        #[arg(default_value = ".")]
        path: String,
    },
    /// Run a task, e.g. `devd project run . test`.
    Run {
        path: String,
        task: String,
        /// Start in the background.
        #[arg(long)]
        background: bool,
        /// Wait for it even if it is a long-running task.
        #[arg(long, conflicts_with = "background")]
        foreground: bool,
        /// Arguments passed to the task.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run an allow-listed framework command, e.g. `devd project exec . -- php artisan route:list`.
    Exec {
        path: String,
        #[arg(long)]
        timeout: Option<u64>,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
    /// List background tasks.
    Ps,
    /// Stop a background task.
    Stop { name: String },
    /// Show a background task's log.
    Logs {
        name: String,
        #[arg(long)]
        lines: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ChatsCommand {
    /// List conversations.
    List,
    /// Delete one conversation and its log. Asks for approval.
    Delete { thread: String },
    /// Delete every finished conversation (or those untouched for --days). Asks for approval.
    Prune {
        /// Only conversations not touched for this many days.
        #[arg(long)]
        days: Option<u64>,
    },
    /// Delete the inbox messages that were answered or dismissed. Asks for approval.
    ClearInbox,
}

#[derive(clap::Args, Debug)]
pub(crate) struct ChatArgs {
    /// What to ask. Omit in a terminal for an interactive conversation.
    pub message: Option<String>,
    /// Continue an earlier conversation.
    #[arg(long)]
    pub thread: Option<String>,
    /// Project directory the agent works in.
    #[arg(long)]
    pub project: Option<String>,
    /// Provider id (see `devd provider list`).
    #[arg(long)]
    pub provider: Option<String>,
    /// Model id (see `devd models list`).
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ProviderCommand {
    /// List configured providers (keys are never shown).
    List,
    /// Show the ready-made provider presets and where to get keys.
    Presets,
    /// Add or update a provider. The key comes from DEVD_API_KEY or a prompt.
    Add {
        /// Provider id, e.g. `anthropic`.
        id: String,
        /// Start from a preset: anthropic, openai, gemini, openrouter, groq, ollama, lm-studio, custom.
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        /// Read the key from this environment variable instead of storing it.
        #[arg(long)]
        key_env: Option<String>,
        /// Default model id.
        #[arg(long)]
        model: Option<String>,
        /// Use for new conversations.
        #[arg(long)]
        default: bool,
    },
    /// Check the URL and key by listing the provider's models.
    Test { id: String },
    /// Fetch and cache the models this provider offers.
    Models { id: String },
    /// Use this provider for new conversations.
    Default { id: String },
    /// Remove a provider and its stored key.
    Remove { id: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TeamCommand {
    /// Start a run. Waits for you to approve the plan unless you pass --yes-plan.
    Start {
        /// What to build or change.
        goal: String,
        /// Existing project directory.
        #[arg(long)]
        project: Option<String>,
        /// Or the name of a new project.
        #[arg(long)]
        new: Option<String>,
        /// Tasks that may run side by side.
        #[arg(long, default_value_t = 2)]
        parallel: i64,
        /// Do not ask for plan approval.
        #[arg(long)]
        yes_plan: bool,
        /// A requirement the finished work must meet; repeat for more. devd checks each
        /// one (with Jev, when configured) and starts another pass for any that are not met.
        #[arg(long = "done-when")]
        done_when: Vec<String>,
        /// Extra passes allowed to meet the definition of done.
        #[arg(long, default_value_t = 2)]
        rounds: i64,
        /// Return immediately instead of following the run.
        #[arg(long)]
        detach: bool,
    },
    /// List runs.
    List,
    /// Show a run: plan, tasks, evidence and log. Follows it with --follow.
    Show {
        run: String,
        #[arg(long)]
        follow: bool,
    },
    /// The specialists, what each may do and what each must prove.
    Roles,
    /// Approve the plan; the work starts.
    Approve { run: String },
    /// Give the team a new instruction.
    Say { run: String, text: String },
    /// Pause a run.
    Pause { run: String },
    /// Resume a paused run.
    Resume { run: String },
    /// Stop a run.
    Cancel { run: String },
    /// Delete a finished run with its tasks, log and conversations. Asks for approval.
    Delete { run: String },
    /// Delete every finished run (or those untouched for --days). Asks for approval.
    Prune {
        /// Only runs not touched for this many days.
        #[arg(long)]
        days: Option<u64>,
    },
    /// Run a failed, blocked or unverified task again.
    Retry {
        task: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Accept a task without the required proof (recorded as accepted by hand).
    Accept { task: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SkillsCommand {
    /// List skills.
    List {
        query: Option<String>,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Print one skill.
    Show { name: String },
    /// Add your own skill from a Markdown file (the body).
    Add {
        name: String,
        #[arg(long)]
        description: String,
        #[arg(long, default_value = "technique")]
        kind: String,
        #[arg(long)]
        roles: Vec<String>,
        /// File containing the instructions.
        file: String,
    },
    /// Remove one of your own skills.
    Remove { name: String },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ModelsCommand {
    /// Which model does which kind of work.
    Roles,
    /// Choose the models for a kind of work, first choice first (provider:model ...).
    Assign {
        role: String,
        /// Entries like `anthropic:claude-opus-5-5`; none clears the role.
        models: Vec<String>,
        /// Only for this project directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Show cached models.
    List {
        /// `anthropic-docs` or `provider:<id>`.
        #[arg(long)]
        source: Option<String>,
    },
    /// Fetch the official Claude model list from Anthropic's documentation.
    Refresh,
}

#[derive(Subcommand, Debug)]
pub(crate) enum InboxCommand {
    /// Show messages from agents.
    List {
        /// Include resolved ones.
        #[arg(long)]
        all: bool,
    },
    /// Answer a message.
    Reply {
        id: String,
        /// Your answer; empty just resolves it.
        text: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ApprovalsCommand {
    /// Show requests waiting for you.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Approve and run a request.
    Approve { id: String },
    /// Decline a request.
    Decline { id: String },
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
pub(crate) enum RuntimeCommand {
    /// List installed runtimes.
    List,
    /// Show the builds the vendor publishes, and the digest each is checked against.
    Available {
        /// Runtime kind: `php`, `node` or `go`.
        #[arg(default_value = "php")]
        kind: String,
        /// Prefer the thread-safe PHP build, which the Apache module requires.
        #[arg(long)]
        thread_safe: bool,
    },
    /// Install a runtime, e.g. `php@8.3`, `node@lts`, `go@latest`.
    Install {
        /// Kind and version joined by `@`; the version may be a series (`8.3`),
        /// a full version (`8.3.35`), `latest` or a channel (`lts`).
        spec: String,
        /// Prefer the thread-safe PHP build.
        #[arg(long)]
        thread_safe: bool,
        /// Reinstall over an existing copy of the same version.
        #[arg(long)]
        force: bool,
    },
    /// Run an installed runtime and report its version.
    Verify {
        /// Kind and version, e.g. `php@8.3.35`.
        spec: String,
    },
    /// Remove an installed runtime. This deletes files.
    Remove {
        /// Kind and version, e.g. `php@8.3.14`.
        spec: String,
    },
    /// Choose the PHP version new sites use.
    Use {
        /// An installed version, e.g. `8.3.35`.
        version: String,
    },
    /// Run a project with its own runtime version (`python@3.12`), or clear the pin with `--clear`.
    Pin {
        /// Kind and version or series, e.g. `python@3.12`.
        spec: String,
        /// The project folder (default: the current folder).
        #[arg(long)]
        project: Option<std::path::PathBuf>,
        /// Remove the pin instead of setting it.
        #[arg(long)]
        clear: bool,
    },
    /// List the runtime versions projects are pinned to.
    Pins,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ServiceCommand {
    /// List every service with a live check of what is really running.
    List,
    /// Show the builds the vendor publishes for a service.
    Available {
        /// Service name: `nginx`, `apache` or `mariadb`.
        name: String,
    },
    /// Look for a service's program already on this machine: Apache, Redis, PostgreSQL, MongoDB or Memcached.
    Find {
        /// Service name: `apache`, `redis`, `postgres`, `mongodb` or `memcached`.
        name: String,
    },
    /// Use a program that is already on disk, without copying or changing it.
    Adopt {
        /// Service name: `apache`, `redis`, `postgres`, `mongodb` or `memcached`.
        name: String,
        /// The program's folder, for example `C:\\Apache24`.
        path: String,
        /// Port it should listen on; the configured HTTP port by default.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Download, verify and install a service.
    Install {
        /// Service name: `nginx`, `apache` or `mariadb`.
        name: String,
        /// Version, series or `latest`.
        #[arg(long, default_value = "latest")]
        version: String,
        /// Port it should listen on. MariaDB defaults to 3306, nginx to the
        /// configured HTTP port.
        #[arg(long)]
        port: Option<u16>,
        /// Replace an existing installation of a different version.
        #[arg(long)]
        force: bool,
    },
    /// Start a service and confirm it answers.
    Start {
        /// Service name, e.g. `nginx`, `php-cgi`, `mariadb`.
        name: String,
    },
    /// Stop a service.
    Stop {
        /// Service name.
        name: String,
    },
    /// Stop and start a service.
    Restart {
        /// Service name.
        name: String,
    },
    /// Show one service's state.
    Status {
        /// Service name.
        name: String,
    },
    /// Show the tail of a service's own log.
    Logs {
        /// Service name.
        name: String,
        /// How many lines.
        #[arg(short = 'n', long, default_value_t = 40)]
        lines: u32,
    },
    /// Uninstall a service. This deletes files.
    Remove {
        /// Service name.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum PhpCommand {
    /// Show the PHP limits, the profile and the worker count.
    Settings {
        /// A series such as 8.4. Omit for every installed one.
        series: Option<String>,
    },
    /// Change the profile (`local` or `production`), the worker count, or single values.
    Configure {
        /// `local` (generous limits) or `production` (a typical Ploi server).
        #[arg(long)]
        profile: Option<String>,
        /// PHP worker processes per version, 1 to 16.
        #[arg(long)]
        workers: Option<u8>,
        /// A value to set, as `key=value` (memory_limit, upload_max_filesize, post_max_size, max_execution_time, ...). Repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// Limit the value changes to one series.
        #[arg(long)]
        series: Option<String>,
    },
    /// List a PHP version's extensions and which are on.
    Extensions {
        /// A series such as 8.4. Omit for the primary one.
        series: Option<String>,
    },
    /// Switch an extension on or off.
    Ext {
        /// The extension, e.g. gd.
        name: String,
        /// `on` or `off`.
        state: String,
        /// A series such as 8.4. Omit for the primary one.
        #[arg(long)]
        series: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SiteCommand {
    /// List the sites.
    List,
    /// List the folders whose projects become sites.
    Parked,
    /// Report the projects in the parked folders: what each is, and whether it could be a site. Changes nothing.
    Discover,
    /// Park a folder: look for projects in it. This creates no site; `devd site sync` does.
    Park {
        /// The folder, as an absolute path.
        path: String,
    },
    /// Open a public link to a site (Cloudflare quick tunnel) for a limited time.
    Share {
        /// Site name.
        name: String,
        /// How long the link stays open (default 120, at most 1440).
        #[arg(long)]
        minutes: Option<u64>,
    },
    /// Close a site's public link.
    Unshare {
        /// Site name.
        name: String,
    },
    /// List the open public links.
    Shares,
    /// Guard a site's php_value / php_flag lines in .htaccess so Apache accepts them.
    FixHtaccess {
        /// Site name.
        name: String,
    },
    /// Stop parking a folder. Its sites stay.
    Unpark {
        /// The folder, as an absolute path.
        path: String,
    },
    /// Make a site of each project in the parked folders that has a web entry point and no site yet.
    Sync {
        /// Only these site names (as `devd site discover` lists them).
        names: Vec<String>,
    },
    /// Say what a site's directory serves: `php` or `static`. With no value, read it from the folder again.
    Serving {
        /// Site name.
        name: String,
        /// `php` or `static`. Omit to read it from the folder.
        serving: Option<String>,
    },
    /// Create a site: directory, certificate, record and web server block.
    Create {
        /// Project name, used for the directory and, by default, the hostname.
        name: String,
        /// Fully qualified hostname. Defaults to `<name>.<tld>`.
        #[arg(long)]
        host: Option<String>,
        /// Project directory. Defaults to `<www>/<name>`.
        #[arg(long)]
        root: Option<String>,
        /// Serve over plain HTTP and issue no certificate.
        #[arg(long)]
        no_tls: bool,
        /// Create a database named after the site, when MariaDB is running.
        #[arg(long)]
        db: bool,
        /// Serve a program that is already running instead of a directory: a
        /// port such as `3000`, or an address such as `localhost:5173`.
        #[arg(long)]
        upstream: Option<String>,
        /// What the directory serves, when you know: `php` or `static`. Left out, it is read from the folder.
        #[arg(long)]
        serving: Option<String>,
    },
    /// Point a site at a running program, or back at its directory.
    Upstream {
        /// Project name.
        name: String,
        /// A port or an address on this machine. Omit to serve the project
        /// directory again.
        upstream: Option<String>,
    },
    /// Remove a site. This can delete the project directory.
    Remove {
        /// Project name.
        name: String,
        /// Also delete the project directory, which must be inside the www root.
        #[arg(long)]
        delete_files: bool,
    },
    /// Pin a site to a PHP version, or let it follow the default again.
    Php {
        /// Project name.
        name: String,
        /// The PHP version to pin it to. Omit to let it follow the default.
        version: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum HostsCommand {
    /// Show what devd has in the hosts file, and what a sync would change.
    Show,
    /// Write the managed block in the hosts file. Needs administrator rights;
    /// on Windows, `--elevate` asks for them with the usual consent prompt.
    Sync {
        /// Ask Windows for permission instead of failing when the file is read-only to you.
        #[arg(long)]
        elevate: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TemplateCommand {
    /// List the templates, with their parameters.
    List,
    /// Show one template: parameters, definition of done and goal.
    Show {
        /// Template id.
        id: String,
    },
    /// Start the team from a template and follow the run.
    Run {
        /// Template id.
        id: String,
        /// Existing project directory.
        #[arg(long)]
        project: Option<String>,
        /// Or the name of a new project.
        #[arg(long)]
        new: Option<String>,
        /// A value for a template parameter, as name=value; repeat for more.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// More criteria for the definition of done.
        #[arg(long = "done-when")]
        done_when: Vec<String>,
        /// Do not ask for plan approval.
        #[arg(long)]
        yes_plan: bool,
        /// Return immediately instead of following the run.
        #[arg(long)]
        detach: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScheduleCommand {
    /// List the schedules and when each is next due.
    List,
    /// Run the team on a clock: `every 30m`, `daily 09:30`, `weekdays 09:30`, `mon,wed 18:00`.
    Create {
        /// A short unique name.
        name: String,
        /// When (local time).
        spec: String,
        /// The project directory the runs work in.
        #[arg(long)]
        project: String,
        /// The template to start.
        #[arg(long)]
        template: Option<String>,
        /// A value for a template parameter, as name=value; repeat for more.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
        /// Or a goal written out.
        #[arg(long)]
        goal: Option<String>,
        /// More criteria for the definition of done.
        #[arg(long = "done-when")]
        done_when: Vec<String>,
        /// Waive plan approval for the runs it starts (never in production mode).
        #[arg(long)]
        yes_plan: bool,
    },
    /// Turn a schedule on.
    Enable {
        /// Schedule name.
        name: String,
    },
    /// Turn a schedule off.
    Disable {
        /// Schedule name.
        name: String,
    },
    /// Start a schedule's run now.
    Run {
        /// Schedule name.
        name: String,
    },
    /// Delete a schedule.
    Delete {
        /// Schedule name.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum PathCommand {
    /// Whether devd's `php` and `composer` are on your PATH, and which exist.
    Status,
    /// Put devd's folder first on your user PATH. No administrator rights; new terminals see it.
    Enable,
    /// Take it off again. Every other PATH entry is left alone.
    Disable,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ComposerCommand {
    /// Download and install the newest stable Composer.
    Install {
        /// Replace an installed Composer.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum UpdateCommand {
    /// Is there a newer release, and is it signed?
    Check,
    /// Download, verify and install the latest signed release.
    Apply,
    /// Make a release signing key. Writes the private seed to a file (keep it secret) and
    /// prints the public key to put in `RELEASE_PUBLIC_KEY`.
    Keygen {
        /// Where to write the private seed. Refuses to overwrite.
        #[arg(long)]
        out: std::path::PathBuf,
    },
    /// Sign a file (normally SHA256SUMS.txt), writing `<file>.sig`.
    Sign {
        /// The file to sign.
        file: std::path::PathBuf,
        /// The private seed file. Defaults to $DEVD_RELEASE_KEY_FILE.
        #[arg(long)]
        key: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum DaemonCommand {
    /// Run the resident process in this terminal.
    Run {
        /// Port to listen on. Defaults to the last used port, then 8787 upward.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Start it detached, with no window, and wait until it answers.
    Start,
    /// Stop it. Services it started keep running.
    Stop,
    /// Whether it is running, and where.
    Status,
}

#[derive(Subcommand, Debug)]
pub(crate) enum AutostartCommand {
    /// Whether the stack starts when you sign in.
    Status,
    /// Run `devd stack up` when you sign in. No administrator rights.
    Enable,
    /// Stop doing that. Running services keep running.
    Disable,
}

#[derive(Subcommand, Debug)]
pub(crate) enum WebCommand {
    /// Rebuild the web server configuration from the state store.
    Render,
    /// Check the configuration with the web server (`nginx -t` or `httpd -t`).
    Test,
    /// Check and then reload the running web server.
    Reload,
    /// Serve sites on different HTTP and HTTPS ports; the running server restarts.
    Ports {
        /// Port for plain HTTP.
        #[arg(long)]
        http: Option<u16>,
        /// Port for HTTPS.
        #[arg(long)]
        https: Option<u16>,
    },
    /// Serve sites with `nginx` or `apache` (applies when devd next starts).
    Use {
        /// `nginx` or `apache`.
        server: String,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum StackCommand {
    /// Start every enabled service, in dependency order.
    Up,
    /// Stop every running service.
    Down,
}

#[derive(Subcommand, Debug)]
pub(crate) enum TlsCommand {
    /// Create the local certificate authority if it does not exist.
    Ca,
    /// Print the authority certificate, for importing into a browser.
    Show,
    /// Add the authority to the system trust store. Needs approval.
    Trust {
        /// Add it to the machine-wide store instead of this user's. Needs an
        /// elevated shell.
        #[arg(long)]
        machine: bool,
    },
    /// Remove the authority from the system trust store.
    Untrust {
        /// Remove it from the machine-wide store.
        #[arg(long)]
        machine: bool,
    },
    /// Issue a certificate for a hostname.
    Issue {
        /// The hostname, e.g. `blog.test`.
        host: String,
    },
    /// List issued certificates.
    List,
}

#[derive(Subcommand, Debug)]
pub(crate) enum DbCommand {
    /// List the databases.
    List,
    /// Create a database with the utf8mb4 character set.
    Create {
        /// Database name.
        name: String,
    },
    /// Drop a database and everything in it.
    Drop {
        /// Database name.
        name: String,
    },
    /// Write a database to a SQL file (devd's MariaDB, or another server on this machine with --from-port).
    Dump {
        /// Database name.
        database: String,
        /// Where to write it. Defaults to a timestamped file under the devd backups folder.
        #[arg(long)]
        path: Option<String>,
        /// Dump from another server on this machine, such as a MySQL on 3306.
        #[arg(long)]
        from_port: Option<u16>,
    },
    /// Run a SQL file against a database on devd's MariaDB.
    Restore {
        /// Database name; created when missing.
        database: String,
        /// The SQL file.
        path: String,
        /// Run it even though the database already has tables.
        #[arg(long)]
        replace: bool,
    },
    /// Copy a database from another server on this machine (a MySQL) into devd's MariaDB.
    Copy {
        /// Database name on the source.
        database: String,
        /// The source server's port, usually 3306.
        #[arg(long)]
        from_port: u16,
        /// Name for the copy; the same name by default.
        #[arg(long)]
        as_name: Option<String>,
        /// Copy over a database that already has tables.
        #[arg(long)]
        replace: bool,
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

/// What a launcher needs to decide which program to run: the existing state, read, and nothing
/// else. It does not start the application (which needs Git, registers every capability and
/// would create a missing home), so `php` works on a machine without Git and is quick.
fn shim_context() -> Result<devd_core::capability::Ctx> {
    let paths = Paths::discover()?;
    if !paths.state_db().is_file() {
        return Err(DevdError::not_found(format!(
            "devd has no data at {} yet; if devd lives elsewhere, set DEVD_HOME to its folder",
            paths.root().display()
        )));
    }
    let settings = devd_core::config::Settings::load(&paths.settings_file())?;
    // Read only: a launcher runs somebody's program and must not write devd's database. A database
    // from before this build's newest migration cannot answer, and the first thing that starts devd
    // migrates it, so the launcher then waits for that rather than doing it itself.
    let store = devd_core::state::Store::open_read_only(&paths.state_db())?;
    if store.schema_version()? < devd_core::state::LATEST_SCHEMA {
        return Err(DevdError::conflict(
            "devd was updated and has not started since; start devd once (`devd doctor` does not count, it changes nothing) and this command will work",
        ));
    }
    Ok(devd_core::capability::Ctx {
        store: Arc::new(store),
        paths: Arc::new(paths),
        settings: Arc::new(settings),
        events: devd_core::events::EventBus::new(),
        actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
        run: None,
        dry_run: false,
    })
}

/// `devd shim <command> [arguments]`: what the `php` and `composer` launchers call.
///
/// It is read from the raw arguments, before the command line is parsed, because devd's own
/// options (`--json`, `--plan`, `-y`) would otherwise take arguments meant for the program, and
/// `php artisan route:list --json` is a real command.
fn shim_main(raw: &[String]) -> i32 {
    let Some((command, args)) = raw.split_first() else {
        eprintln!("devd: shim needs a command name (php or composer)");
        return 2;
    };
    let result = shim_context().and_then(|ctx| {
        let cwd = std::env::current_dir()?;
        devd_ops::shell::run(&ctx, command, args, &cwd)
    });
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("devd: {e}");
            exit_code_for(&e)
        }
    }
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("shim") {
        std::process::exit(shim_main(&raw[1..]));
    }
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

    // The database is what is being repaired, so nothing may have it open yet.
    if let Command::RepairState = cli.command {
        return repair_state(cli.json);
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
        Command::New(a) => {
            devcat::new_project(&app, a)?;
            Ok(0)
        }
        Command::Chat(a) => {
            app.resume_agents();
            aicat::chat(&app, a)?;
            Ok(0)
        }
        Command::Chats { cmd } => {
            aicat::chats(&app, cmd)?;
            Ok(0)
        }
        Command::Team { cmd } => {
            app.resume_agents();
            aicat::team(&app, cmd)?;
            Ok(0)
        }
        Command::Template { cmd } => {
            app.resume_agents();
            aicat::template(&app, cmd)?;
            Ok(0)
        }
        Command::Schedule { cmd } => {
            aicat::schedule(&app, cmd)?;
            Ok(0)
        }
        Command::Skills { cmd } => {
            aicat::skills(&app, cmd)?;
            Ok(0)
        }
        Command::Provider { cmd } => {
            aicat::provider(&app, cmd)?;
            Ok(0)
        }
        Command::Models { cmd } => {
            aicat::models(&app, cmd)?;
            Ok(0)
        }
        Command::Inbox { cmd } => {
            aicat::inbox(&app, cmd)?;
            Ok(0)
        }
        Command::Approvals { cmd } => {
            aicat::approvals(&app, cmd)?;
            Ok(0)
        }
        Command::Stacks { cmd } => {
            devcat::stacks(&app, cmd)?;
            Ok(0)
        }
        Command::Tools { cmd } => {
            devcat::tools(&app, cmd)?;
            Ok(0)
        }
        Command::Project { cmd } => {
            devcat::project(&app, cmd)?;
            Ok(0)
        }
        Command::Git { cmd } => {
            gitcat::run(&app, cmd)?;
            Ok(0)
        }
        Command::Runtime { cmd } => {
            runtimecat::run(&app, cmd)?;
            Ok(0)
        }
        Command::Service { cmd } => {
            stackcat::service(&app, cmd)?;
            Ok(0)
        }
        Command::Site { cmd } => {
            stackcat::site(&app, cmd)?;
            Ok(0)
        }
        Command::Php { cmd } => {
            stackcat::php(&app, cmd)?;
            Ok(0)
        }
        Command::Hosts { cmd } => {
            stackcat::hosts(&app, cmd)?;
            Ok(0)
        }
        Command::Web { cmd } => {
            stackcat::web(&app, cmd)?;
            Ok(0)
        }
        Command::Stack { cmd } => {
            stackcat::stack(&app, cmd)?;
            Ok(0)
        }
        Command::Tls { cmd } => {
            stackcat::tls(&app, cmd)?;
            Ok(0)
        }
        Command::Db { engine, cmd } => {
            stackcat::db(&app, engine, cmd)?;
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
        Command::Update { cmd } => match cmd {
            UpdateCommand::Check => {
                app.invoke("update.check", serde_json::json!({}))?;
                Ok(0)
            }
            UpdateCommand::Apply => {
                app.invoke("update.apply", serde_json::json!({}))?;
                Ok(0)
            }
            UpdateCommand::Keygen { out } => {
                if out.exists() {
                    return Err(DevdError::conflict(format!("{} exists; it will not be overwritten", out.display())));
                }
                let seed = devd_core::update::generate_seed()?;
                let public = devd_core::update::public_key_of(&seed)?;
                devd_core::util::atomic_write(&out, &seed)?;
                println!("private seed written to {} (keep it secret and backed up)", out.display());
                println!("public key (put this in RELEASE_PUBLIC_KEY): {public}");
                Ok(0)
            }
            UpdateCommand::Sign { file, key } => {
                let key = key
                    .or_else(|| std::env::var_os("DEVD_RELEASE_KEY_FILE").map(std::path::PathBuf::from))
                    .ok_or_else(|| DevdError::invalid("name the key with --key or DEVD_RELEASE_KEY_FILE"))?;
                let seed = std::fs::read_to_string(&key)
                    .map_err(|e| DevdError::not_found(format!("cannot read the key {}: {e}", key.display())))?;
                let message = std::fs::read(&file)
                    .map_err(|e| DevdError::not_found(format!("cannot read {}: {e}", file.display())))?;
                let signature = devd_core::update::sign_hex(&seed, &message)?;
                let out = std::path::PathBuf::from(format!("{}.sig", file.display()));
                devd_core::util::atomic_write(&out, &signature)?;
                println!("signed {} -> {}", file.display(), out.display());
                Ok(0)
            }
        },
        Command::Autostart { cmd } => {
            let id = match cmd {
                AutostartCommand::Status => "autostart.status",
                AutostartCommand::Enable => "autostart.enable",
                AutostartCommand::Disable => "autostart.disable",
            };
            app.invoke(id, serde_json::json!({}))?;
            Ok(0)
        }
        Command::PathSetup { cmd } => {
            let id = match cmd {
                PathCommand::Status => "shell.status",
                PathCommand::Enable => "shell.path_enable",
                PathCommand::Disable => "shell.path_disable",
            };
            app.invoke(id, serde_json::json!({}))?;
            Ok(0)
        }
        Command::Composer { cmd } => {
            match cmd {
                ComposerCommand::Install { force } => {
                    app.invoke("composer.install", serde_json::json!({ "force": force }))?
                }
            }
            Ok(0)
        }
        Command::Doctor => unreachable!("handled before bootstrap"),
        Command::RepairState => unreachable!("handled before bootstrap"),
        Command::Tui => {
            app.resume_agents();
            let git_executable = app.settings.git_executable.clone();
            let extra: Vec<devd_tui::ExtraChecks> =
                vec![Arc::new(move || vec![git_check(git_executable.as_deref())])];
            devd_tui::run(app.registry.clone(), app.ctx(), extra)?;
            Ok(0)
        }
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
            let queue_store = app.store.clone();
            let actor_label = format!("agent:{client}");
            let queue: devd_mcp::ApprovalQueue = Arc::new(move |cap, args, plan, run_id| {
                queue_store
                    .create_request(&actor_label, cap, args, plan, None, run_id, None)
                    .ok()
                    .map(|r| r.id)
            });
            let server = devd_mcp::McpServer::new(app.registry.clone(), app.ctx(), client)
                .with_extra_checks(extra)
                .with_approval_queue(queue);
            // Nothing but protocol traffic may reach stdout.
            server.serve_stdio()?;
            Ok(0)
        }
        Command::Setup {
            services,
            php,
            profile,
        } => setup::run(
            cli.json,
            cli.plan,
            cli.yes,
            setup::Choices {
                services,
                php,
                profile,
            },
        ),
        Command::Daemon { cmd } => match cmd {
            DaemonCommand::Run { port } => daemon::run(app, port),
            DaemonCommand::Start => daemon::start(&app),
            DaemonCommand::Stop => daemon::stop(&app),
            DaemonCommand::Status => daemon::status(&app),
        },
        Command::Ui { port, no_open } => {
            web::serve(app, port, !no_open, "ui")?;
            Ok(0)
        }
        Command::Serve { port } => {
            let started = web::serve(app, port, false, "ui")?;
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
fn repair_state(json: bool) -> Result<i32> {
    let paths = Paths::discover()?;
    let report = devd_core::repair::repair_state(&paths)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| DevdError::internal(e.to_string()))?
        );
    } else {
        println!("rebuilt {}", paths.state_db().display());
        for t in report.tables.iter().filter(|t| t.kept > 0 || t.lost > 0) {
            println!(
                "  {:<20} kept {:>6}   could not be read {:>5}",
                t.name, t.kept, t.lost
            );
        }
        println!("the damaged files are kept in {}", report.backup.display());
    }
    Ok(0)
}

fn doctor(json: bool) -> Result<i32> {
    let paths = Paths::discover()?;
    let settings = Settings::load(&paths.settings_file()).unwrap_or_default();

    let mut checks = devd_core::doctor::core_checks(&paths, &settings);
    checks.push(app::git_check(settings.git_executable.as_deref()));
    // The component checks open the state database, which would create a home that is not
    // there. With no home there is nothing for them to inspect.
    if devd_core::doctor::state_is_current(&paths) {
        checks.extend(devd_runtime::doctor_check(&paths));
        checks.extend(devd_services::doctor_check(&paths));
        checks.extend(devd_sites::doctor_check(&paths));
    }

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
