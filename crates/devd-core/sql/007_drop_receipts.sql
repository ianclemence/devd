-- The receipts table was meant to back a `devd undo` command that was never
-- built, and nothing read or wrote it. Dropping it removes a promise the
-- product does not keep.
DROP TABLE IF EXISTS receipts;
