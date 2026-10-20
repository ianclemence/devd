//! The state the dashboard renders, and the parsing that fills it.
//!
//! Everything here is plain data. Nothing in this module touches a terminal, a
//! registry or a file, which is what makes the whole dashboard testable: a test
//! constructs a `State`, feeds it a keystroke, and asserts on the result.

use devd_core::capability::InvokeRequest;
use devd_core::capability::Plan;
use serde_json::Value;

/// A key, independent of any terminal library.
///
/// The controller speaks this, and the terminal layer translates crossterm's
/// key events into it. That is the seam that lets the controller be tested
/// without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// A control chord, e.g. `Ctrl+C`.
    Ctrl(char),
    /// Enter.
    Enter,
    /// Escape.
    Esc,
    /// Arrow up. Moves the cursor.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Tab. Moves to the next view.
    Tab,
    /// Shift-Tab.
    BackTab,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Backspace, for text prompts.
    Backspace,
}

/// A screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// Environment and repository summary.
    Dashboard,
    /// The repositories devd knows about.
    Projects,
    /// The working tree of the selected repository.
    Changes,
    /// Commit history.
    History,
    /// Branches.
    Branches,
    /// Create a project from a stack.
    Create,
    /// Tools, runtimes, databases and services.
    Toolchain,
    /// The capability registry.
    Capabilities,
    /// The audit trail.
    Activity,
    /// The environment check.
    Doctor,
    /// The agent team's runs.
    Team,
    /// What agents are waiting on you for.
    Inbox,
    /// Key reference.
    Help,
}

impl View {
    /// Every view, in tab order.
    pub const ALL: [View; 13] = [
        View::Dashboard,
        View::Projects,
        View::Create,
        View::Toolchain,
        View::Team,
        View::Inbox,
        View::Changes,
        View::History,
        View::Branches,
        View::Capabilities,
        View::Activity,
        View::Doctor,
        View::Help,
    ];

    /// Title shown in the tab strip and the panel header.
    pub fn title(self) -> &'static str {
        match self {
            View::Dashboard => "Dashboard",
            View::Projects => "Projects",
            View::Changes => "Changes",
            View::History => "History",
            View::Branches => "Branches",
            View::Create => "Create",
            View::Toolchain => "Toolchain",
            View::Capabilities => "Capabilities",
            View::Activity => "Activity",
            View::Doctor => "Doctor",
            View::Team => "Team",
            View::Inbox => "Inbox",
            View::Help => "Help",
        }
    }

    /// Single keystroke that jumps to this view, for the hint line.
    pub fn shortcut(self) -> char {
        match self {
            View::Dashboard => '1',
            View::Projects => '2',
            View::Changes => '3',
            View::History => '4',
            View::Branches => '5',
            View::Capabilities => '6',
            View::Activity => '7',
            View::Doctor => '8',
            View::Help => '9',
            View::Create => '0',
            View::Toolchain => '-',
            View::Team => '=',
            View::Inbox => '+',
        }
    }

    /// The view at a digit key, if any.
    pub fn from_digit(c: char) -> Option<View> {
        View::ALL.iter().copied().find(|v| v.shortcut() == c)
    }
}

/// Which part of the working tree a change belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    /// In the index; a commit would record it.
    Staged,
    /// Modified but not staged.
    Unstaged,
    /// Untracked.
    Untracked,
    /// Unmerged.
    Conflicted,
}

impl Bucket {
    /// Label used in the working-tree list.
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Staged => "staged",
            Bucket::Unstaged => "unstaged",
            Bucket::Untracked => "untracked",
            Bucket::Conflicted => "conflicted",
        }
    }
}

/// One row of the working-tree list, flattened across the buckets.
///
/// The list is deliberately one list rather than four panes: the common
/// keyboard interaction is "move down and act", and four panes means four
/// cursor positions and four sets of keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeItem {
    /// Which bucket the path is in.
    pub bucket: Bucket,
    /// Porcelain status character, or a space for an untracked file.
    pub code: char,
    /// Repository-relative path.
    pub path: String,
}

