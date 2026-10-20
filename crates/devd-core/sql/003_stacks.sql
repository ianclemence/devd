-- devd schema, version 3: projects known to the stack engine and the history of
-- everything it ran. Additive only.
CREATE TABLE IF NOT EXISTS stack_projects (
    path        TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    stack_id    TEXT,
    package_manager TEXT,
    source      TEXT NOT NULL,          -- init | import
    context     TEXT NOT NULL,          -- detected context, JSON
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stack_runs (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,          -- init | task | exec | toolchain
    subject     TEXT NOT NULL,          -- project path or tool id
    title       TEXT NOT NULL,
    command     TEXT NOT NULL,          -- redacted display form
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    exit_code   INTEGER,
    ok          INTEGER,
    duration_ms INTEGER,
    actor       TEXT NOT NULL,
    log_path    TEXT
);
CREATE INDEX IF NOT EXISTS idx_stack_runs_started ON stack_runs (started_at DESC);
