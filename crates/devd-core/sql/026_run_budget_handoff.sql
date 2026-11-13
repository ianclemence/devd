-- A run can be bounded, and what one task hands the next is kept in a shape.
--
-- Every change is additive, so an older devd can still read the database.

-- The most a run may spend, and the most model turns its agents may take together. Neither set means no bound.
ALTER TABLE team_runs ADD COLUMN budget_usd REAL;
ALTER TABLE team_runs ADD COLUMN max_turns INTEGER;

-- What a finished task passes on: its summary, the files it changed, what it decided and what it left open, as JSON.
CREATE TABLE IF NOT EXISTS team_handoffs (
    task_id    TEXT PRIMARY KEY,
    data       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
