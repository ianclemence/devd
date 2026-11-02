//! What devd may do on its own, written down before it does it.
//!
//! Three records live here, and they are what keeps an always-on agent honest:
//!
//! * a **mandate** - why a piece of work is allowed, to whom it was given and
//!   under which rules, so resuming it can ask whether the authority still holds;
//! * a **suggestion** - work devd would do, recorded without being done, so a
//!   person can watch what it *would* have done before letting it;
//! * an **effect** - work that must happen outside the transaction that decided
//!   it, retried with backoff, so a failure is a row rather than a lost log line.
//!
//! Intake rules live here too: the project's own statement of what event it wants
//! a run started for, and how many times that event must happen first.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::Result;
use crate::state::Store;
use crate::util;

// ------------------------------------------------------------------ mandates

/// Why one piece of work may run, and how far it may go.
///
/// It is deliberately immutable in spirit: a wider scope is a new task, not an
/// edited mandate, and only a person's stop withdraws it. Before a task resumes
/// after a pause, a restart or a change of mode, [`crate::store_team`] reads this
/// back and asks whether it still holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mandate {
    /// `explicit` (a person asked for it), `schedule` (a standing instruction),
    /// `inferred` (devd or an agent proposed it and a person accepted), or
    /// `system` (the harness added it, such as the final verification).
    pub basis: String,
    /// Who asked, as the actor label: `human:http`, `agent:devd-team`.
    pub origin: String,
    /// The team run it belongs to.
    pub run: String,
    /// The settings the work was accepted under, hashed. A change to the
    /// definition of done, the parallel limit or the environment mode changes it.
    pub policy_version: String,
    /// What it may touch.
    #[serde(default)]
    pub scope: Vec<String>,
    /// The role whose permissions bound it.
    pub role: String,
    /// When the authority was granted.
    pub granted_at: String,
}

impl Mandate {
    /// A mandate for one task.
    pub fn new(
        basis: &str,
        origin: &str,
        run: &str,
        role: &str,
        scope: &[String],
        policy_version: &str,
    ) -> Self {
        Self {
            basis: basis.into(),
            origin: origin.into(),
            run: run.into(),
            policy_version: policy_version.into(),
            scope: scope.to_vec(),
            role: role.into(),
            granted_at: util::now(),
        }
    }

    /// The one line a person reads in the run view.
    pub fn sentence(&self) -> String {
        let who = self.origin.replace("human:", "").replace("agent:", "agent ");
        match self.basis.as_str() {
            "explicit" => format!("{who} asked for this"),
            "schedule" => format!("a schedule set up by {who} asked for this"),
            "inferred" => "devd proposed this; a person accepted it".to_string(),
            "system" => "devd added this step to verify the run".into(),
            other => format!("basis: {other}"),
        }
    }
}

/// A version string for the rules a run was accepted under.
///
/// It covers the things that change what a task may do rather than how it is
/// worded: the environment mode, the definition of done, the parallel limit and
/// the working agreement. When any of those change, a resumed task re-checks
/// instead of assuming its approval still means the same thing.
pub fn policy_version(store: &Store, run_id: &str, agreement: &[String]) -> String {
    let mode = store.environment_mode();
    let (done_when, max_rounds) = store
        .goal_loop(run_id)
        .map(|g| (g.done_when, g.max_rounds))
        .unwrap_or_default();
    let parallel = store
        .get_run(run_id)
        .ok()
        .flatten()
        .map(|r| r.parallel)
        .unwrap_or(1);
    let text = json!({
        "mode": mode,
        "done_when": done_when,
        "max_rounds": max_rounds,
        "parallel": parallel,
        "agreement": agreement,
    })
    .to_string();
    format!("v1:{}", &util::sha256_hex(text.as_bytes())[..16])
}

fn mandate_of(text: &str) -> Option<Mandate> {
    serde_json::from_str::<Mandate>(text).ok()
}

