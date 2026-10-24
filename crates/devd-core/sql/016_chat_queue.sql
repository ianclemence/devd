-- Messages a person writes while the assistant is working.
--
-- They are not part of the conversation yet: a user message appended between a tool call and its
-- result would break the sequence the model requires. The loop takes them at the next step boundary
-- and appends each as an ordinary user event, so what the model sees is still the log.
CREATE TABLE IF NOT EXISTS chat_queue (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id  TEXT NOT NULL,
    text       TEXT NOT NULL,
    from_actor TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_queue_thread ON chat_queue (thread_id, id);
