-- Full-text search across conversations.
--
-- One row per chat event; the text is extracted in Rust and written by the store, so
-- agent and tool output is searchable without making SQL parse every payload shape.
-- Additive: an older devd ignores the table.

CREATE VIRTUAL TABLE IF NOT EXISTS chat_search USING fts5(
  thread_id UNINDEXED,
  event_id UNINDEXED,
  kind UNINDEXED,
  body,
  tokenize = 'unicode61'
);
