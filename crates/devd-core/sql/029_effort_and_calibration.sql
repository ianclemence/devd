-- Two things a long-lived conversation should not forget: how much its model may think, and how
-- many characters that model's tokens actually hold.
--
-- Every change is additive, so an older devd can still read the database.

-- The reasoning effort chosen for this conversation. The hub keeps a live copy for speed; this is
-- what survives a restart, so "Thinking off" is still off tomorrow.
ALTER TABLE chat_threads ADD COLUMN effort TEXT;

-- What one provider-counted token of this model's traffic held, in characters, learned from calls
-- that actually completed. `samples` says how much the figure should be trusted.
CREATE TABLE IF NOT EXISTS model_calibration (
    model           TEXT PRIMARY KEY,
    chars_per_token REAL NOT NULL,
    samples         INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL
);
