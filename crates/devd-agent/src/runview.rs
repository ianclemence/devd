//! A run as it unfolded: for each task, what its agent spent its time on.
//!
//! The log of a conversation already holds it: each model turn, each tool call and when it came back. This reads that log
//! and the team's own events into spans with a kind and a length, so a person (or an agent) can see at a glance that a task
//! spent forty minutes writing, nine rejected and retried, and three waiting on the provider, without opening a transcript.
//! Nothing is recorded again: the spans are derived from what was already kept, so they cannot disagree with it.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use devd_core::store_ai::ChatEvent;
use devd_core::TeamEvent;
use serde_json::{json, Value};

/// What a stretch of a task's time was spent on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The model reading and writing: from the last thing it was given to the reply.
    Model,
    /// A tool call that changed or read something.
    Tool,
    /// A check: the project's own tests, build or lint, run for proof.
    Check,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Model => "model",
            Kind::Tool => "tool",
            Kind::Check => "check",
        }
    }
}

fn at(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Whether a capability call is a check.
fn is_check(capability: &str, args: &Value) -> bool {
    match capability {
        "project.verify" => true,
        "project.run" => matches!(
            args["task"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some(
                "test"
                    | "tests"
                    | "lint"
                    | "typecheck"
                    | "check"
                    | "build"
                    | "verify"
                    | "ci"
                    | "compile"
                    | "format-check"
                    | "fmt-check"
            )
        ),
        _ => false,
    }
}

/// The spans of one conversation, as `(kind, label, start, end)`, in the order they began.
pub fn spans_of(
    events: &[ChatEvent],
    began: Option<DateTime<Utc>>,
) -> Vec<(Kind, String, DateTime<Utc>, DateTime<Utc>)> {
    let mut out = Vec::new();
    let mut calls: HashMap<String, (String, Value, DateTime<Utc>)> = HashMap::new();
    let mut cursor = began;
    for e in events {
        let Some(when) = at(&e.created_at) else {
            continue;
        };
        match e.kind.as_str() {
            "assistant" => {
                if let Some(from) = cursor.filter(|from| *from < when) {
                    out.push((Kind::Model, "model turn".to_string(), from, when));
                }
                for b in e.data["content"].as_array().cloned().unwrap_or_default() {
                    if b["type"] == "tool_use" {
                        let (name, input) = (b["name"].as_str().unwrap_or(""), &b["input"]);
                        // The tools devd gives an agent are reached through one door; the capability is what was asked.
                        let (cap, args) = if name == "devd_invoke" {
                            (
                                input["id"].as_str().unwrap_or(name).to_string(),
                                input["args"].clone(),
                            )
                        } else {
                            (name.to_string(), input.clone())
                        };
                        calls.insert(
                            b["id"].as_str().unwrap_or("").to_string(),
                            (cap, args, when),
                        );
                    }
                }
                cursor = Some(when);
            }
            "tool_result" => {
                if let Some((cap, args, from)) =
                    calls.remove(e.data["tool_use_id"].as_str().unwrap_or(""))
                {
                    let kind = if is_check(&cap, &args) {
                        Kind::Check
                    } else {
                        Kind::Tool
                    };
                    if from < when {
                        out.push((kind, cap, from, when));
                    }
                }
                cursor = Some(cursor.map_or(when, |c| c.max(when)));
            }
            "user" => cursor = Some(when),
            _ => {}
        }
    }
    out
}

/// What one task contributes to a trace.
pub struct TaskTrace<'a> {
    /// Its key.
    pub key: &'a str,
    /// Its role.
    pub role: &'a str,
    /// Its state.
    pub state: &'a str,
    /// When it began, if it has.
    pub started: Option<&'a str>,
    /// When it ended, if it has.
    pub finished: Option<&'a str>,
    /// Its conversation's log.
    pub events: &'a [ChatEvent],
    /// The team's events about it, which say when it was rejected, retried or held up.
    pub team: Vec<&'a TeamEvent>,
}

/// The marks the team's events put on a task's line: things that happened to it rather than things it did.
fn mark_of(e: &TeamEvent) -> Option<(&'static str, String)> {
    match e.kind.as_str() {
        "rejected" => Some((
            "rejected",
            format!(
                "sent back: {}",
                e.data["why"].as_str().unwrap_or("not accepted")
            ),
        )),
        "retrying" => Some((
            "provider",
            format!(
                "waiting on the provider (attempt {})",
                e.data["attempt"].as_u64().unwrap_or(1)
            ),
        )),
        "stalled" => Some((
            "stalled",
            format!(
                "quiet for {} minutes",
                e.data["minutes"].as_u64().unwrap_or(0)
            ),
        )),
        "merge_blocked" => Some((
            "held",
            format!("could not merge: {}", e.data["why"].as_str().unwrap_or("")),
        )),
        "wrap_up" => Some(("wrap_up", "asked to wrap up and hand over".to_string())),
        "accepted" => Some(("accepted", "accepted".to_string())),
        "blocked" => Some(("blocked", "blocked".to_string())),
        "retry" | "started" => Some(("started", e.kind.clone())),
        _ => None,
    }
}

/// A run's trace: every task's spans and marks, in seconds from the start of the run.
pub fn trace(tasks: &[TaskTrace<'_>], run_started: &str, now: DateTime<Utc>) -> Value {
    let origin = at(run_started)
        .or_else(|| tasks.iter().filter_map(|t| t.started.and_then(at)).min())
        .unwrap_or(now);
    let secs = |t: DateTime<Utc>| (t - origin).num_milliseconds() as f64 / 1000.0;
    let rows: Vec<Value> = tasks
        .iter()
        .map(|t| {
            let began = t.started.and_then(at);
            let ended = t.finished.and_then(at);
            let spans: Vec<Value> = spans_of(t.events, began).into_iter().map(|(kind, label, from, to)| json!({ "kind": kind.as_str(), "label": label, "start": secs(from), "end": secs(to) })).collect();
            let marks: Vec<Value> = t.team.iter().filter_map(|e| mark_of(e).and_then(|(kind, label)| at(&e.at).map(|w| json!({ "kind": kind, "label": label, "at": secs(w) })))).collect();
            json!({
                "key": t.key,
                "role": t.role,
                "state": t.state,
                "start": began.map(secs),
                "end": ended.map(secs),
                "spans": spans,
                "marks": marks,
            })
        })
        .collect();
    json!({ "started": run_started, "length": secs(now), "tasks": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, when: &str, data: Value) -> ChatEvent {
        ChatEvent {
            id: 0,
            thread_id: "t".into(),
            kind: kind.into(),
            data,
            created_at: when.into(),
        }
    }

    #[test]
    fn a_conversation_becomes_model_turns_and_tool_calls_with_the_checks_told_apart() {
        let events = vec![
            event(
                "assistant",
                "2026-11-13T13:00:30Z",
                json!({ "content": [{ "type": "tool_use", "id": "a", "name": "devd_invoke", "input": { "id": "fs.write", "args": { "path": "x.go" } } }] }),
            ),
            event(
                "tool_result",
                "2026-11-13T13:00:31Z",
                json!({ "tool_use_id": "a" }),
            ),
            event(
                "assistant",
                "2026-11-13T13:01:00Z",
                json!({ "content": [{ "type": "tool_use", "id": "b", "name": "devd_invoke", "input": { "id": "project.verify", "args": {} } }] }),
            ),
            event(
                "tool_result",
                "2026-11-13T13:01:40Z",
                json!({ "tool_use_id": "b" }),
            ),
        ];
        let began = at("2026-11-13T13:00:00Z");
        let spans = spans_of(&events, began);
        let kinds: Vec<(Kind, &str)> = spans.iter().map(|s| (s.0, s.1.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Model, "model turn"),
                (Kind::Tool, "fs.write"),
                (Kind::Model, "model turn"),
                (Kind::Check, "project.verify")
            ]
        );
        let check = &spans[3];
        assert_eq!(
            (check.3 - check.2).num_seconds(),
            40,
            "the check took forty seconds"
        );
    }

    #[test]
    fn project_run_is_a_check_only_for_a_checking_task() {
        assert!(is_check("project.run", &json!({ "task": "test" })));
        assert!(!is_check("project.run", &json!({ "task": "dev" })));
        assert!(is_check("project.verify", &json!({})));
        assert!(!is_check("fs.read", &json!({})));
    }

    #[test]
    fn a_trace_is_in_seconds_from_the_start_of_the_run_and_carries_what_happened_to_each_task() {
        let events = vec![event(
            "assistant",
            "2026-11-13T13:00:20Z",
            json!({ "content": [] }),
        )];
        let rejected = TeamEvent {
            id: 1,
            run_id: "r".into(),
            task_id: Some("t".into()),
            kind: "rejected".into(),
            data: json!({ "why": "no check was run" }),
            at: "2026-11-13T13:00:25Z".into(),
        };
        let tasks = vec![TaskTrace {
            key: "store",
            role: "backend",
            state: "done",
            started: Some("2026-11-13T13:00:10Z"),
            finished: Some("2026-11-13T13:01:00Z"),
            events: &events,
            team: vec![&rejected],
        }];
        let now = at("2026-11-13T13:02:00Z").unwrap();
        let v = trace(&tasks, "2026-11-13T13:00:00Z", now);
        assert_eq!(v["length"], 120.0);
        let task = &v["tasks"][0];
        assert_eq!(
            (task["start"].as_f64(), task["end"].as_f64()),
            (Some(10.0), Some(60.0))
        );
        assert_eq!(task["spans"][0]["kind"], "model");
        assert_eq!(task["spans"][0]["end"], 20.0);
        assert_eq!(task["marks"][0]["kind"], "rejected");
        assert!(task["marks"][0]["label"]
            .as_str()
            .unwrap()
            .contains("no check was run"));
    }
}