impl Store {
    /// The mandate a task was granted.
    pub fn task_mandate(&self, task_id: &str) -> Result<Option<Mandate>> {
        self.with(|c| {
            let text: Option<String> = c
                .query_row(
                    "SELECT mandate FROM team_tasks WHERE id=?1",
                    params![task_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(text.as_deref().and_then(mandate_of).filter(|m| !m.basis.is_empty()))
        })
    }

    // -------------------------------------------------------------- heartbeat

    /// Record that a task did something just now. Called on every model turn and
    /// every tool call, so the watchdog measures activity rather than liveness.
    pub fn touch_task(&self, task_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_tasks SET last_step_at=?2 WHERE id=?1",
                params![task_id, util::now()],
            )?;
            Ok(())
        })
    }

    /// Start a new stall count, for a task a person has just chosen to resume.
    pub fn reset_stalls(&self, task_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_tasks SET stalls=0, last_step_at=NULL WHERE id=?1",
                params![task_id],
            )?;
            Ok(())
        })
    }

    /// Running tasks that have not done anything since `before`.
    pub fn stalled_tasks(&self, before: &str) -> Result<Vec<crate::store_team::TeamTask>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {} FROM team_tasks
                  WHERE state IN ('running','waiting_approval') AND last_step_at IS NOT NULL AND last_step_at < ?1
                  ORDER BY last_step_at",
                crate::store_team::TASK_COLUMNS
            ))?;
            let rows = st.query_map(params![before], crate::store_team::task_row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Count one more stall on a task and say how many there are now.
    pub fn bump_stall(&self, task_id: &str) -> Result<i64> {
        self.with(|c| {
            c.execute("UPDATE team_tasks SET stalls=stalls+1 WHERE id=?1", params![task_id])?;
            Ok(c.query_row("SELECT stalls FROM team_tasks WHERE id=?1", params![task_id], |r| r.get(0))?)
        })
    }

    // ----------------------------------------------------------- suggestions

    /// Record work devd would do. Nothing runs; this is the record of intent.
    #[allow(clippy::too_many_arguments)]
    pub fn add_suggestion(
        &self,
        project: Option<&str>,
        role: &str,
        title: &str,
        why: &str,
        plan: &[String],
        accepts: &[String],
        source: &str,
        source_ref: Option<&str>,
    ) -> Result<Suggestion> {
        let s = Suggestion {
            id: util::new_id(),
            project: project.map(str::to_string),
            role: role.into(),
            title: title.trim().chars().take(120).collect(),
            why: why.into(),
            plan: plan.to_vec(),
            accepts: accepts.to_vec(),
            source: source.into(),
            source_ref: source_ref.map(str::to_string),
            state: "open".into(),
            run_id: None,
            decided_by: None,
            decided_at: None,
            created_at: util::now(),
        };
        self.with(|c| {
            c.execute(
                "INSERT INTO suggestions (id,project,role,title,why,plan,accepts,source,source_ref,state,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'open',?10)",
                params![
                    s.id,
                    s.project,
                    s.role,
                    s.title,
                    s.why,
                    serde_json::to_string(&s.plan)?,
                    serde_json::to_string(&s.accepts)?,
                    s.source,
                    s.source_ref,
                    s.created_at
                ],
            )?;
            Ok(())
        })?;
        Ok(s)
    }

    /// Suggestions, newest first.
    pub fn list_suggestions(&self, state: Option<&str>, limit: i64) -> Result<Vec<Suggestion>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id,project,role,title,why,plan,accepts,source,source_ref,state,run_id,decided_by,decided_at,created_at
                   FROM suggestions
                  WHERE (?1 IS NULL OR state=?1)
                  ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![state, limit], row_suggestion)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One suggestion.
    pub fn get_suggestion(&self, id: &str) -> Result<Option<Suggestion>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id,project,role,title,why,plan,accepts,source,source_ref,state,run_id,decided_by,decided_at,created_at
                   FROM suggestions WHERE id=?1",
                params![id],
                row_suggestion,
            )
            .optional()?)
        })
    }

    /// An open suggestion with the same title for the same project, so a repeated
    /// rule does not fill the inbox with the same proposal.
    pub fn open_suggestion_titled(&self, project: Option<&str>, title: &str) -> Result<Option<Suggestion>> {
        Ok(self
            .list_suggestions(Some("open"), 200)?
            .into_iter()
            .find(|s| s.project.as_deref() == project && s.title == title))
    }

    /// Decide a suggestion. `accepted` also records the run a person started from
    /// it; `dismissed` is final and is what teaches devd that this is not wanted.
    pub fn decide_suggestion(
        &self,
        id: &str,
        accepted: bool,
        run_id: Option<&str>,
        by: &str,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE suggestions SET state=?2, run_id=COALESCE(?3,run_id), decided_by=?4, decided_at=?5 WHERE id=?1",
                params![id, if accepted { "accepted" } else { "dismissed" }, run_id, by, util::now()],
            )?;
            Ok(())
        })
    }

    /// Remove suggestions older than `before` that a person already dealt with.
    pub fn prune_suggestions_before(&self, before: &str) -> Result<usize> {
        self.with(|c| {
            Ok(c.execute(
                "DELETE FROM suggestions WHERE state<>'open' AND created_at < ?1",
                params![before],
            )?)
        })
    }

    // -------------------------------------------------------------- effects

    /// Queue work that must happen after this transaction commits. Enqueuing the
    /// same kind and key again replaces the payload and makes it due at once, so
    /// a repeated condition does not queue twice.
    pub fn enqueue_effect(&self, kind: &str, key: &str, payload: &Value) -> Result<()> {
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO effects (kind,key,payload,state,attempts,not_before,created_at,updated_at)
                 VALUES (?1,?2,?3,'pending',0,?4,?4,?4)
                 ON CONFLICT(kind,key) DO UPDATE SET payload=?3, state='pending', attempts=0, last_error=NULL, not_before=?4, updated_at=?4",
                params![kind, key, payload.to_string(), now],
            )?;
            Ok(())
        })
    }

    /// Effects that are due, oldest first.
    ///
    /// The time is a parameter rather than "now" so a test can move the clock,
    /// and it compares as an instant rather than as text: two timestamps written
    /// by RFC 3339 should not be ordered by whether one happens to have a
    /// fractional second.
    pub fn due_effects(&self, now: &str, limit: i64) -> Result<Vec<PendingEffect>> {
        let now = chrono::DateTime::parse_from_rfc3339(now)
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT kind,key,payload,state,attempts,last_error,not_before,created_at FROM effects
                  WHERE state='pending' ORDER BY created_at",
            )?;
            let rows = st.query_map([], |r| {
                Ok(PendingEffect {
                    kind: r.get(0)?,
                    key: r.get(1)?,
                    payload: serde_json::from_str(&r.get::<_, String>(2)?)
                        .unwrap_or(Value::Null),
                    state: r.get(3)?,
                    attempts: r.get(4)?,
                    last_error: r.get(5)?,
                    not_before: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                let row = row?;
                let due = chrono::DateTime::parse_from_rfc3339(&row.not_before)
                    .map(|t| t.with_timezone(&chrono::Utc) <= now)
                    .unwrap_or(true);
                if due {
                    out.push(row);
                    if out.len() as i64 >= limit {
                        break;
                    }
                }
            }
            Ok(out)
        })
    }
    pub fn settle_effect(&self, kind: &str, key: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE effects SET state='done', updated_at=?3 WHERE kind=?1 AND key=?2",
                params![kind, key, util::now()],
            )?;
            Ok(())
        })
    }

    /// Record a failed attempt and when to try again.
    pub fn fail_effect(&self, kind: &str, key: &str, error: &str, retry_at: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE effects SET attempts=attempts+1, last_error=?3, not_before=?4, updated_at=?5 WHERE kind=?1 AND key=?2",
                params![kind, key, util::truncate(error, 2000), retry_at, util::now()],
            )?;
            Ok(())
        })
    }

    /// Give up on an effect without calling it a success.
    pub fn drop_effect(&self, kind: &str, key: &str, why: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE effects SET state='dropped', last_error=?3, updated_at=?4 WHERE kind=?1 AND key=?2",
                params![kind, key, util::truncate(why, 2000), util::now()],
            )?;
            Ok(())
        })
    }

    /// How a kind of effect is doing, for the health screen. An error is reported
    /// only after one of its effects failed three times in a row, so a one-off
    /// network blip does not make devd look broken.
    pub fn effect_health(&self) -> Result<Vec<EffectHealth>> {
        self.with(|c| {
            // An effect that is waiting to be retried is still pending, so the
            // failing count is over the pending rows that have failed, not over
            // a separate state.
            let mut st = c.prepare(
                "SELECT kind,
                        SUM(CASE WHEN state='pending' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN state='pending' AND attempts>=3 THEN 1 ELSE 0 END),
                        SUM(CASE WHEN state='pending' AND attempts>=3 AND last_error IS NOT NULL THEN 1 ELSE 0 END),
                        MAX(CASE WHEN last_error IS NOT NULL THEN last_error END),
                        SUM(CASE WHEN state='dropped' THEN 1 ELSE 0 END)
                   FROM effects GROUP BY kind ORDER BY kind",
            )?;
            let rows = st.query_map([], |r| {
                Ok(EffectHealth {
                    kind: r.get(0)?,
                    pending: r.get(1)?,
                    failing: r.get(3)?,
                    last_error: r.get(4)?,
                    dropped: r.get(5)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Remove effects that finished or were dropped before `before`.
    pub fn prune_effects_before(&self, before: &str) -> Result<usize> {
        self.with(|c| {
            Ok(c.execute(
                "DELETE FROM effects WHERE state<>'pending' AND updated_at < ?1",
                params![before],
            )?)
        })
    }

    // ---------------------------------------------------------- intake rules

    /// Add or replace a rule, keeping the failure count a person did not touch.
    pub fn upsert_intake_rule(&self, rule: &IntakeRule) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO intake_rules (project,name,trigger,condition,goal,template,role,mode,countdown,fired_at,enabled,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                 ON CONFLICT(project,name) DO UPDATE SET trigger=?3,condition=?4,goal=?5,template=?6,role=?7,mode=?8,enabled=?11,updated_at=?12",
                params![
                    rule.project,
                    rule.name,
                    rule.trigger,
                    serde_json::to_string(&rule.condition)?,
                    rule.goal,
                    rule.template,
                    rule.role,
                    rule.mode,
                    rule.countdown,
                    rule.fired_at,
                    rule.enabled as i32,
                    util::now()
                ],
            )?;
            Ok(())
        })
    }

    /// The rules for one project, or every project when `project` is `None`.
    pub fn list_intake_rules(&self, project: Option<&str>) -> Result<Vec<IntakeRule>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT project,name,trigger,condition,goal,template,role,mode,countdown,fired_at,enabled
                   FROM intake_rules WHERE (?1 IS NULL OR project=?1) ORDER BY project,name",
            )?;
            let rows = st.query_map(params![project], row_rule)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Enabled rules for one trigger, over every project.
    pub fn intake_rules_for(&self, trigger: &str) -> Result<Vec<IntakeRule>> {
        Ok(self
            .list_intake_rules(None)?
            .into_iter()
            .filter(|r| r.enabled && r.trigger == trigger)
            .collect())
    }

    /// Record what a rule saw: how many times in a row the event matched, and when
    /// it last started a run.
    pub fn record_intake(&self, project: &str, name: &str, countdown: i64, fired: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE intake_rules SET countdown=?3, fired_at=CASE WHEN ?4 THEN ?5 ELSE fired_at END, updated_at=?5
                  WHERE project=?1 AND name=?2",
                params![project, name, countdown, fired as i32, util::now()],
            )?;
            Ok(())
        })
    }

    /// Remove a rule.
    pub fn delete_intake_rule(&self, project: &str, name: &str) -> Result<bool> {
        self.with(|c| {
            Ok(c.execute(
                "DELETE FROM intake_rules WHERE project=?1 AND name=?2",
                params![project, name],
            )? > 0)
        })
    }
}

