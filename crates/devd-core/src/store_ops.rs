//! Persistence for site operations: cron jobs, queue daemons, redirects, deploys, notification
//! targets and the per-site documents (deploy script, repository link).
//!
//! Every kind is a small JSON document in one table, read and written whole. The kinds are named by
//! the crate that owns them; this module only stores them.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// One stored document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsItem {
    /// Id, unique within its kind.
    pub id: String,
    /// What it is: `cron`, `daemon`, `redirect`, `deploy`, `notify`, `deploy_script`, `repo`, ...
    pub kind: String,
    /// The site it belongs to; empty when it belongs to none.
    pub site: String,
    /// The document.
    pub data: Value,
    /// When it was created.
    pub created_at: String,
    /// When it was last written.
    pub updated_at: String,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OpsItem> {
    let text: String = r.get(3)?;
    Ok(OpsItem {
        id: r.get(0)?,
        kind: r.get(1)?,
        site: r.get(2)?,
        data: serde_json::from_str(&text).unwrap_or(Value::Null),
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

const COLUMNS: &str = "id, kind, site, data, created_at, updated_at";

impl Store {
    /// Write a document, keeping its creation time when it exists.
    pub fn ops_put(&self, kind: &str, id: &str, site: &str, data: &Value) -> Result<()> {
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO ops_items (id, kind, site, data, created_at, updated_at) VALUES (?1,?2,?3,?4,?5,?5)
                 ON CONFLICT(kind, id) DO UPDATE SET site=excluded.site, data=excluded.data, updated_at=excluded.updated_at",
                params![id, kind, site, serde_json::to_string(data)?, now],
            )?;
            Ok(())
        })
    }

    /// One document.
    pub fn ops_get(&self, kind: &str, id: &str) -> Result<Option<OpsItem>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM ops_items WHERE kind=?1 AND id=?2"), params![kind, id], row).optional()?))
    }

    /// The documents of a kind, oldest first, optionally one site's.
    pub fn ops_list(&self, kind: &str, site: Option<&str>) -> Result<Vec<OpsItem>> {
        self.with(|c| {
            let mut out = Vec::new();
            match site {
                Some(s) => {
                    let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM ops_items WHERE kind=?1 AND site=?2 ORDER BY created_at, id"))?;
                    for r in st.query_map(params![kind, s], row)? {
                        out.push(r?);
                    }
                }
                None => {
                    let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM ops_items WHERE kind=?1 ORDER BY created_at, id"))?;
                    for r in st.query_map(params![kind], row)? {
                        out.push(r?);
                    }
                }
            }
            Ok(out)
        })
    }

    /// The newest documents of a kind for a site, newest first.
    pub fn ops_recent(&self, kind: &str, site: &str, limit: usize) -> Result<Vec<OpsItem>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM ops_items WHERE kind=?1 AND site=?2 ORDER BY created_at DESC, id DESC LIMIT ?3"))?;
            let rows = st.query_map(params![kind, site, limit as i64], row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Remove one document. False when there was none.
    pub fn ops_delete(&self, kind: &str, id: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM ops_items WHERE kind=?1 AND id=?2", params![kind, id])? > 0))
    }

    /// Remove every document of a kind for a site. Returns how many.
    pub fn ops_delete_site(&self, kind: &str, site: &str) -> Result<usize> {
        self.with(|c| Ok(c.execute("DELETE FROM ops_items WHERE kind=?1 AND site=?2", params![kind, site])?))
    }
}
