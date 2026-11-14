-- A conversation can be pinned to the top of the list, and one made from another remembers where it came from.
--
-- Every change is additive, so an older devd can still read the database.

-- When the person pinned it. Not set means not pinned.
ALTER TABLE chat_threads ADD COLUMN pinned_at TEXT;

-- The conversation this one was forked from, and the last event it took from it.
ALTER TABLE chat_threads ADD COLUMN forked_from TEXT;
ALTER TABLE chat_threads ADD COLUMN forked_at_event INTEGER;