/// A suggestion: work devd would do, recorded before doing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Id.
    pub id: String,
    /// Project it concerns.
    pub project: Option<String>,
    /// The member of the team who would do it.
    pub role: String,
    /// One line.
    pub title: String,
    /// What it noticed, in its words.
    pub why: String,
    /// The steps it would take.
    pub plan: Vec<String>,
    /// How the work would be known to be done.
    pub accepts: Vec<String>,
    /// `manual`, `intake` or `schedule`.
    pub source: String,
    /// The rule or schedule that produced it.
    pub source_ref: Option<String>,
    /// `open`, `accepted`, `dismissed` or `expired`.
    pub state: String,
    /// The run a person started from it.
    pub run_id: Option<String>,
    /// Who decided.
    pub decided_by: Option<String>,
    /// When.
    pub decided_at: Option<String>,
    /// Created.
    pub created_at: String,
}

fn row_suggestion(r: &rusqlite::Row) -> rusqlite::Result<Suggestion> {
    let list = |s: String| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default();
    Ok(Suggestion {
        id: r.get(0)?,
        project: r.get(1)?,
        role: r.get(2)?,
        title: r.get(3)?,
        why: r.get(4)?,
        plan: list(r.get(5)?),
        accepts: list(r.get(6)?),
        source: r.get(7)?,
        source_ref: r.get(8)?,
        state: r.get(9)?,
        run_id: r.get(10)?,
        decided_by: r.get(11)?,
        decided_at: r.get(12)?,
        created_at: r.get(13)?,
    })
}

