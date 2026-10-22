//! Capabilities for project processes and shared project state.
//!
//! These are the same declarations the CLI, the application window and the MCP
//! server see. The HTTP transport has a few direct endpoints for polling output
//! (auditing an 80ms poll would be noise, not security), but every *mutation* an
//! agent can make is declared here and goes through [`devd_core::Registry`].

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::error::{DevdError, Result};
use devd_core::store_work::{Scratchpad, Todo};
use devd_core::{util, Registry};

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
    register_projects(reg, workspace.clone())?;
    register_scratchpads(reg, workspace.clone())?;
    register_todos(reg, workspace)?;
    Ok(())
}

// ---------------------------------------------------------------- projects

fn register_projects(reg: &mut Registry, ws: Arc<Workspace>) -> Result<()> {
    let w = ws.clone();
    reg.define::<ProjectArgs>("project.plan", "Review a devd.yml manifest")
    .description("Parses and validates the project manifest and returns the commands a \
         person is being asked to approve, with a semantic hash. Nothing runs.")
    .group("project")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<ProjectArgs>("project.approve", "Approve a project's commands")
    .description("Records a human's approval of the current manifest hash. Starting any \
         process requires this approval, and editing devd.yml invalidates it. \
         Agents cannot approve executable configuration.")
    .group("project")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .dry_run()
    .handlers(
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
    reg.define::<ProjectArgs>("project.status", "Inspect project processes")
    .description("Returns the manifest's declared processes with live state, health, pid, \
         restart count and the last failure.")
    .group("project")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<ProcessArgs>("project.start", "Start project processes")
    .description("Starts the named process, or every declared process in dependency order \
         when `all` is true. Requires a current approval for the manifest.")
    .group("project")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<ProcessArgs>("project.stop", "Stop project processes")
    .description("Stops the named process tree, or every process in the project when \
         `all` is true.")
    .group("project")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .handlers(
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
    reg.define::<ProcessArgs>("project.restart", "Restart a project process")
    .description("Stops and starts one declared process, resetting its health state.")
    .group("project")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .idempotent()
    .handlers(
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
    reg.define::<LogsArgs>("project.logs", "Read project process logs")
    .description("Returns the tail of a process's combined stdout and stderr, optionally \
         filtered. Values declared as secret environment variables are redacted.")
    .group("project")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<ScratchListArgs>("scratch.list", "List shared scratchpads")
    .description("Lists project-scoped notes. Scratchpads are shared project state, not \
         chat history: they survive sessions, reloads and restarts.")
    .group("scratch")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<ScratchArgs>("scratch.get", "Read a shared scratchpad")
    .description("Returns the note content and its revision. Pass the revision to \
         scratch.update so a concurrent edit cannot be silently overwritten.")
    .group("scratch")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<ScratchCreateArgs>("scratch.create", "Create a shared scratchpad")
    .description("Creates a named, project-scoped note. Content is Markdown. Names are \
         unique within a project.")
    .group("scratch")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<ScratchUpdateArgs>("scratch.update", "Update a shared scratchpad")
    .description("Replaces the content if `expected_revision` is current. A mismatch is a \
         conflict: read the note, merge, and try again.")
    .group("scratch")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<ScratchAppendArgs>("scratch.append", "Append to a shared scratchpad")
    .description("Adds a section to an existing note, creating it if needed. Useful for \
         an agent recording findings without replacing a human's notes.")
    .group("scratch")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<ScratchArgs>("scratch.delete", "Delete a shared scratchpad")
    .description("Deletes a project note. Destructive: it requires human approval and is \
         never performed unattended by an agent.")
    .group("scratch")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoListArgs>("todo.list", "List shared todos")
    .description("Lists structured, project-scoped todos ordered by priority and age, \
         optionally filtered by task or status.")
    .group("todo")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoArgs>("todo.get", "Read a todo")
    .description("Returns one todo and its revision.")
    .group("todo")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoCreateArgs>("todo.create", "Create a todo")
    .description("Creates a structured todo with a stable id, priority, optional assignee \
         and task scope. Todos are project state, not conversation history.")
    .group("todo")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<TodoUpdateArgs>("todo.update", "Update a todo")
    .description("Updates the fields supplied, guarded by `expected_revision`. Status \
         changes are recorded in the todo's history.")
    .group("todo")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
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
    reg.define::<TodoCompleteArgs>("todo.complete", "Complete a todo")
    .description("Moves a todo to `done`, recording the transition and optional note.")
    .group("todo")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoReopenArgs>("todo.reopen", "Reopen a todo")
    .description("Moves a completed or cancelled todo back to `open`.")
    .group("todo")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoNoteArgs>("todo.note", "Record a note on a todo")
    .description("Adds a history entry without changing the todo's status.")
    .group("todo")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("todo.note", format!("note on todo {}", a.id))),
        move |ctx, a| {
            w.store().note_todo(&a.id, &ctx.actor.label(), &a.note)?;
            Ok(Outcome::ok(format!("noted on {}", a.id)))
        },
    )?;

    let w = ws.clone();
    reg.define::<TodoHistoryArgs>("todo.history", "Read todo history")
    .description("Returns the recorded status transitions and notes, newest first.")
    .group("todo")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
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
    reg.define::<TodoArgs>("todo.delete", "Delete a todo")
    .description("Deletes a todo and its history. Destructive: requires human approval \
         and is never performed unattended by an agent.")
    .group("todo")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .idempotent()
    .handlers(
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
                cap.group == "project"
                    || cap.group == "scratch"
                    || cap.group == "todo"
            );
            assert!(cap.args_schema.is_object(), "{} has no schema", cap.id);
        }
    }
}
