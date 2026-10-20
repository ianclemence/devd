-- devd schema, version 4: AI providers, the model catalogue, built-in agent
-- conversations, the inbox and the queue of operations agents asked a person to
-- approve. Additive only. API keys are never stored here; see the providers
-- module for where they live.
CREATE TABLE IF NOT EXISTS providers (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,           -- anthropic | openai | openai-compatible
    label         TEXT NOT NULL,
    base_url      TEXT NOT NULL,
    key_env       TEXT,                    -- environment variable holding the key, if used
    default_model TEXT,
    is_default    INTEGER NOT NULL DEFAULT 0,
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_catalogue (
    source         TEXT NOT NULL,          -- anthropic-docs | provider:<id>
    model_id       TEXT NOT NULL,
    display        TEXT NOT NULL,
    description    TEXT,
    context_window INTEGER,
    max_output     INTEGER,
    input_price    REAL,                   -- USD per million tokens
    output_price   REAL,
    retirement     TEXT,
    url            TEXT,
    fetched_at     TEXT NOT NULL,
    PRIMARY KEY (source, model_id)
);

CREATE TABLE IF NOT EXISTS chat_threads (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    project     TEXT,
    provider_id TEXT,
    model       TEXT,
    state       TEXT NOT NULL DEFAULT 'idle', -- idle | running | waiting_approval | error
    error       TEXT,
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- The session: an append-only log. The harness holds no state of its own.
CREATE TABLE IF NOT EXISTS chat_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id  TEXT NOT NULL,
    kind       TEXT NOT NULL,              -- user | assistant | tool_result | notice | error
    data       TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_events_thread ON chat_events (thread_id, id);

CREATE TABLE IF NOT EXISTS inbox (
    id          TEXT PRIMARY KEY,
    from_actor  TEXT NOT NULL,
    project     TEXT,
    subject     TEXT NOT NULL,
    body        TEXT NOT NULL,
    needs_reply INTEGER NOT NULL DEFAULT 0,
    state       TEXT NOT NULL DEFAULT 'open', -- open | resolved
    reply       TEXT,
    replied_at  TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_requests (
    id          TEXT PRIMARY KEY,
    actor       TEXT NOT NULL,             -- e.g. agent:claude-code
    capability  TEXT NOT NULL,
    args        TEXT NOT NULL,
    plan        TEXT,
    state       TEXT NOT NULL DEFAULT 'pending', -- pending | approved | declined | failed
    result      TEXT,
    error       TEXT,
    thread_id   TEXT,
    tool_use_id TEXT,
    created_at  TEXT NOT NULL,
    decided_at  TEXT,
    decided_by  TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_requests_state ON agent_requests (state, created_at DESC);
