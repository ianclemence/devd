-- Link an inbox message to the run and task that produced it.
--
-- Until now the relation was guessed from the subject text: a message was
-- "about a run" only if the words after the first colon happened to equal a
-- live run title. That is why deleting a run turned its messages into new
-- decisions, and why the badge, the Inbox and Home could each count a
-- different number. The origin columns make the relation a fact the database
-- keeps, so every surface can derive from it.
--
-- Existing rows keep NULL: they predate the column, and are matched by title
-- once, during the delete or cancel that first needs to resolve them.
ALTER TABLE inbox ADD COLUMN run_id TEXT;
ALTER TABLE inbox ADD COLUMN task_id TEXT;
ALTER TABLE inbox ADD COLUMN kind TEXT;

CREATE INDEX IF NOT EXISTS idx_inbox_run ON inbox(run_id);

-- One-time repair of notifications written before this migration.
--
-- Those rows duplicated a run's own Needs-you card: the run view, Home and the
-- Inbox each offered the same decision, and deleting the run left the copy
-- behind as a fresh question. The run is the source of truth now, so the
-- machine-written copy is removed. This covers the formats the old code
-- produced, including `Plan ready for approval:`, which no code writes any
-- more. A row a person wrote is not matched by these prefixes.
DELETE FROM inbox
WHERE state = 'open'
  AND (
    subject LIKE 'Run paused:%'
    OR subject LIKE 'Plan ready:%'
    OR subject LIKE 'Plan ready for approval:%'
  );

