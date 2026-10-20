//! Keystrokes in, capability calls out.
//!
//! The controller is the whole dashboard minus the terminal. It is generic over
//! a [`Backend`], so a test drives real keystrokes against canned capability
//! results and asserts on the resulting state — including the property that
//! matters most, that a destructive operation does not run until it is approved
//! and is *recorded as refused* when it is not.
//!
//! # Approvals are two-step here as well
//!
//! When an action needs approval, the controller does not simply fail. It asks
//! the backend for the plan, shows it, and keeps **the request it already
//! built**. Pressing `y` replays that stored request; it is never rebuilt from
//! the screen, so what the person read and what runs cannot differ. Pressing `n`
//! dispatches the same request with [`Approval::Denied`], so the refusal lands in
//! the audit trail instead of disappearing — which is exactly what the CLI and
//! the application window do.

use std::sync::Arc;

use serde_json::{json, Value};

use devd_core::capability::{Approval, Ctx, InvokeRequest, InvokeResult, Plan};
use devd_core::doctor::{self, Check, Level};
use devd_core::{Registry, Result};

use crate::state::{
    AuditRow, BranchRow, CapRow, CheckRow, CommitRow, Key, Overlay, Pending, PromptKind, RepoRow,
    State, StatusData, View,
};

/// Everything the controller needs from the outside world.
///
/// The real implementation dispatches through the shared registry. A test
/// implements this with canned values, which is what lets the dashboard's
/// behaviour be asserted without a terminal, a repository or a subprocess.
pub trait Backend {
    /// Run a capability. `approval` is the *evidence* attached to the call; the
    /// backend must not invent one.
    fn invoke(&mut self, request: &InvokeRequest, approval: Approval) -> Result<InvokeResult>;
    /// Produce a plan without applying anything.
    fn plan(&mut self, request: &InvokeRequest) -> Result<Option<Plan>>;
    /// The capability registry, filtered by a search string.
    fn capabilities(&mut self, query: &str) -> Result<Vec<CapRow>>;
    /// The full description of one capability.
    fn describe(&mut self, id: &str) -> Result<Value>;
    /// The audit trail, newest first.
    fn audit(&mut self, limit: u32) -> Result<Vec<AuditRow>>;
    /// The environment checks.
    fn doctor(&mut self) -> Result<Vec<CheckRow>>;
    /// Home directory, for the header.
    fn home(&self) -> String;
    /// Git version, for the header.
    fn git_version(&self) -> String;
}

/// The registry as a [`Backend`].
pub struct RegistryBackend {
    registry: Arc<Registry>,
    ctx: Ctx,
    extra_checks: Vec<ExtraChecks>,
}

/// A function producing checks only some callers can perform.
pub type ExtraChecks = Arc<dyn Fn() -> Vec<Check> + Send + Sync>;

impl RegistryBackend {
    /// Wrap a registry and its invocation context.
    pub fn new(registry: Arc<Registry>, ctx: Ctx, extra_checks: Vec<ExtraChecks>) -> Self {
        Self {
            registry,
            ctx,
            extra_checks,
        }
    }
}

impl Backend for RegistryBackend {
    fn invoke(&mut self, request: &InvokeRequest, approval: Approval) -> Result<InvokeResult> {
        self.registry.dispatch(&self.ctx, request, approval)
    }

    fn plan(&mut self, request: &InvokeRequest) -> Result<Option<Plan>> {
        let dry = InvokeRequest {
            id: request.id.clone(),
            args: request.args.clone(),
            dry_run: true,
        };
        Ok(self
            .registry
            .dispatch(&self.ctx, &dry, Approval::None)?
            .plan)
    }

    fn capabilities(&mut self, query: &str) -> Result<Vec<CapRow>> {
        Ok(self
            .registry
            .search(query, None, usize::MAX, 0)
            .iter()
            .map(|cap| CapRow {
                id: cap.id.clone(),
                tier: cap.tier.as_str().to_string(),
                summary: cap.summary.clone(),
                requires_approval: cap.tier.requires_approval(),
                agent_safe: cap.agent_safe,
            })
            .collect())
    }

    fn describe(&mut self, id: &str) -> Result<Value> {
        self.registry.describe_json(id)
    }

    fn audit(&mut self, limit: u32) -> Result<Vec<AuditRow>> {
        Ok(self
            .ctx
            .store
            .list_audit(limit)?
            .into_iter()
            .map(|e| AuditRow {
                at: e.at,
                actor: e.actor,
                capability: e.capability,
                tier: e.tier,
                ok: e.ok,
                dry_run: e.dry_run,
                message: e.message,
            })
            .collect())
    }

    fn doctor(&mut self) -> Result<Vec<CheckRow>> {
        let mut checks = doctor::core_checks(&self.ctx.paths, &self.ctx.settings);
        for extra in &self.extra_checks {
            checks.extend(extra());
        }
        Ok(checks
            .into_iter()
            .map(|c| CheckRow {
                name: c.name,
                level: match c.level {
                    Level::Ok => "ok".to_string(),
                    Level::Info => "info".to_string(),
                    Level::Fail => "fail".to_string(),
                },
                detail: c.detail,
            })
            .collect())
    }

    fn home(&self) -> String {
        self.ctx.paths.root().display().to_string()
    }

    fn git_version(&self) -> String {
        devd_git::GitService::discover(self.ctx.settings.git_executable.as_deref())
            .map(|git| git.version().to_string())
            .unwrap_or_else(|_| "not found".to_string())
    }
}

/// What a capability call produced, from the controller's point of view.
enum Call {
    /// Structured data.
    Data(Value),
    /// Success with no data.
    Nothing,
    /// The operation was replaced by an approval prompt.
    NeedsApproval,
    /// The call failed.
    Failed(String),
}

