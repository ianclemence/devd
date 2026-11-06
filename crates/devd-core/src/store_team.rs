//! Persistence for provider health, role-based model assignments, decisions
//! about services that already exist, and the multi-agent team.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// A task to add to a run.
#[derive(Debug, Clone, Copy, Default)]
pub struct NewTask<'a> {
    /// Unique within the run.
    pub key: &'a str,
    /// The role that does it.
    pub role: &'a str,
    /// One line.
    pub title: &'a str,
    /// What to do.
    pub instructions: &'a str,
    /// Keys of the tasks it waits for.
    pub depends_on: &'a [String],
    /// Paths it may change.
    pub scope: &'a [String],
    /// Why it may run. Absent means the caller did not say, and the task records
    /// an empty mandate rather than an invented one.
    pub mandate: Option<&'a serde_json::Value>,
}

/// A change to a task. Fields left out keep their value, except `state` and `error`, which are always written.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskUpdate<'a> {
    /// The new state.
    pub state: &'a str,
    /// What it reported.
    pub result: Option<&'a str>,
    /// Why it is not going well.
    pub error: Option<&'a str>,
    /// Evidence gathered.
    pub evidence: Option<&'a serde_json::Value>,
    /// Where it works.
    pub workdir: Option<&'a str>,
    /// Its branch.
    pub branch: Option<&'a str>,
    /// Its conversation.
    pub thread_id: Option<&'a str>,
    /// Why it may run, when the caller is changing it.
    pub mandate: Option<&'a serde_json::Value>,
}

/// The last check of a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCheck {
    /// Provider id.
    pub provider_id: String,
    /// When it ran.
    pub checked_at: String,
    /// Whether the provider answered and accepted the key.
    pub ok: bool,
    /// What happened, in words.
    pub message: String,
    /// Models the provider listed.
    pub models: Option<i64>,
}

/// A model choice for a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleModel {
    /// Role: `default`, `planner`, `implementer`, ...
    pub role: String,
    /// `''` for everywhere, `project:<path>` for one project.
    pub scope: String,
    /// 0 is the first choice; later ones are fallbacks.
    pub position: i64,
    /// Provider id.
    pub provider_id: String,
    /// Model id.
    pub model: String,
}

/// A person's decision about a tool or service found on the machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDecision {
    /// Tool id.
    pub tool: String,
    /// `adopt`, `observe`, `external` or `ignore`.
    pub mode: String,
    /// Port of an instance the person runs themselves.
    pub port: Option<i64>,
    /// Why.
    pub note: Option<String>,
    /// Who decided.
    pub decided_by: String,
    /// When.
    pub decided_at: String,
}

/// A multi-agent run: one goal, worked by a team.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRun {
    /// Id.
    pub id: String,
    /// Short title.
    pub title: String,
    /// What the person asked for.
    pub goal: String,
    /// Project directory.
    pub project: Option<String>,
    /// `planning`, `awaiting_plan`, `running`, `paused`, `done`, `failed`, `cancelled`.
    pub state: String,
    /// Whether a person approved the plan.
    pub plan_approved: bool,
    /// What was delivered, once finished.
    pub summary: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
    /// Tasks that may run side by side.
    pub parallel: i64,
    /// The conversation that asked for this run, when one did.
    ///
    /// A run reports its result here rather than into the inbox: the person
    /// who asked is in a conversation, and that is where the answer belongs.
    pub thread_id: Option<String>,
    /// Who started it.
    pub created_by: String,
    /// Created.
    pub created_at: String,
    /// Updated.
    pub updated_at: String,
}

/// What a run must achieve, in the person's words, and how many extra passes devd may take to get there.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalLoop {
    /// Criteria the finished work must meet.
    pub done_when: Vec<String>,
    /// Extra passes made so far.
    pub rounds: i64,
    /// Most extra passes allowed.
    pub max_rounds: i64,
}

