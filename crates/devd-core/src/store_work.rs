//! Durable work state: project processes, shared notes and todos.
//!
//! These methods are deliberately on [`Store`] rather than in the runtime
//! crates. The database is the source of truth for anything that has to survive
//! a process, and a runtime that kept its own private format would be a second
//! truth. The runtime decides *what* a state means; the store only writes it.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{DevdError, Result};
use crate::state::Store;
use crate::util;

/// A declared project process and its last known state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProcess {
    /// Row identifier.
    pub id: String,
    /// Absolute project path this belongs to.
    pub project: String,
    /// Process name from `devd.yml`.
    pub name: String,
    /// Command argv, JSON array.
    pub command: String,
    /// Absolute working directory.
    pub working_dir: String,
    /// Explicit environment, JSON object.
    pub env: String,
    /// Health specification, JSON object.
    pub health: Option<String>,
    /// `never`, `on-failure` or `always`.
    pub restart: String,
    /// Dependency names, JSON array.
    pub depends_on: String,
    /// `stopped`, `starting`, `running`, `failed` or `stopping`.
    pub status: String,
    /// `unknown`, `healthy`, `unhealthy` or `none`.
    pub health_status: String,
    /// Why health last failed.
    pub health_error: Option<String>,
    /// Current pid.
    pub pid: Option<u32>,
    /// devd process that supervises this row.
    pub owner_pid: u32,
    /// `started` or `stopped`.
    pub desired_state: String,
    /// How many times the supervisor restarted it.
    pub restart_count: i64,
    /// Exit code of the previous run.
    pub last_exit_code: Option<i32>,
    /// Last failure detail.
    pub last_error: Option<String>,
    /// Start of the current run.
    pub started_at: Option<String>,
    /// Last update.
    pub updated_at: String,
}

/// A project-scoped shared note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scratchpad {
    /// Stable identifier.
    pub id: String,
    /// Project scope, absolute path.
    pub project: String,
    /// Optional task scope.
    pub task_id: Option<String>,
    /// Unique name within the project.
    pub name: String,
    /// Markdown content.
    pub content: String,
    /// Optimistic-concurrency revision.
    pub revision: i64,
    /// Last author (`human:http`, `agent:claude-code`, ...).
    pub author: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
}

/// A structured shared todo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    /// Stable identifier.
    pub id: String,
    /// Project scope, absolute path.
    pub project: String,
    /// Optional task scope.
    pub task_id: Option<String>,
    /// Title.
    pub title: String,
    /// Details.
    pub description: String,
    /// `open`, `in_progress`, `blocked`, `done` or `cancelled`.
    pub status: String,
    /// `low`, `normal`, `high` or `urgent`.
    pub priority: String,
    /// Optional assignee (`agent:opencode`, a person, ...).
    pub assignee: Option<String>,
    /// Who created it.
    pub origin: Option<String>,
    /// Dependency todo ids, JSON array.
    pub dependencies: String,
    /// Optimistic-concurrency revision.
    pub revision: i64,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update.
    pub updated_at: String,
    /// Completion timestamp.
    pub completed_at: Option<String>,
}

/// One recorded todo transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoEvent {
    /// Row identifier.
    pub id: i64,
    /// Todo it belongs to.
    pub todo_id: String,
    /// Timestamp.
    pub at: String,
    /// Who changed it.
    pub actor: String,
    /// Previous status.
    pub from_status: Option<String>,
    /// New status.
    pub to_status: String,
    /// Optional note.
    pub note: String,
}

const PROCESS_COLUMNS: &str = "id, project, name, command, working_dir, env, health, restart, \
     depends_on, status, health_status, health_error, pid, owner_pid, desired_state, restart_count, \
     last_exit_code, last_error, started_at, updated_at";