/// The dashboard.
pub struct Controller {
    /// What to draw.
    pub state: State,
    backend: Box<dyn Backend>,
}

impl Controller {
    /// Build a controller over a backend.
    pub fn new(backend: Box<dyn Backend>) -> Self {
        let mut state = State::default();
        state.home = backend.home();
        state.git_version = backend.git_version();
        Self { state, backend }
    }

    /// Reload whatever the current view shows.
    pub fn refresh(&mut self) {
        let view = self.state.view;
        self.refresh_view(view);
    }

    /// Handle one keystroke. Returns `false` when the user asked to quit.
    pub fn handle_key(&mut self, key: Key) -> bool {
        if key == Key::Ctrl('c') {
            return false;
        }

        // A modal takes every key, so a keystroke cannot both answer a prompt
        // and act on the view behind it.
        match self.state.overlay.clone() {
            Overlay::Prompt { kind, mut input } => {
                match key {
                    Key::Esc => {
                        self.state.overlay = Overlay::None;
                        self.state.say("cancelled");
                    }
                    Key::Enter => {
                        self.state.overlay = Overlay::None;
                        self.submit_prompt(kind, input);
                    }
                    Key::Backspace => {
                        input.pop();
                        self.state.overlay = Overlay::Prompt { kind, input };
                    }
                    Key::Char(c) => {
                        input.push(c);
                        self.state.overlay = Overlay::Prompt { kind, input };
                    }
                    _ => {}
                }
                return true;
            }
            Overlay::Detail {
                title,
                lines,
                scroll,
            } => {
                let scroll = match key {
                    Key::Esc | Key::Enter | Key::Char('q') => {
                        self.state.overlay = Overlay::None;
                        return true;
                    }
                    Key::Down | Key::Char('j') => scroll.saturating_add(1),
                    Key::Up | Key::Char('k') => scroll.saturating_sub(1),
                    Key::PageDown => scroll.saturating_add(10),
                    Key::PageUp => scroll.saturating_sub(10),
                    Key::Home => 0,
                    Key::End => u16::MAX,
                    _ => scroll,
                };
                self.state.overlay = Overlay::Detail {
                    title,
                    lines,
                    scroll,
                };
                return true;
            }
            Overlay::Approval(_) => {
                match key {
                    Key::Char('y') | Key::Char('Y') => self.approve(),
                    Key::Char('n') | Key::Char('N') | Key::Esc => self.decline(),
                    _ => {}
                }
                return true;
            }
            Overlay::None => {}
        }

        // A prompt is the one place a bare letter is text rather than a command.
        match key {
            Key::Char('q') => return false,
            Key::Char('?') => self.switch_view(View::Help),
            Key::Char('r') => {
                self.refresh();
                self.state.say("refreshed");
            }
            Key::Char(c) if View::from_digit(c).is_some() => {
                let view = View::from_digit(c).expect("checked");
                self.switch_view(view);
            }
            Key::Tab => self.step_view(1),
            Key::BackTab => self.step_view(-1),
            Key::Up | Key::Char('k') => self.state.move_cursor(-1),
            Key::Down | Key::Char('j') => self.state.move_cursor(1),
            Key::PageUp => self.state.move_cursor(-10),
            Key::PageDown => self.state.move_cursor(10),
            Key::Home => self.state.cursor = 0,
            Key::End => {
                let last = self.state.row_count().saturating_sub(1);
                self.state.cursor = last;
            }
            Key::Enter => self.activate(),
            Key::Char('/') if self.state.view == View::Capabilities => {
                self.open_prompt(PromptKind::CapabilitySearch, String::new());
            }
            Key::Char('x') if self.state.view == View::Capabilities => self.invoke_selected(),
            Key::Char('s') if self.state.view == View::Changes => self.stage_selected(),
            Key::Char('u') if self.state.view == View::Changes => self.unstage_selected(),
            Key::Char('d') if self.state.view == View::Changes => self.discard_selected(),
            Key::Char('a') if self.state.view == View::Changes => self.stage_all(),
            Key::Char('c') if self.state.view == View::Changes => {
                self.open_prompt(PromptKind::CommitMessage, String::new());
            }
            Key::Char('n') if self.state.view == View::Branches => {
                self.open_prompt(PromptKind::NewBranch, String::new());
            }
            Key::Char('t') if self.state.view == View::Projects => {
                self.open_prompt(PromptKind::TrackPath, String::new());
            }
            _ => {}
        }
        true
    }

    /// Change view by name, reloading on the way in.
    pub fn switch_view(&mut self, view: View) {
        self.state.view = view;
        self.state.cursor = 0;
        self.state.scroll = 0;
        self.refresh_view(view);
    }

    fn step_view(&mut self, delta: isize) {
        let views = View::ALL;
        let current = views
            .iter()
            .position(|v| *v == self.state.view)
            .unwrap_or_default() as isize;
        let next = (current + delta).rem_euclid(views.len() as isize) as usize;
        self.switch_view(views[next]);
    }

    /// Reload the data a view needs.
    pub fn refresh_view(&mut self, view: View) {
        match view {
            View::Dashboard => {
                self.load_checks();
                self.load_projects();
                self.load_status();
            }
            View::Projects => self.load_projects(),
            View::Changes => self.load_status(),
            View::History => self.load_log(),
            View::Branches => self.load_branches(),
            View::Capabilities => self.load_capabilities(),
            View::Activity => self.load_audit(),
            View::Doctor => self.load_checks(),
            View::Help => {}
        }
        let height = 20;
        self.state.clamp(height);
    }

    fn open_prompt(&mut self, kind: PromptKind, input: String) {
        self.state.overlay = Overlay::Prompt { kind, input };
    }

