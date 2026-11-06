-- Conversation reliability, true usage, pull requests and the event router.
--
-- Every column is additive so an older devd can still read the database.

ALTER TABLE chat_threads ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chat_threads ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chat_threads ADD COLUMN cost_usd REAL;
ALTER TABLE chat_threads ADD COLUMN resume_at TEXT;
ALTER TABLE chat_threads ADD COLUMN pr_url TEXT;
ALTER TABLE chat_threads ADD COLUMN settled_at TEXT;

-- One message from a client is delivered once, even when a reconnect retries it.
CREATE TABLE IF NOT EXISTS chat_client_messages (
  client_id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  created_at TEXT NOT NULL
);

-- Incoming hooks: a token, an event, an optional filter, and one action.
CREATE TABLE IF NOT EXISTS automations (
  name TEXT PRIMARY KEY,
  token TEXT NOT NULL UNIQUE,
  event TEXT NOT NULL,
  filter TEXT NOT NULL DEFAULT '{}',
  action TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  last_at TEXT,
  last_error TEXT
);

CREATE INDEX IF NOT EXISTS automations_event ON automations(event);
