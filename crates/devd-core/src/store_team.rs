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

const RUN_COLS: &str = "id,title,goal,project,state,plan_approved,summary,error,parallel,created_by,created_at,updated_at";
const TASK_COLS: &str = "id,run_id,key,role,title,instructions,depends_on,scope,state,attempts,workdir,branch,thread_id,result,evidence,error,created_at,started_at,finished_at";

fn row_run(r: &rusqlite::Row) -> rusqlite::Result<TeamRun> {
    Ok(TeamRun {
        id: r.get(0)?, title: r.get(1)?, goal: r.get(2)?, project: r.get(3)?, state: r.get(4)?, plan_approved: r.get::<_, i64>(5)? != 0,
        summary: r.get(6)?, error: r.get(7)?, parallel: r.get(8)?, created_by: r.get(9)?, created_at: r.get(10)?, updated_at: r.get(11)?,
    })
}

fn row_task(r: &rusqlite::Row) -> rusqlite::Result<TeamTask> {
    let list = |s: String| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default();
    Ok(TeamTask {
        id: r.get(0)?, run_id: r.get(1)?, key: r.get(2)?, role: r.get(3)?, title: r.get(4)?, instructions: r.get(5)?,
        depends_on: list(r.get(6)?), scope: list(r.get(7)?), state: r.get(8)?, attempts: r.get(9)?, workdir: r.get(10)?, branch: r.get(11)?,
        thread_id: r.get(12)?, result: r.get(13)?, evidence: serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or(serde_json::json!([])),
        error: r.get(15)?, created_at: r.get(16)?, started_at: r.get(17)?, finished_at: r.get(18)?,
    })
}

impl Store {
    // ---------------------------------------------------------- environment

    /// `local` (the default) or `production`.
    pub fn environment_mode(&self) -> String {
        self.with(|c| Ok(c.query_row("SELECT value FROM kv WHERE key='environment.mode'", [], |r| r.get::<_, String>(0)).optional()?))
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
    pub fn set_provider_check(&self, provider_id: &str, ok: bool, message: &str, models: Option<i64>) -> Result<()> {
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
    pub fn set_role_models(&self, role: &str, scope: &str, models: &[(String, String)]) -> Result<()> {
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
            c.execute("DELETE FROM model_roles WHERE provider_id=?1", params![provider_id])?;
            Ok(())
        })
    }

    // -------------------------------------------------------- service decisions

    /// Record a decision about a tool or service.
    pub fn set_service_decision(&self, tool: &str, mode: &str, port: Option<i64>, note: Option<&str>, by: &str) -> Result<()> {
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
        self.with(|c| Ok(c.execute("DELETE FROM service_decisions WHERE tool=?1", params![tool])? > 0))
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
    pub fn create_run(&self, title: &str, goal: &str, project: Option<&str>, parallel: i64, by: &str) -> Result<TeamRun> {
        let now = util::now();
        let run = TeamRun {
            id: util::new_id(), title: title.into(), goal: goal.into(), project: project.map(str::to_string), state: "planning".into(), plan_approved: false,
            summary: None, error: None, parallel: parallel.clamp(1, 6), created_by: by.into(), created_at: now.clone(), updated_at: now,
        };
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO team_runs ({RUN_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"),
                params![run.id, run.title, run.goal, run.project, run.state, run.plan_approved as i32, run.summary, run.error, run.parallel, run.created_by, run.created_at, run.updated_at],
            )?;
            Ok(())
        })?;
        Ok(run)
    }