    /// The repository the Git views act on, filling it in from the project list
    /// the first time it is needed.
    fn repo(&mut self) -> Option<String> {
        if let Some(repo) = &self.state.repo {
            return Some(repo.clone());
        }
        if self.state.projects.is_empty() {
            self.load_projects();
        }
        let first = self.state.projects.first()?.path.clone();
        self.state.repo = Some(first.clone());
        Some(first)
    }

    /// Ask for a repository, reporting honestly when there is none.
    fn require_repo(&mut self) -> Option<String> {
        match self.repo() {
            Some(repo) => Some(repo),
            None => {
                self.state
                    .fail("no repository selected - press 2 to choose one, or t to add a path");
                None
            }
        }
    }

    /// Run a capability, turning an approval requirement into a prompt.
    fn call(&mut self, id: &str, args: Value) -> Call {
        let request = InvokeRequest {
            id: id.to_string(),
            args,
            dry_run: false,
        };
        match self.backend.invoke(&request, Approval::None) {
            Ok(result) => match result.outcome {
                Some(outcome) => Call::Data(outcome.data),
                None => Call::Nothing,
            },
            Err(e) if e.code() == "approval_required" => {
                // Ask for the plan, and keep the request the person is about to
                // approve. Rebuilding it later would be a way for what runs to
                // differ from what was read.
                let plan = self.backend.plan(&request).ok().flatten();
                self.state.overlay = Overlay::Approval(Pending {
                    id: id.to_string(),
                    plan,
                    request,
                });
                Call::NeedsApproval
            }
            Err(e) => Call::Failed(e.to_string()),
        }
    }

    /// Replay the pending request with the approval the person just gave.
    fn approve(&mut self) {
        let Overlay::Approval(pending) = self.state.overlay.clone() else {
            return;
        };
        self.state.overlay = Overlay::None;
        match self.backend.invoke(&pending.request, Approval::Granted) {
            Ok(result) => {
                let message = result
                    .outcome
                    .map(|o| o.message)
                    .unwrap_or_else(|| format!("{} ran", pending.id));
                self.state.say(message);
            }
            Err(e) => self.state.fail(e.to_string()),
        }
        self.refresh();
    }

    /// Replay the pending request as *refused*, so the refusal is recorded.
    fn decline(&mut self) {
        let Overlay::Approval(pending) = self.state.overlay.clone() else {
            return;
        };
        self.state.overlay = Overlay::None;
        match self.backend.invoke(&pending.request, Approval::Denied) {
            // A refusal is expected to come back as an error; it is still the
            // correct outcome, and it is now in the audit trail.
            Err(e) if e.code() == "permission_denied" => {
                self.state.say(format!("declined {}: {e}", pending.id))
            }
            Ok(result) => {
                let message = result
                    .outcome
                    .map(|o| o.message)
                    .unwrap_or_else(|| format!("declined {}", pending.id));
                self.state.say(message);
            }
            Err(e) => self.state.fail(format!("declined {}: {e}", pending.id)),
        }
        self.refresh();
    }

    // ---- loads -----------------------------------------------------------

    fn load_projects(&mut self) {
        match self.call("git.repo.list", json!({})) {
            Call::Data(data) => self.state.projects = RepoRow::list_from_json(&data),
            Call::Failed(message) => {
                self.state.projects.clear();
                self.state.fail(message);
            }
            _ => {}
        }
    }

    fn load_status(&mut self) {
        let Some(repo) = self.require_repo() else {
            self.state.status = None;
            return;
        };
        match self.call("git.status", json!({ "repo": repo })) {
            Call::Data(data) => self.state.status = Some(StatusData::from_json(&data)),
            Call::Failed(message) => {
                self.state.status = None;
                self.state.fail(message);
            }
            _ => {}
        }
    }

    fn load_log(&mut self) {
        let Some(repo) = self.require_repo() else {
            self.state.commits.clear();
            return;
        };
        match self.call("git.log", json!({ "repo": repo, "limit": 200 })) {
            Call::Data(data) => self.state.commits = CommitRow::list_from_json(&data),
            Call::Failed(message) => {
                self.state.commits.clear();
                self.state.fail(message);
            }
            _ => {}
        }
    }

    fn load_branches(&mut self) {
        let Some(repo) = self.require_repo() else {
            self.state.branches.clear();
            return;
        };
        match self.call("git.branch.list", json!({ "repo": repo, "all": false })) {
            Call::Data(data) => self.state.branches = BranchRow::list_from_json(&data),
            Call::Failed(message) => {
                self.state.branches.clear();
                self.state.fail(message);
            }
            _ => {}
        }
    }

    fn load_capabilities(&mut self) {
        let query = self.state.capability_query.clone();
        match self.backend.capabilities(&query) {
            Ok(caps) => {
                let count = caps.len();
                self.state.capabilities = caps;
                self.state
                    .say(format!("{count} capabilities matching `{query}`"));
            }
            Err(e) => self.state.fail(e.to_string()),
        }
    }

    fn load_audit(&mut self) {
        match self.backend.audit(200) {
            Ok(rows) => self.state.audit = rows,
            Err(e) => {
                self.state.audit.clear();
                self.state.fail(e.to_string());
            }
        }
    }

    fn load_checks(&mut self) {
        match self.backend.doctor() {
            Ok(checks) => {
                let failing = checks.iter().filter(|c| c.level == "fail").count();
                self.state.checks = checks;
                if failing > 0 {
                    self.state
                        .fail(format!("{failing} environment checks failed"));
                }
            }
            Err(e) => {
                self.state.checks.clear();
                self.state.fail(e.to_string());
            }
        }
    }

    // ---- actions ---------------------------------------------------------

