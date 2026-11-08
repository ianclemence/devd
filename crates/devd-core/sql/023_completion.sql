-- Completion correctness, isolated writers and durable checkpoints.
--
-- Additive only: an older devd ignores these columns and tables.

-- The canonical completion record. A run is only `done` when `verdict` is
-- `approve` (or `accepted_by_hand`, which is a person's explicit override).
ALTER TABLE team_runs ADD COLUMN verdict TEXT;
ALTER TABLE team_runs ADD COLUMN verdict_at TEXT;
ALTER TABLE team_runs ADD COLUMN reviewed_commit TEXT;
ALTER TABLE team_runs ADD COLUMN base_commit TEXT;

-- What a task must satisfy, the gatekeeper's verdict on it, and the checkpoint
-- that lets a task continue after a turn or process ends.
ALTER TABLE team_tasks ADD COLUMN criteria TEXT NOT NULL DEFAULT '[]';
ALTER TABLE team_tasks ADD COLUMN verdict TEXT;
ALTER TABLE team_tasks ADD COLUMN verdict_data TEXT;
ALTER TABLE team_tasks ADD COLUMN reviewed_commit TEXT;
ALTER TABLE team_tasks ADD COLUMN checkpoint TEXT;
ALTER TABLE team_tasks ADD COLUMN failure_kind TEXT;

-- One writer at a time per project unless it holds a real worktree. The fencing
-- value makes a stale holder unable to keep writing after ownership moves.
CREATE TABLE IF NOT EXISTS write_leases (
    project    TEXT PRIMARY KEY,
    run_id     TEXT NOT NULL,
    task_id    TEXT NOT NULL,
    owner      TEXT NOT NULL,
    fencing    INTEGER NOT NULL DEFAULT 0,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- A check captured before the run changes anything, so a failure can be named
-- pre-existing instead of guessed. `checks` is a JSON array of
-- {name, command, ok, failures:[...], output}.
CREATE TABLE IF NOT EXISTS run_baselines (
    run_id      TEXT PRIMARY KEY,
    project     TEXT,
    commit_sha  TEXT,
    environment TEXT NOT NULL DEFAULT '{}',
    checks      TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_tasks_state ON team_tasks(state);
