//! Incoming hooks: a token, an event, an optional filter and one action.
//!
//! A hook is not a new permission path. It stores a rule; the server resolves
//! the token and dispatches the ordinary capability the action names, as a
//! system caller, so the audit and approval rules are the same as any other run.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use serde_json::Value;

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// One hook rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Automation {
    /// Stable name.
    pub name: String,
    /// The secret in the hook address.
    pub token: String,
    /// `github.pull_request`, `github.push` or `custom`.
    pub event: String,
    /// Optional match conditions.
    pub filter: Value,
    /// What to do: `{ "type": "team" | "chat" | "inbox", ... }`.
    pub action: Value,
    /// Whether it accepts calls.
    pub enabled: bool,
    /// Created.
    pub created_at: String,
    /// Last accepted call.
    pub last_at: Option<String>,
    /// Last failure, kept until the next success.
    pub last_error: Option<String>,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Automation> {
    let filter: String = r.get(3)?;
    let action: String = r.get(4)?;
    Ok(Automation {
        name: r.get(0)?,
        token: r.get(1)?,
        event: r.get(2)?,
        filter: serde_json::from_str(&filter).unwrap_or(Value::Null),
        action: serde_json::from_str(&action).unwrap_or(Value::Null),
        enabled: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
        last_at: r.get(7)?,
        last_error: r.get(8)?,
    })
}

impl Store {
    /// Every hook rule.
    pub fn list_automations(&self) -> Result<Vec<Automation>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT name,token,event,filter,action,enabled,created_at,last_at,last_error FROM automations ORDER BY name",
            )?;
            let rows = st.query_map([], row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One rule by name.
    pub fn get_automation(&self, name: &str) -> Result<Option<Automation>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT name,token,event,filter,action,enabled,created_at,last_at,last_error FROM automations WHERE name=?1",
                params![name],
                row,
            )
            .optional()?)
        })
    }

    /// One rule by its hook token.
    pub fn automation_by_token(&self, token: &str) -> Result<Option<Automation>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT name,token,event,filter,action,enabled,created_at,last_at,last_error FROM automations WHERE token=?1",
                params![token],
                row,
            )
            .optional()?)
        })
    }

    /// Create or replace a rule.
    pub fn put_automation(
        &self,
        name: &str,
        token: &str,
        event: &str,
        filter: &Value,
        action: &Value,
        enabled: bool,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO automations (name,token,event,filter,action,enabled,created_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7) \
                 ON CONFLICT(name) DO UPDATE SET token=?2,event=?3,filter=?4,action=?5,enabled=?6",
                params![
                    name,
                    token,
                    event,
                    serde_json::to_string(filter).unwrap_or_else(|_| "{}".into()),
                    serde_json::to_string(action).unwrap_or_else(|_| "{}".into()),
                    if enabled { 1 } else { 0 },
                    util::now()
                ],
            )?;
            Ok(())
        })
    }

    /// Remove a rule by name.
    pub fn delete_automation(&self, name: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM automations WHERE name=?1", params![name])? > 0))
    }

    /// Record the outcome of a hook call.
    pub fn mark_automation(&self, name: &str, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE automations SET last_at=?2, last_error=?3 WHERE name=?1",
                params![name, util::now(), error],
            )?;
            Ok(())
        })
    }
}
