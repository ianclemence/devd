-- The audit log becomes a hash chain: each row carries the hash of the one before it.
-- Strictly additive. Rows written before this migration keep an empty hash and are
-- reported as unchained, never as broken.
ALTER TABLE audit_log ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE audit_log ADD COLUMN hash TEXT NOT NULL DEFAULT '';
