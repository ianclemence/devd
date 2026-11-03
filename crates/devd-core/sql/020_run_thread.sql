-- devd schema, version 20: a run remembers the conversation that asked for it.
--
-- Until now a team run had no way to address the person who started it. Its
-- result went to the inbox, a separate channel, so a person who asked the
-- assistant for something got their answer on another screen - if at all. The
-- run carries the conversation now, and reports into it.
--
-- Additive only: an older devd opens this database and ignores the column.
ALTER TABLE team_runs ADD COLUMN thread_id TEXT;

CREATE INDEX IF NOT EXISTS idx_team_runs_thread ON team_runs(thread_id);

-- A run may hold at most one pending report per conversation turn, so the same
-- result cannot be announced twice.
CREATE TABLE IF NOT EXISTS team_reports (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL,
    thread_id   TEXT NOT NULL,
    kind        TEXT NOT NULL,             -- finished | blocked | paused | plan
    body        TEXT NOT NULL,
    delivered   INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_team_reports_thread ON team_reports(thread_id, delivered);
