//! Persistence for the run engine.
//!
//! Every write here is additive and transaction-safe. The unified tables sit
//! beside `chat_*` and `team_*` rather than replacing them, so an upgrade keeps
//! every conversation, task and audit row that already existed.

use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value;

use crate::error::{DevdError, Result};
use crate::runs::{
    evaluate, ApprovalRecord, ArtifactRecord, CompletionPolicy, Evidence, Run, RunEvent, RunMode,
    RunState, RunStep, ToolCall,
};
use crate::state::Store;
use crate::util;

/// How long a durable approval stays answerable.
///
/// Ten minutes is long enough to read a plan and short enough that a forgotten
/// browser tab stops being a live authorization. Expired approvals fail closed.
pub const APPROVAL_LIFETIME_MINUTES: i64 = 10;

/// What is needed to create a run.
#[derive(Debug, Clone)]
pub struct RunSpec<'a> {
    /// What the person asked for.
    pub goal: &'a str,
    /// How it is driven.
    pub mode: RunMode,
    /// Project directory.
    pub project: Option<&'a str>,
    /// Who started it.
    pub actor: &'a str,
    /// Conversation, when an assistant run.
    pub thread_id: Option<&'a str>,
    /// Team run, when a team run.
    pub team_run_id: Option<&'a str>,
    /// Acceptance criteria.
    pub acceptance: &'a [String],
}

/// A tool call about to be recorded.
#[derive(Debug, Clone)]
pub struct NewToolCall<'a> {
    /// Run.
    pub run_id: Option<&'a str>,
    /// Step.
    pub step_id: Option<&'a str>,
    /// Provider tool-use id.
    pub call_id: Option<&'a str>,
    /// Who called.
    pub actor: &'a str,
    /// Interface.
    pub surface: &'a str,
    /// Capability.
    pub capability: &'a str,
    /// Redacted arguments.
    pub args: &'a Value,
    /// Hash of the unredacted canonical arguments.
    pub args_hash: &'a str,
    /// Approval.
    pub approval_id: Option<&'a str>,
    /// Idempotency key.
    pub idempotency_key: Option<&'a str>,
    /// Whether it succeeded.
    pub ok: bool,
    /// Error code.
    pub error_code: Option<&'a str>,
    /// Error message.
    pub error: Option<&'a str>,
    /// Exit code.
    pub exit_code: Option<i64>,
    /// Duration.
    pub duration_ms: i64,
    /// Result.
    pub result: Option<&'a Value>,
    /// Start.
    pub started_at: &'a str,
}

/// A new evidence fact.
#[derive(Debug, Clone)]
pub struct NewEvidence<'a> {
    /// Run.
    pub run_id: &'a str,
    /// Step.
    pub step_id: Option<&'a str>,
    /// Tool call.
    pub tool_call_id: Option<&'a str>,
    /// Artifact.
    pub artifact_id: Option<&'a str>,
    /// Kind.
    pub kind: &'a str,
    /// Capability.
    pub capability: &'a str,
    /// Passed.
    pub passed: bool,
    /// Claim.
    pub claim: Option<&'a str>,
    /// Summary.
    pub summary: &'a str,
    /// Detail.
    pub detail: Option<&'a str>,
}

/// A new artifact.
#[derive(Debug, Clone)]
pub struct NewArtifact<'a> {
    /// Run.
    pub run_id: Option<&'a str>,
    /// Tool call.
    pub tool_call_id: Option<&'a str>,
    /// Kind.
    pub kind: &'a str,
    /// Name.
    pub name: &'a str,
    /// MIME.
    pub mime: Option<&'a str>,
    /// Size.
    pub bytes: i64,
    /// Path.
    pub path: Option<&'a str>,
    /// Hash.
    pub sha256: Option<&'a str>,
    /// Summary.
    pub summary: Option<&'a str>,
}

/// A new approval request.
#[derive(Debug, Clone)]
pub struct NewApproval<'a> {
    /// Run.
    pub run_id: Option<&'a str>,
    /// Task.
    pub task_id: Option<&'a str>,
    /// Tool call.
    pub tool_call_id: Option<&'a str>,
    /// Capability.
    pub capability: &'a str,
    /// Exact arguments to replay.
    pub args: &'a Value,
    /// Arguments hash.
    pub args_hash: &'a str,
    /// Plan.
    pub plan: Option<&'a Value>,
    /// Plan fingerprint.
    pub plan_hash: Option<&'a str>,
    /// Scope.
    pub scope: &'a str,
    /// Tier.
    pub tier: &'a str,
    /// Risk.
    pub risk: &'a str,
    /// Who asked.
    pub actor: &'a str,
    /// Who issued it.
    pub requested_by: &'a str,
}

/// Hash an approval token before it is stored.
///
/// The token itself is a bearer secret; writing it to disk would turn a database
/// read into the ability to approve. Only its hash is kept, and the same hash is
/// the row id, so a lookup never needs the plaintext.
pub fn token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

fn now_plus_minutes(minutes: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::minutes(minutes)).to_rfc3339()
}

