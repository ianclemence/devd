//! Persistence for schedules: work devd starts on its own, on a clock.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{DevdError, Result};
use crate::state::Store;
use crate::util;

/// A standing instruction to start a team run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// Id.
    pub id: String,
    /// A short unique name.
    pub name: String,
    /// When, as the person wrote it (see [`crate::schedule`]).
    pub spec: String,
    /// The run template to start, when it uses one.
    pub template: Option<String>,
    /// The goal to start, when it does not use a template.
    pub goal: Option<String>,
    /// The project directory the run works in.
    pub project: String,
    /// Values for the template's parameters.
    pub params: serde_json::Value,
    /// A definition of done added to the run.
    pub done_when: Vec<String>,
    /// Whether the person who made the schedule waived plan approval for its runs.
    pub auto_approve: bool,
    /// Whether it is running.
    pub enabled: bool,
    /// When it is next due, RFC 3339.
    pub next_run_at: Option<String>,
    /// When it last started a run.
    pub last_run_at: Option<String>,
    /// The run it last started.
    pub last_run_id: Option<String>,
    /// Why the last attempt did not start a run.
    pub last_error: Option<String>,
    /// Who made it.
    pub created_by: String,
    /// When.
    pub created_at: String,
}

const COLUMNS: &str = "id,name,spec,template,goal,project,params,done_when,auto_approve,enabled,next_run_at,last_run_at,last_run_id,last_error,created_by,created_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Schedule> {
    Ok(Schedule {
        id: r.get(0)?,
        name: r.get(1)?,
        spec: r.get(2)?,
        template: r.get(3)?,
        goal: r.get(4)?,
        project: r.get(5)?,
        params: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(serde_json::json!({})),
        done_when: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        auto_approve: r.get::<_, i64>(8)? != 0,
        enabled: r.get::<_, i64>(9)? != 0,
        next_run_at: r.get(10)?,
        last_run_at: r.get(11)?,
        last_run_id: r.get(12)?,
        last_error: r.get(13)?,
        created_by: r.get(14)?,
        created_at: r.get(15)?,
    })
}

impl Store {
    /// Add a schedule. Names are unique.
    pub fn create_schedule(&self, s: &Schedule) -> Result<()> {
        self.with(|c| {
            if c.query_row("SELECT 1 FROM schedules WHERE name=?1", params![s.name], |_| Ok(())).optional()?.is_some() {
                return Err(DevdError::conflict(format!("a schedule named `{}` already exists", s.name)));
            }
            c.execute(
                &format!("INSERT INTO schedules ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"),
                params![
                    s.id, s.name, s.spec, s.template, s.goal, s.project,
                    serde_json::to_string(&s.params)?, serde_json::to_string(&s.done_when)?,
                    s.auto_approve as i32, s.enabled as i32, s.next_run_at, s.last_run_at, s.last_run_id, s.last_error,
                    s.created_by, s.created_at
                ],
            )?;
            Ok(())
        })
    }

    /// Every schedule, by name.
    pub fn list_schedules(&self) -> Result<Vec<Schedule>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM schedules ORDER BY name"))?;
            let rows = st.query_map([], row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One schedule by name.
    pub fn get_schedule(&self, name: &str) -> Result<Option<Schedule>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM schedules WHERE name=?1"), params![name], row).optional()?))
    }

    /// Remove a schedule. Runs it already started are not touched.
    pub fn delete_schedule(&self, name: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM schedules WHERE name=?1", params![name])? > 0))
    }

    /// Turn a schedule on or off, and say when it is next due.
    pub fn set_schedule_enabled(&self, name: &str, enabled: bool, next_run_at: Option<&str>) -> Result<bool> {
        self.with(|c| {
            Ok(c.execute("UPDATE schedules SET enabled=?2, next_run_at=?3 WHERE name=?1", params![name, enabled as i32, next_run_at])? > 0)
        })
    }

    /// The enabled schedules due at or before `now` (RFC 3339 strings sort in time order).
    pub fn due_schedules(&self, now: &str) -> Result<Vec<Schedule>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM schedules WHERE enabled=1 AND next_run_at IS NOT NULL AND next_run_at<=?1 ORDER BY next_run_at"))?;
            let rows = st.query_map(params![now], row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Record that a schedule started a run (or why it did not), and when it is next due.
    pub fn record_schedule_attempt(&self, name: &str, run_id: Option<&str>, error: Option<&str>, next_run_at: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE schedules SET last_run_at=?2, last_run_id=COALESCE(?3,last_run_id), last_error=?4, next_run_at=?5 WHERE name=?1",
                params![name, util::now(), run_id, error, next_run_at],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.migrate().unwrap();
        s
    }

    fn schedule(name: &str, next: &str) -> Schedule {
        Schedule {
            id: util::new_id(), name: name.into(), spec: "daily 09:00".into(), template: Some("fix-bug".into()), goal: None,
            project: "C:/p".into(), params: serde_json::json!({ "report": "x" }), done_when: vec!["tests pass".into()],
            auto_approve: false, enabled: true, next_run_at: Some(next.into()), last_run_at: None, last_run_id: None,
            last_error: None, created_by: "human:cli".into(), created_at: util::now(),
        }
    }

    #[test]
    fn schedules_round_trip_and_names_are_unique() {
        let s = store();
        s.create_schedule(&schedule("nightly", "2026-10-23T09:00:00+00:00")).unwrap();
        let got = s.get_schedule("nightly").unwrap().unwrap();
        assert_eq!(got.params["report"], "x");
        assert_eq!(got.done_when, vec!["tests pass"]);
        assert_eq!(s.create_schedule(&schedule("nightly", "2026-10-23T09:00:00+00:00")).unwrap_err().code(), "conflict");
        assert!(s.delete_schedule("nightly").unwrap());
        assert!(s.get_schedule("nightly").unwrap().is_none());
    }

    #[test]
    fn only_enabled_schedules_that_are_due_come_back_in_order() {
        let s = store();
        s.create_schedule(&schedule("later", "2026-10-23T09:00:00+00:00")).unwrap();
        s.create_schedule(&schedule("second", "2026-10-22T08:00:00+00:00")).unwrap();
        s.create_schedule(&schedule("first", "2026-10-22T07:00:00+00:00")).unwrap();
        s.create_schedule(&schedule("off", "2026-10-22T06:00:00+00:00")).unwrap();
        s.set_schedule_enabled("off", false, None).unwrap();
        let due: Vec<String> = s.due_schedules("2026-10-22T12:00:00+00:00").unwrap().into_iter().map(|x| x.name).collect();
        assert_eq!(due, vec!["first", "second"]);
        s.record_schedule_attempt("first", Some("run1"), None, Some("2026-10-23T07:00:00+00:00")).unwrap();
        let first = s.get_schedule("first").unwrap().unwrap();
        assert_eq!(first.last_run_id.as_deref(), Some("run1"));
        assert_eq!(first.next_run_at.as_deref(), Some("2026-10-23T07:00:00+00:00"));
        assert_eq!(s.due_schedules("2026-10-22T12:00:00+00:00").unwrap().len(), 1);
    }
}
