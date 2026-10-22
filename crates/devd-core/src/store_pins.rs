//! Per-project runtime pins: which installed version of PHP, Node, Go, Python
//! or Ruby a project's commands run with, ahead of the machine default.

use rusqlite::params;

use crate::error::Result;
use crate::state::Store;
use crate::util;

impl Store {
    /// Pin `kind` to `version` (an installed version or a series such as `3.12`) for a project.
    pub fn set_runtime_pin(&self, project: &str, kind: &str, version: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO runtime_pins (project,kind,version,created_at) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT(project,kind) DO UPDATE SET version=excluded.version",
                params![project, kind, version, util::now()],
            )?;
            Ok(())
        })
    }

    /// Drop a pin. Returns whether there was one.
    pub fn clear_runtime_pin(&self, project: &str, kind: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM runtime_pins WHERE project=?1 AND kind=?2", params![project, kind])? > 0))
    }

    /// The pins of one project as `(kind, version)`.
    pub fn runtime_pins_of(&self, project: &str) -> Result<Vec<(String, String)>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT kind,version FROM runtime_pins WHERE project=?1 ORDER BY kind")?;
            let rows = st.query_map(params![project], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Every pin as `(project, kind, version)`.
    pub fn runtime_pins(&self) -> Result<Vec<(String, String, String)>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT project,kind,version FROM runtime_pins ORDER BY project,kind")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}