    fn activate(&mut self) {
        match self.state.view {
            View::Projects => {
                let Some(project) = self.state.selected_project().cloned() else {
                    return;
                };
                self.state.repo = Some(project.path.clone());
                self.state.say(format!("working in {}", project.name));
                self.switch_view(View::Changes);
            }
            View::Changes => {
                // Enter on a row toggles it into or out of the index, which is
                // the single most common Git action.
                match self.state.selected_change() {
                    Some(item) if item.bucket == crate::state::Bucket::Staged => {
                        self.unstage_selected()
                    }
                    Some(_) => self.stage_selected(),
                    None => {}
                }
            }
            View::History => self.show_commit(),
            View::Branches => self.switch_branch(),
            View::Capabilities => self.describe_selected(),
            _ => {}
        }
    }

    fn stage_selected(&mut self) {
        let Some(item) = self.state.selected_change() else {
            return;
        };
        if item.bucket == crate::state::Bucket::Staged {
            return;
        }
        let Some(repo) = self.repo() else { return };
        match self.call("git.stage", json!({ "repo": repo, "paths": [item.path] })) {
            Call::Data(_) | Call::Nothing => self.state.say(format!("staged {}", item.path)),
            Call::Failed(message) => self.state.fail(message),
            Call::NeedsApproval => return,
        }
        self.load_status();
    }

    fn unstage_selected(&mut self) {
        let Some(item) = self.state.selected_change() else {
            return;
        };
        if item.bucket != crate::state::Bucket::Staged {
            return;
        }
        let Some(repo) = self.repo() else { return };
        match self.call("git.unstage", json!({ "repo": repo, "paths": [item.path] })) {
            Call::Data(_) | Call::Nothing => self.state.say(format!("unstaged {}", item.path)),
            Call::Failed(message) => self.state.fail(message),
            Call::NeedsApproval => return,
        }
        self.load_status();
    }

    fn stage_all(&mut self) {
        let Some(repo) = self.repo() else { return };
        match self.call("git.stage", json!({ "repo": repo, "paths": ["."] })) {
            Call::Data(_) | Call::Nothing => self.state.say("staged everything"),
            Call::Failed(message) => self.state.fail(message),
            Call::NeedsApproval => return,
        }
        self.load_status();
    }

    fn discard_selected(&mut self) {
        let Some(item) = self.state.selected_change() else {
            return;
        };
        let Some(repo) = self.repo() else { return };
        // Discarding is destructive, so this goes through the same approval path
        // as any other destructive operation rather than having its own key.
        match self.call("git.discard", json!({ "repo": repo, "paths": [item.path] })) {
            Call::Data(_) | Call::Nothing => {
                self.state.say(format!("discarded {}", item.path));
                self.load_status();
            }
            Call::NeedsApproval => {}
            Call::Failed(message) => self.state.fail(message),
        }
    }

    fn show_commit(&mut self) {
        let Some(commit) = self.state.selected_commit().cloned() else {
            return;
        };
        let Some(repo) = self.repo() else { return };
        match self.call("git.show", json!({ "repo": repo, "rev": commit.hash })) {
            Call::Data(data) => {
                let lines = value_lines(&data);
                self.state.overlay = Overlay::Detail {
                    title: format!("{} {}", commit.short, commit.subject),
                    lines,
                    scroll: 0,
                };
            }
            Call::Failed(message) => self.state.fail(message),
            _ => {}
        }
    }

    fn switch_branch(&mut self) {
        let Some(branch) = self.state.selected_branch().cloned() else {
            return;
        };
        if branch.current {
            self.state.say(format!("already on {}", branch.name));
            return;
        }
        let Some(repo) = self.repo() else { return };

        // Ask the registry what a switch would endanger before offering it. The
        // overwrite check is the product's own, not a second opinion invented
        // here.
        let at_risk = match self.call(
            "git.switch.check",
            json!({ "repo": repo, "target": branch.name }),
        ) {
            Call::Data(data) => data
                .get("at_risk")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Call::Failed(message) => {
                self.state.fail(message);
                return;
            }
            _ => Vec::new(),
        };

        let request = InvokeRequest {
            id: "git.switch".to_string(),
            args: json!({ "repo": repo, "name": branch.name, "create": false }),
            dry_run: false,
        };

        if at_risk.is_empty() {
            match self.call("git.switch", request.args.clone()) {
                Call::Data(_) | Call::Nothing => {
                    self.state.say(format!("switched to {}", branch.name));
                    self.load_status();
                    self.load_branches();
                }
                Call::NeedsApproval => {}
                Call::Failed(message) => self.state.fail(message),
            }
            return;
        }

        // A switch that would overwrite local work is a confirm, not a
        // refusal: the plan carries the registry's own list of what is at risk.
        let mut plan = Plan::new("git.switch", format!("switch to {}", branch.name));
        for path in &at_risk {
            plan = plan.step(format!(
                "{} would be overwritten",
                path.as_str().unwrap_or("(unnamed)")
            ));
        }
        plan = plan
            .destructive(true)
            .effect("git refuses a switch that would overwrite these files");
        self.state.overlay = Overlay::Approval(Pending {
            id: "git.switch".to_string(),
            plan: Some(plan),
            request,
        });
    }

    fn describe_selected(&mut self) {
        let Some(cap) = self.state.selected_capability().cloned() else {
            return;
        };
        match self.backend.describe(&cap.id) {
            Ok(value) => {
                let mut lines = value_lines(&value);
                lines.push(String::new());
                lines.push(cap_log_hint(&cap));
                self.state.overlay = Overlay::Detail {
                    title: cap.id.clone(),
                    lines,
                    scroll: 0,
                };
            }
            Err(e) => self.state.fail(e.to_string()),
        }
    }

