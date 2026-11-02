//! One question at a time.
//!
//! A run that needs a person used to say it in a sentence assembled from every
//! task that went wrong: `Needs you: a (unverified): ...; b (blocked): ...`. That
//! is a status report, not a question. The person has to read it and work out
//! what to do, and the Inbox and the run view each phrased it slightly
//! differently.
//!
//! [`Needs`] is the answer to "what does this run need from me?" as data: the one
//! task to look at, what is missing, and the actions that resolve it. The Inbox,
//! the run view, `team.get` and the terminal all print the same object, and the
//! same words come from one [`Needs::question`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use devd_core::{TeamRun, TeamTask};

use crate::roles;

/// The single thing a run cannot continue without.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Needs {
    /// `run` when the whole run is blocked, `task` when one task is.
    pub scope: String,
    /// The run it belongs to.
    pub run_id: String,
    /// The task to look at, when the answer is about one.
    pub task_id: Option<String>,
    /// The task's key, so a person can match it with the log.
    pub task_key: Option<String>,
    /// The role that owns it.
    pub role: Option<String>,
    /// The task's title, which is what a person actually reads.
    pub title: String,
    /// One line: the part of the plan that the run is holding on.
    pub line: String,
    /// `no-report`, `not-proved`, `blocked`, `failed`, `merge`, `approval`,
    /// `plan`, `criteria`.
    pub reason: String,
    /// What is missing, in devd's words, not the model's.
    pub missing: Vec<String>,
    /// The steps that would resolve it, in the order that makes sense.
    pub actions: Vec<NeedAction>,
    /// The definition-of-done statements still open, when the reason is `criteria`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
}

/// One thing a person can do about a [`Needs`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NeedAction {
    /// Stable name: `retry`, `accept`, `resume`, `approve_plan`, `cancel`,
    /// `look`.
    pub id: String,
    /// What it says on the button.
    pub label: String,
    /// What it will do, in one sentence.
    pub detail: String,
    /// The capability to invoke, when the action is one.
    pub capability: Option<String>,
    /// Its arguments.
    pub args: Value,
    /// Whether it changes something that cannot be undone.
    #[serde(default)]
    pub dangerous: bool,
}

impl Needs {
    /// The question a person reads first, in one sentence.
    ///
    /// It names the work and what is missing, and nothing else. Everything else
    /// - which task, which role, which evidence - is in the fields.
    pub fn question(&self) -> String {
        match self.reason.as_str() {
            "no-report" => format!("\"{}\" stopped without reporting what it did.", self.title),
            "not-proved" => format!("\"{}\" says it is done, but nothing proves it.", self.title),
            "blocked" => format!("\"{}\" is blocked: {}", self.title, self.first_missing()),
            "failed" => format!("\"{}\" failed: {}", self.title, self.first_missing()),
            "merge" => format!("\"{}\" is finished, but its work could not be merged: {}", self.title, self.first_missing()),
            "approval" => format!("\"{}\" is waiting for your decision.", self.title),
            "plan" => format!("The plan for \"{}\" is ready to read.", self.title),
            "criteria" => format!("\"{}\" finished, but {} requirement(s) of the definition of done may not hold.", self.title, self.open.len()),
            _ => format!("\"{}\" needs you.", self.title),
        }
    }

    /// The evidence line, for a person who wants to know why.
    pub fn detail(&self) -> String {
        if self.missing.is_empty() {
            self.line.clone()
        } else {
            self.missing.join("; ")
        }
    }

    fn first_missing(&self) -> String {
        self.missing
            .first()
            .cloned()
            .or_else(|| Some(self.line.clone()))
            .unwrap_or_default()
    }

    /// The primary action: the one button a person can press without thinking.
    pub fn primary(&self) -> Option<&NeedAction> {
        self.actions.iter().find(|a| !a.dangerous).or(self.actions.first())
    }

    /// The whole object as one line, for a log or a terminal.
    pub fn one_line(&self) -> String {
        format!("{} — {}", self.question(), self.detail())
    }
}

