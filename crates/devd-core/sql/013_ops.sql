-- Everything a site's operations need that is more than a setting: cron jobs, queue daemons, redirects,
-- deploy history, notification targets, the deploy script and the repository link. One table of small
-- JSON documents, keyed by kind, because each kind is read and written whole and none is joined to another.
CREATE TABLE IF NOT EXISTS ops_items (
    id         TEXT NOT NULL,
    kind       TEXT NOT NULL,
    site       TEXT NOT NULL DEFAULT '',
    data       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (kind, id)
);
CREATE INDEX IF NOT EXISTS idx_ops_items_site ON ops_items (kind, site, created_at);
