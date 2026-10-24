-- Model Context Protocol servers that devd connects to as a client.
--
-- These are other programs' MCP servers: devd's assistant uses their tools the
-- way Claude Code uses the servers configured for it. The row is configuration,
-- not state: `enabled` and `trust` are a person's decisions, `tools` and
-- `last_error` are the observed result of the last connection and are replaced
-- every time.
--
-- Environment values are NOT here. An MCP server is often given an API token in
-- its environment, and devd keeps secrets in owner-only files (see the `mcp`
-- folder under the devd home), never in the database, logs or audit trail.
CREATE TABLE IF NOT EXISTS mcp_servers (
    id           TEXT NOT NULL PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    transport    TEXT NOT NULL DEFAULT 'stdio',
    command      TEXT,
    args         TEXT NOT NULL DEFAULT '[]',
    url          TEXT,
    cwd          TEXT,
    enabled      INTEGER NOT NULL DEFAULT 1,
    trust        TEXT NOT NULL DEFAULT 'ask',
    timeout_secs INTEGER NOT NULL DEFAULT 120,
    tools        TEXT NOT NULL DEFAULT '[]',
    last_error   TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
