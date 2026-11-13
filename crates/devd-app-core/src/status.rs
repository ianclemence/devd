//! What a state means to a person.
//!
//! devd records states as identifiers: `waiting_approval`, `unverified`,
//! `setup_incomplete`. Those are for code. Shown as they are, they make every
//! list read like a database, and each screen that mapped them to a colour by
//! itself got one wrong (a run waiting for approval was matched under a name
//! devd never writes, so it showed as nothing at all).
//!
//! This is the one place a state becomes a colour, a word and an answer to
//! "does this need me?". Screens ask here and draw what they are given.

use serde_json::Value;

/// How much a state matters, which decides its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nothing to act on and nothing happening.
    Neutral,
    /// Work in progress.
    Info,
    /// Finished well, or healthy.
    Ok,
    /// Waiting on a person, or not as it should be without being broken.
    Warn,
    /// Failed or broken.
    Danger,
}

/// A state, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Its colour.
    pub level: Level,
    /// The word or short phrase a person reads. Empty when the state is the
    /// ordinary resting one and saying it would be noise.
    pub label: String,
    /// Something is happening right now, so its mark should move.
    pub live: bool,
    /// It will not move until the person does something.
    pub needs_you: bool,
}

impl Status {
    fn new(level: Level, label: &str) -> Self {
        Self { level, label: label.to_string(), live: false, needs_you: false }
    }
    fn live(mut self) -> Self {
        self.live = true;
        self
    }
    fn yours(mut self) -> Self {
        self.needs_you = true;
        self
    }
}

/// The state of a run, a task, a conversation, a service or the machine.
///
/// They share one vocabulary in devd, so they share one table here. A state
/// this table has not met is shown in plain words and neutral colour, never
/// hidden: a new state should look unfamiliar, not absent.
pub fn of(state: &str) -> Status {
    use Level::*;
    match state {
        "running" | "working" => Status::new(Info, "Working").live(),
        "planning" => Status::new(Info, "Planning").live(),
        "starting" => Status::new(Info, "Starting").live(),
        "pending" | "queued" => Status::new(Neutral, "Queued"),
        "idle" | "" => Status::new(Neutral, ""),
        "awaiting_plan" => Status::new(Warn, "Plan needs your approval").yours(),
        "waiting_approval" | "awaiting_approval" => Status::new(Warn, "Waiting for your approval").yours(),
        "waiting" | "open" => Status::new(Warn, "Waiting for you").yours(),
        "paused" => Status::new(Warn, "Paused").yours(),
        "blocked" => Status::new(Warn, "Blocked").yours(),
        "unverified" => Status::new(Warn, "Not verified").yours(),
        "missing" => Status::new(Warn, "Missing"),
        "degraded" => Status::new(Warn, "Needs attention"),
        "setup_incomplete" => Status::new(Warn, "Setup incomplete"),
        "done" | "completed" => Status::new(Ok, "Done"),
        "verified" => Status::new(Ok, "Verified"),
        "accepted" => Status::new(Ok, "Accepted"),
        "approved" => Status::new(Ok, "Approved"),
        "resolved" | "answered" | "consumed" => Status::new(Neutral, "Answered"),
        "ok" | "healthy" => Status::new(Ok, "Healthy"),
        "failed" => Status::new(Danger, "Failed"),
        "error" => Status::new(Danger, "Stopped on an error"),
        "critical" => Status::new(Danger, "Not working"),
        "cancelled" => Status::new(Neutral, "Cancelled"),
        "skipped" => Status::new(Neutral, "Skipped"),
        other => Status::new(Neutral, &plain(other)),
    }
}