/// One task's need, when it has one.
fn task_need(run: &TeamRun, t: &TeamTask) -> Option<Needs> {
    let role = roles::get(&t.role);
    let why = t.error.clone().unwrap_or_default();
    let (reason, missing) = match t.state.as_str() {
        "unverified" => {
            // The two ways a task is unverified are different questions: it
            // stopped without saying anything, or it said it was done and the
            // tool results do not support it.
            if why.contains("stopped without reporting") {
                ("no-report", vec![why.clone()])
            } else {
                let gaps = role.map(|r| gaps_of(r, &t.evidence)).unwrap_or_default();
                (
                    "not-proved",
                    if gaps.is_empty() {
                        vec![if why.is_empty() { "the work is not supported by the recorded evidence".into() } else { why.clone() }]
                    } else {
                        gaps
                    },
                )
            }
        }
        "blocked" if why.starts_with("merge") || why.contains("could not be merged") => {
            ("merge", vec![why.clone()])
        }
        "blocked" => ("blocked", vec![trim_blocked(&why)]),
        "failed" => ("failed", vec![why.clone()]),
        _ => return None,
    };
    let mut actions = vec![NeedAction {
        id: "retry".into(),
        label: "Try it again".into(),
        detail: "Starts the same task again in the same conversation, with what was missing as the note.".into(),
        capability: Some("team.retry".into()),
        args: json!({ "task": t.id, "note": format!("The previous attempt was not accepted: {}", missing.join("; ")) }),
        dangerous: false,
    }];
    // Accepting by hand is for a person who has looked, not for the agent.
    actions.push(NeedAction {
        id: "accept".into(),
        label: "Accept it as it is".into(),
        detail: "Counts the work without the proof devd asked for. The run's report will say you did this.".into(),
        capability: Some("team.accept".into()),
        args: json!({ "task": t.id }),
        dangerous: true,
    });
    // Reading what it did is always offered: a task can be stuck before its
    // conversation exists, and the run's own log is then what there is to read.
    actions.push(NeedAction {
        id: "look".into(),
        label: "Read what it did".into(),
        detail: "Opens the specialist's conversation, so you can decide before choosing.".into(),
        capability: None,
        args: match &t.thread_id {
            Some(thread) => json!({ "thread": thread }),
            None => json!({ "run": run.id }),
        },
        dangerous: false,
    });
    Some(Needs {
        scope: "task".into(),
        run_id: run.id.clone(),
        task_id: Some(t.id.clone()),
        task_key: Some(t.key.clone()),
        role: Some(t.role.clone()),
        title: t.title.clone(),
        line: format!("{} ({})", t.key, t.role),
        reason: reason.into(),
        missing,
        actions,
        open: Vec::new(),
    })
}

/// The prose a specialist used to explain a block, without devd's prefix.
fn trim_blocked(why: &str) -> String {
    why.strip_prefix("blocked: ").unwrap_or(why).to_string()
}

/// What a role's evidence lacks. The same list the scheduler rejects a report for,
/// shown to a person instead of only written to the log.
fn gaps_of(role: &roles::Role, evidence: &Value) -> Vec<String> {
    let facts: Vec<&Value> = evidence.as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let any = |kind: &str| facts.iter().any(|f| f["kind"] == kind && f["ok"] == true);
    let mut gaps = Vec::new();
    if role.needs.change && !any("change") {
        gaps.push("no file was changed".to_string());
    }
    if role.needs.verify && !any("verify") {
        gaps.push("no check (test, lint, build, check) passed".to_string());
    }
    if role.needs.read && !any("read") {
        gaps.push("the code was not read".to_string());
    }
    if role.needs.command && !facts.iter().any(|f| matches!(f["kind"].as_str(), Some("verify") | Some("command"))) {
        gaps.push("no command ran".to_string());
    }
    gaps
}

