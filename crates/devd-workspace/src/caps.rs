//! Capabilities for sessions, delegated tasks, project processes and shared
//! project state.
//!
//! These are the same declarations the CLI, the application window and the MCP
//! server see. The HTTP transport has a few direct endpoints for polling output
//! (auditing an 80ms poll would be noise, not security), but every *mutation* an
//! agent can make is declared here and goes through [`devd_core::Registry`].

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
use devd_core::error::{DevdError, Result};
use devd_core::store_work::{Scratchpad, Todo};
use devd_core::{util, Registry};

use crate::orchestrate::SpawnRequest;
use crate::session::StartOptions;
use crate::Workspace;

macro_rules! args {
    ($( $(#[$m:meta])* $name:ident { $( $f:ident : $t:ty ),* $(,)? } )*) => {
        $(
            $(#[$m])*
            #[derive(Debug, Clone, Deserialize, JsonSchema)]
            pub struct $name {
                $( pub $f : $t, )*
            }
        )*
    };
}

args! {
    /// No arguments.
    NoArgs {}
    /// Start a session.
    SessionStartArgs {
        harness: String,
        cwd: String,
        project: Option<String>,
        label: Option<String>,
        cols: Option<u16>,
        rows: Option<u16>,
        command: Option<Vec<String>>,
    }
    /// List sessions.
    SessionListArgs { project: Option<String> }
    /// One session.
    SessionArgs { id: String }
    /// Output since a cursor.
    SessionOutputArgs { id: String, cursor: Option<u64> }
    /// Terminal input.
    SessionInputArgs { id: String, data: String }
    /// Terminal resize.
    SessionResizeArgs { id: String, cols: u16, rows: u16 }
    /// Rename a session.
    SessionRenameArgs { id: String, label: String }
    /// Spawn a delegated task.
    SpawnArgs {
        title: String,
        harness: String,
        cwd: String,
        instructions: Option<String>,
        project: Option<String>,
        parent_session_id: Option<String>,
        parent_task_id: Option<String>,
        timeout_secs: Option<u64>,
        command: Option<Vec<String>>,
    }
    /// Wait for a task.
    WaitArgs { task_id: String, timeout_secs: Option<u64> }
    /// One task.
    TaskArgs { task_id: String }
    /// List tasks.
    TaskListArgs { parent_session_id: Option<String>, project: Option<String> }
    /// A project path.
    ProjectArgs { project: String }
    /// A project process.
    ProcessArgs { project: String, name: String, all: Option<bool> }
    /// Project logs.
    LogsArgs { project: String, name: String, lines: Option<usize>, query: Option<String> }
    /// A scratchpad.
    ScratchArgs { project: String, name: String }
    /// List scratchpads.
    ScratchListArgs { project: String, task_id: Option<String> }
    /// Create or replace a scratchpad.
    ScratchCreateArgs {
        project: String,
        name: String,
        content: Option<String>,
        task_id: Option<String>,
    }
    /// Update a scratchpad, guarded by revision.
    ScratchUpdateArgs {
        project: String,
        name: String,
        content: String,
        expected_revision: i64,
    }
    /// Append to a scratchpad.
    ScratchAppendArgs {
        project: String,
        name: String,
        content: String,
        task_id: Option<String>,
    }
    /// A todo id.
    TodoArgs { id: String }
    /// List todos.
    TodoListArgs { project: String, task_id: Option<String>, status: Option<String> }
    /// Create a todo.
    TodoCreateArgs {
        project: String,
        title: String,
        description: Option<String>,
        priority: Option<String>,
        assignee: Option<String>,
        task_id: Option<String>,
        dependencies: Option<Vec<String>>,
    }
    /// Update a todo.
    TodoUpdateArgs {
        id: String,
        expected_revision: i64,
        title: Option<String>,
        description: Option<String>,
        status: Option<String>,
        priority: Option<String>,
        assignee: Option<String>,
        note: Option<String>,
    }
    /// Complete a todo.
    TodoCompleteArgs { id: String, expected_revision: i64, note: Option<String> }
    /// Reopen a todo.
    TodoReopenArgs { id: String, expected_revision: i64, note: Option<String> }
    /// Note against a todo.
    TodoNoteArgs { id: String, note: String }
    /// Todo history.
    TodoHistoryArgs { id: String, limit: Option<u32> }
}

/// Register every workspace capability.
pub fn register(reg: &mut Registry, workspace: Arc<Workspace>) -> Result<()> {
    register_sessions(reg, workspace.clone())?;
    register_orchestration(reg, workspace.clone())?;
    register_projects(reg, workspace.clone())?;
    register_scratchpads(reg, workspace.clone())?;
    register_todos(reg, workspace)?;
    Ok(())
}

// ---------------------------------------------------------------- sessions

fn register_sessions(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    add::<SessionStartArgs, _, _>(
        reg,
        "session.start",
        "Start a coding-agent session",
        "Starts the named harness in a real PTY. The backend owns the process, so \
         a frontend reload or a view change does not stop it. Returns a stable \
         session id that can be attached to later.",
        "session",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |_, a| {
            Ok(
                Plan::new("session.start", format!("start {} in {}", a.harness, a.cwd))
                    .step(format!("open a PTY and run `{}`", a.harness))
                    .effect(format!("a child process in {}", a.cwd))
                    .data(json!({ "harness": a.harness, "cwd": a.cwd })),
            )
        },
        move |_ctx, a| {
            let view = w.sessions().start(StartOptions {
                harness: a.harness.clone(),
                cwd: a.cwd.clone(),
                project: a.project.clone(),
                label: a.label.clone(),
                cols: a.cols.unwrap_or(100),
                rows: a.rows.unwrap_or(30),
                args: Vec::new(),
                command: a.command.clone(),
                ..Default::default()
            })?;
            Ok(Outcome::with_data(
                format!("started session {}", view.session.id),
                serde_json::to_value(&view).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<SessionListArgs, _, _>(
        reg,
        "session.list",
        "List agent sessions",
        "Every recorded session, newest first, with live state folded in. A \
         running session owned by another devd process is reported as \
         unavailable rather than runnable, and a row left by a dead process is \
         reported as stale.",
        "session",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, _| Ok(Plan::new("session.list", "list agent sessions")),
        move |_ctx, a| {
            let rows = w.sessions().list(a.project.as_deref())?;
            Ok(Outcome::with_data(
                format!("{} session(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<SessionArgs, _, _>(
        reg,
        "session.get",
        "Inspect an agent session",
        "Returns one session's metadata, its live state and whether it can be \
         attached to in this process.",
        "session",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "session.get",
                format!("inspect session {}", a.id),
            ))
        },
        move |_ctx, a| {
            let view = w.sessions().get(&a.id)?;
            Ok(Outcome::with_data(
                format!("session {} is {}", a.id, view.live_state),
                serde_json::to_value(&view).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<SessionArgs, _, _>(
        reg,
        "session.attach",
        "Attach to an agent session",
        "Returns the session plus recent output from the beginning and a cursor, \
         so a client that reloaded can restore the terminal. A finished session \
         replays its transcript; a stale or foreign one explains why it cannot.",
        "session",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "session.attach",
                format!("attach to session {}", a.id),
            ))
        },
        move |_ctx, a| {
            let value = w.sessions().attach(&a.id)?;
            Ok(Outcome::with_data(format!("attached to {}", a.id), value))
        },
    )?;

    let w = ws.clone();
    add::<SessionOutputArgs, _, _>(
        reg,
        "session.output",
        "Read session output",
        "Returns base64 bytes since the caller's cursor. Output is bounded in \
         memory and on disk; a reset flag tells the client to rebuild its screen \
         from the replay.",
        "session",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "session.output",
                format!("read output of {}", a.id),
            ))
        },
        move |_ctx, a| {
            let value = w.sessions().output(&a.id, a.cursor.unwrap_or(0))?;
            Ok(Outcome::with_data(format!("output for {}", a.id), value))
        },
    )?;

    let w = ws.clone();
    add::<SessionInputArgs, _, _>(
        reg,
        "session.input",
        "Write to an agent terminal",
        "Sends keystrokes or pasted text to a live PTY. Reserved for humans: an \
         agent typing into a user's terminal is indistinguishable from prompt \
         injection, and delegated work already receives its instructions at \
         spawn time.",
        "session",
        Tier::Exec,
        Effect::Process,
        false,
        false,
        false,
        |_, a| Ok(Plan::new("session.input", format!("write to {}", a.id))),
        move |ctx, a| {
            ctx.require_human("writing to a terminal")?;
            w.sessions().write(&a.id, &a.data)?;
            Ok(Outcome::ok(format!("wrote to {}", a.id)))
        },
    )?;

    let w = ws.clone();
    add::<SessionResizeArgs, _, _>(
        reg,
        "session.resize",
        "Resize an agent terminal",
        "Tells the PTY its new size so full-screen terminal programs redraw.",
        "session",
        Tier::Write,
        Effect::Process,
        false,
        false,
        false,
        |_, a| {
            Ok(Plan::new(
                "session.resize",
                format!("resize {} to {}x{}", a.id, a.cols, a.rows),
            ))
        },
        move |ctx, a| {
            ctx.require_human("resizing a terminal")?;
            w.sessions().resize(&a.id, a.cols, a.rows)?;
            Ok(Outcome::ok(format!("resized {}", a.id)))
        },
    )?;

    let w = ws.clone();
    add::<SessionRenameArgs, _, _>(
        reg,
        "session.rename",
        "Rename an agent session",
        "Changes the display label only. The id, process and history are \
         unchanged.",
        "session",
        Tier::Write,
        Effect::Write,
        false,
        false,
        false,
        |_, a| {
            Ok(Plan::new(
                "session.rename",
                format!("rename {} to `{}`", a.id, a.label),
            ))
        },
        move |ctx, a| {
            ctx.require_human("renaming a session")?;
            let view = w.sessions().rename(&a.id, &a.label)?;
            Ok(Outcome::with_data(
                format!("renamed to {}", a.label),
                serde_json::to_value(&view).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<SessionArgs, _, _>(
        reg,
        "session.stop",
        "Stop an agent session",
        "Kills the PTY and its child process tree, retains the transcript and \
         records the session as cancelled. An agent may stop a delegated \
         session, but not a person's interactive one.",
        "session",
        Tier::Exec,
        Effect::Process,
        true,
        false,
        false,
        |_, a| {
            Ok(Plan::new("session.stop", format!("stop session {}", a.id))
                .effect("the terminal process and its children are terminated"))
        },
        move |ctx, a| {
            guard_agent_session(ctx, w.store(), &a.id)?;
            let view = w.sessions().stop(&a.id)?;
            Ok(Outcome::with_data(
                format!("stopped {}", a.id),
                serde_json::to_value(&view).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<SessionArgs, _, _>(
        reg,
        "session.dismiss",
        "Dismiss a finished session",
        "Deletes the metadata row for a session that is no longer attachable. \
         The transcript file is retained. A running session cannot be \
         dismissed.",
        "session",
        Tier::Destructive,
        Effect::Data,
        false,
        true,
        true,
        |_, a| {
            Ok(
                Plan::new("session.dismiss", format!("forget session {}", a.id))
                    .destructive(true)
                    .step("the session disappears from history"),
            )
        },
        move |ctx, a| {
            ctx.require_human("dismissing a session")?;
            w.sessions().dismiss(&a.id)?;
            Ok(Outcome::ok(format!("dismissed {}", a.id)))
        },
    )?;

    Ok(())
}

// ------------------------------------------------------------- orchestration

fn register_orchestration(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    add::<SpawnArgs, _, _>(
        reg,
        "agent.spawn",
        "Delegate a task to another agent",
        "Starts a real child process in its own PTY, with its own working \
         directory and harness invocation. Returns a stable task id and session \
         id. Nesting is limited to three levels and eight unfinished delegated \
         tasks per installation.",
        "agent",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |_, a| {
            Ok(Plan::new(
                "agent.spawn",
                format!("delegate `{}` to {}", a.title, a.harness),
            )
            .step(format!("start {} in {}", a.harness, a.cwd))
            .effect("a child agent process runs independently")
            .data(json!({ "title": a.title, "harness": a.harness, "cwd": a.cwd })))
        },
        move |_ctx, a| {
            let (task, view) = w.orchestrator().spawn(SpawnRequest {
                title: a.title.clone(),
                instructions: a.instructions.clone().unwrap_or_default(),
                harness: a.harness.clone(),
                cwd: a.cwd.clone(),
                project: a.project.clone(),
                parent_session_id: a.parent_session_id.clone(),
                parent_task_id: a.parent_task_id.clone(),
                timeout_secs: a.timeout_secs,
                command: a.command.clone(),
            })?;
            Ok(Outcome::with_data(
                format!("delegated task {} is {}", task.id, task.state),
                json!({ "task": task, "session": view }),
            ))
        },
    )?;

    let w = ws.clone();
    add::<WaitArgs, _, _>(
        reg,
        "agent.wait",
        "Wait for a delegated task",
        "Waits up to a bounded timeout for a task to reach a terminal state. A \
         timeout does not kill the child: use agent.cancel to cancel it \
         explicitly. Returns the final task and its state.",
        "agent",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "agent.wait",
                format!("wait for task {}", a.task_id),
            ))
        },
        move |_ctx, a| {
            let outcome = w.orchestrator().wait(&a.task_id, a.timeout_secs)?;
            let mut out = Outcome::with_data(
                if outcome.timed_out {
                    format!("task {} is still {}", a.task_id, outcome.task.state)
                } else {
                    format!("task {} finished as {}", a.task_id, outcome.task.state)
                },
                json!({ "task": outcome.task, "timed_out": outcome.timed_out }),
            );
            if outcome.timed_out {
                out = out.warn("the wait expired; the child process is still running");
            }
            Ok(out)
        },
    )?;

    let w = ws.clone();
    add::<TaskArgs, _, _>(
        reg,
        "agent.status",
        "Inspect a delegated task",
        "Returns a task's state, exit code, error and parent/child relationship.",
        "agent",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "agent.status",
                format!("inspect task {}", a.task_id),
            ))
        },
        move |_ctx, a| {
            let task = w
                .store()
                .get_agent_task(&a.task_id)?
                .ok_or_else(|| DevdError::not_found(format!("no task `{}`", a.task_id)))?;
            Ok(Outcome::with_data(
                format!("task {} is {}", task.id, task.state),
                serde_json::to_value(&task).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TaskListArgs, _, _>(
        reg,
        "agent.list",
        "List delegated tasks",
        "Lists tasks, optionally for one parent session or project.",
        "agent",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, _| Ok(Plan::new("agent.list", "list delegated tasks")),
        move |_ctx, a| {
            let rows = w
                .orchestrator()
                .list(a.parent_session_id.as_deref(), a.project.as_deref())?;
            Ok(Outcome::with_data(
                format!("{} task(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TaskArgs, _, _>(
        reg,
        "agent.result",
        "Read a delegated task's result",
        "Returns the task and the tail of its terminal output. The result is \
         recorded when the child exits, so it survives a frontend reload.",
        "agent",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "agent.result",
                format!("read result of task {}", a.task_id),
            ))
        },
        move |_ctx, a| {
            let value = w.orchestrator().result(&a.task_id)?;
            Ok(Outcome::with_data(
                format!("result of {}", a.task_id),
                value,
            ))
        },
    )?;

    let w = ws;
    add::<TaskArgs, _, _>(
        reg,
        "agent.cancel",
        "Cancel a delegated task",
        "Stops the child session and records the task as cancelled. Idempotent \
         for a task that has already finished.",
        "agent",
        Tier::Exec,
        Effect::Process,
        true,
        false,
        true,
        |_, a| {
            Ok(
                Plan::new("agent.cancel", format!("cancel task {}", a.task_id))
                    .effect("the child process is terminated"),
            )
        },
        move |_ctx, a| {
            let task = w.orchestrator().cancel(&a.task_id)?;
            Ok(Outcome::with_data(
                format!("task {} is {}", task.id, task.state),
                serde_json::to_value(&task).unwrap_or(Value::Null),
            ))
        },
    )?;

    Ok(())
}

// ---------------------------------------------------------------- projects

fn register_projects(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    add::<ProjectArgs, _, _>(
        reg,
        "project.plan",
        "Review a devd.yml manifest",
        "Parses and validates the project manifest and returns the commands a \
         person is being asked to approve, with a semantic hash. Nothing runs.",
        "project",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| Ok(Plan::new("project.plan", format!("review {}", a.project))),
        move |_ctx, a| {
            let value = w.projects().plan(&a.project)?;
            Ok(Outcome::with_data(
                format!(
                    "{} declares {} process(es)",
                    a.project,
                    value["processes"].as_array().map(|v| v.len()).unwrap_or(0)
                ),
                value,
            ))
        },
    )?;

    let w = ws.clone();
    add::<ProjectArgs, _, _>(
        reg,
        "project.approve",
        "Approve a project's commands",
        "Records a human's approval of the current manifest hash. Starting any \
         process requires this approval, and editing devd.yml invalidates it. \
         Agents cannot approve executable configuration.",
        "project",
        Tier::Write,
        Effect::Config,
        false,
        true,
        false,
        |_, a| {
            Ok(Plan::new(
                "project.approve",
                format!("approve the commands in {}", a.project),
            )
            .step("record the manifest hash as approved")
            .data(json!({ "project": a.project })))
        },
        move |ctx, a| {
            let manifest = w.projects().approve(&a.project, &ctx.actor.label())?;
            Ok(Outcome::with_data(
                format!(
                    "approved {} process(es) in {}",
                    manifest.processes.len(),
                    manifest.project
                ),
                json!({ "hash": manifest.hash, "root": manifest.root }),
            ))
        },
    )?;

    let w = ws.clone();
    add::<ProjectArgs, _, _>(
        reg,
        "project.status",
        "Inspect project processes",
        "Returns the manifest's declared processes with live state, health, pid, \
         restart count and the last failure.",
        "project",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "project.status",
                format!("inspect processes in {}", a.project),
            ))
        },
        move |_ctx, a| {
            let value = w.projects().status(&a.project)?;
            Ok(Outcome::with_data(
                format!("status of {}", a.project),
                value,
            ))
        },
    )?;

    let w = ws.clone();
    add::<ProcessArgs, _, _>(
        reg,
        "project.start",
        "Start project processes",
        "Starts the named process, or every declared process in dependency order \
         when `all` is true. Requires a current approval for the manifest.",
        "project",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |_, a| {
            let what = if a.all.unwrap_or(false) {
                "all processes".to_string()
            } else {
                a.name.clone()
            };
            Ok(
                Plan::new("project.start", format!("start {what} in {}", a.project))
                    .step("spawn each argv array as a supervised child")
                    .effect("project processes start"),
            )
        },
        move |_ctx, a| {
            let value = if a.all.unwrap_or(false) {
                w.projects().start_all(&a.project)?
            } else {
                w.projects().start(&a.project, &a.name)?
            };
            Ok(Outcome::with_data(format!("started {}", a.name), value))
        },
    )?;

    let w = ws.clone();
    add::<ProcessArgs, _, _>(
        reg,
        "project.stop",
        "Stop project processes",
        "Stops the named process tree, or every process in the project when \
         `all` is true.",
        "project",
        Tier::Exec,
        Effect::Process,
        true,
        false,
        false,
        |_, a| {
            Ok(
                Plan::new("project.stop", format!("stop processes in {}", a.project))
                    .effect("project processes and their children are terminated"),
            )
        },
        move |_ctx, a| {
            let value = if a.all.unwrap_or(false) {
                w.projects().stop_all(&a.project)?
            } else {
                w.projects().stop(&a.project, &a.name)?
            };
            Ok(Outcome::with_data(format!("stopped {}", a.name), value))
        },
    )?;

    let w = ws.clone();
    add::<ProcessArgs, _, _>(
        reg,
        "project.restart",
        "Restart a project process",
        "Stops and starts one declared process, resetting its health state.",
        "project",
        Tier::Exec,
        Effect::Process,
        true,
        false,
        true,
        |_, a| {
            Ok(Plan::new(
                "project.restart",
                format!("restart {} in {}", a.name, a.project),
            )
            .effect("the process tree is replaced"))
        },
        move |_ctx, a| {
            let value = w.projects().restart(&a.project, &a.name)?;
            Ok(Outcome::with_data(format!("restarted {}", a.name), value))
        },
    )?;

    let w = ws;
    add::<LogsArgs, _, _>(
        reg,
        "project.logs",
        "Read project process logs",
        "Returns the tail of a process's combined stdout and stderr, optionally \
         filtered. Values declared as secret environment variables are redacted.",
        "project",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| Ok(Plan::new("project.logs", format!("read {} logs", a.name))),
        move |_ctx, a| {
            let value = w.projects().logs(
                &a.project,
                &a.name,
                a.lines.unwrap_or(200),
                a.query.as_deref(),
            )?;
            Ok(Outcome::with_data(format!("logs for {}", a.name), value))
        },
    )?;

    Ok(())
}

// -------------------------------------------------------------- scratchpads

fn register_scratchpads(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    add::<ScratchListArgs, _, _>(
        reg,
        "scratch.list",
        "List shared scratchpads",
        "Lists project-scoped notes. Scratchpads are shared project state, not \
         chat history: they survive sessions, reloads and restarts.",
        "scratch",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "scratch.list",
                format!("list notes in {}", a.project),
            ))
        },
        move |_ctx, a| {
            let project = scope(&a.project)?;
            let rows = w.store().list_scratchpads(&project, a.task_id.as_deref())?;
            let summary: Vec<Value> = rows.iter().map(scratch_summary).collect();
            Ok(Outcome::with_data(
                format!("{} note(s)", rows.len()),
                Value::Array(summary),
            ))
        },
    )?;

    let w = ws.clone();
    add::<ScratchArgs, _, _>(
        reg,
        "scratch.get",
        "Read a shared scratchpad",
        "Returns the note content and its revision. Pass the revision to \
         scratch.update so a concurrent edit cannot be silently overwritten.",
        "scratch",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| Ok(Plan::new("scratch.get", format!("read note `{}`", a.name))),
        move |_ctx, a| {
            let project = scope(&a.project)?;
            let note = w
                .store()
                .get_scratchpad(&project, &a.name)?
                .ok_or_else(|| DevdError::not_found(format!("no scratchpad `{}`", a.name)))?;
            Ok(Outcome::with_data(
                format!("read `{}`", note.name),
                serde_json::to_value(&note).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<ScratchCreateArgs, _, _>(
        reg,
        "scratch.create",
        "Create a shared scratchpad",
        "Creates a named, project-scoped note. Content is Markdown. Names are \
         unique within a project.",
        "scratch",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| {
            Ok(
                Plan::new("scratch.create", format!("create note `{}`", a.name))
                    .data(json!({ "project": a.project, "name": a.name })),
            )
        },
        move |ctx, a| {
            let project = scope(&a.project)?;
            validate_note_name(&a.name)?;
            if w.store().get_scratchpad(&project, &a.name)?.is_some() {
                return Err(DevdError::conflict(format!(
                    "a scratchpad named `{}` already exists in this project",
                    a.name
                )));
            }
            let now = util::now();
            let note = Scratchpad {
                id: util::new_id(),
                project,
                task_id: a.task_id.clone(),
                name: a.name.clone(),
                content: a.content.clone().unwrap_or_default(),
                revision: 1,
                author: ctx.actor.label(),
                created_at: now.clone(),
                updated_at: now,
            };
            w.store().insert_scratchpad(&note)?;
            Ok(Outcome::with_data(
                format!("created `{}`", note.name),
                serde_json::to_value(&note).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<ScratchUpdateArgs, _, _>(
        reg,
        "scratch.update",
        "Update a shared scratchpad",
        "Replaces the content if `expected_revision` is current. A mismatch is a \
         conflict: read the note, merge, and try again.",
        "scratch",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| {
            Ok(
                Plan::new("scratch.update", format!("update note `{}`", a.name))
                    .step(format!("require revision {}", a.expected_revision))
                    .data(json!({ "bytes": a.content.len() })),
            )
        },
        move |ctx, a| {
            let project = scope(&a.project)?;
            let note = w
                .store()
                .get_scratchpad(&project, &a.name)?
                .ok_or_else(|| DevdError::not_found(format!("no scratchpad `{}`", a.name)))?;
            let revision = w.store().update_scratchpad(
                &note.id,
                &a.content,
                a.expected_revision,
                &ctx.actor.label(),
            )?;
            Ok(Outcome::with_data(
                format!("updated `{}` to revision {revision}", a.name),
                json!({ "id": note.id, "revision": revision }),
            ))
        },
    )?;

    let w = ws.clone();
    add::<ScratchAppendArgs, _, _>(
        reg,
        "scratch.append",
        "Append to a shared scratchpad",
        "Adds a section to an existing note, creating it if needed. Useful for \
         an agent recording findings without replacing a human's notes.",
        "scratch",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| {
            Ok(Plan::new(
                "scratch.append",
                format!("append to `{}`", a.name),
            ))
        },
        move |ctx, a| {
            let project = scope(&a.project)?;
            validate_note_name(&a.name)?;
            if let Some(note) = w.store().get_scratchpad(&project, &a.name)? {
                let addition = format!("\n\n---\n\n{}\n", a.content);
                let revision = w.store().update_scratchpad(
                    &note.id,
                    &format!("{}{}", note.content, addition),
                    note.revision,
                    &ctx.actor.label(),
                )?;
                Ok(Outcome::with_data(
                    format!("appended to `{}` at revision {revision}", a.name),
                    json!({ "id": note.id, "revision": revision }),
                ))
            } else {
                let now = util::now();
                let note = Scratchpad {
                    id: util::new_id(),
                    project,
                    task_id: a.task_id.clone(),
                    name: a.name.clone(),
                    content: a.content.clone(),
                    revision: 1,
                    author: ctx.actor.label(),
                    created_at: now.clone(),
                    updated_at: now,
                };
                w.store().insert_scratchpad(&note)?;
                Ok(Outcome::with_data(
                    format!("created `{}`", note.name),
                    serde_json::to_value(&note).unwrap_or(Value::Null),
                ))
            }
        },
    )?;

    let w = ws;
    add::<ScratchArgs, _, _>(
        reg,
        "scratch.delete",
        "Delete a shared scratchpad",
        "Deletes a project note. Destructive: it requires human approval and is \
         never performed unattended by an agent.",
        "scratch",
        Tier::Destructive,
        Effect::Data,
        false,
        true,
        true,
        |_, a| {
            Ok(
                Plan::new("scratch.delete", format!("delete note `{}`", a.name))
                    .destructive(true)
                    .step("the note and its content are removed"),
            )
        },
        move |ctx, a| {
            ctx.require_human("deleting a scratchpad")?;
            let project = scope(&a.project)?;
            if !w.store().delete_scratchpad(&project, &a.name)? {
                return Err(DevdError::not_found(format!("no scratchpad `{}`", a.name)));
            }
            Ok(Outcome::ok(format!("deleted `{}`", a.name)))
        },
    )?;

    Ok(())
}

// -------------------------------------------------------------------- todos

fn register_todos(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    add::<TodoListArgs, _, _>(
        reg,
        "todo.list",
        "List shared todos",
        "Lists structured, project-scoped todos ordered by priority and age, \
         optionally filtered by task or status.",
        "todo",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "todo.list",
                format!("list todos in {}", a.project),
            ))
        },
        move |_ctx, a| {
            let project = scope(&a.project)?;
            let rows = w
                .store()
                .list_todos(&project, a.task_id.as_deref(), a.status.as_deref())?;
            Ok(Outcome::with_data(
                format!("{} todo(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoArgs, _, _>(
        reg,
        "todo.get",
        "Read a todo",
        "Returns one todo and its revision.",
        "todo",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| Ok(Plan::new("todo.get", format!("read todo {}", a.id))),
        move |_ctx, a| {
            let todo = w
                .store()
                .get_todo(&a.id)?
                .ok_or_else(|| DevdError::not_found(format!("no todo `{}`", a.id)))?;
            Ok(Outcome::with_data(
                format!("todo {} is {}", todo.id, todo.status),
                serde_json::to_value(&todo).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoCreateArgs, _, _>(
        reg,
        "todo.create",
        "Create a todo",
        "Creates a structured todo with a stable id, priority, optional assignee \
         and task scope. Todos are project state, not conversation history.",
        "todo",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| {
            Ok(
                Plan::new("todo.create", format!("create todo `{}`", a.title))
                    .data(json!({ "project": a.project })),
            )
        },
        move |ctx, a| {
            let project = scope(&a.project)?;
            validate_title(&a.title)?;
            let status = "open".to_string();
            let priority = a.priority.clone().unwrap_or_else(|| "normal".into());
            validate_priority(&priority)?;
            let dependencies = a.dependencies.clone().unwrap_or_default();
            for dep in &dependencies {
                if w.store().get_todo(dep)?.is_none() {
                    return Err(DevdError::invalid(format!(
                        "dependency `{dep}` does not exist"
                    )));
                }
            }
            let now = util::now();
            let todo = Todo {
                id: util::new_id(),
                project,
                task_id: a.task_id.clone(),
                title: a.title.clone(),
                description: a.description.clone().unwrap_or_default(),
                status,
                priority,
                assignee: a.assignee.clone(),
                origin: Some(ctx.actor.label()),
                dependencies: serde_json::to_string(&dependencies).unwrap_or_else(|_| "[]".into()),
                revision: 1,
                created_at: now.clone(),
                updated_at: now,
                completed_at: None,
            };
            w.store().insert_todo(&todo)?;
            Ok(Outcome::with_data(
                format!("created todo {}", todo.id),
                serde_json::to_value(&todo).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoUpdateArgs, _, _>(
        reg,
        "todo.update",
        "Update a todo",
        "Updates the fields supplied, guarded by `expected_revision`. Status \
         changes are recorded in the todo's history.",
        "todo",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| {
            Ok(Plan::new("todo.update", format!("update todo {}", a.id))
                .step(format!("require revision {}", a.expected_revision)))
        },
        move |ctx, a| {
            if let Some(status) = &a.status {
                validate_status(status)?;
            }
            if let Some(priority) = &a.priority {
                validate_priority(priority)?;
            }
            let todo = w.store().update_todo(
                &a.id,
                a.expected_revision,
                &ctx.actor.label(),
                a.title.as_deref(),
                a.description.as_deref(),
                a.status.as_deref(),
                a.priority.as_deref(),
                a.assignee.as_ref().map(|s| Some(s.as_str())),
                a.note.as_deref().unwrap_or(""),
            )?;
            Ok(Outcome::with_data(
                format!("todo {} is {}", todo.id, todo.status),
                serde_json::to_value(&todo).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoCompleteArgs, _, _>(
        reg,
        "todo.complete",
        "Complete a todo",
        "Moves a todo to `done`, recording the transition and optional note.",
        "todo",
        Tier::Write,
        Effect::Write,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "todo.complete",
                format!("complete todo {}", a.id),
            ))
        },
        move |ctx, a| {
            let todo = w.store().update_todo(
                &a.id,
                a.expected_revision,
                &ctx.actor.label(),
                None,
                None,
                Some("done"),
                None,
                None,
                a.note.as_deref().unwrap_or("completed"),
            )?;
            Ok(Outcome::with_data(
                format!("completed {}", todo.id),
                serde_json::to_value(&todo).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoReopenArgs, _, _>(
        reg,
        "todo.reopen",
        "Reopen a todo",
        "Moves a completed or cancelled todo back to `open`.",
        "todo",
        Tier::Write,
        Effect::Write,
        true,
        true,
        true,
        |_, a| Ok(Plan::new("todo.reopen", format!("reopen todo {}", a.id))),
        move |ctx, a| {
            let todo = w.store().update_todo(
                &a.id,
                a.expected_revision,
                &ctx.actor.label(),
                None,
                None,
                Some("open"),
                None,
                None,
                a.note.as_deref().unwrap_or("reopened"),
            )?;
            Ok(Outcome::with_data(
                format!("reopened {}", todo.id),
                serde_json::to_value(&todo).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws.clone();
    add::<TodoNoteArgs, _, _>(
        reg,
        "todo.note",
        "Record a note on a todo",
        "Adds a history entry without changing the todo's status.",
        "todo",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |_, a| Ok(Plan::new("todo.note", format!("note on todo {}", a.id))),
        move |ctx, a| {
            w.store().note_todo(&a.id, &ctx.actor.label(), &a.note)?;
            Ok(Outcome::ok(format!("noted on {}", a.id)))
        },
    )?;

    let w = ws.clone();
    add::<TodoHistoryArgs, _, _>(
        reg,
        "todo.history",
        "Read todo history",
        "Returns the recorded status transitions and notes, newest first.",
        "todo",
        Tier::Read,
        Effect::Read,
        true,
        true,
        true,
        |_, a| {
            Ok(Plan::new(
                "todo.history",
                format!("history of todo {}", a.id),
            ))
        },
        move |_ctx, a| {
            let rows = w.store().list_todo_events(&a.id, a.limit.unwrap_or(50))?;
            Ok(Outcome::with_data(
                format!("{} event(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            ))
        },
    )?;

    let w = ws;
    add::<TodoArgs, _, _>(
        reg,
        "todo.delete",
        "Delete a todo",
        "Deletes a todo and its history. Destructive: requires human approval \
         and is never performed unattended by an agent.",
        "todo",
        Tier::Destructive,
        Effect::Data,
        false,
        true,
        true,
        |_, a| {
            Ok(Plan::new("todo.delete", format!("delete todo {}", a.id))
                .destructive(true)
                .step("the todo and its history are removed"))
        },
        move |ctx, a| {
            ctx.require_human("deleting a todo")?;
            if !w.store().delete_todo(&a.id)? {
                return Err(DevdError::not_found(format!("no todo `{}`", a.id)));
            }
            Ok(Outcome::ok(format!("deleted {}", a.id)))
        },
    )?;

    Ok(())
}

// ------------------------------------------------------------------ helpers

/// Canonicalise a project path so two spellings cannot create two scopes.
fn scope(path: &str) -> Result<String> {
    let canonical = std::path::Path::new(path)
        .canonicalize()
        .map_err(|e| DevdError::invalid(format!("project path is not usable: {e}")))?;
    let canonical = util::strip_verbatim(&canonical);
    if !canonical.is_dir() {
        return Err(DevdError::invalid("the project path is not a directory"));
    }
    Ok(canonical.to_string_lossy().to_string())
}

fn scratch_summary(note: &Scratchpad) -> Value {
    let preview: String = note.content.chars().take(240).collect();
    json!({
        "id": note.id,
        "project": note.project,
        "task_id": note.task_id,
        "name": note.name,
        "revision": note.revision,
        "author": note.author,
        "created_at": note.created_at,
        "updated_at": note.updated_at,
        "bytes": note.content.len(),
        "preview": preview,
    })
}

fn guard_agent_session(ctx: &Ctx, store: &devd_core::Store, id: &str) -> Result<()> {
    if !ctx.actor.is_agent() {
        return Ok(());
    }
    let session = store
        .get_session(id)?
        .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
    if session.task_id.is_none() {
        return Err(DevdError::denied(
            "an agent may only stop a session it delegated, not a person's interactive session",
        ));
    }
    Ok(())
}

fn validate_note_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(DevdError::invalid(
            "a scratchpad name must be between 1 and 80 characters",
        ));
    }
    if name.starts_with('.') || name.contains(['/', '\\']) || name.chars().any(|c| c.is_control()) {
        return Err(DevdError::invalid(
            "a scratchpad name may not contain slashes or control characters",
        ));
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<()> {
    if title.trim().is_empty() || title.len() > 200 {
        return Err(DevdError::invalid(
            "a todo title must be between 1 and 200 characters",
        ));
    }
    Ok(())
}

fn validate_status(status: &str) -> Result<()> {
    match status {
        "open" | "in_progress" | "blocked" | "done" | "cancelled" => Ok(()),
        other => Err(DevdError::invalid(format!(
            "unknown todo status `{other}`; expected open, in_progress, blocked, done or cancelled"
        ))),
    }
}

fn validate_priority(priority: &str) -> Result<()> {
    match priority {
        "low" | "normal" | "high" | "urgent" => Ok(()),
        other => Err(DevdError::invalid(format!(
            "unknown priority `{other}`; expected low, normal, high or urgent"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn add<A, P, Q>(
    reg: &mut Registry,
    id: &str,
    summary: &str,
    description: &str,
    group: &str,
    tier: Tier,
    effect: Effect,
    agent_safe: bool,
    dry_run: bool,
    idempotent: bool,
    plan: P,
    apply: Q,
) -> Result<()>
where
    A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
    P: Fn(&Ctx, &A) -> Result<Plan> + Send + Sync + 'static,
    Q: Fn(&Ctx, &A) -> Result<Outcome> + Send + Sync + 'static,
{
    let cap = Capability::builder(id, summary)
        .description(description)
        .group(group)
        .tier(tier)
        .effect(effect)
        .agent_safe(agent_safe)
        .dry_run(dry_run)
        .idempotent(idempotent)
        .args_schema(
            serde_json::to_value(schemars::schema_for!(A))
                .map_err(|e| DevdError::internal(format!("cannot build schema for {id}: {e}")))?,
        )
        .handler(
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                plan(ctx, &typed)
            },
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                apply(ctx, &typed)
            },
        )
        .build()?;
    reg.register(cap)
}

// Keep the manifest module reachable for capability tests that need the schema.

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::capability::{Approval, InvokeRequest, Surface};
    use devd_core::{Actor, Ctx, EventBus, Paths, Settings, Store};

    struct Fixture {
        _dir: tempfile::TempDir,
        project: String,
    }

    fn fixture() -> (Fixture, Registry, Arc<Workspace>, Ctx) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.state_db()).unwrap());
        store.migrate().unwrap();
        let workspace = Workspace::new(store.clone(), Arc::new(paths.clone())).unwrap();
        let mut registry = Registry::new();
        register(&mut registry, workspace.clone()).unwrap();
        let ctx = Ctx {
            store,
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Http),
            dry_run: false,
        };
        let project = project
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        (Fixture { _dir: dir, project }, registry, workspace, ctx)
    }

    fn invoke(reg: &Registry, ctx: &Ctx, id: &str, args: Value) -> Value {
        let req = InvokeRequest {
            id: id.to_string(),
            args,
            dry_run: false,
        };
        let result = reg
            .dispatch(ctx, &req, Approval::None)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        result.outcome.unwrap().data
    }

    #[test]
    fn scratchpad_capabilities_detect_stale_writes() {
        let (f, reg, _ws, ctx) = fixture();
        let note = invoke(
            &reg,
            &ctx,
            "scratch.create",
            json!({
                "project": f.project, "name": "findings", "content": "one"
            }),
        );
        assert_eq!(note["revision"], 1);
        let updated = invoke(
            &reg,
            &ctx,
            "scratch.update",
            json!({
                "project": f.project, "name": "findings", "content": "two", "expected_revision": 1
            }),
        );
        assert_eq!(updated["revision"], 2);
        let req = InvokeRequest {
            id: "scratch.update".into(),
            args: json!({
                "project": f.project, "name": "findings", "content": "lost", "expected_revision": 1
            }),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::None).unwrap_err();
        assert!(matches!(err, DevdError::Conflict(_)), "{err:?}");
    }

    #[test]
    fn todo_capabilities_track_status_and_history() {
        let (f, reg, _ws, ctx) = fixture();
        let todo = invoke(
            &reg,
            &ctx,
            "todo.create",
            json!({
                "project": f.project, "title": "ship it", "priority": "high"
            }),
        );
        let id = todo["id"].as_str().unwrap();
        let done = invoke(
            &reg,
            &ctx,
            "todo.complete",
            json!({
                "id": id, "expected_revision": 1
            }),
        );
        assert_eq!(done["status"], "done");
        let history = invoke(&reg, &ctx, "todo.history", json!({ "id": id }));
        assert!(history.as_array().unwrap().len() >= 2);
        let listed = invoke(&reg, &ctx, "todo.list", json!({ "project": f.project }));
        assert_eq!(listed[0]["status"], "done");
    }

    #[test]
    fn an_agent_cannot_write_to_a_persons_terminal() {
        let (f, reg, ws, mut ctx) = fixture();
        let view = ws
            .sessions()
            .start(crate::StartOptions {
                harness: "custom".into(),
                cwd: f.project.clone(),
                command: Some(if cfg!(windows) {
                    vec![
                        "cmd.exe".into(),
                        "/c".into(),
                        "ping -n 10 127.0.0.1 > NUL".into(),
                    ]
                } else {
                    vec!["/bin/sh".into(), "-c".into(), "sleep 10".into()]
                }),
                ..Default::default()
            })
            .unwrap();
        ctx.actor = Actor::agent("test-agent");
        let req = InvokeRequest {
            id: "session.input".into(),
            args: json!({ "id": view.session.id, "data": "x" }),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::None).unwrap_err();
        assert!(matches!(err, DevdError::Denied(_)), "{err:?}");

        // Stopping someone else's interactive session is refused too.
        let req = InvokeRequest {
            id: "session.stop".into(),
            args: json!({ "id": view.session.id }),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::None).unwrap_err();
        assert!(matches!(err, DevdError::Denied(_)), "{err:?}");
        ws.sessions().stop(&view.session.id).unwrap();
    }

    #[test]
    fn only_a_human_can_approve_executable_configuration() {
        let (f, reg, _ws, mut ctx) = fixture();
        std::fs::write(
            std::path::Path::new(&f.project).join("devd.yml"),
            "version: 1\nproject:\n  name: app\nprocesses:\n  web:\n    command: [\"echo\", \"hi\"]\n",
        )
        .unwrap();
        ctx.actor = Actor::agent("test-agent");
        let req = InvokeRequest {
            id: "project.approve".into(),
            args: json!({ "project": f.project }),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::None).unwrap_err();
        assert!(matches!(err, DevdError::Denied(_)), "{err:?}");

        ctx.actor = Actor::human(Surface::Http);
        invoke(
            &reg,
            &ctx,
            "project.approve",
            json!({ "project": f.project }),
        );
        let plan = invoke(&reg, &ctx, "project.plan", json!({ "project": f.project }));
        assert_eq!(plan["approved"], true);
    }

    #[test]
    fn every_workspace_capability_has_a_schema_and_a_group() {
        let (_f, reg, _ws, _ctx) = fixture();
        for cap in reg.list() {
            assert!(
                cap.group == "session"
                    || cap.group == "agent"
                    || cap.group == "project"
                    || cap.group == "scratch"
                    || cap.group == "todo"
            );
            assert!(cap.args_schema.is_object(), "{} has no schema", cap.id);
        }
    }
}
