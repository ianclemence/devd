-- A conversation may work in its own worktree instead of the project's folder.
--
-- The project stays the project (what the sidebar groups under, what the run
-- records), and `workdir` is the folder the agent's tools actually run in. Both
-- are absolute paths. Null means "the project's own folder", which is what every
-- conversation did before this column existed.
ALTER TABLE chat_threads ADD COLUMN workdir TEXT;