/// An identifier as words: `setup_incomplete` becomes "Setup incomplete".
fn plain(id: &str) -> String {
    let words = id.replace(['_', '-', '.'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A message in the inbox, by what kind it is and whether it is still open.
///
/// A finished run is news, not a problem: only what stops work is a warning.
pub fn inbox(kind: &str, open: bool) -> Status {
    use Level::*;
    let (level, label, yours) = match kind {
        "done" | "finished" => (Ok, "Finished", false),
        "paused" => (Warn, "Paused", true),
        "blocked" => (Warn, "Blocked", true),
        "question" => (Warn, "Question", true),
        "approval" => (Warn, "Approval", true),
        "plan" => (Warn, "Plan to approve", true),
        "failed" => (Danger, "Failed", true),
        "note" => (Info, "Note", false),
        "proposal" | "suggestion" => (Info, "Proposal", false),
        "" | "message" => (Info, "Message", false),
        other => return Status { level: if open { Info } else { Neutral }, ..Status::new(Neutral, &plain(other)) },
    };
    Status { level: if open { level } else { Neutral }, label: label.to_string(), live: false, needs_you: open && yours }
}

/// A health check's level.
pub fn check(level: &str) -> Level {
    match level {
        "ok" | "pass" => Level::Ok,
        "warn" | "warning" | "note" => Level::Warn,
        "error" | "critical" | "fail" => Level::Danger,
        "info" => Level::Info,
        _ => Level::Neutral,
    }
}

/// Whether a model provider can be used, from its inventory row.
///
/// The order is the order a person would fix things in: switched off, then no
/// key, then a failed check. A provider that has never been checked says so
/// instead of claiming to be ready.
pub fn provider(row: &Value) -> Status {
    if row["enabled"] == Value::Bool(false) {
        return Status::new(Level::Neutral, "Off");
    }
    if row["needs_key"] == Value::Bool(true) && row["has_key"] != Value::Bool(true) {
        return Status::new(Level::Warn, "Needs a key").yours();
    }
    match row["check_ok"].as_bool() {
        Some(true) => Status::new(Level::Ok, "Ready"),
        Some(false) => Status::new(Level::Danger, "Not answering").yours(),
        None => Status::new(Level::Neutral, "Not checked yet"),
    }
}

/// The last part of a path: `D:\devd\www\shop` is `shop`.
fn leaf(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

/// What to tell the person when a write worked, if it is worth telling.
///
/// Most writes show their own result: a service's row changes, a message
/// appears. These are the ones whose result is otherwise invisible, where
/// silence after pressing a button reads as "nothing happened".
pub fn confirmation(capability: &str, data: &Value) -> Option<String> {
    let count = |v: &Value| v.as_array().map(Vec::len).or_else(|| v.as_u64().map(|n| n as usize));
    let task = || data["task"].as_str().or(data["command"].as_str()).unwrap_or("The task").to_string();
    Some(match capability {
        "provider.test" | "provider.verify" => data["message"].as_str()?.to_string(),
        "provider.models" => format!("{} models listed", count(data)?),
        "provider.save" => format!("Saved {}", data["label"].as_str().unwrap_or("the provider")),
        "provider.default" => format!("{} is now the default", data["label"].as_str().or(data["id"].as_str())?),
        "provider.remove" => "Provider removed".to_string(),
        "roles.save" => format!("{} joined the team", data["title"].as_str().unwrap_or("A new member")),
        "roles.delete" => "Member removed".to_string(),
        "skills.install" => format!("Installed {} skills", count(&data["installed"])?),
        "skills.delete" => "Skill deleted.".to_string(),
        "models.refresh" => {
            let models = data["models"].as_u64()?;
            if data["changed"] == Value::Bool(false) {
                format!("The catalogue is up to date ({models} models)")
            } else {
                format!("Catalogue updated: {models} models")
            }
        }
        "models.assign" => "Model assignment saved".to_string(),
        "hosts.sync" => "Site names written to the hosts file".to_string(),
        "inbox.clear" => "Answered messages cleared".to_string(),
        // A scaffold is the one write whose result is a whole new folder, so say
        // what it made and what state it is in. Partial and cancelled are not
        // successes even though the folder may exist.
        "project.init" | "project.create" => {
            let name = data["project_dir"].as_str().map(leaf).unwrap_or("the project").to_string();
            match data["status"].as_str().unwrap_or("") {
                "ok" => format!("Created {name}. Open Projects to find it."),
                "partial" => format!("Created {name}, but one of its checks did not pass. Open Projects to look."),
                "cancelled" => format!("Stopped creating {name}. The files it made are still in the folder."),
                _ => format!("Could not create {name}: {}", data["error"].as_str().unwrap_or("the scaffold did not finish")),
            }
        }
        "project.import" => format!("Added {} to devd", data["name"].as_str().unwrap_or("the project")),
        "project.run" | "project.run_destructive" => {
            if data["mode"].as_str() == Some("background") {
                format!("{} is running in the background.", task())
            } else if data["ok"] == Value::Bool(true) {
                format!("{} finished.", task())
            } else if data["cancelled"] == Value::Bool(true) {
                format!("{} was stopped.", task())
            } else if data["timed_out"] == Value::Bool(true) {
                format!("{} took too long and was stopped.", task())
            } else {
                format!("{} did not finish.", task())
            }
        }
        "project.forget" => "Removed the project from devd. Its files are untouched.".to_string(),
        // The work it stopped reports its own end; a second toast is noise.
        "stacks.cancel" => return None,
        "runtime.use" => "That runtime is now the one new sites use.".to_string(),
        "runtime.install" => format!("Installed {}.", data["version"].as_str().or(data["kind"].as_str()).unwrap_or("the runtime")),
        "runtime.remove" => "Runtime removed.".to_string(),
        "db.create" => format!("Created the database {}.", data["database"].as_str().or(data["name"].as_str()).unwrap_or("")),
        "db.drop" => "Database removed.".to_string(),
        "db.dump" => format!("Saved a backup to {}.", data["file"].as_str().unwrap_or("the backups folder")),
        "db.user_create" => format!("Created the database user {}.", data["user"].as_str().unwrap_or("")),
        "db.user_drop" => "Database user removed.".to_string(),
        "backup.add" => format!("Backups are scheduled for {}.", data["database"].as_str().unwrap_or("the database")),
        "backup.run" => format!("Backup written to {}.", data["file"].as_str().unwrap_or("the backups folder")),
        "backup.remove" => "Backup schedule removed. The copies already taken are kept.".to_string(),
        "tls.issue" => format!("Issued a certificate for {}.", data["host"].as_str().unwrap_or("")),
        "tls.trust" => "This machine now trusts devd's certificate authority.".to_string(),
        "tls.untrust" => "This machine no longer trusts devd's certificate authority.".to_string(),
        "schedule.create" => format!("Scheduled {}.", data["name"].as_str().unwrap_or("the work")),
        "schedule.run_now" => "Started the schedule now.".to_string(),
        // A pause or resume shows in the row itself; a toast would have to guess
        // which way it went, because the reply carries no state.
        "schedule.enable" => return None,
        "schedule.delete" => "Schedule deleted.".to_string(),
        "cron.add" => "Added the cron job.".to_string(),
        "cron.run" => "Ran the cron job now.".to_string(),
        "cron.enable" => return None,
        "cron.remove" => "Cron job removed. Its log is kept.".to_string(),
        "automation.delete" => "Automation deleted.".to_string(),
        "mcp.add" => format!("Added {}. It starts turned off.", data["name"].as_str().unwrap_or("the tool server")),
        "mcp.test" => format!("{} published {}.", data["server"].as_str().unwrap_or("The tool server"), if count(&data["tools"])? == 1 { "1 tool".to_string() } else { format!("{} tools", count(&data["tools"])?) }),
        "mcp.enable" => return None,
        "mcp.remove" => "Tool server removed.".to_string(),
        "share.start" => format!("The link for {} is public at {}.", data["site"].as_str().unwrap_or("the site"), data["url"].as_str().unwrap_or("")),
        "share.stop" => format!("{} is no longer public.", data["site"].as_str().unwrap_or("The site")),
        "memory.apply" => "Kept the note in AGENTS.md.".to_string(),
        "agent_client.add" => format!("{} can use devd now. Restart it to pick up the change.", data["client"].as_str().unwrap_or("The agent")),
        "agent_client.remove" => format!("{} can no longer use devd.", data["client"].as_str().unwrap_or("The agent")),
        "flags.set" => "Flag saved.".to_string(),
        "autostart.enable" => "devd will start when you sign in.".to_string(),
        "autostart.disable" => "devd will no longer start when you sign in.".to_string(),
        "update.apply" => "The update is installed. devd restarts itself.".to_string(),
        "toolchain.install" => format!("Installed {}.", data["tool"].as_str().or(data["name"].as_str()).unwrap_or("the tool")),
        "toolchain.remove" => "Tool removed.".to_string(),
        "mail.delete" => "Message deleted.".to_string(),
        "mail.clear" => "All caught mail deleted.".to_string(),
        "project.rename" => format!("Renamed the project to {}.", data["name"].as_str().unwrap_or("")),
        "fs.write" => "Saved.".to_string(),
        "fs.move" => "Renamed.".to_string(),
        "fs.delete" => "Deleted. devd kept a copy you can restore from Activity.".to_string(),
        // Git, from a project's page. A stage or unstage shows in the lists themselves, so it says nothing.
        "git.stage" | "git.unstage" => return None,
        "git.discard" => "Changes discarded.".to_string(),
        "git.commit" => format!("Committed {}.", data["short"].as_str().or(data["hash"].as_str()).map(|h| h.chars().take(7).collect::<String>()).unwrap_or_default()).replace("Committed .", "Committed."),
        "git.push" => "Pushed.".to_string(),
        "git.pull" => "Pulled.".to_string(),
        "git.fetch" => "Fetched from the remote.".to_string(),
        "git.switch" => "Switched branch.".to_string(),
        "git.branch.create" => format!("Created the branch {}.", data["name"].as_str().unwrap_or("")).replace("branch .", "branch."),
        "git.branch.delete" => "Branch deleted.".to_string(),
        "git.stash.push" => "Changes put aside in a stash.".to_string(),
        "git.stash.apply" => "Stash brought back.".to_string(),
        "git.stash.drop" => "Stash dropped.".to_string(),
        "git.tag.create" => format!("Created the tag {}.", data["name"].as_str().unwrap_or("")).replace("tag .", "tag."),
        "git.revert" => "Reverted. A new commit undoes the old one.".to_string(),
        "todo.create" => "Added the to-do.".to_string(),
        "todo.complete" => "To-do marked done.".to_string(),
        "review.start" => "Started a review. Its findings appear on the Team tab.".to_string(),
        "project.terminal_external" => "Opened a terminal in the project's folder.".to_string(),
        _ => return None,
    })
}

/// A server's refusal, in a sentence a person can act on.
///
/// Most are already readable. The ones that are not are the validator's raw
/// output: a full path, a parenthesised field name, or a machine verb. This
/// keeps the meaning and drops the plumbing.
pub fn failure(message: &str) -> String {
    let text = message.trim();
    // Service errors name the capability to run. A person in the window needs
    // the page to fix it on, not a command line.
    if text.contains("no database server is installed") || text.contains("MariaDB is not installed") {
        return "MariaDB is not installed in this devd. Add it from the Tools page.".to_string();
    }
    // `the request is not valid: <path> already exists and is not empty; choose
    // another name or destination (name)`.
    if let Some(rest) = text.strip_prefix("the request is not valid: ") {
        if let Some(path) = rest.strip_suffix(" already exists and is not empty; choose another name or destination (name)") {
            return format!("A project folder already exists at {path}. Choose another name or folder.");
        }
        // Any other validation issue: drop a trailing `(field)`.
        let clean = rest.rsplit_once(" (").map(|(before, after)| if after.ends_with(')') { before } else { rest }).unwrap_or(rest);
        return capitalise(clean);
    }
    if let Some(rest) = text.strip_prefix("prerequisites are not met: ") {
        return format!("This stack needs something that is not installed: {rest} Install it on the Tools page, then try again.");
    }
    if text.starts_with("not found: no capability") || text.starts_with("no capability") {
        return "devd does not know that action. It may belong to a newer devd; update and try again.".to_string();
    }
    if let Some(rest) = text.strip_prefix("not found: ") {
        return capitalise(rest);
    }
    if let Some(rest) = text.strip_prefix("invalid input: ") {
        return capitalise(rest);
    }
    capitalise(text)
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_states_devd_writes_all_read_as_words() {
        // Every state devd's store and team write. None may reach a screen as an identifier.
        for state in [
            "running", "pending", "paused", "waiting_approval", "failed", "done", "missing", "skipped", "idle", "blocked",
            "unverified", "cancelled", "awaiting_plan", "starting", "planning", "open", "healthy", "error", "setup_incomplete",
            "resolved", "degraded", "consumed", "approved", "ok", "verified", "accepted", "critical",
        ] {
            let status = of(state);
            assert!(!status.label.contains('_'), "{state} -> {}", status.label);
            assert!(state == "idle" || !status.label.is_empty(), "{state} has no word");
            assert!(state == "idle" || status.label.chars().next().unwrap().is_uppercase(), "{state}");
        }
    }

    #[test]
    fn waiting_for_approval_is_a_warning_that_needs_the_person() {
        // The state devd writes, and the spelling a screen once matched instead.
        for state in ["waiting_approval", "awaiting_approval", "awaiting_plan"] {
            let status = of(state);
            assert_eq!(status.level, Level::Warn, "{state}");
            assert!(status.needs_you && !status.live, "{state}");
        }
    }

    #[test]
    fn only_work_in_progress_moves() {
        assert!(of("running").live && of("planning").live && of("starting").live);
        assert!(!of("paused").live && !of("done").live && !of("failed").live);
        assert!(!of("running").needs_you);
    }

    #[test]
    fn an_unknown_state_is_shown_in_plain_words_not_hidden() {
        let status = of("rate_limited");
        assert_eq!((status.level, status.label.as_str()), (Level::Neutral, "Rate limited"));
    }

    #[test]
    fn a_finished_run_is_news_and_an_answered_message_is_quiet() {
        assert_eq!(inbox("done", true).level, Level::Ok);
        assert!(!inbox("done", true).needs_you);
        assert!(inbox("paused", true).needs_you);
        assert_eq!(inbox("plan", true).label, "Plan to approve");
        let answered = inbox("paused", false);
        assert_eq!(answered.level, Level::Neutral);
        assert!(!answered.needs_you);
    }

    #[test]
    fn a_provider_says_what_stands_between_it_and_use() {
        let label = |row: Value| provider(&row).label;
        assert_eq!(label(json!({ "enabled": false, "needs_key": true })), "Off");
        assert_eq!(label(json!({ "enabled": true, "needs_key": true, "has_key": false, "check_ok": true })), "Needs a key");
        assert_eq!(label(json!({ "enabled": true, "needs_key": true, "has_key": true, "check_ok": false })), "Not answering");
        assert_eq!(label(json!({ "enabled": true, "needs_key": false, "check_ok": true })), "Ready");
        assert_eq!(label(json!({ "enabled": true, "needs_key": true, "has_key": true })), "Not checked yet");
    }

    #[test]
    fn an_invisible_success_is_confirmed_and_a_visible_one_is_not() {
        assert_eq!(confirmation("provider.models", &json!([{}, {}, {}])).as_deref(), Some("3 models listed"));
        assert_eq!(confirmation("models.refresh", &json!({ "models": 900, "changed": false })).as_deref(), Some("The catalogue is up to date (900 models)"));
        assert_eq!(confirmation("provider.test", &json!({ "message": "DeepSeek answered in 210 ms" })).as_deref(), Some("DeepSeek answered in 210 ms"));
        assert_eq!(confirmation("service.start", &json!({})), None, "the row itself changes");
        assert_eq!(confirmation("chat.send", &json!({})), None);
    }

    #[test]
    fn a_scaffold_says_which_project_and_how_it_ended() {
        let ok = json!({ "status": "ok", "project_dir": "D:\\devd\\www\\gotest", "error": null });
        assert_eq!(confirmation("project.init", &ok).as_deref(), Some("Created gotest. Open Projects to find it."));
        let partial = json!({ "status": "partial", "project_dir": "/srv/shop", "error": null });
        assert_eq!(confirmation("project.init", &partial).as_deref(), Some("Created shop, but one of its checks did not pass. Open Projects to look."));
        let cancelled = json!({ "status": "cancelled", "project_dir": "/srv/shop", "error": "cancelled" });
        assert_eq!(confirmation("project.init", &cancelled).as_deref(), Some("Stopped creating shop. The files it made are still in the folder."));
        let failed = json!({ "status": "failed", "project_dir": "/srv/shop", "error": "npm could not reach the registry" });
        assert_eq!(confirmation("project.init", &failed).as_deref(), Some("Could not create shop: npm could not reach the registry"));
    }

    #[test]
    fn a_task_says_whether_it_finished_started_or_stopped() {
        let done = json!({ "task": "build", "ok": true, "cancelled": false, "timed_out": false });
        assert_eq!(confirmation("project.run", &done).as_deref(), Some("build finished."));
        let background = json!({ "task": "dev", "mode": "background", "ok": true });
        assert_eq!(confirmation("project.run", &background).as_deref(), Some("dev is running in the background."));
        let stopped = json!({ "task": "test", "ok": false, "cancelled": true, "timed_out": false });
        assert_eq!(confirmation("project.run", &stopped).as_deref(), Some("test was stopped."));
        let failed = json!({ "task": "test", "ok": false, "cancelled": false, "timed_out": false });
        assert_eq!(confirmation("project.run", &failed).as_deref(), Some("test did not finish."));
    }

    #[test]
    fn a_validator_message_is_turned_into_a_sentence() {
        let raw = r"the request is not valid: D:\devd\www\GOTEST already exists and is not empty; choose another name or destination (name)";
        assert_eq!(failure(raw), r"A project folder already exists at D:\devd\www\GOTEST. Choose another name or folder.");
        assert_eq!(failure("the request is not valid: `X` is not a valid Go project name (name)"), "`X` is not a valid Go project name");
        assert_eq!(failure("prerequisites are not met: uv (missing). Use toolchain.install to fix them."), "This stack needs something that is not installed: uv (missing). Use toolchain.install to fix them. Install it on the Tools page, then try again.");
        assert_eq!(failure("no capability 'stack.list'"), "devd does not know that action. It may belong to a newer devd; update and try again.");
        assert_eq!(failure("not found: MariaDB is not installed; `devd service install mariadb` adds it"), "MariaDB is not installed in this devd. Add it from the Tools page.");
        assert_eq!(failure("not found: no log for run 123"), "No log for run 123");
        assert_eq!(failure("the folder is in use"), "The folder is in use");
    }
}
