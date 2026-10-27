-- devd schema, version 17: the unified run engine.
--
-- A run is one goal from intake to completion. The assistant, the team,
-- schedules and headless callers all create the same record, use the same
-- lifecycle, append the same sequenced events and prove completion with the
-- same evidence. The older chat_threads / team_runs tables remain the
-- authoritative stores for their own surface data; `runs` is the shared spine
-- and is linked to them rather than replacing them, so no conversation or team
-- run is lost by upgrading.
--
-- Additive only. A 0.13 binary ignores these tables and keeps working.

CREATE TABLE IF NOT EXISTS runs (
    id                TEXT PRIMARY KEY,
    goal              TEXT NOT NULL,
    mode              TEXT NOT NULL DEFAULT 'assistant', -- assistant | team | scheduled | headless
    state             TEXT NOT NULL DEFAULT 'intake',    -- intake | planning | awaiting_approval | executing | verifying | review | completed | paused | blocked | failed | unverified | cancelled
    project           TEXT,
    environment       TEXT,                              -- environment manifest fingerprint or JSON
    acceptance        TEXT NOT NULL DEFAULT '[]',        -- JSON array of acceptance criteria
    completion_policy TEXT NOT NULL DEFAULT '{}',        -- JSON completion policy
    plan              TEXT,                              -- JSON plan snapshot
    plan_fingerprint  TEXT,
    parent_run        TEXT,                              -- fork / checkout point
    thread_id         TEXT,                              -- the conversation that drives this run
    team_run_id       TEXT,                              -- the team run this mirrors, if any
    actor             TEXT NOT NULL,
    cancellation      TEXT,                              -- JSON { requested_at, reason, requested_by }
    usage             TEXT NOT NULL DEFAULT '{}',        -- JSON token / duration counters
    error             TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    finished_at       TEXT
);
CREATE INDEX IF NOT EXISTS idx_runs_state ON runs (state, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_thread ON runs (thread_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_runs_team ON runs (team_run_id) WHERE team_run_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS run_steps (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    ordinal     INTEGER NOT NULL,
    kind        TEXT NOT NULL,               -- task | turn | tool | verify | review | recovery
    title       TEXT NOT NULL,
    state       TEXT NOT NULL,               -- pending | running | done | failed | cancelled
    data        TEXT NOT NULL DEFAULT '{}',
    started_at  TEXT,
    finished_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_run_steps_run ON run_steps (run_id, ordinal);

CREATE TABLE IF NOT EXISTS tool_calls (
    id               TEXT PRIMARY KEY,
    run_id           TEXT REFERENCES runs(id) ON DELETE SET NULL,
    step_id          TEXT,
    call_id          TEXT,                   -- provider tool_use id, when there is one
    actor            TEXT NOT NULL,
    surface          TEXT NOT NULL,
    capability       TEXT NOT NULL,
    args             TEXT NOT NULL DEFAULT '{}',   -- redacted
    args_hash        TEXT NOT NULL DEFAULT '',
    approval_id      TEXT,
    idempotency_key  TEXT,
    ok               INTEGER NOT NULL DEFAULT 0,
    error_code       TEXT,
    error            TEXT,
    exit_code        INTEGER,
    duration_ms      INTEGER NOT NULL DEFAULT 0,
    result           TEXT,                   -- bounded, redacted outcome JSON
    started_at       TEXT NOT NULL,
    finished_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_run ON tool_calls (run_id, started_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tool_calls_idem ON tool_calls (idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS artifacts (
    id         TEXT PRIMARY KEY,
    run_id     TEXT REFERENCES runs(id) ON DELETE SET NULL,
    tool_call_id TEXT,
    kind       TEXT NOT NULL,               -- output | diff | test-report | log | probe | file
    name       TEXT NOT NULL,
    mime       TEXT,
    bytes      INTEGER NOT NULL DEFAULT 0,
    path       TEXT,                        -- where the bounded copy lives, if on disk
    sha256     TEXT,
    summary    TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_artifacts_run ON artifacts (run_id, created_at);

CREATE TABLE IF NOT EXISTS evidence (
    id           TEXT PRIMARY KEY,
    run_id       TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    step_id      TEXT,
    tool_call_id TEXT,
    artifact_id  TEXT,
    kind         TEXT NOT NULL,             -- change | read | verify | command | service | http | runtime | database | probe | structured
    capability   TEXT NOT NULL,
    passed       INTEGER NOT NULL DEFAULT 0,
    claim        TEXT,                      -- the acceptance criterion or claim this supports
    summary      TEXT NOT NULL,
    detail       TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_evidence_run ON evidence (run_id, created_at);

CREATE TABLE IF NOT EXISTS approvals (
    id           TEXT PRIMARY KEY,          -- SHA-256 of the opaque token
    run_id       TEXT REFERENCES runs(id) ON DELETE SET NULL,
    task_id      TEXT,
    tool_call_id TEXT,
    capability   TEXT NOT NULL,
    args         TEXT NOT NULL DEFAULT '{}',
    args_hash    TEXT NOT NULL DEFAULT '',
    plan         TEXT,
    plan_hash    TEXT,
    scope        TEXT NOT NULL DEFAULT 'single', -- single | plan | task
    tier         TEXT NOT NULL DEFAULT 'write',
    risk         TEXT NOT NULL DEFAULT 'normal', -- normal | sensitive | destructive
    actor        TEXT NOT NULL,
    requested_by TEXT NOT NULL DEFAULT '',
    expires_at   TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'pending', -- pending | approved | declined | expired | consumed | failed | cancelled
    decided_by   TEXT,
    decided_at   TEXT,
    reason       TEXT,
    consumed_at  TEXT,
    result       TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_approvals_state ON approvals (state, expires_at);
CREATE INDEX IF NOT EXISTS idx_approvals_run ON approvals (run_id, created_at);

CREATE TABLE IF NOT EXISTS run_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    step_id    TEXT,
    kind       TEXT NOT NULL,
    message    TEXT NOT NULL DEFAULT '',
    data       TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (run_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_run_events_run ON run_events (run_id, seq);
CREATE INDEX IF NOT EXISTS idx_run_events_id ON run_events (id);

-- Link a team task to the step it executes, so a tool call can be attributed
-- to the task that caused it.
ALTER TABLE team_tasks ADD COLUMN step_id TEXT;

-- The run that a conversation belongs to. Deterministic ids make the backfill
-- idempotent and make a missing link recoverable without guessing.
INSERT OR IGNORE INTO runs
    (id, goal, mode, state, project, thread_id, actor, created_at, updated_at, completion_policy)
SELECT
    'run-chat-' || id,
    title,
    'assistant',
    CASE state
        WHEN 'running' THEN 'executing'
        WHEN 'waiting_approval' THEN 'awaiting_approval'
        WHEN 'error' THEN 'failed'
        ELSE 'unverified'
    END,
    project,
    id,
    'system:migrated',
    created_at,
    updated_at,
    '{"kind":"conversation","source":"migrated"}'
FROM chat_threads;

INSERT OR IGNORE INTO runs
    (id, goal, mode, state, project, team_run_id, actor, acceptance, created_at, updated_at, finished_at, completion_policy)
SELECT
    'run-team-' || id,
    goal,
    'team',
    CASE state
        WHEN 'planning' THEN 'planning'
        WHEN 'awaiting_plan' THEN 'awaiting_approval'
        WHEN 'running' THEN 'executing'
        WHEN 'paused' THEN 'paused'
        WHEN 'done' THEN 'completed'
        WHEN 'failed' THEN 'failed'
        WHEN 'cancelled' THEN 'cancelled'
        ELSE 'unverified'
    END,
    project,
    id,
    created_by,
    done_when,
    created_at,
    updated_at,
    CASE WHEN state IN ('done', 'failed', 'cancelled') THEN updated_at ELSE NULL END,
    '{"kind":"team","source":"migrated"}'
FROM team_runs;

-- Pending requests made by an agent become durable approvals. They are not
-- granted: a migration must never turn a question into a yes. The arguments are
-- exactly the stored ones, so what a person answers is what was asked.
INSERT OR IGNORE INTO approvals
    (id, run_id, capability, args, args_hash, actor, requested_by, scope, tier, risk, state, expires_at, created_at)
SELECT
    r.id,
    (SELECT ru.id FROM runs ru WHERE ru.thread_id = r.thread_id LIMIT 1),
    r.capability,
    r.args,
    '',
    r.actor,
    r.actor,
    'single',
    'unknown',
    'sensitive',
    'pending',
    strftime('%Y-%m-%dT%H:%M:%SZ', r.created_at, '+24 hours'),
    r.created_at
FROM agent_requests r
WHERE r.state = 'pending';

ALTER TABLE agent_requests ADD COLUMN expires_at TEXT;
ALTER TABLE agent_requests ADD COLUMN approval_id TEXT;
ALTER TABLE agent_requests ADD COLUMN consumed_at TEXT;

UPDATE agent_requests
   SET approval_id = id,
       expires_at = (SELECT a.expires_at FROM approvals a WHERE a.id = agent_requests.id)
 WHERE approval_id IS NULL;
