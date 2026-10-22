-- Schedules: standing instructions to start a team run on a clock.
CREATE TABLE IF NOT EXISTS schedules (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    spec         TEXT NOT NULL,
    template     TEXT,
    goal         TEXT,
    project      TEXT NOT NULL,
    params       TEXT NOT NULL DEFAULT '{}',
    done_when    TEXT NOT NULL DEFAULT '[]',
    auto_approve INTEGER NOT NULL DEFAULT 0,
    enabled      INTEGER NOT NULL DEFAULT 1,
    next_run_at  TEXT,
    last_run_at  TEXT,
    last_run_id  TEXT,
    last_error   TEXT,
    created_by   TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_schedules_due ON schedules (enabled, next_run_at);
