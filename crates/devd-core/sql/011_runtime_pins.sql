-- A project's own choice of runtime version, ahead of the machine default.
CREATE TABLE IF NOT EXISTS runtime_pins (
    project    TEXT NOT NULL,
    kind       TEXT NOT NULL,
    version    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project, kind)
);
