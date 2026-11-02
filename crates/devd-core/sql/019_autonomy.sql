-- devd schema, version 19: what a task is allowed to do, a heartbeat for the
-- stall watchdog, proactive suggestions, a retried queue for effects that leave
-- the process, the kind of work a conversation expected, the prompts it ran
-- under, and the intake rules a project wrote for itself.
--
-- Additive only: an older devd opens this database and ignores every column and
-- table here.

-- ---------------------------------------------------------- a task's mandate
-- What a team task was authorised to do, and on whose authority. The scheduler
-- reads this before it resumes work, so a change of mode or policy can stop a
-- run deliberately instead of blindly continuing it.
ALTER TABLE team_tasks ADD COLUMN mandate TEXT NOT NULL DEFAULT '{}';

-- The last time this task did anything (a model turn, a tool call). `NULL` means
-- it has not started. The stall watchdog compares it with now.
ALTER TABLE team_tasks ADD COLUMN last_step_at TEXT;

-- How many times the watchdog nudged it, so it is nudged once and then stopped.
ALTER TABLE team_tasks ADD COLUMN stalls INTEGER NOT NULL DEFAULT 0;

-- ------------------------------------------------------------ suggestions
-- Work devd would do, written down before it is allowed to do it. A suggestion
-- is never work: a person turns it into a run, which asks for plan approval as
-- any other run does.
CREATE TABLE IF NOT EXISTS suggestions (
    id            TEXT PRIMARY KEY,
    project       TEXT,
    role          TEXT NOT NULL,
    title         TEXT NOT NULL,
    why           TEXT NOT NULL,            -- what it noticed, in its words
    plan          TEXT NOT NULL DEFAULT '[]', -- JSON array of the steps it would take
    accepts       TEXT NOT NULL DEFAULT '[]', -- JSON array: how the work would be known to be done
    source        TEXT NOT NULL,            -- manual | intake | schedule
    source_ref    TEXT,                     -- the intake rule name or the schedule name
    state         TEXT NOT NULL DEFAULT 'open', -- open | accepted | dismissed | expired
    run_id        TEXT,                     -- the run a person started from it
    decided_by    TEXT,
    decided_at    TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_suggestions_state ON suggestions (state, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_suggestions_key ON suggestions (project, title, state);

-- -------------------------------------------------------------- effects
-- Work that must happen outside the transaction that decided it: a message that
-- leaves the process, a review nobody is waiting for, a scheduled start. One row
-- per kind and key, retried with backoff, so a failure is a row a person can see
-- rather than a log line that scrolls away.
CREATE TABLE IF NOT EXISTS effects (
    kind        TEXT NOT NULL,              -- inbox.health | review.run | team.schedule | inbox.post
    key         TEXT NOT NULL,
    payload     TEXT NOT NULL DEFAULT '{}',
    state       TEXT NOT NULL DEFAULT 'pending', -- pending | done | dropped
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    not_before  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (kind, key)
);
CREATE INDEX IF NOT EXISTS idx_effects_due ON effects (state, not_before);

-- --------------------------------------------------- what a conversation is
-- The kind of work a conversation was opened for, decided in code before a
-- model is asked anything, so the same request reaches the same runner and the
-- same model every time.
ALTER TABLE chat_threads ADD COLUMN kind TEXT NOT NULL DEFAULT 'answer';

-- The prompts a turn ran under: the role and skill text, by name and hash. A
-- run that behaved differently can be compared with one that did not.
ALTER TABLE run_steps ADD COLUMN prompts TEXT;

-- The intake rules a project wrote: "a failing check opens a fix run at most
-- once a day", and so on. Stored apart from the charter so a rule can be added
-- without rewriting the rest of it.
CREATE TABLE IF NOT EXISTS intake_rules (
    project     TEXT NOT NULL,
    name        TEXT NOT NULL,
    trigger     TEXT NOT NULL,              -- health.down | check.failed | schedule.missed
    condition   TEXT NOT NULL DEFAULT '{}', -- JSON: min_failures, cooldown_minutes, ...
    goal        TEXT NOT NULL,              -- what the run should do
    template    TEXT,                       -- a run template to use instead of the goal
    role        TEXT NOT NULL DEFAULT 'implementer',
    mode        TEXT NOT NULL DEFAULT 'observe', -- observe | auto
    countdown   INTEGER NOT NULL DEFAULT 0, -- consecutive matching failures seen
    fired_at    TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (project, name)
);
