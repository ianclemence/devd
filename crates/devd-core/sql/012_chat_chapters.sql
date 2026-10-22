-- A message a person pinned as a chapter of a conversation, so the conversation has an outline.
CREATE TABLE IF NOT EXISTS chat_chapters (
    id         TEXT PRIMARY KEY,
    thread_id  TEXT NOT NULL,
    event_id   INTEGER NOT NULL,
    title      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (thread_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_chat_chapters_thread ON chat_chapters (thread_id, event_id);