/// The question a run is holding on. `None` when it is not waiting for anyone.
///
/// The order matters: an approval that is already queued is the most urgent,
/// because the agent has stopped and is idle until it is answered. Then a plan
/// nobody has read, then the tasks, then the definition of done.
pub fn of(run: &TeamRun, tasks: &[TeamTask]) -> Option<Needs> {
    if run.state == "awaiting_plan" {
        return Some(Needs {
            scope: "run".into(),
            run_id: run.id.clone(),
            task_id: None,
            task_key: None,
            role: None,
            title: run.title.clone(),
            line: "the plan".into(),
            reason: "plan".into(),
            missing: vec![format!("{} task(s) are planned and nothing runs until a person approves them", tasks.iter().filter(|t| t.role != "planner").count())],
            actions: vec![NeedAction {
                id: "approve_plan".into(),
                label: "Read the plan and start".into(),
                detail: "Starts the work as planned. The tasks each become their own conversation.".into(),
                capability: Some("team.approve_plan".into()),
                args: json!({ "run": run.id }),
                dangerous: false,
            }],
            open: Vec::new(),
        });
    }
    // An approval that is already waiting outranks everything: the agent is idle.
    let waiting: Vec<&TeamTask> = tasks
        .iter()
        .filter(|t| t.state == "waiting_approval")
        .collect();
    if let Some(t) = waiting.first() {
        return Some(Needs {
            scope: "task".into(),
            run_id: run.id.clone(),
            task_id: Some(t.id.clone()),
            task_key: Some(t.key.clone()),
            role: Some(t.role.clone()),
            title: t.title.clone(),
            line: format!("{} ({})", t.key, t.role),
            reason: "approval".into(),
            missing: vec!["an operation is queued in the Inbox and the specialist cannot continue until it is answered".into()],
            actions: vec![NeedAction {
                id: "look".into(),
                label: "Answer the request".into(),
                detail: "Opens the decision it is waiting for; nothing runs until you approve.".into(),
                capability: None,
                args: json!({ "view": "inbox" }),
                dangerous: false,
            }],
            open: Vec::new(),
        });
    }
    let real: Vec<&TeamTask> = tasks.iter().filter(|t| t.role != "planner").collect();
    // One question, not a list: the first task that is stuck is the one to
    // answer. Dependants of it are waiting on the same answer.
    if let Some(n) = real.iter().find_map(|t| task_need(run, t)) {
        return Some(n);
    }
    None
}