/// One row of the durable effect queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEffect {
    /// What kind of work it is.
    pub kind: String,
    /// Its identity within the kind.
    pub key: String,
    /// What the handler needs.
    pub payload: Value,
    /// `pending`, `done` or `dropped`.
    pub state: String,
    /// Attempts so far.
    pub attempts: i64,
    /// Why the last attempt failed.
    pub last_error: Option<String>,
    /// Earliest time to try it.
    pub not_before: String,
    /// Created.
    pub created_at: String,
}

/// How one kind of effect is doing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectHealth {
    /// Kind.
    pub kind: String,
    /// Waiting to run or be retried.
    pub pending: i64,
    /// Waiting and failed at least three times: worth telling somebody about.
    pub failing: i64,
    /// The last error seen.
    pub last_error: Option<String>,
    /// Given up on.
    pub dropped: i64,
}

/// A project's statement of what event should start a run for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeRule {
    /// The project directory.
    pub project: String,
    /// Short name, unique within the project.
    pub name: String,
    /// `health.down`, `check.failed` or `schedule.missed`.
    pub trigger: String,
    /// Extra conditions: `min_failures`, `cooldown_minutes`, `task`.
    pub condition: Value,
    /// What the run should do.
    pub goal: String,
    /// A template to use instead of the goal.
    pub template: Option<String>,
    /// The member who would do it.
    pub role: String,
    /// `observe` records what it would have done; `auto` starts the run (never in
    /// production, and never without plan approval).
    pub mode: String,
    /// How many times in a row the event has matched.
    pub countdown: i64,
    /// When it last started a run.
    pub fired_at: Option<String>,
    /// Off without deleting it.
    pub enabled: bool,
}

