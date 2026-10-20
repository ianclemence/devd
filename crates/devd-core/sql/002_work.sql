-- devd schema, version 2: durable work state.
--
-- Additive only. These tables hold the parts of the product that must survive a
-- frontend reload, a view change or an application restart: agent session
-- metadata, delegated tasks, project processes, the approval for an executable
-- project manifest, and shared scratchpads and todos.
--
-- A session row is *metadata*. It never claims a terminal is still running just
-- because a row says so: the process that owns the PTY is the authority, and a
-- row whose owner is gone is reconciled to `stale`.

CREATE TABLE IF NOT EXISTS agent_sessions (
    id                TEXT PRIMARY KEY,
    label             TEXT NOT NULL,
    harness           TEXT NOT NULL,
    cwd               TEXT NOT NULL,
    project           TEXT,
    task_id           TEXT,
    parent_session_id TEXT,
    state             TEXT NOT NULL DEFAULT 'running',
    exit_code         INTEGER,
    exit_reason       TEXT,
    pid               INTEGER,
    cols              INTEGER NOT NULL DEFAULT 100,
    rows              INTEGER NOT NULL DEFAULT 30,
    owner_pid         INTEGER NOT NULL,
    owner_started_at  TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    last_activity_at  TEXT NOT NULL,
    ended_at          TEXT,
    transcript_path   TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_created
    ON agent_sessions (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_state
    ON agent_sessions (state);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_project
    ON agent_sessions (project, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_tasks (
    id                TEXT PRIMARY KEY,
    parent_task_id    TEXT,
    parent_session_id TEXT,
    session_id        TEXT,
    title             TEXT NOT NULL,
    instructions      TEXT NOT NULL,
    cwd               TEXT NOT NULL,
    harness           TEXT NOT NULL,
    state             TEXT NOT NULL DEFAULT 'queued',
    depth             INTEGER NOT NULL DEFAULT 0,
    timeout_secs      INTEGER NOT NULL,
    exit_code         INTEGER,
    result            TEXT,
    error             TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    finished_at       TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_tasks_parent_session
    ON agent_tasks (parent_session_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_tasks_state
    ON agent_tasks (state);

CREATE TABLE IF NOT EXISTS project_processes (
    id             TEXT PRIMARY KEY,
    project        TEXT NOT NULL,
    name           TEXT NOT NULL,
    command        TEXT NOT NULL,
    working_dir    TEXT NOT NULL,
    env            TEXT NOT NULL DEFAULT '{}',
    health         TEXT,
    restart        TEXT NOT NULL DEFAULT 'on-failure',
    depends_on     TEXT NOT NULL DEFAULT '[]',
    status         TEXT NOT NULL DEFAULT 'stopped',
    health_status  TEXT NOT NULL DEFAULT 'unknown',
    health_error   TEXT,
    pid            INTEGER,
    owner_pid      INTEGER NOT NULL DEFAULT 0,
    desired_state  TEXT NOT NULL DEFAULT 'stopped',
    restart_count  INTEGER NOT NULL DEFAULT 0,
    last_exit_code INTEGER,
    last_error     TEXT,
    started_at     TEXT,
    updated_at     TEXT NOT NULL,
    UNIQUE (project, name)
);
CREATE INDEX IF NOT EXISTS idx_project_processes_project
    ON project_processes (project, name);

CREATE TABLE IF NOT EXISTS project_approvals (
    project       TEXT PRIMARY KEY,
    manifest_hash TEXT NOT NULL,
    approved_by   TEXT NOT NULL,
    approved_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS scratchpads (
    id         TEXT PRIMARY KEY,
    project    TEXT NOT NULL,
    task_id    TEXT,
    name       TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    revision   INTEGER NOT NULL DEFAULT 1,
    author     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (project, name)
);
CREATE INDEX IF NOT EXISTS idx_scratchpads_project
    ON scratchpads (project, updated_at DESC);

CREATE TABLE IF NOT EXISTS todos (
    id           TEXT PRIMARY KEY,
    project      TEXT NOT NULL,
    task_id      TEXT,
    title        TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'open',
    priority     TEXT NOT NULL DEFAULT 'normal',
    assignee     TEXT,
    origin       TEXT,
    dependencies TEXT NOT NULL DEFAULT '[]',
    revision     INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_todos_project
    ON todos (project, status, created_at DESC);

CREATE TABLE IF NOT EXISTS todo_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    todo_id     TEXT NOT NULL,
    at          TEXT NOT NULL,
    actor       TEXT NOT NULL,
    from_status TEXT,
    to_status   TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_todo_events_todo
    ON todo_events (todo_id, id DESC);