    /// Record the definition of done a run must meet and how many extra passes it may take.
    pub fn set_goal_loop(&self, run_id: &str, done_when: &[String], max_rounds: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET done_when=?2, max_rounds=?3 WHERE id=?1",
                params![run_id, serde_json::to_string(done_when)?, max_rounds.clamp(0, 5)],
            )?;
            Ok(())
        })
    }

    /// A run's definition of done and how far through its extra passes it is.
    pub fn goal_loop(&self, run_id: &str) -> Result<GoalLoop> {
        self.with(|c| {
            Ok(c.query_row("SELECT done_when,rounds,max_rounds FROM team_runs WHERE id=?1", params![run_id], |r| {
                Ok(GoalLoop { done_when: serde_json::from_str(&r.get::<_, String>(0)?).unwrap_or_default(), rounds: r.get(1)?, max_rounds: r.get(2)? })
            })?)
        })
    }

    /// Count one more pass.
    pub fn bump_round(&self, run_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE team_runs SET rounds=rounds+1, updated_at=?2 WHERE id=?1", params![run_id, util::now()])?;
            Ok(())
        })
    }

    /// One run.
    pub fn get_run(&self, id: &str) -> Result<Option<TeamRun>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {RUN_COLS} FROM team_runs WHERE id=?1"), params![id], row_run).optional()?))
    }

    /// Recent runs.
    pub fn list_runs(&self, limit: i64) -> Result<Vec<TeamRun>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {RUN_COLS} FROM team_runs ORDER BY updated_at DESC LIMIT ?1"))?;
            let rows = st.query_map(params![limit], row_run)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Move a run to a state.
    pub fn set_run_state(&self, id: &str, state: &str, summary: Option<&str>, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE team_runs SET state=?2, summary=COALESCE(?3,summary), error=?4, updated_at=?5 WHERE id=?1",
                params![id, state, summary, error, util::now()],
            )?;
            Ok(())
        })
    }

    /// Record that a person approved the plan.
    pub fn approve_run_plan(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE team_runs SET plan_approved=1, state='running', updated_at=?2 WHERE id=?1", params![id, util::now()])?;
            Ok(())
        })
    }

    // -------------------------------------------------------------- team tasks

    /// Add a task to a run.
    pub fn add_task(&self, run_id: &str, new: NewTask) -> Result<TeamTask> {
        let NewTask { key, role, title, instructions, depends_on, scope } = new;
        let t = TeamTask {
            id: util::new_id(), run_id: run_id.into(), key: key.into(), role: role.into(), title: title.into(), instructions: instructions.into(),
            depends_on: depends_on.to_vec(), scope: scope.to_vec(), state: "pending".into(), attempts: 0, workdir: None, branch: None, thread_id: None,
            result: None, evidence: serde_json::json!([]), error: None, created_at: util::now(), started_at: None, finished_at: None,
        };
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO team_tasks ({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)"),
                params![t.id, t.run_id, t.key, t.role, t.title, t.instructions, serde_json::to_string(&t.depends_on).unwrap_or_default(), serde_json::to_string(&t.scope).unwrap_or_default(),
                    t.state, t.attempts, t.workdir, t.branch, t.thread_id, t.result, t.evidence.to_string(), t.error, t.created_at, t.started_at, t.finished_at],
            )?;
            Ok(())
        })?;
        Ok(t)
    }

    /// Tasks of a run in creation order.
    pub fn list_tasks_of_run(&self, run_id: &str) -> Result<Vec<TeamTask>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {TASK_COLS} FROM team_tasks WHERE run_id=?1 ORDER BY rowid"))?;
            let rows = st.query_map(params![run_id], row_task)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One task.
    pub fn get_team_task(&self, id: &str) -> Result<Option<TeamTask>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {TASK_COLS} FROM team_tasks WHERE id=?1"), params![id], row_task).optional()?))
    }

    /// The task a conversation is working on.
    pub fn task_of_thread(&self, thread_id: &str) -> Result<Option<TeamTask>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {TASK_COLS} FROM team_tasks WHERE thread_id=?1"), params![thread_id], row_task).optional()?))
    }

    /// Change a task's state and bookkeeping.
    pub fn update_team_task(&self, id: &str, update: TaskUpdate) -> Result<()> {
        let TaskUpdate { state, result, error, evidence, workdir, branch, thread_id } = update;
        let now = util::now();
        self.with(|c| {
            c.execute(
                "UPDATE team_tasks SET state=?2, result=COALESCE(?3,result), error=?4, evidence=COALESCE(?5,evidence), workdir=COALESCE(?6,workdir),
                   branch=COALESCE(?7,branch), thread_id=COALESCE(?8,thread_id),
                   started_at=CASE WHEN ?2='running' AND started_at IS NULL THEN ?9 ELSE started_at END,
                   attempts=CASE WHEN ?2='running' THEN attempts+1 ELSE attempts END,
                   finished_at=CASE WHEN ?2 IN ('done','unverified','failed','cancelled','skipped') THEN ?9 ELSE NULL END
                 WHERE id=?1",
                params![id, state, result, error, evidence.map(|e| e.to_string()), workdir, branch, thread_id, now],
            )?;
            Ok(())
        })
    }

    // ---------------------------------------------------------------- team log

    /// Append to a run's log.
    pub fn add_team_event(&self, run_id: &str, task_id: Option<&str>, kind: &str, data: &serde_json::Value) -> Result<i64> {
        self.with(|c| {
            c.execute("INSERT INTO team_events (run_id,task_id,kind,data,at) VALUES (?1,?2,?3,?4,?5)", params![run_id, task_id, kind, data.to_string(), util::now()])?;
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
