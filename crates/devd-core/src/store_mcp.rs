//! Persistence for the Model Context Protocol servers devd connects to as a client.
//!
//! The row is configuration (`enabled`, `trust`) plus what the last connection observed (`tools`,
//! `last_error`). Environment values are not stored here: they often carry a token, and devd keeps
//! secrets in owner-only files under the devd home (see [`crate::paths::Paths::mcp`]).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// What a person decided about letting agents use one server's tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTrust {
    /// An agent's call waits for a person's approval, every time.
    Ask,
    /// A person allowed this server's tools to run without asking.
    Allow,
    /// Agents may not use this server at all.
    Deny,
}

impl McpTrust {
    /// Read a stored value; anything unrecognised means the safest choice.
    pub fn parse(s: &str) -> Self {
        match s {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }

    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// One tool a server published. Public metadata only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    /// The name the server uses for it.
    pub name: String,
    /// What it does, as the server describes it.
    #[serde(default)]
    pub description: String,
    /// The JSON Schema of its arguments.
    #[serde(default)]
    pub schema: Value,
}

/// One configured server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    /// Stable id (a generated token), used for file names.
    pub id: String,
    /// The name people use and tools are prefixed with. Unique, lower-case.
    pub name: String,
    /// `stdio` or `http`.
    pub transport: String,
    /// The program, for `stdio`.
    pub command: Option<String>,
    /// Its arguments.
    pub args: Vec<String>,
    /// The endpoint, for `http`.
    pub url: Option<String>,
    /// The directory to start a `stdio` server in.
    pub cwd: Option<String>,
    /// Whether devd may use it.
    pub enabled: bool,
    /// What agents may do with its tools.
    pub trust: McpTrust,
    /// How long one request may take.
    pub timeout_secs: u64,
    /// The catalogue the last connection reported.
    pub tools: Vec<McpTool>,
    /// Why the last connection failed, if it did.
    pub last_error: Option<String>,
    /// When it was added.
    pub created_at: String,
    /// When it was last changed.
    pub updated_at: String,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<McpServer> {
    let args: String = r.get(4)?;
    let tools: String = r.get(10)?;
    Ok(McpServer {
        id: r.get(0)?,
        name: r.get(1)?,
        transport: r.get(2)?,
        command: r.get(3)?,
        args: serde_json::from_str(&args).unwrap_or_default(),
        url: r.get(5)?,
        cwd: r.get(6)?,
        enabled: r.get::<_, i64>(7)? != 0,
        trust: McpTrust::parse(&r.get::<_, String>(8)?),
        timeout_secs: r.get::<_, i64>(9)?.max(1) as u64,
        tools: serde_json::from_str(&tools).unwrap_or_default(),
        last_error: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
    })
}

const COLUMNS: &str = "id, name, transport, command, args, url, cwd, enabled, trust, timeout_secs, tools, last_error, created_at, updated_at";

impl Store {
    /// Every configured server, by name.
    pub fn mcp_servers(&self) -> Result<Vec<McpServer>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {COLUMNS} FROM mcp_servers ORDER BY name"))?;
            let rows = st.query_map([], row)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One server by id or name.
    pub fn mcp_server(&self, id_or_name: &str) -> Result<Option<McpServer>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLUMNS} FROM mcp_servers WHERE id=?1 OR name=?1"),
                params![id_or_name],
                row,
            )
            .optional()?)
        })
    }

    /// Add or replace a server, keeping its creation time.
    pub fn mcp_put(&self, s: &McpServer) -> Result<()> {
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO mcp_servers (id, name, transport, command, args, url, cwd, enabled, trust, timeout_secs, tools, last_error, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, transport=excluded.transport, command=excluded.command,
                   args=excluded.args, url=excluded.url, cwd=excluded.cwd, enabled=excluded.enabled, trust=excluded.trust,
                   timeout_secs=excluded.timeout_secs, tools=excluded.tools, last_error=excluded.last_error, updated_at=excluded.updated_at",
                params![
                    s.id,
                    s.name,
                    s.transport,
                    s.command,
                    serde_json::to_string(&s.args)?,
                    s.url,
                    s.cwd,
                    s.enabled as i64,
                    s.trust.as_str(),
                    s.timeout_secs as i64,
                    serde_json::to_string(&s.tools)?,
                    s.last_error,
                    now
                ],
            )?;
            Ok(())
        })
    }

    /// Remove one server. False when there was none.
    pub fn mcp_delete(&self, id: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM mcp_servers WHERE id=?1", params![id])? > 0))
    }

    /// Record what a connection found, or why it failed.
    ///
    /// This is an observation, so it does not touch `updated_at`: that column is the configuration's
    /// stamp, and the live connection pool keys its reuse on it.
    pub fn mcp_set_catalogue(&self, id: &str, tools: &[McpTool], error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE mcp_servers SET tools=?2, last_error=?3 WHERE id=?1",
                params![id, serde_json::to_string(tools)?, error],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Store;

    fn server(name: &str) -> McpServer {
        McpServer {
            id: util::new_id(),
            name: name.into(),
            transport: "stdio".into(),
            command: Some("node".into()),
            args: vec!["server.js".into()],
            url: None,
            cwd: None,
            enabled: true,
            trust: McpTrust::Ask,
            timeout_secs: 30,
            tools: vec![],
            last_error: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn a_server_round_trips_and_a_catalogue_replaces_the_old_one() {
        let s = Store::open_in_memory().unwrap();
        let mut a = server("files");
        s.mcp_put(&a).unwrap();
        let got = s.mcp_server("files").unwrap().unwrap();
        assert_eq!(got.command.as_deref(), Some("node"));
        assert_eq!(got.args, vec!["server.js".to_string()]);
        assert_eq!(got.trust, McpTrust::Ask);

        a.trust = McpTrust::Allow;
        a.enabled = false;
        s.mcp_put(&a).unwrap();
        let got = s.mcp_server("files").unwrap().unwrap();
        assert_eq!(got.trust, McpTrust::Allow);
        assert!(!got.enabled);

        s.mcp_set_catalogue(&a.id, &[McpTool { name: "read".into(), description: "Read a file".into(), schema: serde_json::json!({"type": "object"}) }], None).unwrap();
        assert_eq!(s.mcp_server("files").unwrap().unwrap().tools.len(), 1);
        s.mcp_set_catalogue(&a.id, &[], Some("it did not start")).unwrap();
        let got = s.mcp_server("files").unwrap().unwrap();
        assert!(got.tools.is_empty());
        assert_eq!(got.last_error.as_deref(), Some("it did not start"));

        assert!(s.mcp_delete(&a.id).unwrap());
        assert!(s.mcp_server("files").unwrap().is_none());
    }
}
