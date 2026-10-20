//! Real spawn/wait coordination between agent sessions.
//!
//! A delegated task is a genuine child process: a separate PTY, a separate
//! harness invocation, its own working directory and its own lifecycle. The
//! database records the relationship, but the relationship is not the product -
//! the process is.
//!
//! # Limits
//!
//! Runaway spawning is a real failure mode for a coordinating agent, so the
//! limits are enforced before a process is created: at most
//! [`MAX_ACTIVE_TASKS`] unfinished tasks per installation, at most
//! [`MAX_DEPTH`] levels of nesting, and a bounded wait. A wait never kills the
//! child; cancellation is a separate, explicit operation, so a timeout cannot
//! silently destroy work.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::error::{DevdError, Result};
use devd_core::store_work::{AgentSession, AgentTask};
use devd_core::util;

use crate::session::{SessionManager, SessionView, StartOptions};

/// Most delegated tasks that may be unfinished at once.
pub const MAX_ACTIVE_TASKS: i64 = 8;

/// Most levels of delegation below a root session.
pub const MAX_DEPTH: i64 = 3;

/// Longest wait a single `wait` call may hold.
pub const MAX_WAIT_SECS: u64 = 900;

/// Longest timeout a task may declare.
pub const MAX_TASK_TIMEOUT_SECS: u64 = 3600;

/// Input for spawning a delegated task.
#[derive(Debug, Clone, Deserialize)]
pub struct SpawnRequest {
    /// Short title, used as the session label and task name.
    pub title: String,
    /// Instructions for a harness, or a note for a custom command.
    #[serde(default)]
    pub instructions: String,
    /// `claude`, `opencode`, `shell` or `custom`.
    pub harness: String,
    /// Working directory.
    pub cwd: String,
    /// Project scope.
    #[serde(default)]
    pub project: Option<String>,
    /// Session delegating the work.
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// Task delegating the work, for nesting.
    #[serde(default)]
    pub parent_task_id: Option<String>,
    /// Seconds before a waiter gives up. The process is not killed.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Full argv for the `custom` harness.
    #[serde(default)]
    pub command: Option<Vec<String>>,
}

/// Outcome of a wait.
pub struct WaitOutcome {
    /// Current task state.
    pub task: AgentTask,
    /// Whether the wait expired before a terminal state.
    pub timed_out: bool,
}

/// Spawns and observes delegated agent tasks.
pub struct Orchestrator {
    sessions: Arc<SessionManager>,
}

