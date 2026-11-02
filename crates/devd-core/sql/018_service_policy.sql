-- A service can be off, start when a site or project needs it, or always start
-- with devd. The old `enabled` flag only said "included in stack up".
ALTER TABLE services ADD COLUMN startup_mode TEXT NOT NULL DEFAULT 'on-demand';

UPDATE services SET startup_mode = CASE
    WHEN name LIKE 'php-cgi-%' OR name IN ('nginx', 'apache', 'mariadb') THEN 'on-demand'
    WHEN enabled = 1 THEN 'always'
    ELSE 'off'
END;