/// A repository devd knows about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoRow {
    /// Directory name.
    pub name: String,
    /// Absolute path.
    pub path: String,
    /// Default branch, when the repository has one.
    pub default_branch: Option<String>,
    /// Configured remote, when there is one.
    pub remote_url: Option<String>,
}

/// The parsed result of `git.status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusData {
    /// Current branch, or `None` in a detached or unborn state.
    pub branch: Option<String>,
    /// Upstream ref, when one is set.
    pub upstream: Option<String>,
    /// Whether the upstream has been deleted.
    pub upstream_gone: bool,
    /// Commits ahead of the upstream.
    pub ahead: u64,
    /// Commits behind the upstream.
    pub behind: u64,
    /// Nothing staged, unstaged, untracked or conflicted.
    pub clean: bool,
    /// No commits yet.
    pub unborn: bool,
    /// An interrupted merge, rebase, cherry-pick or revert.
    pub in_progress: Option<String>,
    /// Staged changes.
    pub staged: Vec<ChangeItem>,
    /// Unstaged changes.
    pub unstaged: Vec<ChangeItem>,
    /// Untracked files.
    pub untracked: Vec<ChangeItem>,
    /// Unmerged files.
    pub conflicted: Vec<ChangeItem>,
}

impl StatusData {
    /// The flattened working-tree list, in the order the panel shows it.
    pub fn items(&self) -> Vec<ChangeItem> {
        let mut out = Vec::new();
        out.extend(self.conflicted.iter().cloned());
        out.extend(self.staged.iter().cloned());
        out.extend(self.unstaged.iter().cloned());
        out.extend(self.untracked.iter().cloned());
        out
    }

