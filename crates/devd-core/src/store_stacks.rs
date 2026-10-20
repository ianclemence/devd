//! Persistence for the stack engine: projects it knows and the history of what
//! it ran.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// A project the stack engine created or imported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackProject {
    /// Absolute project directory.
    pub path: String,
    /// Display name.
    pub name: String,
    /// Stack identifier, when one was detected or chosen.
    pub stack_id: Option<String>,
    /// Package manager, when known.
    pub package_manager: Option<String>,
    /// `init` or `import`.
    pub source: String,
    /// Detected context as JSON text.
    pub context: String,
    /// RFC 3339 creation time.
    pub created_at: String,
    /// RFC 3339 update time.
    pub updated_at: String,
}

/// One recorded execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackRun {
    /// Identifier.
    pub id: String,
    /// `init`, `task`, `exec` or `toolchain`.
    pub kind: String,
    /// Project path or tool id.
    pub subject: String,
    /// Human title.
    pub title: String,
    /// Redacted command line.
    pub command: String,
    /// Start time.
    pub started_at: String,
    /// Finish time, when finished.
    pub finished_at: Option<String>,
    /// Exit code, when finished.
    pub exit_code: Option<i32>,
    /// Whether it succeeded.
    pub ok: Option<bool>,
    /// Duration in milliseconds.
    pub duration_ms: Option<i64>,
    /// Who ran it.
    pub actor: String,
    /// Log file, when output was kept.
    pub log_path: Option<String>,
}

impl Store {
    /// Insert or update a stack project.
    pub fn upsert_stack_project(&self, p: &StackProject) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO stack_projects (path,name,stack_id,package_manager,source,context,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(path) DO UPDATE SET name=?2, stack_id=?3, package_manager=?4,
                   context=?6, updated_at=?8",
                params![p.path, p.name, p.stack_id, p.package_manager, p.source, p.context, p.created_at, p.updated_at],
            )?;
            Ok(())
        })
    }

    /// Every stack project, newest first.
    pub fn list_stack_projects(&self) -> Result<Vec<StackProject>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT path,name,stack_id,package_manager,source,context,created_at,updated_at
                 FROM stack_projects ORDER BY updated_at DESC",
            )?;
            let rows = st.query_map([], row_project)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One stack project by path.
    pub fn get_stack_project(&self, path: &str) -> Result<Option<StackProject>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT path,name,stack_id,package_manager,source,context,created_at,updated_at
                 FROM stack_projects WHERE path=?1",
                params![path],
                row_project,
            )
            .optional()?)
        })
    }

    /// Forget a project (files are untouched).
    pub fn delete_stack_project(&self, path: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM stack_projects WHERE path=?1", params![path])? > 0))
    }

    /// Record the start of a run.
    pub fn begin_stack_run(&self, kind: &str, subject: &str, title: &str, command: &str, actor: &str, log: Option<&str>) -> Result<String> {
        let id = util::new_id();
        self.with(|c| {
            c.execute(
                "INSERT INTO stack_runs (id,kind,subject,title,command,started_at,actor,log_path)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![id, kind, subject, title, command, util::now(), actor, log],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Record the end of a run.
    pub fn finish_stack_run(&self, id: &str, exit_code: Option<i32>, ok: bool, duration_ms: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE stack_runs SET finished_at=?2, exit_code=?3, ok=?4, duration_ms=?5 WHERE id=?1",
                params![id, util::now(), exit_code, ok as i32, duration_ms],
            )?;
            Ok(())
        })
    }

    /// Most recent runs, optionally for one subject.
    pub fn list_stack_runs(&self, subject: Option<&str>, limit: i64) -> Result<Vec<StackRun>> {
        self.with(|c| {
            let sql = "SELECT id,kind,subject,title,command,started_at,finished_at,exit_code,ok,duration_ms,actor,log_path
                       FROM stack_runs WHERE (?1 IS NULL OR subject=?1) ORDER BY started_at DESC LIMIT ?2";
            let mut st = c.prepare(sql)?;
            let rows = st.query_map(params![subject, limit], |r| {
                Ok(StackRun {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    subject: r.get(2)?,
                    title: r.get(3)?,
                    command: r.get(4)?,
                    started_at: r.get(5)?,
                    finished_at: r.get(6)?,
                    exit_code: r.get(7)?,
                    ok: r.get::<_, Option<i32>>(8)?.map(|v| v != 0),
                    duration_ms: r.get(9)?,
                    actor: r.get(10)?,
                    log_path: r.get(11)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}

fn row_project(r: &rusqlite::Row) -> rusqlite::Result<StackProject> {
    Ok(StackProject {
        path: r.get(0)?,
        name: r.get(1)?,
        stack_id: r.get(2)?,
        package_manager: r.get(3)?,
        source: r.get(4)?,
        context: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}