/// One unit of work for one specialist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTask {
    /// Id.
    pub id: String,
    /// Run.
    pub run_id: String,
    /// The planner's short name for it.
    pub key: String,
    /// Specialist role.
    pub role: String,
    /// Title.
    pub title: String,
    /// What to do and how to know it is done.
    pub instructions: String,
    /// Keys of tasks that must finish first.
    pub depends_on: Vec<String>,
    /// Paths it may change.
    pub scope: Vec<String>,
    /// State.
    pub state: String,
    /// Starts so far.
    pub attempts: i64,
    /// Where it works.
    pub workdir: Option<String>,
    /// Git branch of its worktree.
    pub branch: Option<String>,
    /// Conversation that does the work.
    pub thread_id: Option<String>,
    /// What the specialist reported.
    pub result: Option<String>,
    /// Facts devd recorded from the specialist's tool calls.
    pub evidence: serde_json::Value,
    /// Why it failed.
    pub error: Option<String>,
    /// Created.
    pub created_at: String,
    /// Started.
    pub started_at: Option<String>,
    /// Finished.
    pub finished_at: Option<String>,
    /// Why this work may run, and how far it may go. See
    /// [`crate::store_autonomy::Mandate`].
    #[serde(default)]
    pub mandate: serde_json::Value,
    /// The last time it did anything. `None` means it has not started. The stall
    /// watchdog compares it with now, so a worker that is alive but asleep is
    /// still found.
    #[serde(default)]
    pub last_step_at: Option<String>,
    /// How many times the stall watchdog intervened.
    #[serde(default)]
    pub stalls: i64,
}

/// A line in a run's log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamEvent {
    /// Sequence number.
    pub id: i64,
    /// Run.
    pub run_id: String,
    /// Task, when about one.
    pub task_id: Option<String>,
    /// Kind.
    pub kind: String,
    /// Payload.
    pub data: serde_json::Value,
    /// When.
    pub at: String,
}

const RUN_COLS: &str = "id,title,goal,project,state,plan_approved,summary,error,parallel,created_by,created_at,updated_at,thread_id";
/// The columns of one task row, for queries that live in other modules.
/// A conversation that a team specialist works in, and what it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadTeamLink {
    /// The specialist's conversation.
    pub thread_id: String,
    /// The team run.
    pub run_id: String,
    /// The specialist's role (`planner`, `implementer`, `reviewer`, `qa`, ...).
    pub role: String,
    /// The task's title.
    pub task: String,
    /// The task's state.
    pub task_state: String,
    /// The run's title.
    pub run_title: String,
    /// The person's conversation that started the run, if one did.
    pub parent_thread: Option<String>,
    /// The run's state.
    pub run_state: String,
}

pub(crate) const TASK_COLUMNS: &str =
    "id,run_id,key,role,title,instructions,depends_on,scope,state,attempts,workdir,branch,thread_id,result,evidence,error,created_at,started_at,finished_at,mandate,last_step_at,stalls";
const TASK_COLS: &str = TASK_COLUMNS;

