-- A conversation can be put aside until a time, and comes back by itself.
--
-- Snoozing is not settling: settled work is finished, snoozed work is waiting for a better hour.
-- An ISO 8601 instant, or NULL when the conversation is not snoozed.
ALTER TABLE chat_threads ADD COLUMN snoozed_until TEXT;