impl IntakeRule {
    /// The condition as an object.
    ///
    /// A caller can hand the condition over as a JSON string instead of an
    /// object - `devd call intake.set ... --arg 'condition={"min_failures":3}'`
    /// does exactly that when the shell keeps the braces - and indexing a string
    /// as if it were an object is a panic, not a validation error. Reading it
    /// here means every rule reader is total.
    pub fn condition_object(&self) -> serde_json::Map<String, Value> {
        match &self.condition {
            Value::Object(m) => m.clone(),
            Value::String(text) => serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default(),
            _ => serde_json::Map::new(),
        }
    }

    fn condition_i64(&self, key: &str) -> Option<i64> {
        self.condition_object().get(key).and_then(Value::as_i64)
    }

    /// How many consecutive matches are required before it fires.
    pub fn min_failures(&self) -> i64 {
        self.condition_i64("min_failures").unwrap_or(1).clamp(1, 50)
    }

    /// The shortest time between two runs this rule starts.
    pub fn cooldown_minutes(&self) -> i64 {
        self.condition_i64("cooldown_minutes").unwrap_or(60).clamp(0, 10_080)
    }

    /// Whether enough time has passed since it last fired.
    pub fn cooled_down(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        let Some(fired) = self
            .fired_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        else {
            return true;
        };
        now.signed_duration_since(fired.with_timezone(&chrono::Utc))
            >= chrono::Duration::minutes(self.cooldown_minutes())
    }
}