    fn invoke_selected(&mut self) {
        let Some(cap) = self.state.selected_capability().cloned() else {
            return;
        };
        self.state.prompt_target = Some(cap.id.clone());
        self.open_prompt(PromptKind::CapabilityArgs, String::new());
    }

    fn submit_prompt(&mut self, kind: PromptKind, text: String) {
        match kind {
            PromptKind::CommitMessage => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    self.state.fail("a commit message is required");
                    return;
                }
                let Some(repo) = self.repo() else { return };
                let args = json!({
                    "repo": repo,
                    "message": text,
                    "amend": false,
                    "allow_empty": false,
                });
                match self.call("git.commit", args) {
                    Call::Data(_) | Call::Nothing => self.state.say("committed"),
                    Call::NeedsApproval => return,
                    Call::Failed(message) => self.state.fail(message),
                }
                self.load_status();
                self.load_log();
            }
            PromptKind::NewBranch => {
                let name = text.trim().to_string();
                if name.is_empty() {
                    self.state.fail("a branch name is required");
                    return;
                }
                let Some(repo) = self.repo() else { return };
                let args = json!({ "repo": repo, "name": name, "checkout": true });
                match self.call("git.branch.create", args) {
                    Call::Data(_) | Call::Nothing => {
                        self.state.say(format!("created and switched to {name}"))
                    }
                    Call::NeedsApproval => return,
                    Call::Failed(message) => self.state.fail(message),
                }
                self.load_branches();
                self.load_status();
            }
            PromptKind::CapabilitySearch => {
                self.state.capability_query = text.trim().to_string();
                self.state.cursor = 0;
                self.load_capabilities();
            }
            PromptKind::CapabilityArgs => {
                let Some(id) = self.state.prompt_target.clone() else {
                    return;
                };
                let trimmed = text.trim();
                let args: Value = if trimmed.is_empty() {
                    json!({})
                } else {
                    match serde_json::from_str(trimmed) {
                        Ok(value) => value,
                        Err(e) => {
                            self.state
                                .fail(format!("arguments are not valid JSON: {e}"));
                            return;
                        }
                    }
                };
                match self.call(&id, args) {
                    Call::Data(data) => {
                        let mut lines = value_lines(&data);
                        if lines.is_empty() {
                            lines.push("(no data)".to_string());
                        }
                        self.state.overlay = Overlay::Detail {
                            title: format!("{id} ran"),
                            lines,
                            scroll: 0,
                        };
                    }
                    Call::Nothing => self.state.say(format!("{id} ran")),
                    // The approval prompt has already replaced any other overlay.
                    Call::NeedsApproval => {}
                    Call::Failed(message) => self.state.fail(format!("{id}: {message}")),
                }
            }
            PromptKind::TrackPath => {
                let path = text.trim().to_string();
                if path.is_empty() {
                    self.state.fail("a path is required");
                    return;
                }
                match self.call("git.repo.track", json!({ "repo": path })) {
                    Call::Data(_) | Call::Nothing => {
                        self.state.repo = Some(path.clone());
                        self.state.say(format!("tracking {path}"));
                    }
                    Call::NeedsApproval => return,
                    Call::Failed(message) => {
                        self.state.fail(message);
                        return;
                    }
                }
                self.load_projects();
                self.load_status();
            }
        }
    }
}

/// Flatten a JSON value into readable lines.
///
/// A capability result is whatever the capability decided to return, and the
/// dashboard has to be able to show one it has never seen — including a
/// registry declaration, whose shape is not fixed either. Rendering the JSON as
/// `key: value` per leaf is honest and cannot silently drop a field.
pub fn value_lines(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    flatten(value, 0, &mut out);
    out
}

fn flatten(value: &Value, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push(format!("{indent}{{}}"));
                return;
            }
            for (key, child) in map {
                match child {
                    Value::Object(_) | Value::Array(_) => {
                        out.push(format!("{indent}{key}:"));
                        flatten(child, depth + 1, out);
                    }
                    other => out.push(format!("{indent}{key}: {}", scalar(other))),
                }
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push(format!("{indent}[]"));
                return;
            }
            for (index, item) in items.iter().enumerate() {
                match item {
                    Value::Object(_) | Value::Array(_) => {
                        out.push(format!("{indent}[{index}]"));
                        flatten(item, depth + 1, out);
                    }
                    other => out.push(format!("{indent}[{index}] {}", scalar(other))),
                }
            }
        }
        other => out.push(format!("{indent}{}", scalar(other))),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "(none)".to_string(),
        other => other.to_string(),
    }
}