fn map_process(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectProcess> {
    Ok(ProjectProcess {
        id: row.get(0)?,
        project: row.get(1)?,
        name: row.get(2)?,
        command: row.get(3)?,
        working_dir: row.get(4)?,
        env: row.get(5)?,
        health: row.get(6)?,
        restart: row.get(7)?,
        depends_on: row.get(8)?,
        status: row.get(9)?,
        health_status: row.get(10)?,
        health_error: row.get(11)?,
        pid: row.get(12)?,
        owner_pid: row.get(13)?,
        desired_state: row.get(14)?,
        restart_count: row.get(15)?,
        last_exit_code: row.get(16)?,
        last_error: row.get(17)?,
        started_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

impl Store {
    // ------------------------------------------------------------- processes

    /// Insert or update a declared project process.
    pub fn upsert_project_process(&self, p: &ProjectProcess) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO project_processes
                 (id, project, name, command, working_dir, env, health, restart, depends_on,
                  status, health_status, health_error, pid, owner_pid, desired_state, restart_count,
                  last_exit_code, last_error, started_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                         ?16, ?17, ?18, ?19, ?20)
                 ON CONFLICT(project, name) DO UPDATE SET
                   command = excluded.command,
                   working_dir = excluded.working_dir,
                   env = excluded.env,
                   health = excluded.health,
                   restart = excluded.restart,
                   depends_on = excluded.depends_on,
                   updated_at = excluded.updated_at",
                params![
                    p.id,
                    p.project,
                    p.name,
                    p.command,
                    p.working_dir,
                    p.env,
                    p.health,
                    p.restart,
                    p.depends_on,
                    p.status,
                    p.health_status,
                    p.health_error,
                    p.pid,
                    p.owner_pid,
                    p.desired_state,
                    p.restart_count,
                    p.last_exit_code,
                    p.last_error,
                    p.started_at,
                    p.updated_at
                ],
            )?;
            Ok(())
        })
    }

    /// Update the observed state of a declared process.
    #[allow(clippy::too_many_arguments)]
    pub fn update_project_process(
        &self,
        project: &str,
        name: &str,
        status: &str,
        health_status: &str,
        pid: Option<u32>,
        exit_code: Option<i32>,
        error: Option<&str>,
        started_at: Option<&str>,
        desired: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE project_processes SET status = ?3, health_status = ?4, pid = ?5,
                   last_exit_code = COALESCE(?6, last_exit_code), last_error = ?7,
                   started_at = COALESCE(?8, started_at),
                   desired_state = COALESCE(?9, desired_state), updated_at = ?10
                 WHERE project = ?1 AND name = ?2",
                params![
                    project,
                    name,
                    status,
                    health_status,
                    pid,
                    exit_code,
                    error,
                    started_at,
                    desired,
                    util::now()
                ],
            )?;
            Ok(())
        })
    }

    /// Record which devd process supervises a project process.
    pub fn set_process_owner(&self, project: &str, name: &str, owner_pid: u32) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE project_processes SET owner_pid = ?3, updated_at = ?4
                 WHERE project = ?1 AND name = ?2",
                params![project, name, owner_pid, util::now()],
            )?;
            Ok(())
        })
    }

    /// Record a health result without changing the lifecycle state.
    pub fn set_process_health(
        &self,
        project: &str,
        name: &str,
        health_status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE project_processes SET health_status = ?3, health_error = ?4,
                 updated_at = ?5 WHERE project = ?1 AND name = ?2",
                params![project, name, health_status, error, util::now()],
            )?;
            Ok(())
        })
    }

    /// Bump the restart counter.
    pub fn bump_process_restarts(&self, project: &str, name: &str) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "UPDATE project_processes SET restart_count = restart_count + 1,
                 updated_at = ?3 WHERE project = ?1 AND name = ?2",
                params![project, name, util::now()],
            )?;
            let n: i64 = conn.query_row(
                "SELECT restart_count FROM project_processes WHERE project = ?1 AND name = ?2",
                params![project, name],
                |row| row.get(0),
            )?;
            Ok(n)
        })
    }

    /// Read one declared process.
    pub fn get_project_process(&self, project: &str, name: &str) -> Result<Option<ProjectProcess>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {PROCESS_COLUMNS} FROM project_processes
                         WHERE project = ?1 AND name = ?2"
                    ),
                    params![project, name],
                    map_process,
                )
                .optional()?)
        })
    }

    /// List declared processes for a project.
    pub fn list_project_processes(&self, project: &str) -> Result<Vec<ProjectProcess>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {PROCESS_COLUMNS} FROM project_processes
                 WHERE project = ?1 ORDER BY name"
            ))?;
            let rows = stmt.query_map(params![project], map_process)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Record that a human approved a manifest hash.
    pub fn set_project_approval(
        &self,
        project: &str,
        manifest_hash: &str,
        actor: &str,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO project_approvals (project, manifest_hash, approved_by, approved_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(project) DO UPDATE SET
                   manifest_hash = excluded.manifest_hash,
                   approved_by = excluded.approved_by,
                   approved_at = excluded.approved_at",
                params![project, manifest_hash, actor, util::now()],
            )?;
            Ok(())
        })
    }

    /// The hash a human last approved for a project, and when.
    pub fn get_project_approval(&self, project: &str) -> Result<Option<(String, String, String)>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT manifest_hash, approved_by, approved_at
                     FROM project_approvals WHERE project = ?1",
                    params![project],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?)
        })
    }

    // ----------------------------------------------------------- scratchpads

    /// Insert a scratchpad.
    pub fn insert_scratchpad(&self, s: &Scratchpad) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO scratchpads
                 (id, project, task_id, name, content, revision, author, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    s.id,
                    s.project,
                    s.task_id,
                    s.name,
                    s.content,
                    s.revision,
                    s.author,
                    s.created_at,
                    s.updated_at
                ],
            )?;
            Ok(())
        })
    }

    /// Read a scratchpad by name.
    pub fn get_scratchpad(&self, project: &str, name: &str) -> Result<Option<Scratchpad>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, project, task_id, name, content, revision, author, created_at,
                            updated_at
                     FROM scratchpads WHERE project = ?1 AND name = ?2",
                    params![project, name],
                    map_scratchpad,
                )
                .optional()?)
        })
    }

    /// Read a scratchpad by id.
    pub fn get_scratchpad_by_id(&self, id: &str) -> Result<Option<Scratchpad>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, project, task_id, name, content, revision, author, created_at,
                            updated_at
                     FROM scratchpads WHERE id = ?1",
                    params![id],
                    map_scratchpad,
                )
                .optional()?)
        })
    }

    /// List scratchpads in a project.
    pub fn list_scratchpads(
        &self,
        project: &str,
        task_id: Option<&str>,
    ) -> Result<Vec<Scratchpad>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project, task_id, name, content, revision, author, created_at,
                        updated_at
                 FROM scratchpads
                 WHERE project = ?1 AND (?2 IS NULL OR task_id = ?2)
                 ORDER BY name",
            )?;
            let rows = stmt.query_map(params![project, task_id], map_scratchpad)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Update a scratchpad if the caller's revision is current.
    ///
    /// Returns the new revision. A stale revision is a conflict, not a silent
    /// overwrite: two agents editing the same note must not lose one another's
    /// work.
    pub fn update_scratchpad(
        &self,
        id: &str,
        content: &str,
        expected_revision: i64,
        author: &str,
    ) -> Result<i64> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE scratchpads SET content = ?2, revision = revision + 1, author = ?3,
                   updated_at = ?4
                 WHERE id = ?1 AND revision = ?5",
                params![id, content, author, util::now(), expected_revision],
            )?;
            if n == 0 {
                return Err(DevdError::conflict(
                    "this scratchpad changed since you read it; reload it and merge your edit",
                ));
            }
            let rev: i64 = conn.query_row(
                "SELECT revision FROM scratchpads WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            Ok(rev)
        })
    }

    /// Delete a scratchpad by name.
    pub fn delete_scratchpad(&self, project: &str, name: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "DELETE FROM scratchpads WHERE project = ?1 AND name = ?2",
                params![project, name],
            )?;
            Ok(n > 0)
        })
    }

    // ----------------------------------------------------------------- todos

    /// Insert a todo and its creation event.
    pub fn insert_todo(&self, t: &Todo) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO todos
                 (id, project, task_id, title, description, status, priority, assignee, origin,
                  dependencies, revision, created_at, updated_at, completed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    t.id,
                    t.project,
                    t.task_id,
                    t.title,
                    t.description,
                    t.status,
                    t.priority,
                    t.assignee,
                    t.origin,
                    t.dependencies,
                    t.revision,
                    t.created_at,
                    t.updated_at,
                    t.completed_at
                ],
            )?;
            conn.execute(
                "INSERT INTO todo_events (todo_id, at, actor, from_status, to_status, note)
                 VALUES (?1, ?2, ?3, NULL, ?4, 'created')",
                params![t.id, t.created_at, t.origin, t.status],
            )?;
            Ok(())
        })
    }

    /// Read a todo.
    pub fn get_todo(&self, id: &str) -> Result<Option<Todo>> {
        self.with(|conn| {
            Ok(conn
                .query_row(
                    "SELECT id, project, task_id, title, description, status, priority, assignee,
                            origin, dependencies, revision, created_at, updated_at, completed_at
                     FROM todos WHERE id = ?1",
                    params![id],
                    map_todo,
                )
                .optional()?)
        })
    }

    /// List todos in a project.
    pub fn list_todos(
        &self,
        project: &str,
        task_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<Todo>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, project, task_id, title, description, status, priority, assignee,
                        origin, dependencies, revision, created_at, updated_at, completed_at
                 FROM todos
                 WHERE project = ?1
                   AND (?2 IS NULL OR task_id = ?2)
                   AND (?3 IS NULL OR status = ?3)
                 ORDER BY
                   CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1
                                 WHEN 'normal' THEN 2 ELSE 3 END,
                   created_at",
            )?;
            let rows = stmt.query_map(params![project, task_id, status], map_todo)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Update a todo with optimistic concurrency, recording the transition.
    ///
    /// Only `Some` fields change. A revision mismatch is a conflict.
    pub fn update_todo(
        &self,
        id: &str,
        expected_revision: i64,
        actor: &str,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
        priority: Option<&str>,
        assignee: Option<Option<&str>>,
        note: &str,
    ) -> Result<Todo> {
        self.with(|conn| {
            let current: Todo = conn
                .query_row(
                    "SELECT id, project, task_id, title, description, status, priority, assignee,
                            origin, dependencies, revision, created_at, updated_at, completed_at
                     FROM todos WHERE id = ?1",
                    params![id],
                    map_todo,
                )
                .optional()?
                .ok_or_else(|| DevdError::not_found(format!("no todo `{id}`")))?;
            if current.revision != expected_revision {
                return Err(DevdError::conflict(format!(
                    "this todo is at revision {}; reload it before changing it",
                    current.revision
                )));
            }
            let next_status = status.unwrap_or(&current.status);
            let completed_at = if next_status == "done" {
                Some(util::now())
            } else if current.status == "done" {
                None
            } else {
                current.completed_at.clone()
            };
            conn.execute(
                "UPDATE todos SET title = ?2, description = ?3, status = ?4, priority = ?5,
                   assignee = ?6, revision = revision + 1, updated_at = ?7, completed_at = ?8
                 WHERE id = ?1 AND revision = ?9",
                params![
                    id,
                    title.unwrap_or(&current.title),
                    description.unwrap_or(&current.description),
                    next_status,
                    priority.unwrap_or(&current.priority),
                    assignee.unwrap_or(current.assignee.as_deref()),
                    util::now(),
                    completed_at,
                    expected_revision
                ],
            )?;
            if next_status != current.status {
                conn.execute(
                    "INSERT INTO todo_events (todo_id, at, actor, from_status, to_status, note)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, util::now(), actor, current.status, next_status, note],
                )?;
            }
            let updated = conn.query_row(
                "SELECT id, project, task_id, title, description, status, priority, assignee,
                        origin, dependencies, revision, created_at, updated_at, completed_at
                 FROM todos WHERE id = ?1",
                params![id],
                map_todo,
            )?;
            Ok(updated)
        })
    }

    /// Record a non-status note against a todo.
    pub fn note_todo(&self, id: &str, actor: &str, note: &str) -> Result<()> {
        self.with(|conn| {
            let status: String = conn
                .query_row(
                    "SELECT status FROM todos WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| DevdError::not_found(format!("no todo `{id}`")))?;
            conn.execute(
                "INSERT INTO todo_events (todo_id, at, actor, from_status, to_status, note)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                params![id, util::now(), actor, status, util::truncate(note, 2000)],
            )?;
            Ok(())
        })
    }

    /// Delete a todo and its events.
    pub fn delete_todo(&self, id: &str) -> Result<bool> {
        self.with(|conn| {
            conn.execute("DELETE FROM todo_events WHERE todo_id = ?1", params![id])?;
            let n = conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    /// The transition history of a todo, newest first.
    pub fn list_todo_events(&self, id: &str, limit: u32) -> Result<Vec<TodoEvent>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, todo_id, at, actor, from_status, to_status, note
                 FROM todo_events WHERE todo_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![id, limit], |row| {
                Ok(TodoEvent {
                    id: row.get(0)?,
                    todo_id: row.get(1)?,
                    at: row.get(2)?,
                    actor: row.get(3)?,
                    from_status: row.get(4)?,
                    to_status: row.get(5)?,
                    note: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }
}

fn map_scratchpad(row: &rusqlite::Row<'_>) -> rusqlite::Result<Scratchpad> {
    Ok(Scratchpad {
        id: row.get(0)?,
        project: row.get(1)?,
        task_id: row.get(2)?,
        name: row.get(3)?,
        content: row.get(4)?,
        revision: row.get(5)?,
        author: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn map_todo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: row.get(0)?,
        project: row.get(1)?,
        task_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        priority: row.get(6)?,
        assignee: row.get(7)?,
        origin: row.get(8)?,
        dependencies: row.get(9)?,
        revision: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        completed_at: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.migrate().unwrap();
        s
    }

    fn note(store: &Store, project: &str, name: &str) -> Scratchpad {
        let now = util::now();
        let note = Scratchpad {
            id: util::new_id(),
            project: project.to_string(),
            task_id: None,
            name: name.to_string(),
            content: "first".into(),
            revision: 1,
            author: "human:test".into(),
            created_at: now.clone(),
            updated_at: now,
        };
        store.insert_scratchpad(&note).unwrap();
        note
    }

    #[test]
    fn scratchpad_updates_are_optimistic_and_conflict_is_reported() {
        let store = store();
        let n = note(&store, "p1", "findings");
        assert_eq!(
            store
                .update_scratchpad(&n.id, "second", 1, "agent:a")
                .unwrap(),
            2
        );
        let err = store
            .update_scratchpad(&n.id, "lost", 1, "agent:b")
            .unwrap_err();
        assert!(matches!(err, DevdError::Conflict(_)), "{err:?}");
        let reread = store.get_scratchpad("p1", "findings").unwrap().unwrap();
        assert_eq!(reread.content, "second");
        assert_eq!(reread.revision, 2);
    }

    #[test]
    fn scratchpads_and_todos_are_project_scoped() {
        let store = store();
        note(&store, "p1", "one");
        note(&store, "p2", "two");
        assert_eq!(store.list_scratchpads("p1", None).unwrap().len(), 1);
        assert_eq!(store.list_scratchpads("p2", None).unwrap()[0].name, "two");
        assert!(store.get_scratchpad("p1", "two").unwrap().is_none());
    }

    #[test]
    fn todo_transitions_are_recorded_and_revision_guarded() {
        let store = store();
        let now = util::now();
        let todo = Todo {
            id: util::new_id(),
            project: "p1".into(),
            task_id: None,
            title: "write it down".into(),
            description: String::new(),
            status: "open".into(),
            priority: "normal".into(),
            assignee: None,
            origin: Some("human:test".into()),
            dependencies: "[]".into(),
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };
        store.insert_todo(&todo).unwrap();
        let updated = store
            .update_todo(
                &todo.id,
                1,
                "agent:a",
                None,
                None,
                Some("in_progress"),
                None,
                None,
                "",
            )
            .unwrap();
        assert_eq!(updated.status, "in_progress");
        assert_eq!(updated.revision, 2);
        let done = store
            .update_todo(
                &todo.id,
                2,
                "agent:a",
                None,
                None,
                Some("done"),
                None,
                None,
                "finished",
            )
            .unwrap();
        assert_eq!(done.status, "done");
        assert!(done.completed_at.is_some());
        let events = store.list_todo_events(&todo.id, 10).unwrap();
        assert_eq!(events.len(), 3, "created, in_progress, done");
        assert_eq!(events[0].to_status, "done");
        let stale = store
            .update_todo(
                &todo.id,
                2,
                "agent:b",
                None,
                None,
                Some("open"),
                None,
                None,
                "",
            )
            .unwrap_err();
        assert!(matches!(stale, DevdError::Conflict(_)));
    }

    #[test]
    fn project_approval_is_a_hash_and_can_be_replaced() {
        let store = store();
        assert!(store.get_project_approval("p1").unwrap().is_none());
        store
            .set_project_approval("p1", "aaa", "human:cli")
            .unwrap();
        assert_eq!(store.get_project_approval("p1").unwrap().unwrap().0, "aaa");
        store
            .set_project_approval("p1", "bbb", "human:http")
            .unwrap();
        let (hash, by, _) = store.get_project_approval("p1").unwrap().unwrap();
        assert_eq!(hash, "bbb");
        assert_eq!(by, "human:http");
    }
}
