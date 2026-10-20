-- devd schema, version 5: provider health, role-based model assignments,
-- decisions about services that already exist on the machine, and the
-- multi-agent team: runs, their tasks and the event log. Additive only.

CREATE TABLE IF NOT EXISTS provider_checks (
    provider_id TEXT PRIMARY KEY,
    checked_at  TEXT NOT NULL,
    ok          INTEGER NOT NULL,
    message     TEXT NOT NULL,
    models      INTEGER
);

-- Which model does which kind of work. scope '' is global; 'project:<path>'
-- overrides it for one project. position orders the fallbacks (0 = first choice).
CREATE TABLE IF NOT EXISTS model_roles (
    role        TEXT NOT NULL,
    scope       TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    provider_id TEXT NOT NULL,
    model       TEXT NOT NULL,
    PRIMARY KEY (role, scope, position)
);

-- What a person decided about a tool or service found on this machine.
CREATE TABLE IF NOT EXISTS service_decisions (
    tool       TEXT PRIMARY KEY,
    mode       TEXT NOT NULL,              -- adopt | observe | external | ignore
    port       INTEGER,                    -- the port of an instance the person runs themselves
    note       TEXT,
    decided_by TEXT NOT NULL,
    decided_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS team_runs (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    goal        TEXT NOT NULL,
    project     TEXT,
    state       TEXT NOT NULL,             -- planning | awaiting_plan | running | paused | done | failed | cancelled
    plan_approved INTEGER NOT NULL DEFAULT 0,
    summary     TEXT,
    error       TEXT,
    parallel    INTEGER NOT NULL DEFAULT 2,
    created_by  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS team_tasks (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL,
    key         TEXT NOT NULL,             -- the planner's short name, used in depends_on
    role        TEXT NOT NULL,
    title       TEXT NOT NULL,
    instructions TEXT NOT NULL,
    depends_on  TEXT NOT NULL DEFAULT '[]',
    scope       TEXT NOT NULL DEFAULT '[]', -- paths this task may touch (for locking)
    state       TEXT NOT NULL,             -- pending | ready | running | waiting_approval | done | unverified | failed | cancelled | skipped
    attempts    INTEGER NOT NULL DEFAULT 0,
    workdir     TEXT,
    branch      TEXT,
    thread_id   TEXT,
    result      TEXT,
    evidence    TEXT NOT NULL DEFAULT '[]',
    error       TEXT,
    created_at  TEXT NOT NULL,
    started_at  TEXT,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_team_tasks_run ON team_tasks(run_id);

CREATE TABLE IF NOT EXISTS team_events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id  TEXT NOT NULL,
    task_id TEXT,
    kind    TEXT NOT NULL,
    data    TEXT NOT NULL,
    at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_team_events_run ON team_events(run_id, id);