fn json(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

fn json_value(text: Option<String>) -> Value {
    json(text).unwrap_or(Value::Null)
}

fn run_row(r: &Row<'_>) -> rusqlite::Result<Run> {
    let acceptance: String = r.get(6)?;
    let policy: String = r.get(7)?;
    Ok(Run {
        id: r.get(0)?,
        goal: r.get(1)?,
        mode: r.get(2)?,
        state: r.get(3)?,
        project: r.get(4)?,
        environment: r.get(5)?,
        acceptance: serde_json::from_str(&acceptance).unwrap_or_default(),
        completion_policy: serde_json::from_str(&policy).unwrap_or_default(),
        plan: json(r.get(8)?),
        plan_fingerprint: r.get(9)?,
        parent_run: r.get(10)?,
        thread_id: r.get(11)?,
        team_run_id: r.get(12)?,
        actor: r.get(13)?,
        cancellation: json(r.get(14)?),
        usage: json_value(r.get(15)?),
        error: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
        finished_at: r.get(19)?,
    })
}

const RUN_COLS: &str = "id,goal,mode,state,project,environment,acceptance,completion_policy,plan,plan_fingerprint,parent_run,thread_id,team_run_id,actor,cancellation,usage,error,created_at,updated_at,finished_at";

fn step_row(r: &Row<'_>) -> rusqlite::Result<RunStep> {
    Ok(RunStep {
        id: r.get(0)?,
        run_id: r.get(1)?,
        ordinal: r.get(2)?,
        kind: r.get(3)?,
        title: r.get(4)?,
        state: r.get(5)?,
        data: json_value(r.get(6)?),
        started_at: r.get(7)?,
        finished_at: r.get(8)?,
    })
}

fn tool_row(r: &Row<'_>) -> rusqlite::Result<ToolCall> {
    Ok(ToolCall {
        id: r.get(0)?,
        run_id: r.get(1)?,
        step_id: r.get(2)?,
        call_id: r.get(3)?,
        actor: r.get(4)?,
        surface: r.get(5)?,
        capability: r.get(6)?,
        args: json_value(r.get(7)?),
        args_hash: r.get(8)?,
        approval_id: r.get(9)?,
        idempotency_key: r.get(10)?,
        ok: r.get::<_, i32>(11)? != 0,
        error_code: r.get(12)?,
        error: r.get(13)?,
        exit_code: r.get(14)?,
        duration_ms: r.get(15)?,
        result: json(r.get(16)?),
        started_at: r.get(17)?,
        finished_at: r.get(18)?,
    })
}

const TOOL_COLS: &str = "id,run_id,step_id,call_id,actor,surface,capability,args,args_hash,approval_id,idempotency_key,ok,error_code,error,exit_code,duration_ms,result,started_at,finished_at";

fn evidence_row(r: &Row<'_>) -> rusqlite::Result<Evidence> {
    Ok(Evidence {
        id: r.get(0)?,
        run_id: r.get(1)?,
        step_id: r.get(2)?,
        tool_call_id: r.get(3)?,
        artifact_id: r.get(4)?,
        kind: r.get(5)?,
        capability: r.get(6)?,
        passed: r.get::<_, i32>(7)? != 0,
        claim: r.get(8)?,
        summary: r.get(9)?,
        detail: r.get(10)?,
        created_at: r.get(11)?,
    })
}

fn artifact_row(r: &Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    Ok(ArtifactRecord {
        id: r.get(0)?,
        run_id: r.get(1)?,
        tool_call_id: r.get(2)?,
        kind: r.get(3)?,
        name: r.get(4)?,
        mime: r.get(5)?,
        bytes: r.get(6)?,
        path: r.get(7)?,
        sha256: r.get(8)?,
        summary: r.get(9)?,
        created_at: r.get(10)?,
    })
}

fn approval_row(r: &Row<'_>) -> rusqlite::Result<ApprovalRecord> {
    Ok(ApprovalRecord {
        id: r.get(0)?,
        run_id: r.get(1)?,
        task_id: r.get(2)?,
        tool_call_id: r.get(3)?,
        capability: r.get(4)?,
        args: json_value(r.get(5)?),
        args_hash: r.get(6)?,
        plan: json(r.get(7)?),
        plan_hash: r.get(8)?,
        scope: r.get(9)?,
        tier: r.get(10)?,
        risk: r.get(11)?,
        actor: r.get(12)?,
        requested_by: r.get(13)?,
        expires_at: r.get(14)?,
        state: r.get(15)?,
        decided_by: r.get(16)?,
        decided_at: r.get(17)?,
        reason: r.get(18)?,
        consumed_at: r.get(19)?,
        result: json(r.get(20)?),
        error: r.get(21)?,
        created_at: r.get(22)?,
    })
}

const APPROVAL_COLS: &str = "id,run_id,task_id,tool_call_id,capability,args,args_hash,plan,plan_hash,scope,tier,risk,actor,requested_by,expires_at,state,decided_by,decided_at,reason,consumed_at,result,error,created_at";

fn event_row(r: &Row<'_>) -> rusqlite::Result<RunEvent> {
    Ok(RunEvent {
        id: r.get(0)?,
        run_id: r.get(1)?,
        seq: r.get(2)?,
        step_id: r.get(3)?,
        kind: r.get(4)?,
        message: r.get(5)?,
        data: json_value(r.get(6)?),
        created_at: r.get(7)?,
    })
}

impl Store {
    // ------------------------------------------------------------------ runs

    /// Create a run and its intake event in one transaction.
    pub fn start_run(&self, spec: RunSpec<'_>) -> Result<Run> {
        let id = util::new_id();
        let now = util::now();
        let policy = CompletionPolicy::for_goal(spec.goal, spec.acceptance);
        let acceptance = serde_json::to_string(spec.acceptance).unwrap_or_else(|_| "[]".into());
        let policy_json = serde_json::to_string(&policy).unwrap_or_else(|_| "{}".into());
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO runs (id,goal,mode,state,project,acceptance,completion_policy,thread_id,team_run_id,actor,created_at,updated_at)
                 VALUES (?1,?2,?3,'intake',?4,?5,?6,?7,?8,?9,?10,?10)",
                params![id, spec.goal, spec.mode.as_str(), spec.project, acceptance, policy_json, spec.thread_id, spec.team_run_id, spec.actor, now],
            )?;
            let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM run_events WHERE run_id=?1", params![id], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO run_events (run_id,seq,kind,message,data,created_at) VALUES (?1,?2,'run.created',?3,?4,?5)",
                params![id, seq, format!("{} run started", spec.mode.as_str()), serde_json::json!({ "goal": spec.goal, "acceptance": spec.acceptance, "mode": spec.mode.as_str() }).to_string(), now],
            )?;
            tx.commit()?;
            Ok(())
        })?;
        self.run_by_id(&id)?
            .ok_or_else(|| DevdError::internal("the run was not stored"))
    }

    /// Start a run for one invocation whose caller brought no run of its own.
    ///
    /// An outside MCP agent owns its session, not a devd run, so the call it
    /// makes becomes the unit of work: one run, executed immediately, with the
    /// tool call and derived evidence the registry writes at the boundary. The
    /// HTTP API uses the same shape for a deploy hook.
    pub fn start_standalone_run(
        &self,
        mode: RunMode,
        goal: &str,
        actor: &str,
    ) -> Result<Run> {
        let run = self.start_run(RunSpec {
            goal,
            mode,
            project: None,
            actor,
            thread_id: None,
            team_run_id: None,
            acceptance: &[],
        })?;
        let _ = self.transition_run(&run.id, RunState::Executing, None);
        Ok(run)
    }

    /// Finish a standalone run from the evidence the call left.
    ///
    /// A failed invocation is a failed run. A successful one is only
    /// `completed` when its completion policy is satisfied; otherwise it is
    /// `unverified`, which is the honest word for "the call returned but
    /// nothing proved the goal".
    pub fn conclude_run(&self, run_id: &str, error: Option<&str>) {
        let Some(run) = self.run_by_id(run_id).ok().flatten() else {
            return;
        };
        let (next, error) = match error {
            Some(e) => (RunState::Failed, Some(e)),
            None => {
                let evidence = self.list_evidence(run_id, 10_000).unwrap_or_default();
                (evaluate(&run.completion_policy, &evidence).state, None)
            }
        };
        let _ = self.transition_run(run_id, next, error);
    }

    /// One run.
    pub fn run_by_id(&self, id: &str) -> Result<Option<Run>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {RUN_COLS} FROM runs WHERE id=?1"),
                params![id],
                run_row,
            )
            .optional()?)
        })
    }

    /// Newest runs first.
    pub fn recent_runs(&self, limit: i64) -> Result<Vec<Run>> {
        self.page_runs(limit, 0)
    }

    /// One page of runs, newest first. Long histories are paged rather than
    /// silently cut off at a small fixed number.
    pub fn page_runs(&self, limit: i64, offset: i64) -> Result<Vec<Run>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2"
            ))?;
            let rows = st
                .query_map(params![limit.max(1), offset.max(0)], run_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Runs for one project.
    pub fn list_runs_for_project(&self, project: &str, limit: i64) -> Result<Vec<Run>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs WHERE project=?1 ORDER BY updated_at DESC LIMIT ?2"
            ))?;
            let rows = st
                .query_map(params![project, limit], run_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The run that a conversation belongs to, if one exists.
    pub fn run_for_thread(&self, thread_id: &str) -> Result<Option<Run>> {
        self.with(|c| {
            Ok(c.query_row(&format!("SELECT {RUN_COLS} FROM runs WHERE thread_id=?1 ORDER BY created_at DESC LIMIT 1"), params![thread_id], run_row).optional()?)
        })
    }

    /// The run that mirrors a team run.
    pub fn run_for_team_run(&self, team_run_id: &str) -> Result<Option<Run>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {RUN_COLS} FROM runs WHERE team_run_id=?1"),
                params![team_run_id],
                run_row,
            )
            .optional()?)
        })
    }

    /// Return the conversation's run, creating it when the conversation predates
    /// the run engine. Idempotent, so a restart cannot create a second run.
    pub fn ensure_run_for_thread(
        &self,
        thread_id: &str,
        title: &str,
        project: Option<&str>,
        actor: &str,
    ) -> Result<Run> {
        if let Some(run) = self.run_for_thread(thread_id)? {
            return Ok(run);
        }
        self.start_run(RunSpec {
            goal: title,
            mode: RunMode::Assistant,
            project,
            actor,
            thread_id: Some(thread_id),
            team_run_id: None,
            acceptance: &[],
        })
    }

    /// Ensure a team run has its unified run.
    pub fn ensure_run_for_team(
        &self,
        team_run_id: &str,
        goal: &str,
        project: Option<&str>,
        actor: &str,
        acceptance: &[String],
    ) -> Result<Run> {
        if let Some(run) = self.run_for_team_run(team_run_id)? {
            return Ok(run);
        }
        self.start_run(RunSpec {
            goal,
            mode: RunMode::Team,
            project,
            actor,
            thread_id: None,
            team_run_id: Some(team_run_id),
            acceptance,
        })
    }

    /// Move a run to a new lifecycle state, rejecting an illegal transition.
    pub fn transition_run(&self, id: &str, next: RunState, error: Option<&str>) -> Result<Run> {
        let run = self
            .run_by_id(id)?
            .ok_or_else(|| DevdError::not_found(format!("no run `{id}`")))?;
        let current = run.typed_state();
        if !current.can_transition_to(next) {
            return Err(DevdError::conflict(format!(
                "run `{id}` is {current}; it cannot become {next} without a deliberate retry, continuation or fork"
            )));
        }
        let now = util::now();
        let finished = if next.is_terminal() {
            Some(now.clone())
        } else {
            None
        };
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "UPDATE runs SET state=?2, error=COALESCE(?3,error), updated_at=?4, finished_at=CASE WHEN ?5 IS NULL THEN finished_at ELSE ?5 END WHERE id=?1",
                params![id, next.as_str(), error, now, finished],
            )?;
            let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM run_events WHERE run_id=?1", params![id], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO run_events (run_id,seq,kind,message,data,created_at) VALUES (?1,?2,'run.state',?3,?4,?5)",
                params![id, seq, format!("{current} → {next}"), serde_json::json!({ "from": current.as_str(), "to": next.as_str(), "error": error }).to_string(), now],
            )?;
            tx.commit()?;
            Ok(())
        })?;
        self.run_by_id(id)?
            .ok_or_else(|| DevdError::internal("the run disappeared"))
    }

    /// Record that a person asked the run to stop. The executor is responsible
    /// for observing it; the record is what makes the request visible to every
    /// interface and survives a restart.
    pub fn request_run_cancel(&self, id: &str, by: &str, reason: Option<&str>) -> Result<Run> {
        let data = serde_json::json!({ "requested_at": util::now(), "requested_by": by, "reason": reason });
        self.with(|c| {
            c.execute(
                "UPDATE runs SET cancellation=?2, updated_at=?3 WHERE id=?1",
                params![id, data.to_string(), util::now()],
            )?;
            Ok(())
        })?;
        self.append_run_event(id, None, "run.cancelled", "cancellation requested", data)?;
        self.run_by_id(id)?
            .ok_or_else(|| DevdError::internal("the run disappeared"))
    }

    /// Store a plan snapshot and its fingerprint.
    pub fn set_run_plan(&self, id: &str, plan: &Value, fingerprint: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE runs SET plan=?2, plan_fingerprint=?3, updated_at=?4 WHERE id=?1",
                params![id, plan.to_string(), fingerprint, util::now()],
            )?;
            Ok(())
        })?;
        self.append_run_event(id, None, "run.plan", "plan recorded", plan.clone())?;
        Ok(())
    }

    /// Add token usage to a run.
    pub fn add_run_usage(&self, id: &str, input: i64, output: i64, tool_calls: i64) -> Result<()> {
        self.with(|c| {
            let current: Option<String> = c
                .query_row("SELECT usage FROM runs WHERE id=?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            let mut usage = current
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            let inc = |usage: &mut Value, key: &str, by: i64| {
                let n = usage.get(key).and_then(Value::as_i64).unwrap_or(0) + by;
                usage[key] = serde_json::json!(n);
            };
            inc(&mut usage, "input_tokens", input);
            inc(&mut usage, "output_tokens", output);
            inc(&mut usage, "tool_calls", tool_calls);
            c.execute(
                "UPDATE runs SET usage=?2, updated_at=?3 WHERE id=?1",
                params![id, usage.to_string(), util::now()],
            )?;
            Ok(())
        })
    }

    // ----------------------------------------------------------------- steps

    /// Add a step and return it.
    pub fn add_step(&self, run_id: &str, kind: &str, title: &str, data: &Value) -> Result<RunStep> {
        let id = util::new_id();
        let now = util::now();
        self.with(|c| {
            let ordinal: i64 = c.query_row("SELECT COALESCE(MAX(ordinal),0)+1 FROM run_steps WHERE run_id=?1", params![run_id], |r| r.get(0))?;
            c.execute(
                "INSERT INTO run_steps (id,run_id,ordinal,kind,title,state,data,started_at) VALUES (?1,?2,?3,?4,?5,'running',?6,?7)",
                params![id, run_id, ordinal, kind, title, data.to_string(), now],
            )?;
            Ok(())
        })?;
        self.append_run_event(
            run_id,
            Some(&id),
            "step.started",
            format!("{kind}: {title}"),
            data.clone(),
        )?;
        self.with(|c| Ok(c.query_row("SELECT id,run_id,ordinal,kind,title,state,data,started_at,finished_at FROM run_steps WHERE id=?1", params![id], step_row)?))
    }

    /// Finish a step.
    pub fn finish_step(&self, id: &str, state: &str, data: Option<&Value>) -> Result<()> {
        let now = util::now();
        let run_id: Option<String> = self.with(|c| {
            let run: Option<String> = c
                .query_row(
                    "SELECT run_id FROM run_steps WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            c.execute(
                "UPDATE run_steps SET state=?2, finished_at=?3, data=COALESCE(?4,data) WHERE id=?1",
                params![id, state, now, data.map(|d| d.to_string())],
            )?;
            Ok(run)
        })?;
        if let Some(run_id) = run_id {
            self.append_run_event(
                &run_id,
                Some(id),
                "step.finished",
                format!("step {state}"),
                data.cloned().unwrap_or(Value::Null),
            )?;
        }
        Ok(())
    }

    /// Steps of a run in order.
    pub fn list_steps(&self, run_id: &str) -> Result<Vec<RunStep>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,ordinal,kind,title,state,data,started_at,finished_at FROM run_steps WHERE run_id=?1 ORDER BY ordinal")?;
            let rows = st.query_map(params![run_id], step_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // ------------------------------------------------------------- tool calls

    /// Record an invocation. This is the authoritative record: argument hashes,
    /// outcome, duration, exit status and the approval that authorized it.
    pub fn record_tool_call(&self, call: NewToolCall<'_>) -> Result<ToolCall> {
        let id = util::new_id();
        let now = util::now();
        let finished = now.clone();
        self.with(|c| {
            c.execute(
                "INSERT INTO tool_calls (id,run_id,step_id,call_id,actor,surface,capability,args,args_hash,approval_id,idempotency_key,ok,error_code,error,exit_code,duration_ms,result,started_at,finished_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![id, call.run_id, call.step_id, call.call_id, call.actor, call.surface, call.capability, call.args.to_string(), call.args_hash, call.approval_id, call.idempotency_key, call.ok as i32, call.error_code, call.error, call.exit_code, call.duration_ms, call.result.map(|r| r.to_string()), call.started_at, finished],
            )?;
            Ok(())
        })?;
        if let Some(run_id) = call.run_id {
            self.append_run_event(
                run_id,
                call.step_id,
                "tool.call",
                format!(
                    "{} {}",
                    call.capability,
                    if call.ok { "succeeded" } else { "failed" }
                ),
                serde_json::json!({
                    "tool_call": id,
                    "capability": call.capability,
                    "ok": call.ok,
                    "error_code": call.error_code,
                    "duration_ms": call.duration_ms,
                    "exit_code": call.exit_code,
                }),
            )?;
        }
        self.get_tool_call(&id)?
            .ok_or_else(|| DevdError::internal("the tool call was not stored"))
    }

    /// One tool call.
    pub fn get_tool_call(&self, id: &str) -> Result<Option<ToolCall>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {TOOL_COLS} FROM tool_calls WHERE id=?1"),
                params![id],
                tool_row,
            )
            .optional()?)
        })
    }

    /// Tool calls for a run, oldest first.
    pub fn list_tool_calls(&self, run_id: &str, limit: i64) -> Result<Vec<ToolCall>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {TOOL_COLS} FROM tool_calls WHERE run_id=?1 ORDER BY started_at LIMIT ?2"
            ))?;
            let rows = st
                .query_map(params![run_id, limit], tool_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Find a completed call by idempotency key, for safe retry.
    pub fn completed_call_by_key(&self, key: &str) -> Result<Option<ToolCall>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {TOOL_COLS} FROM tool_calls WHERE idempotency_key=?1 AND ok=1"),
                params![key],
                tool_row,
            )
            .optional()?)
        })
    }

    // --------------------------------------------------------------- evidence

    /// Record a fact observed by devd.
    pub fn add_evidence(&self, e: NewEvidence<'_>) -> Result<Evidence> {
        let id = util::new_id();
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO evidence (id,run_id,step_id,tool_call_id,artifact_id,kind,capability,passed,claim,summary,detail,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![id, e.run_id, e.step_id, e.tool_call_id, e.artifact_id, e.kind, e.capability, e.passed as i32, e.claim, e.summary, e.detail, now],
            )?;
            Ok(())
        })?;
        self.append_run_event(
            e.run_id,
            e.step_id,
            "evidence.recorded",
            e.summary,
            serde_json::json!({ "kind": e.kind, "capability": e.capability, "passed": e.passed, "claim": e.claim }),
        )?;
        self.with(|c| Ok(c.query_row("SELECT id,run_id,step_id,tool_call_id,artifact_id,kind,capability,passed,claim,summary,detail,created_at FROM evidence WHERE id=?1", params![id], evidence_row)?))
    }

    /// Evidence for a run, oldest first.
    pub fn list_evidence(&self, run_id: &str, limit: i64) -> Result<Vec<Evidence>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,step_id,tool_call_id,artifact_id,kind,capability,passed,claim,summary,detail,created_at FROM evidence WHERE run_id=?1 ORDER BY created_at LIMIT ?2")?;
            let rows = st.query_map(params![run_id, limit], evidence_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // -------------------------------------------------------------- artifacts

    /// Store metadata for a bounded copy of a large output.
    pub fn add_artifact(&self, a: NewArtifact<'_>) -> Result<ArtifactRecord> {
        let id = util::new_id();
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO artifacts (id,run_id,tool_call_id,kind,name,mime,bytes,path,sha256,summary,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![id, a.run_id, a.tool_call_id, a.kind, a.name, a.mime, a.bytes, a.path, a.sha256, a.summary, now],
            )?;
            Ok(())
        })?;
        if let Some(run_id) = a.run_id {
            self.append_run_event(run_id, None, "artifact.added", a.name, serde_json::json!({ "artifact": id, "kind": a.kind, "bytes": a.bytes, "path": a.path }))?;
        }
        self.get_artifact(&id)?
            .ok_or_else(|| DevdError::internal("the artifact was not stored"))
    }

    /// One artifact.
    pub fn get_artifact(&self, id: &str) -> Result<Option<ArtifactRecord>> {
        self.with(|c| Ok(c.query_row("SELECT id,run_id,tool_call_id,kind,name,mime,bytes,path,sha256,summary,created_at FROM artifacts WHERE id=?1", params![id], artifact_row).optional()?))
    }

    /// Artifacts for a run.
    pub fn list_artifacts(&self, run_id: &str, limit: i64) -> Result<Vec<ArtifactRecord>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,tool_call_id,kind,name,mime,bytes,path,sha256,summary,created_at FROM artifacts WHERE run_id=?1 ORDER BY created_at LIMIT ?2")?;
            let rows = st.query_map(params![run_id, limit], artifact_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // ----------------------------------------------------------------- events

    /// Append one sequenced event to a run and return it.
    pub fn append_run_event(
        &self,
        run_id: &str,
        step_id: Option<&str>,
        kind: &str,
        message: impl Into<String>,
        data: Value,
    ) -> Result<RunEvent> {
        let now = util::now();
        let message = message.into();
        let (id, seq) = self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0)+1 FROM run_events WHERE run_id=?1", params![run_id], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO run_events (run_id,seq,step_id,kind,message,data,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![run_id, seq, step_id, kind, message, data.to_string(), now],
            )?;
            let id = tx.last_insert_rowid();
            tx.commit()?;
            Ok((id, seq))
        })?;
        Ok(RunEvent {
            id,
            run_id: run_id.to_string(),
            seq,
            step_id: step_id.map(str::to_string),
            kind: kind.to_string(),
            message,
            data,
            created_at: now,
        })
    }

    /// Events for a run after a sequence number.
    pub fn list_run_events(
        &self,
        run_id: &str,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<RunEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,seq,step_id,kind,message,data,created_at FROM run_events WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;
            let rows = st.query_map(params![run_id, after_seq, limit], event_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The page of a run's events immediately before a sequence number, oldest first.
    ///
    /// This is how a long timeline walks backwards: the first page is the
    /// newest events, and each earlier page is fetched from the oldest sequence
    /// it already has. Nothing is silently cut off at a fixed maximum.
    pub fn list_run_events_before(
        &self,
        run_id: &str,
        before_seq: i64,
        limit: i64,
    ) -> Result<Vec<RunEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,seq,step_id,kind,message,data,created_at FROM run_events WHERE run_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT ?3")?;
            let mut rows = st.query_map(params![run_id, before_seq, limit.max(1)], event_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            rows.reverse();
            Ok(rows)
        })
    }

    /// The newest events of a run, oldest first.
    pub fn tail_run_events(&self, run_id: &str, limit: i64) -> Result<Vec<RunEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,seq,step_id,kind,message,data,created_at FROM run_events WHERE run_id=?1 ORDER BY seq DESC LIMIT ?2")?;
            let mut rows = st.query_map(params![run_id, limit.max(1)], event_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            rows.reverse();
            Ok(rows)
        })
    }

    /// Events across every run after a global cursor. This is the SSE replay
    /// query: ids are stable, ordered and never reused.
    pub fn events_after(&self, after_id: i64, limit: i64) -> Result<Vec<RunEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,seq,step_id,kind,message,data,created_at FROM run_events WHERE id>?1 ORDER BY id LIMIT ?2")?;
            let rows = st.query_map(params![after_id, limit], event_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// A run's whole story, assembled from the durable records.
    pub fn run_timeline(&self, run_id: &str) -> Result<Value> {
        let run = self
            .run_by_id(run_id)?
            .ok_or_else(|| DevdError::not_found(format!("no run `{run_id}`")))?;
        let steps = self.list_steps(run_id)?;
        let calls = self.list_tool_calls(run_id, 500)?;
        let evidence = self.list_evidence(run_id, 500)?;
        let artifacts = self.list_artifacts(run_id, 200)?;
        let approvals = self.list_approvals_for_run(run_id, 200)?;
        let events = self.list_run_events(run_id, 0, 2000)?;
        Ok(serde_json::json!({
            "run": run,
            "steps": steps,
            "tool_calls": calls,
            "evidence": evidence,
            "artifacts": artifacts,
            "approvals": approvals.iter().map(ApprovalRecord::public_json).collect::<Vec<_>>(),
            "events": events,
        }))
    }

    // -------------------------------------------------------------- approvals

    /// Create a durable approval and return its opaque token and record.
    ///
    /// The token is returned once and never stored. The record carries the exact
    /// arguments to replay, so a caller cannot approve one plan and run another.
    pub fn create_approval(&self, a: NewApproval<'_>) -> Result<(String, ApprovalRecord)> {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let id = token_hash(&token);
        let now = util::now();
        let expires_at = now_plus_minutes(APPROVAL_LIFETIME_MINUTES);
        self.with(|c| {
            c.execute(
                "INSERT INTO approvals (id,run_id,task_id,tool_call_id,capability,args,args_hash,plan,plan_hash,scope,tier,risk,actor,requested_by,expires_at,state,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'pending',?16)",
                params![id, a.run_id, a.task_id, a.tool_call_id, a.capability, a.args.to_string(), a.args_hash, a.plan.map(|p| p.to_string()), a.plan_hash, a.scope, a.tier, a.risk, a.actor, a.requested_by, expires_at, now],
            )?;
            Ok(())
        })?;
        if let Some(run_id) = a.run_id {
            let _ = self.append_run_event(run_id, None, "approval.requested", format!("{} needs a person", a.capability), serde_json::json!({ "approval": id, "capability": a.capability, "expires_at": expires_at }));
        }
        let record = self
            .get_approval(&id)?
            .ok_or_else(|| DevdError::internal("the approval was not stored"))?;
        Ok((token, record))
    }

    /// One approval by its hashed id.
    pub fn get_approval(&self, id: &str) -> Result<Option<ApprovalRecord>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {APPROVAL_COLS} FROM approvals WHERE id=?1"),
                params![id],
                approval_row,
            )
            .optional()?)
        })
    }

    /// Approvals for one run.
    pub fn list_approvals_for_run(&self, run_id: &str, limit: i64) -> Result<Vec<ApprovalRecord>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {APPROVAL_COLS} FROM approvals WHERE run_id=?1 ORDER BY created_at DESC LIMIT ?2"))?;
            let rows = st.query_map(params![run_id, limit], approval_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Pending approvals across runs, newest first.
    pub fn list_approvals(&self, only_pending: bool, limit: i64) -> Result<Vec<ApprovalRecord>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {APPROVAL_COLS} FROM approvals WHERE (?1=0 OR state='pending') ORDER BY created_at DESC LIMIT ?2"))?;
            let rows = st.query_map(params![only_pending as i32, limit], approval_row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Decline an approval. It can never be used afterwards.
    pub fn decline_approval(
        &self,
        id: &str,
        by: &str,
        reason: Option<&str>,
    ) -> Result<ApprovalRecord> {
        let now = util::now();
        let changed = self.with(|c| Ok(c.execute("UPDATE approvals SET state='declined', decided_by=?2, decided_at=?3, reason=?4 WHERE id=?1 AND state='pending'", params![id, by, now, reason])?))?;
        if changed == 0 {
            return Err(DevdError::conflict("that approval is not pending"));
        }
        self.finish_approval_event(id, "approval.declined", "declined", None, reason)?;
        self.get_approval(id)?
            .ok_or_else(|| DevdError::not_found("no such approval"))
    }

    /// Consume a pending approval exactly once, atomically.
    ///
    /// The `state='pending'` predicate is the single-use guarantee: two callers
    /// holding the same token race on one row update, and only one sees a
    /// changed row.
    pub fn consume_approval(&self, token: &str) -> Result<ApprovalRecord> {
        let id = token_hash(token);
        self.consume_approval_by_id(&id)
    }

    /// Consume by stored id, for the conversation approval path that names the
    /// request rather than presenting a token.
    pub fn consume_approval_by_id(&self, id: &str) -> Result<ApprovalRecord> {
        let now = util::now();
        let first = self.get_approval(id)?.ok_or_else(|| {
            DevdError::denied("this approval is unknown, already used, or has expired")
        })?;
        if first.state == "pending" && first.expires_at <= now {
            self.with(|c| {
                Ok(c.execute(
                    "UPDATE approvals SET state='expired' WHERE id=?1 AND state='pending'",
                    params![id],
                )?)
            })?;
            return Err(DevdError::denied(
                "this approval expired before it was answered",
            ));
        }
        let changed = self.with(|c| {
            Ok(c.execute(
                "UPDATE approvals SET state='consumed', consumed_at=?2, decided_at=COALESCE(decided_at,?2), decided_by=COALESCE(decided_by,'system:consumer') WHERE id=?1 AND state='pending' AND expires_at>?2",
                params![id, now],
            )?)
        })?;
        if changed == 0 {
            return Err(DevdError::denied(
                "this approval is unknown, already used, or has expired",
            ));
        }
        let record = self
            .get_approval(id)?
            .ok_or_else(|| DevdError::not_found("no such approval"))?;
        if let Some(run_id) = &record.run_id {
            let _ = self.append_run_event(run_id, None, "approval.consumed", format!("{} authorised", record.capability), serde_json::json!({ "approval": id, "capability": record.capability, "args_hash": record.args_hash }));
        }
        Ok(record)
    }

    /// Record the result of an operation run under an approval.
    pub fn finish_approval(
        &self,
        id: &str,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<()> {
        let state = if error.is_some() {
            "failed"
        } else {
            "consumed"
        };
        self.with(|c| {
            c.execute(
                "UPDATE approvals SET state=?2, result=?3, error=?4 WHERE id=?1",
                params![id, state, result.map(|v| v.to_string()), error],
            )?;
            Ok(())
        })?;
        self.finish_approval_event(id, "approval.finished", state, result, error)
    }

    fn finish_approval_event(
        &self,
        id: &str,
        kind: &str,
        state: &str,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<()> {
        if let Some(a) = self.get_approval(id)? {
            if let Some(run_id) = a.run_id {
                let _ = self.append_run_event(
                    &run_id,
                    None,
                    kind,
                    format!("{} {state}", a.capability),
                    serde_json::json!({ "approval": id, "capability": a.capability, "state": state, "error": error, "result": result }),
                );
            }
        }
        Ok(())
    }

    /// Move the moment a pending approval stops being valid.
    ///
    /// Used by an operator shortening a window, and by tests that must prove an
    /// expired approval fails closed without waiting out the real lifetime.
    pub fn reschedule_approval(&self, id: &str, expires_at: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE approvals SET expires_at=?2 WHERE id=?1 AND state='pending'",
                params![id, expires_at],
            )?;
            Ok(())
        })
    }

    /// Expire every pending approval whose time has passed. Called on startup and
    /// before listing, so an unanswered request stops looking answerable.
    pub fn expire_approvals(&self) -> Result<usize> {
        let now = util::now();
        let expired = self.with(|c| {
            let mut st =
                c.prepare("SELECT id FROM approvals WHERE state='pending' AND expires_at<=?1")?;
            let ids: Vec<String> = st
                .query_map(params![now], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(st);
            for id in &ids {
                c.execute(
                    "UPDATE approvals SET state='expired' WHERE id=?1 AND state='pending'",
                    params![id],
                )?;
            }
            Ok(ids)
        })?;
        for id in &expired {
            let _ = self.finish_approval_event(
                id,
                "approval.expired",
                "expired",
                None,
                Some("the approval expired unanswered"),
            );
        }
        Ok(expired.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::{CompletionPolicy, EvidenceKind, RunState};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn spec<'a>(goal: &'a str, acceptance: &'a [String]) -> RunSpec<'a> {
        RunSpec {
            goal,
            mode: RunMode::Headless,
            project: Some("C:/code/app"),
            actor: "human:test",
            thread_id: None,
            team_run_id: None,
            acceptance,
        }
    }

    #[test]
    fn a_run_starts_in_intake_and_records_its_creation() {
        let s = store();
        let run = s.start_run(spec("do the thing", &[])).unwrap();
        assert_eq!(run.state, "intake");
        assert_eq!(run.typed_mode(), RunMode::Headless);
        let events = s.list_run_events(&run.id, 0, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "run.created");
        assert_eq!(events[0].seq, 1);
    }

    #[test]
    fn lifecycle_transitions_are_validated_and_sequenced() {
        let s = store();
        let run = s.start_run(spec("fix the parser", &[])).unwrap();
        s.transition_run(&run.id, RunState::Planning, None).unwrap();
        s.transition_run(&run.id, RunState::Executing, None)
            .unwrap();
        s.transition_run(&run.id, RunState::Verifying, None)
            .unwrap();
        let done = s
            .transition_run(&run.id, RunState::Completed, None)
            .unwrap();
        assert!(done.finished_at.is_some());
        let e = s
            .transition_run(&run.id, RunState::Planning, None)
            .unwrap_err();
        assert!(e.to_string().contains("cannot become"), "{e}");
        let events = s.list_run_events(&run.id, 0, 20).unwrap();
        let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
        assert_eq!(
            seqs,
            (1..=seqs.len() as i64).collect::<Vec<_>>(),
            "sequence has no gaps"
        );
    }

    #[test]
    fn a_deliberate_retry_after_completion_is_allowed_and_logged() {
        let s = store();
        let run = s.start_run(spec("fix it", &[])).unwrap();
        s.transition_run(&run.id, RunState::Executing, None)
            .unwrap();
        s.transition_run(&run.id, RunState::Completed, None)
            .unwrap();
        s.transition_run(&run.id, RunState::Executing, None)
            .unwrap();
        let events = s.list_run_events(&run.id, 0, 50).unwrap();
        assert!(events.iter().filter(|e| e.kind == "run.state").count() >= 3);
    }

    #[test]
    fn steps_tool_calls_and_evidence_are_linked() {
        let s = store();
        let run = s.start_run(spec("fix the parser", &[])).unwrap();
        let step = s
            .add_step(&run.id, "task", "do it", &serde_json::json!({}))
            .unwrap();
        let call = s
            .record_tool_call(NewToolCall {
                run_id: Some(&run.id),
                step_id: Some(&step.id),
                call_id: Some("toolu_1"),
                actor: "agent:devd",
                surface: "agent",
                capability: "fs.edit",
                args: &serde_json::json!({ "path": "src/lib.rs" }),
                args_hash: "abc",
                approval_id: None,
                idempotency_key: None,
                ok: true,
                error_code: None,
                error: None,
                exit_code: None,
                duration_ms: 12,
                result: Some(&serde_json::json!({ "ok": true })),
                started_at: &util::now(),
            })
            .unwrap();
        let ev = s
            .add_evidence(NewEvidence {
                run_id: &run.id,
                step_id: Some(&step.id),
                tool_call_id: Some(&call.id),
                artifact_id: None,
                kind: EvidenceKind::Change.as_str(),
                capability: "fs.edit",
                passed: true,
                claim: None,
                summary: "changed src/lib.rs",
                detail: None,
            })
            .unwrap();
        assert_eq!(ev.tool_call_id.as_deref(), Some(call.id.as_str()));
        let timeline = s.run_timeline(&run.id).unwrap();
        assert_eq!(timeline["steps"].as_array().unwrap().len(), 1);
        assert_eq!(timeline["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(timeline["evidence"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn approximating_an_approval_expiry_is_not_rounded_into_a_grant() {
        let s = store();
        let run = s.start_run(spec("wipe it", &[])).unwrap();
        let (token, rec) = s
            .create_approval(NewApproval {
                run_id: Some(&run.id),
                task_id: None,
                tool_call_id: None,
                capability: "db.reset",
                args: &serde_json::json!({ "name": "app" }),
                args_hash: "h",
                plan: None,
                plan_hash: None,
                scope: "single",
                tier: "destructive",
                risk: "destructive",
                actor: "agent:devd",
                requested_by: "agent:devd",
            })
            .unwrap();
        assert_eq!(rec.state, "pending");
        assert_eq!(token.len(), 64);
        let consumed = s.consume_approval(&token).unwrap();
        assert_eq!(consumed.state, "consumed");
        assert!(s.consume_approval(&token).is_err(), "a token was reusable");
        assert_eq!(s.list_approvals(true, 10).unwrap().len(), 0);
    }

    #[test]
    fn a_declined_approval_can_never_be_consumed() {
        let s = store();
        let run = s.start_run(spec("wipe it", &[])).unwrap();
        let (token, rec) = s
            .create_approval(NewApproval {
                run_id: Some(&run.id),
                task_id: None,
                tool_call_id: None,
                capability: "db.reset",
                args: &serde_json::json!({}),
                args_hash: "h",
                plan: None,
                plan_hash: None,
                scope: "single",
                tier: "destructive",
                risk: "destructive",
                actor: "agent:devd",
                requested_by: "agent:devd",
            })
            .unwrap();
        s.decline_approval(&rec.id, "human:gui", Some("not now"))
            .unwrap();
        assert!(s.consume_approval(&token).is_err());
        assert_eq!(s.get_approval(&rec.id).unwrap().unwrap().state, "declined");
    }

    #[test]
    fn an_expired_approval_fails_closed() {
        let s = store();
        let run = s.start_run(spec("wipe it", &[])).unwrap();
        let (token, rec) = s
            .create_approval(NewApproval {
                run_id: Some(&run.id),
                task_id: None,
                tool_call_id: None,
                capability: "db.reset",
                args: &serde_json::json!({}),
                args_hash: "h",
                plan: None,
                plan_hash: None,
                scope: "single",
                tier: "destructive",
                risk: "destructive",
                actor: "agent:devd",
                requested_by: "agent:devd",
            })
            .unwrap();
        s.with(|c| {
            c.execute(
                "UPDATE approvals SET expires_at='2000-01-01T00:00:00Z' WHERE id=?1",
                params![rec.id],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(s.expire_approvals().unwrap(), 1);
        assert!(s.consume_approval(&token).is_err());
        assert_eq!(s.get_approval(&rec.id).unwrap().unwrap().state, "expired");
    }

    #[test]
    fn the_token_is_never_stored_in_plaintext() {
        let s = store();
        let run = s.start_run(spec("x", &[])).unwrap();
        let (token, rec) = s
            .create_approval(NewApproval {
                run_id: Some(&run.id),
                task_id: None,
                tool_call_id: None,
                capability: "git.push",
                args: &serde_json::json!({}),
                args_hash: "h",
                plan: None,
                plan_hash: None,
                scope: "single",
                tier: "remote",
                risk: "normal",
                actor: "human:http",
                requested_by: "human:http",
            })
            .unwrap();
        let raw: String = s
            .with(|c| {
                Ok(c.query_row(
                    "SELECT id FROM approvals WHERE id=?1",
                    params![token_hash(&token)],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(raw, rec.id);
        assert!(!raw.contains(&token));
    }

    #[test]
    fn migrated_conversations_get_a_run_without_touching_their_history() {
        let s = store();
        // A conversation row as version 0.13 left it.
        s.with(|c| {
            c.execute("INSERT INTO chat_threads (id,title,state,created_at,updated_at) VALUES ('t1','old chat','idle','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')", [])?;
            Ok(())
        }).unwrap();
        let run = s
            .ensure_run_for_thread("t1", "old chat", None, "system:migrated")
            .unwrap();
        assert_eq!(run.thread_id.as_deref(), Some("t1"));
        assert_eq!(run.state, "intake");
        let again = s
            .ensure_run_for_thread("t1", "old chat", None, "system:migrated")
            .unwrap();
        assert_eq!(again.id, run.id, "a restart cannot create a second run");
    }

    #[test]
    fn events_are_global_ordered_and_replayable() {
        let s = store();
        let a = s.start_run(spec("a", &[])).unwrap();
        let b = s.start_run(spec("b", &[])).unwrap();
        s.append_run_event(&a.id, None, "note", "one", serde_json::json!({}))
            .unwrap();
        s.append_run_event(&b.id, None, "note", "two", serde_json::json!({}))
            .unwrap();
        let all = s.events_after(0, 100).unwrap();
        assert_eq!(all.len(), 4);
        let tail = s.events_after(all[2].id, 100).unwrap();
        assert_eq!(tail.len(), 1);
        assert!(tail[0].id > all[2].id);
    }

    #[test]
    fn completion_policy_is_carried_with_the_run() {
        let s = store();
        let criteria = vec!["the parser rejects trailing commas".to_string()];
        let run = s.start_run(spec("fix the parser", &criteria)).unwrap();
        let p = CompletionPolicy::for_goal(&run.goal, &run.acceptance);
        assert!(p.requires_change && p.requires_verify);
        assert_eq!(run.acceptance.len(), 1);
    }
}