impl Orchestrator {
    /// Build an orchestrator over a session manager.
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }

    /// Spawn a real child session for a task.
    pub fn spawn(&self, req: SpawnRequest) -> Result<(AgentTask, SessionView)> {
        let title = req.title.trim();
        if title.is_empty() || title.len() > 160 {
            return Err(DevdError::invalid(
                "a task title must be between 1 and 160 characters",
            ));
        }
        let timeout_secs = req
            .timeout_secs
            .unwrap_or(300)
            .clamp(1, MAX_TASK_TIMEOUT_SECS);

        let active = self.sessions.store().count_active_tasks()?;
        if active >= MAX_ACTIVE_TASKS {
            return Err(DevdError::conflict(format!(
                "there are already {MAX_ACTIVE_TASKS} unfinished delegated tasks; wait for one or cancel it"
            )));
        }

        let mut depth = 0;
        if let Some(parent_id) = &req.parent_task_id {
            let parent = self
                .sessions
                .store()
                .get_agent_task(parent_id)?
                .ok_or_else(|| DevdError::not_found(format!("no parent task `{parent_id}`")))?;
            depth = parent.depth + 1;
        } else if req.parent_session_id.is_some() {
            depth = 1;
        }
        if depth > MAX_DEPTH {
            return Err(DevdError::conflict(format!(
                "delegation may nest at most {MAX_DEPTH} levels deep"
            )));
        }

        let (program_args, command) = task_argv(&req)?;

        let now = util::now();
        let task_id = util::new_id();
        let cwd = std::path::Path::new(&req.cwd).canonicalize().map_err(|e| {
            DevdError::invalid(format!("task working directory is not usable: {e}"))
        })?;
        let cwd = util::strip_verbatim(&cwd);
        let project = req
            .project
            .clone()
            .or_else(|| Some(cwd.to_string_lossy().to_string()));

        let task = AgentTask {
            id: task_id.clone(),
            parent_task_id: req.parent_task_id.clone(),
            parent_session_id: req.parent_session_id.clone(),
            session_id: None,
            title: title.to_string(),
            instructions: req.instructions.clone(),
            cwd: cwd.to_string_lossy().to_string(),
            harness: req.harness.clone(),
            state: "starting".to_string(),
            depth,
            timeout_secs: timeout_secs as i64,
            exit_code: None,
            result: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
            finished_at: None,
        };
        self.sessions.store().insert_agent_task(&task)?;

        let view = match self.sessions.start(StartOptions {
            harness: req.harness.clone(),
            cwd: task.cwd.clone(),
            project,
            label: Some(format!("{} — delegated", title)),
            cols: 120,
            rows: 32,
            task_id: Some(task_id.clone()),
            parent_session_id: req.parent_session_id.clone(),
            args: program_args,
            command,
        }) {
            Ok(view) => view,
            Err(e) => {
                let _ = self.sessions.store().finish_agent_task(
                    &task_id,
                    "failed",
                    None,
                    None,
                    Some(&e.to_string()),
                );
                return Err(e);
            }
        };

        self.sessions
            .store()
            .set_task_session(&task_id, &view.session.id)?;
        let task = self
            .sessions
            .store()
            .get_agent_task(&task_id)?
            .ok_or_else(|| DevdError::internal("the task disappeared after creation"))?;
        Ok((task, view))
    }

    /// Wait for a task, up to a bounded number of seconds.
    pub fn wait(&self, task_id: &str, timeout_secs: Option<u64>) -> Result<WaitOutcome> {
        let limit = timeout_secs.unwrap_or(300).clamp(1, MAX_WAIT_SECS);
        let started = Instant::now();
        loop {
            let task = self
                .sessions
                .store()
                .get_agent_task(task_id)?
                .ok_or_else(|| DevdError::not_found(format!("no task `{task_id}`")))?;
            if task.is_terminal() {
                return Ok(WaitOutcome {
                    task,
                    timed_out: false,
                });
            }
            if started.elapsed() >= Duration::from_secs(limit) {
                return Ok(WaitOutcome {
                    task,
                    timed_out: true,
                });
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Read a task, with the tail of its output when it has a session.
    pub fn result(&self, task_id: &str) -> Result<Value> {
        let task = self
            .sessions
            .store()
            .get_agent_task(task_id)?
            .ok_or_else(|| DevdError::not_found(format!("no task `{task_id}`")))?;
        let output = match &task.session_id {
            Some(session_id) => self
                .sessions
                .tail_text(session_id, 64 * 1024)
                .unwrap_or_default(),
            None => String::new(),
        };
        Ok(json!({
            "task": task,
            "output": output,
            "finished": task.is_terminal(),
        }))
    }

    /// Cancel a task and stop its child session.
    pub fn cancel(&self, task_id: &str) -> Result<AgentTask> {
        let task = self
            .sessions
            .store()
            .get_agent_task(task_id)?
            .ok_or_else(|| DevdError::not_found(format!("no task `{task_id}`")))?;
        if task.is_terminal() {
            return Ok(task);
        }
        if let Some(session_id) = &task.session_id {
            self.sessions.stop(session_id)?;
        } else {
            self.sessions.store().finish_agent_task(
                task_id,
                "cancelled",
                None,
                None,
                Some("cancelled before the session started"),
            )?;
        }
        self.sessions
            .store()
            .get_agent_task(task_id)?
            .ok_or_else(|| DevdError::not_found(format!("no task `{task_id}`")))
    }

    /// List tasks, optionally for one parent session.
    pub fn list(
        &self,
        parent_session_id: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<AgentTask>> {
        self.sessions
            .store()
            .list_agent_tasks(parent_session_id, project, 200)
    }

    /// Every session this installation has recorded.
    pub fn sessions(&self) -> &Arc<SessionManager> {
        &self.sessions
    }
}

fn task_argv(req: &SpawnRequest) -> Result<(Vec<String>, Option<Vec<String>>)> {
    if req.instructions.len() > 64 * 1024 {
        return Err(DevdError::invalid("task instructions are too long"));
    }
    match req.harness.as_str() {
        "claude" => {
            if req.command.is_some() {
                return Err(DevdError::invalid(
                    "a claude task takes instructions, not a command",
                ));
            }
            if req.instructions.trim().is_empty() {
                return Err(DevdError::invalid("a claude task needs instructions"));
            }
            Ok((
                vec![
                    "-p".to_string(),
                    req.instructions.clone(),
                    "--output-format".to_string(),
                    "text".to_string(),
                ],
                None,
            ))
        }
        "opencode" => {
            if req.command.is_some() {
                return Err(DevdError::invalid(
                    "an opencode task takes instructions, not a command",
                ));
            }
            if req.instructions.trim().is_empty() {
                return Err(DevdError::invalid("an opencode task needs instructions"));
            }
            Ok((vec!["run".to_string(), req.instructions.clone()], None))
        }
        "custom" | "shell" => {
            let command = req
                .command
                .clone()
                .ok_or_else(|| DevdError::invalid("a custom task needs a command argv array"))?;
            if command.is_empty() {
                return Err(DevdError::invalid("the task command is empty"));
            }
            for arg in &command {
                if arg.contains('\0') {
                    return Err(DevdError::invalid("a task argument contains a NUL byte"));
                }
            }
            Ok((Vec::new(), Some(command)))
        }
        other => Err(DevdError::invalid(format!(
            "unknown task harness `{other}`; expected claude, opencode, custom or shell"
        ))),
    }
}

/// The session a task is running in, when it has one.
pub fn task_session(task: &AgentTask) -> Option<&str> {
    task.session_id.as_deref()
}

/// Convenience for callers that need the task's parent session.
pub fn parent_of(session: &AgentSession) -> Option<&str> {
    session.parent_session_id.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::state::Store;
    use devd_core::Paths;

    fn setup() -> (tempfile::TempDir, Arc<Orchestrator>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.state_db()).unwrap());
        store.migrate().unwrap();
        let sessions = SessionManager::new(store, Arc::new(paths)).unwrap();
        (dir, Arc::new(Orchestrator::new(sessions)))
    }

    fn sleeper(_dir: &std::path::Path, seconds: u32, exit: u32) -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "cmd.exe".into(),
                "/c".into(),
                format!("echo delegated-output & ping -n {seconds} 127.0.0.1 > NUL & exit {exit}"),
            ]
        }
        #[cfg(unix)]
        {
            vec![
                "/bin/sh".into(),
                "-c".into(),
                format!("echo delegated-output; sleep {seconds}; exit {exit}"),
            ]
        }
    }

    #[test]
    fn spawn_wait_and_result_are_a_real_process() {
        let (dir, orch) = setup();
        let (task, view) = orch
            .spawn(SpawnRequest {
                title: "a real child".into(),
                instructions: "print something".into(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: None,
                timeout_secs: Some(30),
                command: Some(sleeper(dir.path(), 1, 0)),
            })
            .unwrap();
        assert_eq!(task.state, "running");
        assert!(view.attachable);
        let waited = orch.wait(&task.id, Some(20)).unwrap();
        assert!(!waited.timed_out, "task did not finish");
        assert_eq!(waited.task.state, "succeeded", "{:?}", waited.task.error);
        assert_eq!(waited.task.exit_code, Some(0));
        let result = orch.result(&task.id).unwrap();
        assert!(result["output"]
            .as_str()
            .unwrap()
            .contains("delegated-output"));
        assert!(result["finished"].as_bool().unwrap());
    }

    #[test]
    fn a_nonzero_exit_is_a_failure_with_its_code() {
        let (dir, orch) = setup();
        let (task, _) = orch
            .spawn(SpawnRequest {
                title: "fails".into(),
                instructions: String::new(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: None,
                timeout_secs: Some(30),
                command: Some(sleeper(dir.path(), 1, 7)),
            })
            .unwrap();
        let waited = orch.wait(&task.id, Some(20)).unwrap();
        assert_eq!(waited.task.state, "failed");
        assert_eq!(waited.task.exit_code, Some(7));
        assert!(waited.task.error.unwrap().contains("7"));
    }

    #[test]
    fn wait_times_out_without_killing_the_process() {
        let (dir, orch) = setup();
        let (task, view) = orch
            .spawn(SpawnRequest {
                title: "slow".into(),
                instructions: String::new(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: None,
                timeout_secs: Some(30),
                command: Some(sleeper(dir.path(), 20, 0)),
            })
            .unwrap();
        let waited = orch.wait(&task.id, Some(1)).unwrap();
        assert!(waited.timed_out);
        assert_eq!(waited.task.state, "running");
        assert!(view.attachable);
        orch.cancel(&task.id).unwrap();
        let after = orch.sessions().get(&view.session.id).unwrap();
        assert_eq!(after.live_state, "cancelled");
    }

    #[test]
    fn cancellation_is_recorded_on_the_task() {
        let (dir, orch) = setup();
        let (task, _) = orch
            .spawn(SpawnRequest {
                title: "cancel me".into(),
                instructions: String::new(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: None,
                timeout_secs: Some(30),
                command: Some(sleeper(dir.path(), 20, 0)),
            })
            .unwrap();
        let cancelled = orch.cancel(&task.id).unwrap();
        assert_eq!(cancelled.state, "cancelled");
    }

    #[test]
    fn depth_and_concurrency_limits_are_enforced() {
        let (dir, orch) = setup();
        // A synthetic task at maximum depth cannot be extended.
        let now = util::now();
        let parent = AgentTask {
            id: "parent".into(),
            parent_task_id: None,
            parent_session_id: None,
            session_id: None,
            title: "parent".into(),
            instructions: String::new(),
            cwd: dir.path().to_string_lossy().to_string(),
            harness: "custom".into(),
            state: "running".into(),
            depth: MAX_DEPTH,
            timeout_secs: 30,
            exit_code: None,
            result: None,
            error: None,
            created_at: now.clone(),
            updated_at: now,
            finished_at: None,
        };
        orch.sessions().store().insert_agent_task(&parent).unwrap();
        let err = orch
            .spawn(SpawnRequest {
                title: "too deep".into(),
                instructions: String::new(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: Some("parent".into()),
                timeout_secs: Some(30),
                command: Some(sleeper(dir.path(), 1, 0)),
            })
            .unwrap_err();
        assert!(err.to_string().contains("nest"), "{err}");
    }

    #[test]
    fn a_missing_command_is_refused_before_anything_starts() {
        let (dir, orch) = setup();
        let err = orch
            .spawn(SpawnRequest {
                title: "no command".into(),
                instructions: String::new(),
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                project: None,
                parent_session_id: None,
                parent_task_id: None,
                timeout_secs: Some(30),
                command: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("argv"), "{err}");
    }
}
