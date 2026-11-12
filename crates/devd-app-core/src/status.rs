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

/// What to tell the person when a write worked, if it is worth telling.
///
/// Most writes show their own result: a service's row changes, a message
/// appears. These are the ones whose result is otherwise invisible, where
/// silence after pressing a button reads as "nothing happened".
pub fn confirmation(capability: &str, data: &Value) -> Option<String> {
    let count = |v: &Value| v.as_array().map(Vec::len).or_else(|| v.as_u64().map(|n| n as usize));
    Some(match capability {
        "provider.test" | "provider.verify" => data["message"].as_str()?.to_string(),
        "provider.models" => format!("{} models listed", count(data)?),
        "provider.save" => format!("Saved {}", data["label"].as_str().unwrap_or("the provider")),
        "provider.default" => format!("{} is now the default", data["label"].as_str().or(data["id"].as_str())?),
        "provider.remove" => "Provider removed".to_string(),
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
        _ => return None,
    })
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
}
