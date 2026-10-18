-- devd schema, version 1.
--
-- Design rule: this database is the source of truth. Files under `etc/` are
-- rendered from it. Migrations are strictly additive so a newer database stays
-- readable by an older binary (binary rollback safety).

CREATE TABLE IF NOT EXISTS audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    at          TEXT NOT NULL,
    actor       TEXT NOT NULL,
    surface     TEXT NOT NULL,
    capability  TEXT NOT NULL,
    tier        TEXT NOT NULL,
    effect      TEXT NOT NULL,
    dry_run     INTEGER NOT NULL DEFAULT 0,
    ok          INTEGER NOT NULL DEFAULT 0,
    args        TEXT NOT NULL DEFAULT '{}',
    message     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_audit_at ON audit_log (at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_capability ON audit_log (capability);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sites (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    host        TEXT NOT NULL,
    root        TEXT NOT NULL,
    web_server  TEXT NOT NULL DEFAULT 'nginx',
    php         TEXT,
    tls_mode    TEXT NOT NULL DEFAULT 'off',
    db_name     TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runtimes (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    version      TEXT NOT NULL,
    path         TEXT NOT NULL,
    digest       TEXT,
    pinned       INTEGER NOT NULL DEFAULT 0,
    installed_at TEXT NOT NULL,
    UNIQUE (kind, version)
);
CREATE INDEX IF NOT EXISTS idx_runtimes_kind ON runtimes (kind);

CREATE TABLE IF NOT EXISTS services (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL UNIQUE,
    kind           TEXT NOT NULL,
    runtime_id     TEXT REFERENCES runtimes (id) ON DELETE SET NULL,
    argv           TEXT NOT NULL,
    port           INTEGER,
    health         TEXT,
    desired_state  TEXT NOT NULL DEFAULT 'stopped',
    restart_policy TEXT NOT NULL DEFAULT 'on-failure',
    enabled        INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS service_runs (
    id            TEXT PRIMARY KEY,
    service_id    TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    pid           INTEGER,
    started_at    TEXT NOT NULL,
    stopped_at    TEXT,
    exit_code     INTEGER,
    restart_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_service_runs ON service_runs (service_id, started_at DESC);

CREATE TABLE IF NOT EXISTS tasks (
    id               TEXT PRIMARY KEY,
    capability       TEXT NOT NULL,
    state            TEXT NOT NULL DEFAULT 'pending',
    progress         REAL NOT NULL DEFAULT 0.0,
    message          TEXT NOT NULL DEFAULT '',
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    result           TEXT
);

CREATE TABLE IF NOT EXISTS snapshots (
    id          TEXT PRIMARY KEY,
    scope       TEXT NOT NULL,
    path        TEXT NOT NULL,
    bytes       INTEGER NOT NULL DEFAULT 0,
    checksum    TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_snapshots_scope ON snapshots (scope, created_at DESC);

CREATE TABLE IF NOT EXISTS receipts (
    id              TEXT PRIMARY KEY,
    capability      TEXT NOT NULL,
    undo_capability TEXT,
    undo_args       TEXT,
    created_at      TEXT NOT NULL,
    applied         INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS repos (
    id             TEXT PRIMARY KEY,
    path           TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    remote_url     TEXT,
    default_branch TEXT,
    last_seen_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS certs (
    id         TEXT PRIMARY KEY,
    host       TEXT NOT NULL UNIQUE,
    serial     TEXT NOT NULL,
    not_after  TEXT NOT NULL,
    chain_pem  TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS remotes (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    host         TEXT NOT NULL,
    user         TEXT NOT NULL,
    key_ref      TEXT,
    trust_domain TEXT NOT NULL DEFAULT 'dev',
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS deployments (
    id         TEXT PRIMARY KEY,
    remote_id  TEXT NOT NULL REFERENCES remotes (id) ON DELETE CASCADE,
    project    TEXT NOT NULL,
    release    TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'pending',
    flipped_at TEXT
);