    /// Parse the payload of a successful `git.status`.
    pub fn from_json(value: &Value) -> Self {
        let statuses = |key: &str| -> Vec<ChangeItem> {
            value
                .get(key)
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(|row| ChangeItem {
                            // `untracked` arrives as plain strings; the others as
                            // objects with a porcelain code.
                            bucket: match key {
                                "staged" => Bucket::Staged,
                                "unstaged" => Bucket::Unstaged,
                                "conflicted" => Bucket::Conflicted,
                                _ => Bucket::Untracked,
                            },
                            code: row
                                .get("status")
                                .and_then(Value::as_str)
                                .and_then(|s| s.chars().next())
                                .unwrap_or('?'),
                            path: row
                                .get("path")
                                .and_then(Value::as_str)
                                .or_else(|| row.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        })
                        .filter(|item| !item.path.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };

        Self {
            branch: string(value, "branch"),
            upstream: string(value, "upstream"),
            upstream_gone: boolean(value, "upstream_gone"),
            ahead: number(value, "ahead"),
            behind: number(value, "behind"),
            clean: boolean(value, "clean"),
            unborn: boolean(value, "unborn"),
            in_progress: string(value, "in_progress"),
            staged: statuses("staged"),
            unstaged: statuses("unstaged"),
            untracked: statuses("untracked"),
            conflicted: statuses("conflicted"),
        }
    }

    /// A one-line summary for the header.
    pub fn summary(&self) -> String {
        if self.unborn {
            return format!(
                "{}: no commits yet",
                self.branch.as_deref().unwrap_or("(unborn)")
            );
        }
        format!(
            "{}: {} staged, {} unstaged, {} untracked",
            self.branch.as_deref().unwrap_or("(detached)"),
            self.staged.len(),
            self.unstaged.len(),
            self.untracked.len()
        )
    }
}

/// One commit in the history list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitRow {
    /// Abbreviated hash.
    pub short: String,
    /// Full hash.
    pub hash: String,
    /// First line of the message.
    pub subject: String,
    /// Author name.
    pub author: String,
    /// Unix timestamp, as reported by Git.
    pub timestamp: i64,
}

impl CommitRow {
    /// Parse the payload of a successful `git.log`.
    pub fn list_from_json(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Self {
                        short: string(row, "short").unwrap_or_default(),
                        hash: string(row, "hash").unwrap_or_default(),
                        subject: string(row, "subject").unwrap_or_default(),
                        author: string(row, "author").unwrap_or_default(),
                        timestamp: row.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchRow {
    /// Branch name.
    pub name: String,
    /// Whether it is checked out.
    pub current: bool,
    /// Upstream ref, when set.
    pub upstream: Option<String>,
    /// Tip subject, when the service reports one.
    pub subject: Option<String>,
}

impl BranchRow {
    /// Parse the payload of a successful `git.branch.list`.
    pub fn list_from_json(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Self {
                        name: string(row, "name").unwrap_or_default(),
                        current: boolean(row, "current"),
                        upstream: string(row, "upstream"),
                        subject: string(row, "subject"),
                    })
                    .filter(|b| !b.name.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl RepoRow {
    /// Parse the payload of a successful `git.repo.list`.
    pub fn list_from_json(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| Self {
                        name: string(row, "name").unwrap_or_default(),
                        path: string(row, "path").unwrap_or_default(),
                        default_branch: string(row, "default_branch"),
                        remote_url: string(row, "remote_url"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One capability in the registry list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapRow {
    /// Capability id.
    pub id: String,
    /// Permission tier.
    pub tier: String,
    /// One-line summary.
    pub summary: String,
    /// Whether the tier demands an approval.
    pub requires_approval: bool,
    /// Whether an agent may call it at all.
    pub agent_safe: bool,
}

/// A stack that can be created.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackRow {
    /// Stack id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Language.
    pub language: String,
    /// Category.
    pub category: String,
    /// One line.
    pub summary: String,
    /// Can be created (not detect-only).
    pub creatable: bool,
}

impl StackRow {
    /// Parse the payload of `stacks.list`.
    pub fn list_from_json(value: &Value) -> Vec<Self> {
        value
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|r| Self {
                        id: string(r, "id").unwrap_or_default(),
                        name: string(r, "name").unwrap_or_default(),
                        language: string(r, "language").unwrap_or_default(),
                        category: string(r, "category").unwrap_or_default(),
                        summary: string(r, "summary").unwrap_or_default(),
                        creatable: boolean(r, "creatable"),
                    })
                    .filter(|r| r.creatable && !r.id.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A tool or service on the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRow {
    /// Tool id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Category.
    pub category: String,
    /// Installed and running.
    pub installed: bool,
    /// Version reported.
    pub version: Option<String>,
    /// Below the supported minimum.
    pub outdated: bool,
    /// Why it is unusable.
    pub problem: Option<String>,
    /// For services: `Some(true)` when healthy, `Some(false)` when not.
    pub service_healthy: Option<bool>,
    /// For services: the port.
    pub port: Option<u64>,
}

impl ToolRow {
    /// Parse the payload of `toolchain.list`.
    pub fn list_from_json(value: &Value) -> Vec<Self> {
        value
            .get("tools")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        let st = r.get("status").cloned().unwrap_or(Value::Null);
                        let tool = r.get("tool").cloned().unwrap_or(Value::Null);
                        let svc = r.get("service").filter(|s| !s.is_null());
                        Self {
                            id: string(&tool, "id").unwrap_or_default(),
                            name: string(&tool, "name").unwrap_or_default(),
                            category: string(&tool, "category").unwrap_or_default(),
                            installed: boolean(&st, "installed"),
                            version: string(&st, "version"),
                            outdated: boolean(&st, "outdated"),
                            problem: string(&st, "problem"),
                            service_healthy: svc.map(|s| boolean(s, "healthy")),
                            port: svc.and_then(|s| s.get("port").and_then(Value::as_u64)),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// One audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    /// When it happened.
    pub at: String,
    /// Who did it.
    pub actor: String,
    /// Which capability.
    pub capability: String,
    /// Its tier.
    pub tier: String,
    /// Whether it succeeded.
    pub ok: bool,
    /// Whether it was a plan rather than an application.
    pub dry_run: bool,
    /// The message, or the refusal reason.
    pub message: String,
}

/// One environment check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    /// Stable check name.
    pub name: String,
    /// `ok`, `info` or `fail`.
    pub level: String,
    /// What was observed.
    pub detail: String,
}

/// What a text prompt is collecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// A commit message.
    CommitMessage,
    /// A new branch name.
    NewBranch,
    /// A capability search.
    CapabilitySearch,
    /// JSON arguments for a capability.
    CapabilityArgs,
    /// A path to add as a project.
    TrackPath,
}

impl PromptKind {
    /// Prompt title.
    pub fn title(self) -> &'static str {
        match self {
            PromptKind::CommitMessage => "Commit message",
            PromptKind::NewBranch => "New branch name",
            PromptKind::CapabilitySearch => "Search capabilities",
            PromptKind::CapabilityArgs => "Arguments (JSON)",
            PromptKind::TrackPath => "Path to a repository",
        }
    }
}

/// An operation awaiting a human decision.
#[derive(Debug, Clone)]
pub struct Pending {
    /// Capability id.
    pub id: String,
    /// The plan as the registry produced it, when it could produce one.
    pub plan: Option<Plan>,
    /// The exact request to replay. Never rebuilt from the UI.
    pub request: InvokeRequest,
}

/// A modal layer over the current view.
#[derive(Debug, Clone)]
pub enum Overlay {
    /// No overlay.
    None,
    /// A single-line text prompt with the text typed so far.
    Prompt {
        /// What is being collected.
        kind: PromptKind,
        /// The text so far.
        input: String,
    },
    /// A read-only scrollable body of text.
    Detail {
        /// Title of the overlay.
        title: String,
        /// The text, already wrapped into lines.
        lines: Vec<String>,
        /// First visible line.
        scroll: u16,
    },
    /// A destructive or remote operation awaiting approval.
    Approval(Pending),
    /// One question of a guided flow, with the explanation above the input.
    Form {
        /// What is being asked.
        title: String,
        /// Explanation, choices and defaults.
        lines: Vec<String>,
        /// The answer so far.
        input: String,
    },
}

/// Everything the dashboard needs to draw one frame.
#[derive(Debug, Clone)]
pub struct State {
    /// Which screen is showing.
    pub view: View,
    /// Cursor row within the current list.
    pub cursor: usize,
    /// First visible row, kept in step with the cursor when moving.
    pub scroll: usize,
    /// The repository the Git views operate on.
    pub repo: Option<String>,
    /// Repositories devd knows about.
    pub projects: Vec<RepoRow>,
    /// Working tree of `repo`.
    pub status: Option<StatusData>,
    /// History of `repo`.
    pub commits: Vec<CommitRow>,
    /// Branches of `repo`.
    pub branches: Vec<BranchRow>,
    /// Capability list, already filtered by `capability_query`.
    pub capabilities: Vec<CapRow>,
    /// Current capability search text.
    pub capability_query: String,
    /// Audit rows.
    pub audit: Vec<AuditRow>,
    /// Environment checks.
    pub checks: Vec<CheckRow>,
    /// Stacks that can be created.
    pub stacks: Vec<StackRow>,
    /// Tools and services.
    pub tools: Vec<ToolRow>,
    /// Agent team runs (as `team.list` returns them).
    pub runs: Vec<Value>,
    /// Waiting items: approvals and questions, as `{kind, id, title, detail}`.
    pub inbox: Vec<Value>,
    /// Status line text.
    pub message: String,
    /// Whether the status line is reporting a failure.
    pub message_is_error: bool,
    /// The modal layer.
    pub overlay: Overlay,
    /// Capability the arguments prompt is collecting for.
    pub prompt_target: Option<String>,
    /// Version string for the header.
    pub version: String,
    /// Home directory for the header.
    pub home: String,
    /// Git version for the header.
    pub git_version: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            view: View::Dashboard,
            cursor: 0,
            scroll: 0,
            repo: None,
            projects: Vec::new(),
            status: None,
            commits: Vec::new(),
            branches: Vec::new(),
            capabilities: Vec::new(),
            capability_query: String::new(),
            audit: Vec::new(),
            checks: Vec::new(),
            stacks: Vec::new(),
            tools: Vec::new(),
            runs: Vec::new(),
            inbox: Vec::new(),
            message: String::new(),
            message_is_error: false,
            overlay: Overlay::None,
            prompt_target: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            home: String::new(),
            git_version: String::new(),
        }
    }
}

impl State {
    /// Number of rows in the current view's list.
    pub fn row_count(&self) -> usize {
        match self.view {
            View::Dashboard | View::Help => 0,
            View::Projects => self.projects.len(),
            View::Changes => self
                .status
                .as_ref()
                .map(|s| s.items().len())
                .unwrap_or_default(),
            View::History => self.commits.len(),
            View::Branches => self.branches.len(),
            View::Capabilities => self.capabilities.len(),
            View::Activity => self.audit.len(),
            View::Doctor => self.checks.len(),
            View::Create => self.stacks.len(),
            View::Toolchain => self.tools.len(),
            View::Team => self.runs.len(),
            View::Inbox => self.inbox.len(),
        }
    }

    /// The stack under the cursor, if any.
    pub fn selected_stack(&self) -> Option<&StackRow> {
        self.stacks.get(self.cursor)
    }

    /// The tool under the cursor, if any.
    pub fn selected_tool(&self) -> Option<&ToolRow> {
        self.tools.get(self.cursor)
    }

    /// The working-tree item under the cursor, if the cursor is on one.
    pub fn selected_change(&self) -> Option<ChangeItem> {
        let items = self.status.as_ref()?.items();
        items.get(self.cursor).cloned()
    }

    /// The capability under the cursor, if any.
    pub fn selected_capability(&self) -> Option<&CapRow> {
        self.capabilities.get(self.cursor)
    }

    /// The project under the cursor, if any.
    pub fn selected_project(&self) -> Option<&RepoRow> {
        self.projects.get(self.cursor)
    }

    /// The commit under the cursor, if any.
    pub fn selected_commit(&self) -> Option<&CommitRow> {
        self.commits.get(self.cursor)
    }

    /// The branch under the cursor, if any.
    pub fn selected_branch(&self) -> Option<&BranchRow> {
        self.branches.get(self.cursor)
    }

    /// Keep the cursor inside the list and the scroll window on the cursor.
    pub fn clamp(&mut self, height: usize) {
        let count = self.row_count();
        if count == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        if self.cursor >= count {
            self.cursor = count - 1;
        }
        let height = height.max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
    }

    /// Move the cursor by `delta` rows.
    pub fn move_cursor(&mut self, delta: isize) {
        let count = self.row_count() as isize;
        if count == 0 {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, count - 1);
        self.cursor = next as usize;
        let height = 20; // re-clamped by the caller with the real height
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
        if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }
    }

    /// Replace the status line.
    pub fn say(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.message_is_error = false;
    }

    /// Replace the status line with a failure.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.message_is_error = true;
    }

    /// Whether a modal is open, which changes what a key means.
    pub fn has_overlay(&self) -> bool {
        !matches!(self.overlay, Overlay::None)
    }

    /// Number of feeds that failed their last refresh, for the dashboard.
    pub fn failing_checks(&self) -> usize {
        self.checks.iter().filter(|c| c.level == "fail").count()
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn boolean(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_parses_objects_and_plain_strings() {
        let payload = json!({
            "branch": "main",
            "upstream": "origin/main",
            "ahead": 2,
            "behind": 1,
            "clean": false,
            "unborn": false,
            "in_progress": null,
            "staged": [{ "status": "M", "path": "src/main.rs" }],
            "unstaged": [{ "status": "D", "path": "gone.txt" }],
            "untracked": ["new.txt", "notes.md"],
            "conflicted": []
        });
        let status = StatusData::from_json(&payload);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.ahead, 2);
        assert_eq!(status.behind, 1);
        assert!(!status.clean);
        assert_eq!(status.staged.len(), 1);
        assert_eq!(status.staged[0].code, 'M');
        assert_eq!(status.staged[0].bucket, Bucket::Staged);
        assert_eq!(status.untracked.len(), 2);
        assert_eq!(status.untracked[0].path, "new.txt");
        // Untracked entries have no status code, and must not invent one.
        assert_eq!(status.untracked[0].code, '?');
        assert_eq!(status.items().len(), 4);
    }

    #[test]
    fn a_missing_field_never_panics() {
        let status = StatusData::from_json(&json!({}));
        assert!(status.branch.is_none());
        assert!(!status.clean);
        assert!(status.items().is_empty());
        assert_eq!(
            status.summary(),
            "(detached): 0 staged, 0 unstaged, 0 untracked"
        );
    }

    #[test]
    fn an_unborn_repository_says_so() {
        let status = StatusData::from_json(&json!({ "branch": "main", "unborn": true }));
        assert_eq!(status.summary(), "main: no commits yet");
    }

    #[test]
    fn log_and_branches_parse() {
        let commits = CommitRow::list_from_json(&json!([
            { "short": "abc1234", "hash": "abc1234def", "subject": "Add thing", "author": "Ian", "timestamp": 1 }
        ]));
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "Add thing");

        let branches = BranchRow::list_from_json(&json!([
            { "name": "main", "current": true, "upstream": "origin/main", "subject": "Add thing" },
            { "name": "feature", "current": false, "upstream": null }
        ]));
        assert_eq!(branches.len(), 2);
        assert!(branches[0].current);
        assert!(branches[1].upstream.is_none());
    }

    #[test]
    fn cursor_movement_stops_at_the_ends() {
        let mut state = State::default();
        state.projects = (0..3)
            .map(|i| RepoRow {
                name: format!("repo{i}"),
                ..Default::default()
            })
            .collect();
        state.view = View::Projects;

        state.move_cursor(1);
        assert_eq!(state.cursor, 1);
        state.move_cursor(-5);
        assert_eq!(state.cursor, 0);
        state.move_cursor(50);
        assert_eq!(state.cursor, 2);
    }

    #[test]
    fn an_empty_list_keeps_the_cursor_at_zero() {
        let mut state = State::default();
        state.view = View::Branches;
        state.move_cursor(3);
        assert_eq!(state.cursor, 0);
        state.clamp(10);
        assert_eq!(state.cursor, 0);
        assert!(state.selected_branch().is_none());
    }

    #[test]
    fn the_scroll_window_follows_the_cursor() {
        let mut state = State::default();
        state.view = View::History;
        state.commits = (0..100)
            .map(|i| CommitRow {
                short: format!("{i:07}"),
                ..Default::default()
            })
            .collect();
        state.clamp(10);
        for _ in 0..25 {
            state.move_cursor(1);
            state.clamp(10);
        }
        assert_eq!(state.cursor, 25);
        assert!(state.scroll <= state.cursor);
        assert!(
            state.cursor < state.scroll + 10,
            "cursor fell outside the window"
        );
    }

    #[test]
    fn view_digits_round_trip() {
        for view in View::ALL {
            assert_eq!(View::from_digit(view.shortcut()), Some(view));
        }
        assert_eq!(View::from_digit('x'), None);
    }
}
