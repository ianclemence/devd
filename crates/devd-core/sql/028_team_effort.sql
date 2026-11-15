-- A run remembers how much its agents may think, so every task it starts uses the choice the person made.
--
-- Every change is additive, so an older devd can still read the database.

-- The reasoning effort the run was started with: none, low, medium, high, extra or max.
-- Not set means the ordinary default.
ALTER TABLE team_runs ADD COLUMN effort TEXT;
