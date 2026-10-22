-- A run can carry a definition of done: criteria the finished work must meet.
-- `rounds` counts the extra passes devd made because some were not met.
ALTER TABLE team_runs ADD COLUMN done_when TEXT NOT NULL DEFAULT '[]';
ALTER TABLE team_runs ADD COLUMN rounds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE team_runs ADD COLUMN max_rounds INTEGER NOT NULL DEFAULT 2;