fn row_rule(r: &rusqlite::Row) -> rusqlite::Result<IntakeRule> {
    Ok(IntakeRule {
        project: r.get(0)?,
        name: r.get(1)?,
        trigger: r.get(2)?,
        condition: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_else(|_| json!({})),
        goal: r.get(4)?,
        template: r.get(5)?,
        role: r.get(6)?,
        mode: r.get(7)?,
        countdown: r.get(8)?,
        fired_at: r.get(9)?,
        enabled: r.get::<_, i64>(10)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggestion_is_recorded_and_decided_without_ever_running() {
        let s = Store::open_in_memory().unwrap();
        let one = s
            .add_suggestion(
                None,
                "implementer",
                "Fix the failing build",
                "the nightly build has failed three times",
                &["run the build".to_string()],
                &["the build exits 0".to_string()],
                "intake",
                Some("nightly"),
            )
            .unwrap();
        assert_eq!(one.state, "open");
        assert!(s.open_suggestion_titled(None, "Fix the failing build").is_ok_and(|x| x.is_some()));
        s.decide_suggestion(&one.id, true, Some("run-1"), "human:http").unwrap();
        let after = s.get_suggestion(&one.id).unwrap().unwrap();
        assert_eq!(after.state, "accepted");
        assert_eq!(after.run_id.as_deref(), Some("run-1"));
        // A decided suggestion is no longer open, so the same rule can propose again.
        assert!(s.open_suggestion_titled(None, "Fix the failing build").unwrap().is_none());
    }

    #[test]
    fn an_effect_that_fails_is_retried_and_then_settled() {
        let s = Store::open_in_memory().unwrap();
        // A moment after it is queued, so a clock that is one tick behind the
        // row's own timestamp cannot make a fresh effect look early.
        let now = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
        s.enqueue_effect("inbox.health", "grafana", &json!({ "site": "grafana" })).unwrap();
        let due = s.due_effects(&now, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 0);
        // A failed attempt does not become due again immediately.
        let later = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
        s.fail_effect("inbox.health", "grafana", "connection refused", &later).unwrap();
        assert!(s.due_effects(&now, 10).unwrap().is_empty(), "it waits for its retry time");
        let due = s.due_effects(&later, 10).unwrap();
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].last_error.as_deref(), Some("connection refused"));
        s.settle_effect("inbox.health", "grafana").unwrap();
        assert!(s.due_effects(&later, 10).unwrap().is_empty());
        let health = s.effect_health().unwrap();
        assert_eq!(health[0].kind, "inbox.health");
        assert_eq!(health[0].failing, 0, "a settled effect is not failing");
    }

    #[test]
    fn queuing_the_same_effect_twice_replaces_it_instead_of_stacking() {
        let s = Store::open_in_memory().unwrap();
        s.enqueue_effect("inbox.health", "k", &json!({ "n": 1 })).unwrap();
        s.settle_effect("inbox.health", "k").unwrap();
        s.enqueue_effect("inbox.health", "k", &json!({ "n": 2 })).unwrap();
        let due = s.due_effects(&util::now(), 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].payload["n"], 2);
        assert_eq!(due[0].attempts, 0, "a new payload is a fresh try");
    }

    #[test]
    fn an_intake_rule_waits_for_its_count_and_its_cooldown() {
        let s = Store::open_in_memory().unwrap();
        let mut rule = IntakeRule {
            project: "D:\\work\\shop".into(),
            name: "nightly".into(),
            trigger: "check.failed".into(),
            condition: json!({ "min_failures": 3, "cooldown_minutes": 30 }),
            goal: "Make the nightly check pass again".into(),
            template: None,
            role: "implementer".into(),
            mode: "observe".into(),
            countdown: 0,
            fired_at: None,
            enabled: true,
        };
        assert_eq!(rule.min_failures(), 3);
        assert!(rule.cooled_down(chrono::Utc::now()), "a rule never fired may fire");
        s.upsert_intake_rule(&rule).unwrap();
        s.record_intake(&rule.project, &rule.name, 2, false).unwrap();
        let back = s.intake_rules_for("check.failed").unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].countdown, 2);
        s.record_intake(&rule.project, &rule.name, 0, true).unwrap();
        rule.fired_at = s.list_intake_rules(None).unwrap()[0].fired_at.clone();
        assert!(!rule.cooled_down(chrono::Utc::now()), "it fired just now");
        assert!(
            rule.cooled_down(chrono::Utc::now() + chrono::Duration::minutes(31)),
            "the cooldown is over"
        );
        // A person changing the goal keeps the count the rule had reached.
        rule.goal = "Fix it and add a regression test".into();
        rule.countdown = 0;
        s.upsert_intake_rule(&rule).unwrap();
        assert_eq!(s.list_intake_rules(None).unwrap()[0].countdown, 0, "the count is preserved from the edit");
    }

    #[test]
    fn a_mandate_says_in_words_who_allowed_the_work() {
        let m = Mandate::new("schedule", "human:http", "run-1", "implementer", &[], "v1:abc");
        assert!(m.sentence().starts_with("a schedule set up by http"));
        let m = Mandate::new("inferred", "agent:devd-team", "run-1", "qa", &[], "v1:abc");
        assert!(m.sentence().contains("a person accepted it"));
    }
}