fn row_run(r: &rusqlite::Row) -> rusqlite::Result<TeamRun> {
    Ok(TeamRun {
        id: r.get(0)?,
        title: r.get(1)?,
        goal: r.get(2)?,
        project: r.get(3)?,
        state: r.get(4)?,
        plan_approved: r.get::<_, i64>(5)? != 0,
        summary: r.get(6)?,
        error: r.get(7)?,
        parallel: r.get(8)?,
        thread_id: r.get(12).ok(),
        created_by: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn row_task(r: &rusqlite::Row) -> rusqlite::Result<TeamTask> {
    let list = |s: String| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default();
    let mandate: String = r.get(19)?;
    Ok(TeamTask {
        id: r.get(0)?,
        run_id: r.get(1)?,
        key: r.get(2)?,
        role: r.get(3)?,
        title: r.get(4)?,
        instructions: r.get(5)?,
        depends_on: list(r.get(6)?),
        scope: list(r.get(7)?),
        state: r.get(8)?,
        attempts: r.get(9)?,
        workdir: r.get(10)?,
        branch: r.get(11)?,
        thread_id: r.get(12)?,
        result: r.get(13)?,
        evidence: serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or(serde_json::json!([])),
        error: r.get(15)?,
        created_at: r.get(16)?,
        started_at: r.get(17)?,
        finished_at: r.get(18)?,
        mandate: serde_json::from_str(&mandate).unwrap_or(serde_json::Value::Null),
        last_step_at: r.get(20)?,
        stalls: r.get(21)?,
    })
}

/// The task reader, for queries in other modules that select [`TASK_COLUMNS`].
pub(crate) fn task_row(r: &rusqlite::Row) -> rusqlite::Result<TeamTask> {
    row_task(r)
}

impl Store {
    // ---------------------------------------------------------- environment

    /// `local` (the default) or `production`.
    pub fn environment_mode(&self) -> String {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT value FROM kv WHERE key='environment.mode'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .ok()
        .flatten()
        .filter(|m| m == "production")
        .unwrap_or_else(|| "local".to_string())
    }

    /// Switch between `local` and `production`.
    pub fn set_environment_mode(&self, mode: &str) -> Result<()> {
        self.with(|c| {
            c.execute("INSERT INTO kv (key,value) VALUES ('environment.mode',?1) ON CONFLICT(key) DO UPDATE SET value=?1", params![mode])?;
            Ok(())
        })
    }

    // ------------------------------------------------------- provider checks

    /// Record the outcome of testing a provider.
    pub fn set_provider_check(
        &self,
        provider_id: &str,
        ok: bool,
        message: &str,
        models: Option<i64>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO provider_checks (provider_id,checked_at,ok,message,models) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(provider_id) DO UPDATE SET checked_at=?2, ok=?3, message=?4, models=?5",
                params![provider_id, util::now(), ok as i32, message, models],
            )?;
            Ok(())
        })
    }

    /// The last check of a provider.
    pub fn get_provider_check(&self, provider_id: &str) -> Result<Option<ProviderCheck>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT provider_id,checked_at,ok,message,models FROM provider_checks WHERE provider_id=?1",
                params![provider_id],
                |r| Ok(ProviderCheck { provider_id: r.get(0)?, checked_at: r.get(1)?, ok: r.get::<_, i64>(2)? != 0, message: r.get(3)?, models: r.get(4)? }),
            )
            .optional()?)
        })
    }

    // ------------------------------------------------------------ model roles

    /// Replace the model choices for one role in one scope (first = preferred).
    pub fn set_role_models(
        &self,
        role: &str,
        scope: &str,
        models: &[(String, String)],
    ) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM model_roles WHERE role=?1 AND scope=?2", params![role, scope])?;
            for (i, (provider, model)) in models.iter().enumerate() {
                c.execute("INSERT INTO model_roles (role,scope,position,provider_id,model) VALUES (?1,?2,?3,?4,?5)", params![role, scope, i as i64, provider, model])?;
            }
            Ok(())
        })
    }

    /// Every role assignment.
    pub fn list_role_models(&self) -> Result<Vec<RoleModel>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT role,scope,position,provider_id,model FROM model_roles ORDER BY scope, role, position")?;
            let rows = st.query_map([], |r| Ok(RoleModel { role: r.get(0)?, scope: r.get(1)?, position: r.get(2)?, provider_id: r.get(3)?, model: r.get(4)? }))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Drop a provider from every role.
    pub fn purge_role_provider(&self, provider_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM model_roles WHERE provider_id=?1",
                params![provider_id],
            )?;
            Ok(())
        })
    }

    // -------------------------------------------------------- service decisions

    /// Record a decision about a tool or service.
    pub fn set_service_decision(
        &self,
        tool: &str,
        mode: &str,
        port: Option<i64>,
        note: Option<&str>,
        by: &str,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO service_decisions (tool,mode,port,note,decided_by,decided_at) VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(tool) DO UPDATE SET mode=?2, port=?3, note=?4, decided_by=?5, decided_at=?6",
                params![tool, mode, port, note, by, util::now()],
            )?;
            Ok(())
        })
    }

    /// Forget a decision.
    pub fn clear_service_decision(&self, tool: &str) -> Result<bool> {
        self.with(|c| {
            Ok(c.execute("DELETE FROM service_decisions WHERE tool=?1", params![tool])? > 0)
        })
    }

    /// Every decision.
    pub fn list_service_decisions(&self) -> Result<Vec<ServiceDecision>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT tool,mode,port,note,decided_by,decided_at FROM service_decisions ORDER BY tool")?;
            let rows = st.query_map([], |r| Ok(ServiceDecision { tool: r.get(0)?, mode: r.get(1)?, port: r.get(2)?, note: r.get(3)?, decided_by: r.get(4)?, decided_at: r.get(5)? }))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    // --------------------------------------------------------------- team runs

    /// Start a run.
    pub fn create_run(
        &self,
        title: &str,
        goal: &str,
        project: Option<&str>,
        parallel: i64,
        by: &str,
    ) -> Result<TeamRun> {
        let now = util::now();
        let run = TeamRun {
            id: util::new_id(),
            title: title.into(),
            goal: goal.into(),
            project: project.map(str::to_string),
            state: "planning".into(),
            plan_approved: false,
            thread_id: None,
            summary: None,
            error: None,
            parallel: parallel.clamp(1, 6),
            created_by: by.into(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO team_runs ({RUN_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"),
                params![run.id, run.title, run.goal, run.project, run.state, run.plan_approved as i32, run.summary, run.error, run.parallel, run.created_by, run.created_at, run.updated_at, run.thread_id],
            )?;
            Ok(())
        })?;
        // Every team run has its unified run from the moment it exists, so the
        // planner, the specialists, the approvals and the timeline all point at
        // one record.
        let _ = self.ensure_run_for_team(&run.id, &run.goal, run.project.as_deref(), by, &[]);
        let _ = self.sync_team_state(&run.id, &run.state, None);
        Ok(run)
    }

    /// Every conversation a team specialist works in, with what it belongs to.
    ///
    /// One read for the whole list, so the conversation list can nest each
    /// specialist under the run and the conversation that started it without a
    /// query per row.
    pub fn thread_team_links(&self) -> Result<Vec<ThreadTeamLink>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT t.thread_id, t.run_id, t.role, t.title, t.state, r.title, r.thread_id, r.state
                 FROM team_tasks t JOIN team_runs r ON r.id = t.run_id
                 WHERE t.thread_id IS NOT NULL AND t.thread_id <> ''",
            )?;
            let rows = st.query_map([], |row| {
                Ok(ThreadTeamLink {
                    thread_id: row.get(0)?,
                    run_id: row.get(1)?,
                    role: row.get(2)?,
                    task: row.get(3)?,
                    task_state: row.get(4)?,
                    run_title: row.get(5)?,
                    parent_thread: row.get(6)?,
                    run_state: row.get(7)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Point a run at the conversation that asked for it.
    ///
    /// Set after creation rather than passed in: `create_run` has five callers
    /// already and only the assistant-driven one has a conversation to give.
    pub fn set_run_thread(&self, run_id: &str, thread_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE team_runs SET thread_id=?1 WHERE id=?2", params![thread_id, run_id])?;
            Ok(())
        })
    }

    /// The conversation a run belongs to, if it belongs to one.
    pub fn run_thread(&self, run_id: &str) -> Result<Option<String>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT thread_id FROM team_runs WHERE id=?1")?;
            let mut rows = st.query(params![run_id])?;
            Ok(rows.next()?.and_then(|r| r.get::<_, Option<String>>(0).ok().flatten()))
        })
    }

    /// Record a report for a conversation to pick up.
    pub fn queue_report(&self, run_id: &str, thread_id: &str, kind: &str, body: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO team_reports (id,run_id,thread_id,kind,body,delivered,created_at) VALUES (?1,?2,?3,?4,?5,0,?6)",
                params![util::new_id(), run_id, thread_id, kind, body, util::now()],
            )?;
            Ok(())
        })
    }

    /// Reports a conversation has not received yet, oldest first.
    pub fn pending_reports(&self, thread_id: &str) -> Result<Vec<(String, String, String, String)>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,kind,body FROM team_reports WHERE thread_id=?1 AND delivered=0 ORDER BY created_at")?;
            let rows = st.query_map(params![thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Mark reports delivered, so the same result is never announced twice.
    pub fn mark_reports_delivered(&self, ids: &[String]) -> Result<()> {
        self.with(|c| {
            for id in ids {
                c.execute("UPDATE team_reports SET delivered=1 WHERE id=?1", params![id])?;
            }
            Ok(())
        })
    }
    /// Mirror a team-run state onto its unified run.
    ///
    /// The team table is still the scheduler's own record; this keeps the run
    /// every interface reads in step with it. A transition the unified lifecycle
    /// would reject (it is stricter about leaving a cancelled run, for example)
    /// is left alone rather than forced, and the next explicit action decides.
    fn sync_team_state(&self, team_run_id: &str, state: &str, error: Option<&str>) -> Result<()> {
        let Some(run) = self.run_for_team_run(team_run_id)? else {
            return Ok(());
        };
        let next = crate::runs::RunState::parse(state);
        if run.typed_state() != next {
            let _ = self.transition_run(&run.id, next, error);
        }
        Ok(())
    }

    /// Record the definition of done a run must meet and how many extra passes it may take.
    pub fn set_goal_loop(&self, run_id: &str, done_when: &[String], max_rounds: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET done_when=?2, max_rounds=?3 WHERE id=?1",
                params![
                    run_id,
                    serde_json::to_string(done_when)?,
                    max_rounds.clamp(0, 5)
                ],
            )?;
            Ok(())
        })
    }

    /// A run's definition of done and how far through its extra passes it is.
    pub fn goal_loop(&self, run_id: &str) -> Result<GoalLoop> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT done_when,rounds,max_rounds FROM team_runs WHERE id=?1",
                params![run_id],
                |r| {
                    Ok(GoalLoop {
                        done_when: serde_json::from_str(&r.get::<_, String>(0)?)
                            .unwrap_or_default(),
                        rounds: r.get(1)?,
                        max_rounds: r.get(2)?,
                    })
                },
            )?)
        })
    }

    /// Count one more pass.
    pub fn bump_round(&self, run_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET rounds=rounds+1, updated_at=?2 WHERE id=?1",
                params![run_id, util::now()],
            )?;
            Ok(())
        })
    }

    /// One run.
    pub fn get_run(&self, id: &str) -> Result<Option<TeamRun>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {RUN_COLS} FROM team_runs WHERE id=?1"),
                params![id],
                row_run,
            )
            .optional()?)
        })
    }

    /// Recent runs.
    pub fn list_runs(&self, limit: i64) -> Result<Vec<TeamRun>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM team_runs ORDER BY updated_at DESC LIMIT ?1"
            ))?;
            let rows = st.query_map(params![limit], row_run)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Move a run to a state.
    pub fn set_run_state(
        &self,
        id: &str,
        state: &str,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET state=?2, summary=COALESCE(?3,summary), error=?4, updated_at=?5 WHERE id=?1",
                params![id, state, summary, error, util::now()],
            )?;
            Ok(())
        })?;
        self.sync_team_state(id, state, error)
    }

    /// Record that a person approved the plan.
    pub fn approve_run_plan(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET plan_approved=1, state='running', updated_at=?2 WHERE id=?1",
                params![id, util::now()],
            )?;
            Ok(())
        })?;
        self.sync_team_state(id, "running", None)
    }

    // -------------------------------------------------------------- team tasks

    /// Add a task to a run.
    pub fn add_task(&self, run_id: &str, new: NewTask) -> Result<TeamTask> {
        let NewTask {
            key,
            role,
            title,
            instructions,
            depends_on,
            scope,
            mandate,
        } = new;
        let t = TeamTask {
            id: util::new_id(),
            run_id: run_id.into(),
            key: key.into(),
            role: role.into(),
            title: title.into(),
            instructions: instructions.into(),
            depends_on: depends_on.to_vec(),
            scope: scope.to_vec(),
            state: "pending".into(),
            attempts: 0,
            workdir: None,
            branch: None,
            thread_id: None,
            result: None,
            evidence: serde_json::json!([]),
            error: None,
            created_at: util::now(),
            started_at: None,
            finished_at: None,
            mandate: mandate.cloned().unwrap_or(serde_json::Value::Null),
            last_step_at: None,
            stalls: 0,
        };
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO team_tasks ({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)"),
                params![t.id, t.run_id, t.key, t.role, t.title, t.instructions, serde_json::to_string(&t.depends_on).unwrap_or_default(), serde_json::to_string(&t.scope).unwrap_or_default(),
                    t.state, t.attempts, t.workdir, t.branch, t.thread_id, t.result, t.evidence.to_string(), t.error, t.created_at, t.started_at, t.finished_at,
                    t.mandate.to_string(), t.last_step_at, t.stalls],
            )?;
            Ok(())
        })?;
        Ok(t)
    }

    /// Tasks of a run in creation order.
    /// Delete a run with its tasks, its log and the conversations its tasks had. Returns the
    /// conversations that were removed. The caller has checked that nothing is still working.
    pub fn delete_run(&self, id: &str) -> Result<usize> {
        let title = self.get_run(id)?.map(|r| r.title);
        let threads: Vec<String> = self
            .list_tasks_of_run(id)?
            .into_iter()
            .filter_map(|t| t.thread_id)
            .collect();
        for t in &threads {
            self.delete_thread(t)?;
        }
        // A message about a run is the run. If the run goes, the message goes
        // with it - otherwise deleting a run turns its old pause notices into
        // fresh decisions on Home, which is the opposite of deleting it.
        self.forget_run_inbox(id, title.as_deref())?;
        self.with(|c| {
            c.execute("DELETE FROM team_events WHERE run_id=?1", params![id])?;
            c.execute("DELETE FROM team_tasks WHERE run_id=?1", params![id])?;
            c.execute("DELETE FROM team_runs WHERE id=?1", params![id])?;
            Ok(())
        })?;
        Ok(threads.len())
    }

    /// The runs that are over (done, failed or cancelled) and were last touched before `before`.
    pub fn finished_runs_before(&self, before: &str) -> Result<Vec<TeamRun>> {
        Ok(self
            .list_runs(10_000)?
            .into_iter()
            .filter(|r| {
                matches!(r.state.as_str(), "done" | "failed" | "cancelled")
                    && r.updated_at.as_str() < before
            })
            .collect())
    }

    pub fn list_tasks_of_run(&self, run_id: &str) -> Result<Vec<TeamTask>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {TASK_COLS} FROM team_tasks WHERE run_id=?1 ORDER BY rowid"
            ))?;
            let rows = st.query_map(params![run_id], row_task)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One task.
    pub fn get_team_task(&self, id: &str) -> Result<Option<TeamTask>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {TASK_COLS} FROM team_tasks WHERE id=?1"),
                params![id],
                row_task,
            )
            .optional()?)
        })
    }

    /// The task a conversation is working on.
    pub fn task_of_thread(&self, thread_id: &str) -> Result<Option<TeamTask>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {TASK_COLS} FROM team_tasks WHERE thread_id=?1"),
                params![thread_id],
                row_task,
            )
            .optional()?)
        })
    }

    /// Change a task's state and bookkeeping.
    pub fn update_team_task(&self, id: &str, update: TaskUpdate) -> Result<()> {
        let TaskUpdate {
            state,
            result,
            error,
            evidence,
            workdir,
            branch,
            thread_id,
            mandate,
        } = update;
        let now = util::now();
        self.with(|c| {
            c.execute(
                "UPDATE team_tasks SET state=?2, result=COALESCE(?3,result), error=?4, evidence=COALESCE(?5,evidence), workdir=COALESCE(?6,workdir),
                   branch=COALESCE(?7,branch), thread_id=COALESCE(?8,thread_id),
                   mandate=COALESCE(?10,mandate),
                   started_at=CASE WHEN ?2='running' AND started_at IS NULL THEN ?9 ELSE started_at END,
                   last_step_at=CASE WHEN ?2='running' THEN ?9 ELSE last_step_at END,
                   attempts=CASE WHEN ?2='running' THEN attempts+1 ELSE attempts END,
                   finished_at=CASE WHEN ?2 IN ('done','unverified','failed','cancelled','skipped') THEN ?9 ELSE NULL END
                 WHERE id=?1",
                params![id, state, result, error, evidence.map(|e| e.to_string()), workdir, branch, thread_id, now, mandate.map(|m| m.to_string())],
            )?;
            Ok(())
        })
    }

    // ---------------------------------------------------------------- team log

    /// Append to a run's log.
    pub fn add_team_event(
        &self,
        run_id: &str,
        task_id: Option<&str>,
        kind: &str,
        data: &serde_json::Value,
    ) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO team_events (run_id,task_id,kind,data,at) VALUES (?1,?2,?3,?4,?5)",
                params![run_id, task_id, kind, data.to_string(), util::now()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    /// A run's log after a sequence number.
    pub fn list_team_events(&self, run_id: &str, after: i64, limit: i64) -> Result<Vec<TeamEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,run_id,task_id,kind,data,at FROM team_events WHERE run_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
            let rows = st.query_map(params![run_id, after, limit], |r| {
                Ok(TeamEvent { id: r.get(0)?, run_id: r.get(1)?, task_id: r.get(2)?, kind: r.get(3)?, data: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(), at: r.get(5)? })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}