/// The run finished, but something it promised may not hold.
pub fn of_criteria(run: &TeamRun, open: &[String]) -> Needs {
    Needs {
        scope: "run".into(),
        run_id: run.id.clone(),
        task_id: None,
        task_key: None,
        role: None,
        title: run.title.clone(),
        line: "the definition of done".into(),
        reason: "criteria".into(),
        missing: open.to_vec(),
        actions: vec![NeedAction {
            id: "look".into(),
            label: "Read the report".into(),
            detail: "Shows what the run recorded against each statement, so you can decide whether to accept it.".into(),
            capability: None,
            args: json!({ "run": run.id }),
            dangerous: false,
        }],
        open: open.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::state::Store;
    use devd_core::store_team::NewTask;

    fn run_and_task(state: &str, error: Option<&str>) -> (Store, TeamRun, TeamTask) {
        let s = Store::open_in_memory().unwrap();
        let run = s.create_run("Add dark mode", "add a dark mode toggle", None, 2, "human:http").unwrap();
        let t = s
            .add_task(
                &run.id,
                NewTask {
                    key: "theme",
                    role: "implementer",
                    title: "Add the dark mode toggle",
                    instructions: "add a toggle to the settings page and prove it with the tests",
                    depends_on: &[],
                    scope: &[],
                    mandate: None,
                },
            )
            .unwrap();
        s.update_team_task(
            &t.id,
            devd_core::store_team::TaskUpdate {
                state,
                error,
                evidence: Some(&json!([{ "kind": "read", "ok": true }, { "kind": "change", "ok": true }])),
                ..Default::default()
            },
        )
        .unwrap();
        let t = s.get_team_task(&t.id).unwrap().unwrap();
        (s, run, t)
    }

    #[test]
    fn an_unverified_task_becomes_one_question_with_the_missing_proof() {
        let (_s, run, t) = run_and_task("unverified", Some("It said it was done, but no check passed."));
        let needs = of(&run, &[t]).expect("a stuck task needs a person");
        assert_eq!(needs.reason, "not-proved");
        assert_eq!(needs.task_key.as_deref(), Some("theme"));
        assert_eq!(needs.missing, vec!["no check (test, lint, build, check) passed".to_string()]);
        assert_eq!(needs.question(), "\"Add the dark mode toggle\" says it is done, but nothing proves it.");
        let primary = needs.primary().expect("there is an action");
        assert_eq!(primary.id, "retry");
        assert_eq!(primary.capability.as_deref(), Some("team.retry"));
        // Accepting without proof exists, but it is never the button offered first.
        assert!(needs.actions.iter().any(|a| a.id == "accept" && a.dangerous));
    }

    #[test]
    fn a_task_that_stopped_silently_says_that_instead() {
        let (_s, run, t) = run_and_task("unverified", Some("it stopped without reporting"));
        let needs = of(&run, &[t]).expect("needs a person");
        assert_eq!(needs.reason, "no-report");
        assert!(needs.question().contains("stopped without reporting"));
    }

    #[test]
    fn only_one_task_is_asked_about_even_when_several_are_stuck() {
        let (s, run, first) = run_and_task("unverified", Some("no check passed"));
        let second = s
            .add_task(
                &run.id,
                NewTask {
                    key: "docs",
                    role: "docs",
                    title: "Document the toggle",
                    instructions: "write the docs and stop",
                    depends_on: &[],
                    scope: &[],
                    mandate: None,
                },
            )
            .unwrap();
        s.update_team_task(&second.id, devd_core::store_team::TaskUpdate { state: "failed", error: Some("the provider refused the request"), ..Default::default() }).unwrap();
        let tasks = s.list_tasks_of_run(&run.id).unwrap();
        let needs = of(&run, &tasks).expect("needs a person");
        assert_eq!(needs.scope, "task", "one concrete question, not a status report");
        assert!(needs.actions.iter().any(|a| a.id == "retry"));
        // The one that is reported is the first in plan order, which is the one
        // whose fix unblocks the rest.
        assert_eq!(needs.task_id.as_deref(), Some(first.id.as_str()));
    }

    #[test]
    fn a_healthy_run_asks_for_nothing() {
        let (_s, mut run, t) = run_and_task("done", None);
        run.state = "running".into();
        assert!(of(&run, &[t]).is_none());
    }

    #[test]
    fn a_waiting_approval_outranks_a_stuck_task() {
        let (_s, mut run, mut t) = run_and_task("unverified", Some("no check passed"));
        t.state = "waiting_approval".into();
        run.state = "running".into();
        let needs = of(&run, &[t]).unwrap();
        assert_eq!(needs.reason, "approval");
    }

    #[test]
    fn a_finished_run_with_open_criteria_asks_to_be_read_rather_than_retried() {
        let (_, run, _) = run_and_task("done", None);
        let needs = of_criteria(&run, &["the README documents every command".to_string()]);
        assert_eq!(needs.reason, "criteria");
        assert_eq!(needs.primary().unwrap().id, "look");
    }

    #[test]
    fn the_same_words_are_produced_for_every_surface() {
        let (_s, run, t) = run_and_task("blocked", Some("blocked: I need the API key to continue"));
        let needs = of(&run, &[t]).unwrap();
        // The Inbox, the run view and the terminal all print these two lines.
        assert_eq!(needs.question(), "\"Add the dark mode toggle\" is blocked: I need the API key to continue");
        assert!(needs.one_line().starts_with(&needs.question()));
    }
}