fn cap_log_hint(cap: &CapRow) -> String {
    if cap.requires_approval {
        format!(
            "tier {} - needs approval: press x, then confirm the plan",
            cap.tier
        )
    } else {
        format!("tier {} - press x to run", cap.tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use devd_core::capability::{Outcome, Plan as CorePlan};
    use devd_core::DevdError;

    /// A backend that answers from a table, and records what it was asked for.
    ///
    /// `approval_required` lists the capabilities that refuse until an approval
    /// is presented, which is how the two-step flow is exercised.
    struct Fake {
        answers: HashMap<String, Value>,
        approval_required: Vec<String>,
        human_only: Vec<String>,
        /// Every call, in order, as `id` or `id?denied`.
        seen: Arc<Mutex<Vec<String>>>,
        /// How many times each capability actually ran.
        ran: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                answers: HashMap::new(),
                approval_required: Vec::new(),
                human_only: Vec::new(),
                seen: Arc::new(Mutex::new(Vec::new())),
                ran: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    #[derive(Clone, Default)]
    struct Calls {
        seen: Arc<Mutex<Vec<String>>>,
        ran: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl Calls {
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
        fn ran(&self, id: &str) -> usize {
            *self.ran.lock().unwrap().get(id).unwrap_or(&0)
        }
    }

    impl Fake {
        fn calls(&self) -> Calls {
            Calls {
                seen: self.seen.clone(),
                ran: self.ran.clone(),
            }
        }
    }

    impl Backend for Fake {
        fn invoke(&mut self, request: &InvokeRequest, approval: Approval) -> Result<InvokeResult> {
            let record = if approval == Approval::Denied {
                format!("{} denied", request.id)
            } else {
                request.id.clone()
            };
            self.seen.lock().unwrap().push(record);

            if self.human_only.contains(&request.id) {
                return Err(DevdError::denied(format!("`{}` is human-only", request.id)));
            }
            if self.approval_required.contains(&request.id)
                && approval != Approval::Granted
                && !request.dry_run
            {
                return Err(DevdError::ApprovalRequired {
                    capability: request.id.clone(),
                    reason: format!("`{}` needs approval", request.id),
                });
            }
            if approval == Approval::Denied {
                return Err(DevdError::denied(format!(
                    "`{}` was declined by the operator",
                    request.id
                )));
            }

            if request.dry_run {
                let plan = CorePlan::new(&request.id, format!("plan for {}", request.id))
                    .step("the plan")
                    .destructive(self.approval_required.contains(&request.id));
                return Ok(InvokeResult {
                    capability: request.id.clone(),
                    dry_run: true,
                    plan: Some(plan),
                    outcome: None,
                });
            }

            // Only an application counts as having run; a plan changes nothing
            // and must not be counted, or "nothing ran" would be untestable.
            *self
                .ran
                .lock()
                .unwrap()
                .entry(request.id.clone())
                .or_insert(0) += 1;

            let data = self
                .answers
                .get(&request.id)
                .cloned()
                .unwrap_or(Value::Null);
            Ok(InvokeResult {
                capability: request.id.clone(),
                dry_run: false,
                plan: None,
                outcome: Some(Outcome::with_data(format!("{} done", request.id), data)),
            })
        }

        fn plan(&mut self, request: &InvokeRequest) -> Result<Option<CorePlan>> {
            let dry = InvokeRequest {
                id: request.id.clone(),
                args: request.args.clone(),
                dry_run: true,
            };
            Ok(self.invoke(&dry, Approval::None)?.plan)
        }

        fn capabilities(&mut self, query: &str) -> Result<Vec<CapRow>> {
            let all = vec![
                CapRow {
                    id: "git.status".to_string(),
                    tier: "read".to_string(),
                    summary: "Show status".to_string(),
                    requires_approval: false,
                    agent_safe: true,
                },
                CapRow {
                    id: "git.discard".to_string(),
                    tier: "destructive".to_string(),
                    summary: "Discard changes".to_string(),
                    requires_approval: true,
                    agent_safe: true,
                },
                CapRow {
                    id: "git.config.set".to_string(),
                    tier: "admin".to_string(),
                    summary: "Set config".to_string(),
                    requires_approval: true,
                    agent_safe: false,
                },
            ];
            Ok(all
                .into_iter()
                .filter(|c| query.is_empty() || c.id.contains(query))
                .collect())
        }

        fn describe(&mut self, id: &str) -> Result<Value> {
            Ok(json!({ "id": id, "tier": "read", "args_schema": { "type": "object" } }))
        }

        fn audit(&mut self, _limit: u32) -> Result<Vec<AuditRow>> {
            Ok(vec![AuditRow {
                at: "2026-10-19T00:00:00Z".to_string(),
                actor: "human:tui".to_string(),
                capability: "git.discard".to_string(),
                tier: "destructive".to_string(),
                ok: false,
                dry_run: false,
                message: "declined".to_string(),
            }])
        }

        fn doctor(&mut self) -> Result<Vec<CheckRow>> {
            Ok(vec![
                CheckRow {
                    name: "layout".to_string(),
                    level: "ok".to_string(),
                    detail: "D:\\devd-test-home".to_string(),
                },
                CheckRow {
                    name: "port.http".to_string(),
                    level: "info".to_string(),
                    detail: "8080 held by pid 12".to_string(),
                },
            ])
        }

        fn home(&self) -> String {
            "D:\\devd-test-home".to_string()
        }

        fn git_version(&self) -> String {
            "2.46.0.windows.1".to_string()
        }
    }

    fn status_payload() -> Value {
        json!({
            "branch": "main",
            "clean": false,
            "staged": [{ "status": "M", "path": "src/main.rs" }],
            "unstaged": [{ "status": "M", "path": "README.md" }],
            "untracked": ["notes.txt"],
            "conflicted": [],
            "ahead": 0,
            "behind": 0,
            "unborn": false,
            "in_progress": null
        })
    }

    fn controller() -> (Controller, Calls) {
        let mut fake = Fake::default();
        fake.answers
            .insert("git.status".to_string(), status_payload());
        fake.answers.insert(
            "git.log".to_string(),
            json!([{ "short": "abc1234", "hash": "abc1234def", "subject": "First", "author": "Ian", "timestamp": 1 }]),
        );
        fake.answers.insert(
            "git.branch.list".to_string(),
            json!([
                { "name": "main", "current": true, "upstream": "origin/main" },
                { "name": "feature", "current": false, "upstream": null }
            ]),
        );
        fake.answers.insert(
            "git.repo.list".to_string(),
            json!([{ "name": "demo", "path": "D:/tmp/demo", "default_branch": "main", "remote_url": null }]),
        );
        fake.answers.insert(
            "git.show".to_string(),
            json!({ "subject": "First", "patch": "diff --git a/x b/x" }),
        );
        fake.answers
            .insert("git.switch.check".to_string(), json!({ "at_risk": [] }));
        fake.approval_required.push("git.discard".to_string());
        fake.human_only.push("git.config.set".to_string());
        let calls = fake.calls();
        (Controller::new(Box::new(fake)), calls)
    }

    fn key(c: char) -> Key {
        Key::Char(c)
    }

    #[test]
    fn a_digit_switches_view_and_loads_it() {
        let (mut controller, calls) = controller();
        assert!(controller.handle_key(key('2')));
        assert_eq!(controller.state.view, View::Projects);
        assert_eq!(controller.state.projects.len(), 1);
        assert!(calls.seen().iter().any(|c| c == "git.repo.list"));
    }

    #[test]
    fn tab_cycles_through_every_view_and_wraps() {
        let (mut controller, _) = controller();
        for _ in 0..View::ALL.len() {
            controller.handle_key(Key::Tab);
        }
        assert_eq!(controller.state.view, View::Dashboard);
    }

    #[test]
    fn q_quits_and_control_c_quits() {
        let (mut controller, _) = controller();
        assert!(!controller.handle_key(key('q')));
        assert!(!Controller::handle_key(&mut controller, Key::Ctrl('c')));
    }

    #[test]
    fn the_working_tree_list_is_flat_and_cursor_can_move_over_it() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Changes);
        let status = controller.state.status.clone().unwrap();
        assert_eq!(status.items().len(), 3);
        controller.handle_key(key('j'));
        assert_eq!(controller.state.cursor, 1);
        // The cursor is on README.md, which is unstaged.
        assert_eq!(
            controller.state.selected_change().unwrap().path,
            "README.md"
        );
    }

    #[test]
    fn enter_stages_the_selected_unstaged_file_then_unstages_it() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.handle_key(key('j')); // README.md, unstaged
        controller.handle_key(Key::Enter);
        assert_eq!(calls.ran("git.stage"), 1);

        // Now put the cursor on the staged file and press Enter again.
        controller.state.cursor = 0;
        controller.handle_key(Key::Enter);
        assert_eq!(calls.ran("git.unstage"), 1);
    }

    #[test]
    fn a_destructive_action_does_not_run_until_it_is_approved() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.state.cursor = 2; // notes.txt
        controller.handle_key(key('d'));

        // Nothing ran, and the person is looking at a plan.
        assert_eq!(calls.ran("git.discard"), 0);
        let Overlay::Approval(pending) = &controller.state.overlay else {
            panic!(
                "expected an approval prompt, got {:?}",
                controller.state.overlay
            );
        };
        assert_eq!(pending.id, "git.discard");
        assert!(pending.plan.is_some(), "the plan was not fetched");
        assert!(pending.plan.as_ref().unwrap().destructive);
        // The request that will run is the one that was shown, not a rebuild.
        assert_eq!(pending.request.args["paths"][0], "notes.txt");

        controller.handle_key(key('y'));
        assert_eq!(calls.ran("git.discard"), 1);
        assert!(!controller.state.has_overlay());
    }

    #[test]
    fn declining_a_destructive_action_records_the_refusal_and_runs_nothing() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.state.cursor = 2;
        controller.handle_key(key('d'));
        controller.handle_key(key('n'));

        assert_eq!(calls.ran("git.discard"), 0, "a declined action ran");
        assert!(!controller.state.has_overlay());
        // The refusal was dispatched, so the backend recorded it.
        assert!(
            calls.seen().iter().any(|c| c == "git.discard denied"),
            "the refusal was not sent to the backend: {:?}",
            calls.seen()
        );
        assert!(controller.state.message.contains("declined"));
    }

    #[test]
    fn escape_also_declines() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.state.cursor = 2;
        controller.handle_key(key('d'));
        controller.handle_key(Key::Esc);
        assert_eq!(calls.ran("git.discard"), 0);
        assert!(calls.seen().iter().any(|c| c == "git.discard denied"));
    }

    #[test]
    fn a_commit_prompt_collects_text_and_commits_once() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.handle_key(key('c'));
        for ch in "fix the thing".chars() {
            controller.handle_key(key(ch));
        }
        controller.handle_key(Key::Backspace);
        controller.handle_key(Key::Enter);
        assert_eq!(calls.ran("git.commit"), 1);
        assert!(!controller.state.has_overlay());
    }

    #[test]
    fn an_empty_commit_message_is_refused_without_calling_git() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.handle_key(key('c'));
        controller.handle_key(Key::Enter);
        assert_eq!(calls.ran("git.commit"), 0);
        assert!(controller.state.message_is_error);
    }

    #[test]
    fn escape_abandons_a_prompt_without_acting() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Changes);
        controller.handle_key(key('c'));
        controller.handle_key(key('x'));
        controller.handle_key(Key::Esc);
        assert!(!controller.state.has_overlay());
        assert_eq!(calls.ran("git.commit"), 0);
        // A key pressed at the prompt did not leak through to the view.
        assert!(!calls.seen().iter().any(|c| c == "git.stage"));
    }

    #[test]
    fn a_human_only_capability_reports_the_refusal() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Capabilities);
        let index = controller
            .state
            .capabilities
            .iter()
            .position(|c| c.id == "git.config.set")
            .unwrap();
        controller.state.cursor = index;
        controller.handle_key(key('x'));
        controller.handle_key(Key::Enter);
        assert_eq!(calls.ran("git.config.set"), 0);
        assert!(controller.state.message_is_error);
        assert!(controller.state.message.contains("human-only"));
    }

    #[test]
    fn finding_a_capability_filters_the_list() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Capabilities);
        assert_eq!(controller.state.capabilities.len(), 3);
        controller.handle_key(key('/'));
        for ch in "discard".chars() {
            controller.handle_key(key(ch));
        }
        controller.handle_key(Key::Enter);
        assert_eq!(controller.state.capabilities.len(), 1);
        assert_eq!(controller.state.capabilities[0].id, "git.discard");
    }

    #[test]
    fn bad_capability_arguments_are_reported_and_nothing_runs() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Capabilities);
        controller.handle_key(key('x'));
        for ch in "{not json".chars() {
            controller.handle_key(key(ch));
        }
        controller.handle_key(Key::Enter);
        assert!(controller.state.message_is_error);
        assert!(controller.state.message.contains("not valid JSON"));
        assert!(!calls.seen().iter().any(|c| c == "git.status"));
    }

    #[test]
    fn describing_a_capability_opens_a_detail_overlay() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Capabilities);
        controller.handle_key(Key::Enter);
        match &controller.state.overlay {
            Overlay::Detail { title, lines, .. } => {
                assert_eq!(title, "git.status");
                assert!(lines.iter().any(|l| l.contains("args_schema")));
            }
            other => panic!("expected a detail overlay, got {other:?}"),
        }
    }

    #[test]
    fn a_commit_detail_overlay_scrolls_and_closes() {
        let (mut controller, _) = controller();
        controller.switch_view(View::History);
        controller.handle_key(Key::Enter);
        assert!(matches!(controller.state.overlay, Overlay::Detail { .. }));
        controller.handle_key(key('j'));
        match &controller.state.overlay {
            Overlay::Detail { scroll, .. } => assert_eq!(*scroll, 1),
            other => panic!("expected detail, got {other:?}"),
        }
        controller.handle_key(Key::Esc);
        assert!(!controller.state.has_overlay());
    }

    #[test]
    fn switching_branch_asks_the_registry_what_is_at_risk() {
        let (mut controller, calls) = controller();
        controller.switch_view(View::Branches);
        controller.state.cursor = 1; // feature
        controller.handle_key(Key::Enter);
        assert!(calls.seen().iter().any(|c| c == "git.switch.check"));
        assert_eq!(calls.ran("git.switch"), 1);
        assert!(controller.state.message.contains("switched to feature"));
    }

    #[test]
    fn a_switch_that_would_lose_work_becomes_a_confirm() {
        let mut fake = Fake::default();
        fake.answers
            .insert("git.status".to_string(), status_payload());
        fake.answers.insert(
            "git.repo.list".to_string(),
            json!([{ "name": "demo", "path": "D:/tmp/demo" }]),
        );
        fake.answers.insert(
            "git.branch.list".to_string(),
            json!([{ "name": "feature", "current": false }]),
        );
        fake.answers.insert(
            "git.switch.check".to_string(),
            json!({ "at_risk": ["src/main.rs"] }),
        );
        let calls = fake.calls();
        let mut controller = Controller::new(Box::new(fake));
        controller.switch_view(View::Branches);
        controller.handle_key(Key::Enter);

        assert_eq!(
            calls.ran("git.switch"),
            0,
            "an unsafe switch ran unreviewed"
        );
        let Overlay::Approval(pending) = &controller.state.overlay else {
            panic!("expected a confirm, got {:?}", controller.state.overlay);
        };
        let plan = pending.plan.as_ref().unwrap();
        assert!(plan.destructive);
        assert!(plan.steps.iter().any(|s| s.contains("src/main.rs")));

        controller.handle_key(key('y'));
        assert_eq!(calls.ran("git.switch"), 1);
    }

    #[test]
    fn the_repository_defaults_to_the_first_project() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Changes);
        assert_eq!(controller.state.repo.as_deref(), Some("D:/tmp/demo"));
        assert!(controller.state.status.is_some());
    }

    #[test]
    fn with_no_repository_the_git_views_say_so_instead_of_failing_obscurely() {
        let mut fake = Fake::default();
        fake.answers.insert("git.repo.list".to_string(), json!([]));
        let mut controller = Controller::new(Box::new(fake));
        controller.switch_view(View::Changes);
        assert!(controller.state.status.is_none());
        assert!(controller.state.message_is_error);
        assert!(controller.state.message.contains("no repository"));
    }

    #[test]
    fn the_doctor_reports_the_checks_it_was_given() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Doctor);
        assert_eq!(controller.state.checks.len(), 2);
        assert_eq!(controller.state.failing_checks(), 0);
        assert_eq!(controller.state.checks[1].level, "info");
    }

    #[test]
    fn the_activity_view_labels_a_refusal_as_a_refusal() {
        let (mut controller, _) = controller();
        controller.switch_view(View::Activity);
        assert_eq!(controller.state.audit.len(), 1);
        assert!(!controller.state.audit[0].ok);
        assert_eq!(controller.state.audit[0].capability, "git.discard");
    }

    #[test]
    fn the_header_comes_from_the_backend() {
        let (controller, _) = controller();
        assert_eq!(controller.state.home, "D:\\devd-test-home");
        assert!(controller.state.git_version.starts_with("2.46"));
    }

    #[test]
    fn flatten_renders_nested_json_without_dropping_a_field() {
        let value = json!({
            "id": "git.status",
            "nested": { "a": 1, "b": [ { "c": true } ] },
            "empty_obj": {},
            "empty_arr": [],
            "nothing": null
        });
        let lines = value_lines(&value);
        let joined = lines.join("\n");
        for expected in [
            "id: git.status",
            "nested:",
            "a: 1",
            "[0]",
            "c: true",
            "empty_obj:",
            "{}",
            "empty_arr:",
            "[]",
            "nothing: (none)",
        ] {
            assert!(
                joined.contains(expected),
                "missing `{expected}` in:\n{joined}"
            );
        }
    }
}
