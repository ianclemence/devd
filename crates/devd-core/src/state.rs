//! The authoritative state store.
//!
//! A pile of regenerated config files is a poor source of truth. `devd`
//! inverts that: **SQLite is the truth and configuration files are rendered
//! output**. That inversion is what makes version switching, rollback, audit and
//! agent-driven change deterministic instead of best-effort.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util;

/// Schema migrations, applied in order and tracked with `PRAGMA user_version`.
///
/// Migrations are strictly additive: an older `devd` can still open a database
/// written by a newer one because unknown columns and tables are ignored. That
/// property is what makes binary rollback safe.
const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../sql/001_init.sql")),
    (2, include_str!("../sql/002_work.sql")),
    (3, include_str!("../sql/003_stacks.sql")),
    (4, include_str!("../sql/004_agent.sql")),
    (5, include_str!("../sql/005_team.sql")),
    (6, include_str!("../sql/006_site_upstream.sql")),
    (7, include_str!("../sql/007_drop_receipts.sql")),
    (8, include_str!("../sql/008_goal_loop.sql")),
    (9, include_str!("../sql/009_drop_sessions.sql")),
    (10, include_str!("../sql/010_schedules.sql")),
    (11, include_str!("../sql/011_runtime_pins.sql")),
];

/// One audit row. Every capability attempt, successful or not, writes one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Row identifier.
    pub id: i64,
    /// RFC 3339 timestamp.
    pub at: String,
    /// `human:cli`, `agent:claude-code`, `system:daemon`.
    pub actor: String,
    /// Interface the call arrived through.
    pub surface: String,
    /// Capability identifier.
    pub capability: String,
    /// Permission tier of the capability.
    pub tier: String,
    /// Side-effect class.
    pub effect: String,
    /// Whether this was a plan.
    pub dry_run: bool,
    /// Whether the attempt succeeded.
    pub ok: bool,
    /// Redacted arguments.
    pub args: String,
    /// Result or error message.
    pub message: String,
}

/// A managed site (a virtual host backed by a project directory).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Site {
    /// Stable identifier.
    pub id: String,
    /// Project name, unique.
    pub name: String,
    /// Fully qualified local hostname, e.g. `blog.test`.
    pub host: String,
    /// Document root on disk.
    pub root: String,
    /// `nginx` or `apache`.
    pub web_server: String,
    /// Pinned PHP version, when set.
    pub php: Option<String>,
    /// `off`, `internal` or `trusted`.
    pub tls_mode: String,
    /// Database created alongside the site, when set.
    pub db_name: Option<String>,
    /// Address of a running program that serves this site, when it is one
    /// (`http://127.0.0.1:3000`). The site has no PHP and no document root
    /// served from disk.
    #[serde(default)]
    pub upstream: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
}

impl Site {
    /// Document root to hand to a web server: `public/` when it exists, which
    /// is what Laravel, Symfony and WordPress layouts expect.
    pub fn document_root(&self) -> String {
        let root = std::path::Path::new(&self.root);
        let public = root.join("public");
        if public.is_dir() {
            public.to_string_lossy().to_string()
        } else {
            self.root.clone()
        }
    }
}

/// An installed runtime build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    /// Row identifier.
    pub id: String,
    /// Runtime kind, e.g. `php`, `node`, `go`.
    pub kind: String,
    /// Version string.
    pub version: String,
    /// Absolute install directory.
    pub path: String,
    /// Verified artifact digest, when the provider publishes one.
    pub digest: Option<String>,
    /// Whether the version is protected from automatic removal.
    pub pinned: bool,
    /// Installation timestamp.
    pub installed_at: String,
}

/// A supervised service definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    /// Row identifier.
    pub id: String,
    /// Unique service name, e.g. `nginx`, `mariadb`.
    pub name: String,
    /// Service kind, used to pick a driver.
    pub kind: String,
    /// Runtime this service belongs to, when applicable.
    pub runtime_id: Option<String>,
    /// Executable and arguments, as a JSON array. Never a shell string.
    pub argv: String,
    /// Primary port, when the service listens.
    pub port: Option<u16>,
    /// Health check specification.
    pub health: Option<String>,
    /// `running` or `stopped` as desired by the user.
    pub desired_state: String,
    /// `never`, `on-failure` or `always`.
    pub restart_policy: String,
    /// Whether the service participates in `devd up`.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: String,
}

/// A record of one service process lifetime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRun {
    /// Row identifier.
    pub id: String,
    /// Service this run belongs to.
    pub service_id: String,
    /// Operating-system process id.
    pub pid: Option<u32>,
    /// Start timestamp.
    pub started_at: String,
    /// Stop timestamp.
    pub stopped_at: Option<String>,
    /// Process exit code.
    pub exit_code: Option<i32>,
    /// How many times the supervisor had restarted this service when the run
    /// began.
    pub restart_count: i64,
}

/// A long-running operation, surfaced to the GUI and to MCP tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Row identifier.
    pub id: String,
    /// Capability that produced the task.
    pub capability: String,
    /// `pending`, `running`, `succeeded`, `failed` or `cancelled`.
    pub state: String,
    /// Completion ratio in the range 0.0-1.0.
    pub progress: f64,
    /// Latest human-readable status line.
    pub message: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
    /// Whether cancellation has been requested.
    pub cancel_requested: bool,
    /// Structured result once finished.
    pub result: Option<String>,
}

/// A snapshot taken before a sensitive operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Row identifier.
    pub id: String,
    /// What was captured, e.g. `hosts`, `database:app`.
    pub scope: String,
    /// Path on disk.
    pub path: String,
    /// Size in bytes.
    pub bytes: i64,
    /// SHA-256 of the snapshot.
    pub checksum: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Free-text description.
    pub description: String,
}

/// A Git repository `devd` knows about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    /// Row identifier.
    pub id: String,
    /// Absolute path to the working tree.
    pub path: String,
    /// Display name, usually the directory name.
    pub name: String,
    /// `origin` URL when configured.
    pub remote_url: Option<String>,
    /// Default branch name, when known.
    pub default_branch: Option<String>,
    /// When the repository was last inspected.
    pub last_seen_at: String,
}

/// An issued leaf certificate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRecord {
    /// Row identifier.
    pub id: String,
    /// Hostname the leaf is valid for.
    pub host: String,
    /// Serial number, hex.
    pub serial: String,
    /// Expiry timestamp.
    pub not_after: String,
    /// Certificate chain, PEM.
    pub chain_pem: String,
    /// Creation timestamp.
    pub created_at: String,
}

/// Thread-safe handle to the SQLite database.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open a private in-memory database, for tests.
    ///
    /// Migrated, like [`Store::open`]. An in-memory store with no schema is a
    /// trap rather than a convenience: every table read fails with "no such
    /// table", which looks like a bug in the caller rather than in the setup.
    /// There is no persisted state to protect here, so there is nothing to be
    /// careful about.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    pub(crate) fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| crate::error::DevdError::internal("state store lock poisoned"))?;
        f(&guard)
    }

    /// Apply any outstanding migrations.
    pub fn migrate(&self) -> Result<()> {
        self.with(|conn| {
            let current: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            for (version, sql) in MIGRATIONS {
                if *version > current {
                    let tx = conn.unchecked_transaction()?;
                    tx.execute_batch(sql)?;
                    tx.pragma_update(None, "user_version", version)?;
                    tx.commit()?;
                    tracing::info!(version, "applied migration");
                }
            }
            Ok(())
        })
    }

    /// Current schema version.
    pub fn schema_version(&self) -> Result<i32> {
        self.with(|conn| {
            let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            Ok(v)
        })
    }

    /// Run SQLite's own integrity check. Used by `devd doctor`.
    pub fn integrity_check(&self) -> Result<String> {
        self.with(|conn| {
            let v: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            Ok(v)
        })
    }

    // ---------------------------------------------------------------- audit

    /// Append an audit row.
    #[allow(clippy::too_many_arguments)]
    pub fn record_audit(
        &self,
        actor: &str,
        surface: &str,
        capability: &str,
        tier: &str,
        effect: &str,
        dry_run: bool,
        ok: bool,
        args: &str,
        message: &str,
    ) -> Result<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO audit_log
                 (at, actor, surface, capability, tier, effect, dry_run, ok, args, message)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    util::now(),
                    actor,
                    surface,
                    capability,
                    tier,
                    effect,
                    dry_run as i32,
                    ok as i32,
                    args,
                    util::truncate(message, 2000)
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Most recent audit rows, newest first.
    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditEntry>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, at, actor, surface, capability, tier, effect, dry_run, ok, args, message
                 FROM audit_log ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    actor: row.get(2)?,
                    surface: row.get(3)?,
                    capability: row.get(4)?,
                    tier: row.get(5)?,
                    effect: row.get(6)?,
                    dry_run: row.get::<_, i32>(7)? != 0,
                    ok: row.get::<_, i32>(8)? != 0,
                    args: row.get(9)?,
                    message: row.get(10)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    // ------------------------------------------------------------- settings

    /// Read a value from the settings key/value table.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.with(|conn| {
            let v = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            Ok(v)
        })
    }

    /// Write a value into the settings key/value table.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    // ---------------------------------------------------------------- sites

    /// Insert or update a site.
    pub fn upsert_site(&self, site: &Site) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO sites (id, name, host, root, web_server, php, tls_mode, db_name, upstream, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(name) DO UPDATE SET
                   host = excluded.host,
                   root = excluded.root,
                   web_server = excluded.web_server,
                   php = excluded.php,
                   tls_mode = excluded.tls_mode,
                   db_name = excluded.db_name,
                   upstream = excluded.upstream",
                params![
                    site.id,
                    site.name,
                    site.host,
                    site.root,
                    site.web_server,
                    site.php,
                    site.tls_mode,
                    site.db_name,
                    site.upstream,
                    site.created_at
                ],
            )?;
            Ok(())
        })
    }

    /// Look up a site by name.
    pub fn get_site(&self, name: &str) -> Result<Option<Site>> {
        self.with(|conn| {
            let s = conn
                .query_row(
                    "SELECT id, name, host, root, web_server, php, tls_mode, db_name, upstream, created_at
                     FROM sites WHERE name = ?1",
                    params![name],
                    map_site,
                )
                .optional()?;
            Ok(s)
        })
    }

    /// All sites, ordered by name.
    pub fn list_sites(&self) -> Result<Vec<Site>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, host, root, web_server, php, tls_mode, db_name, upstream, created_at
                 FROM sites ORDER BY name",
            )?;
            let rows = stmt.query_map([], map_site)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Remove a site.
    pub fn delete_site(&self, name: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM sites WHERE name = ?1", params![name])?;
            Ok(n > 0)
        })
    }

    // ------------------------------------------------------------- runtimes

    /// Record an installed runtime.
    pub fn upsert_runtime(&self, rt: &Runtime) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO runtimes (id, kind, version, path, digest, pinned, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(kind, version) DO UPDATE SET
                   path = excluded.path,
                   digest = excluded.digest,
                   installed_at = excluded.installed_at",
                params![
                    rt.id,
                    rt.kind,
                    rt.version,
                    rt.path,
                    rt.digest,
                    rt.pinned as i32,
                    rt.installed_at
                ],
            )?;
            Ok(())
        })
    }

    /// All installed runtimes.
    pub fn list_runtimes(&self) -> Result<Vec<Runtime>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, kind, version, path, digest, pinned, installed_at
                 FROM runtimes ORDER BY kind, version",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(Runtime {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    version: row.get(2)?,
                    path: row.get(3)?,
                    digest: row.get(4)?,
                    pinned: row.get::<_, i32>(5)? != 0,
                    installed_at: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Installed versions of one runtime kind.
    pub fn runtimes_of_kind(&self, kind: &str) -> Result<Vec<Runtime>> {
        Ok(self
            .list_runtimes()?
            .into_iter()
            .filter(|r| r.kind == kind)
            .collect())
    }

    /// Remove a runtime record.
    pub fn delete_runtime(&self, kind: &str, version: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "DELETE FROM runtimes WHERE kind = ?1 AND version = ?2",
                params![kind, version],
            )?;
            Ok(n > 0)
        })
    }

    // ------------------------------------------------------------- services

    /// Insert or update a service definition.
    pub fn upsert_service(&self, svc: &Service) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO services
                   (id, name, kind, runtime_id, argv, port, health, desired_state, restart_policy, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(name) DO UPDATE SET
                   kind = excluded.kind,
                   runtime_id = excluded.runtime_id,
                   argv = excluded.argv,
                   port = excluded.port,
                   health = excluded.health,
                   desired_state = excluded.desired_state,
                   restart_policy = excluded.restart_policy,
                   enabled = excluded.enabled",
                params![
                    svc.id,
                    svc.name,
                    svc.kind,
                    svc.runtime_id,
                    svc.argv,
                    svc.port,
                    svc.health,
                    svc.desired_state,
                    svc.restart_policy,
                    svc.enabled as i32,
                    svc.created_at
                ],
            )?;
            Ok(())
        })
    }

    /// Look up a service by name.
    pub fn get_service(&self, name: &str) -> Result<Option<Service>> {
        self.with(|conn| {
            let s = conn
                .query_row(
                    "SELECT id, name, kind, runtime_id, argv, port, health, desired_state, restart_policy, enabled, created_at
                     FROM services WHERE name = ?1",
                    params![name],
                    map_service,
                )
                .optional()?;
            Ok(s)
        })
    }

    /// All service definitions.
    pub fn list_services(&self) -> Result<Vec<Service>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, kind, runtime_id, argv, port, health, desired_state, restart_policy, enabled, created_at
                 FROM services ORDER BY name",
            )?;
            let rows = stmt.query_map([], map_service)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Remove a service and the runs recorded for it.
    ///
    /// The run history goes with it: `service_runs` is a log of what this
    /// installation did, and a log about a service that no longer exists is
    /// noise. Nothing outside the store is touched - deleting files is the
    /// caller's decision, and for MariaDB the data directory is deliberately not
    /// part of the installation.
    pub fn delete_service(&self, name: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM services WHERE name = ?1", params![name])?;
            conn.execute(
                "DELETE FROM service_runs WHERE service_id NOT IN (SELECT id FROM services)",
                [],
            )?;
            Ok(n > 0)
        })
    }

    /// Record a service run starting.
    pub fn start_service_run(
        &self,
        service_id: &str,
        pid: Option<u32>,
        restart_count: i64,
    ) -> Result<String> {
        let id = util::new_id();
        self.with(|conn| {
            conn.execute(
                "INSERT INTO service_runs (id, service_id, pid, started_at, restart_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, service_id, pid, util::now(), restart_count],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Record a service run ending, plus the resulting desired state.
    pub fn finish_service_run(&self, id: &str, exit_code: i32) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE service_runs SET stopped_at = ?2, exit_code = ?3 WHERE id = ?1",
                params![id, util::now(), exit_code],
            )?;
            Ok(())
        })
    }

    /// Recent runs for a service.
    pub fn list_service_runs(&self, service_id: &str, limit: u32) -> Result<Vec<ServiceRun>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, service_id, pid, started_at, stopped_at, exit_code, restart_count
                 FROM service_runs WHERE service_id = ?1 ORDER BY started_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![service_id, limit], |row| {
                Ok(ServiceRun {
                    id: row.get(0)?,
                    service_id: row.get(1)?,
                    pid: row.get::<_, Option<i64>>(2)?.map(|v| v as u32),
                    started_at: row.get(3)?,
                    stopped_at: row.get(4)?,
                    exit_code: row.get(5)?,
                    restart_count: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    // ---------------------------------------------------------------- tasks

    /// Create a task row.
    pub fn create_task(&self, capability: &str, message: &str) -> Result<String> {
        let id = util::new_id();
        self.with(|conn| {
            conn.execute(
                "INSERT INTO tasks (id, capability, state, progress, message, created_at, updated_at, cancel_requested)
                 VALUES (?1, ?2, 'pending', 0.0, ?3, ?4, ?4, 0)",
                params![id, capability, message, util::now()],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Update a task's state, progress and message.
    pub fn update_task(&self, id: &str, state: &str, progress: f64, message: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE tasks SET state = ?2, progress = ?3, message = ?4, updated_at = ?5 WHERE id = ?1",
                params![id, state, progress, message, util::now()],
            )?;
            Ok(())
        })
    }

    /// Store a finished task's structured result.
    pub fn finish_task(&self, id: &str, state: &str, result: &str) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE tasks SET state = ?2, progress = 1.0, result = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, state, result, util::now()],
            )?;
            Ok(())
        })
    }

    /// Request cancellation of a task.
    pub fn request_task_cancel(&self, id: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute(
                "UPDATE tasks SET cancel_requested = 1, updated_at = ?2 WHERE id = ?1",
                params![id, util::now()],
            )?;
            Ok(n > 0)
        })
    }

    /// A single task.
    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        self.with(|conn| {
            let t = conn
                .query_row(
                    "SELECT id, capability, state, progress, message, created_at, updated_at, cancel_requested, result
                     FROM tasks WHERE id = ?1",
                    params![id],
                    map_task,
                )
                .optional()?;
            Ok(t)
        })
    }

    /// Recent tasks, newest first.
    pub fn list_tasks(&self, limit: u32) -> Result<Vec<Task>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, capability, state, progress, message, created_at, updated_at, cancel_requested, result
                 FROM tasks ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], map_task)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    // ------------------------------------------------------------ snapshots

    /// Record a snapshot.
    pub fn create_snapshot(&self, snap: &Snapshot) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO snapshots (id, scope, path, bytes, checksum, created_at, description)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    snap.id,
                    snap.scope,
                    snap.path,
                    snap.bytes,
                    snap.checksum,
                    snap.created_at,
                    snap.description
                ],
            )?;
            Ok(())
        })
    }

    /// Snapshots for a scope, newest first.
    pub fn list_snapshots(&self, scope: Option<&str>, limit: u32) -> Result<Vec<Snapshot>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, scope, path, bytes, checksum, created_at, description
                 FROM snapshots
                 WHERE (?1 IS NULL OR scope = ?1)
                 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![scope, limit], |row| {
                Ok(Snapshot {
                    id: row.get(0)?,
                    scope: row.get(1)?,
                    path: row.get(2)?,
                    bytes: row.get(3)?,
                    checksum: row.get(4)?,
                    created_at: row.get(5)?,
                    description: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// A single snapshot by id.
    pub fn get_snapshot(&self, id: &str) -> Result<Option<Snapshot>> {
        self.with(|conn| {
            let s = conn
                .query_row(
                    "SELECT id, scope, path, bytes, checksum, created_at, description
                     FROM snapshots WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(Snapshot {
                            id: row.get(0)?,
                            scope: row.get(1)?,
                            path: row.get(2)?,
                            bytes: row.get(3)?,
                            checksum: row.get(4)?,
                            created_at: row.get(5)?,
                            description: row.get(6)?,
                        })
                    },
                )
                .optional()?;
            Ok(s)
        })
    }

    // ---------------------------------------------------------------- repos

    /// Insert or update a known repository.
    pub fn upsert_repo(&self, repo: &Repo) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO repos (id, path, name, remote_url, default_branch, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                   name = excluded.name,
                   remote_url = excluded.remote_url,
                   default_branch = excluded.default_branch,
                   last_seen_at = excluded.last_seen_at",
                params![
                    repo.id,
                    repo.path,
                    repo.name,
                    repo.remote_url,
                    repo.default_branch,
                    repo.last_seen_at
                ],
            )?;
            Ok(())
        })
    }

    /// A repository by path.
    pub fn get_repo_by_path(&self, path: &str) -> Result<Option<Repo>> {
        self.with(|conn| {
            let r = conn
                .query_row(
                    "SELECT id, path, name, remote_url, default_branch, last_seen_at
                     FROM repos WHERE path = ?1",
                    params![path],
                    map_repo,
                )
                .optional()?;
            Ok(r)
        })
    }

    /// All known repositories.
    pub fn list_repos(&self) -> Result<Vec<Repo>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, path, name, remote_url, default_branch, last_seen_at
                 FROM repos ORDER BY last_seen_at DESC",
            )?;
            let rows = stmt.query_map([], map_repo)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Forget a repository.
    pub fn delete_repo(&self, path: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM repos WHERE path = ?1", params![path])?;
            Ok(n > 0)
        })
    }

    // ---------------------------------------------------------------- certs

    /// Record an issued leaf certificate.
    pub fn upsert_cert(&self, cert: &CertRecord) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO certs (id, host, serial, not_after, chain_pem, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(host) DO UPDATE SET
                   serial = excluded.serial,
                   not_after = excluded.not_after,
                   chain_pem = excluded.chain_pem",
                params![
                    cert.id,
                    cert.host,
                    cert.serial,
                    cert.not_after,
                    cert.chain_pem,
                    cert.created_at
                ],
            )?;
            Ok(())
        })
    }

    /// A certificate by hostname.
    pub fn get_cert(&self, host: &str) -> Result<Option<CertRecord>> {
        self.with(|conn| {
            let c = conn
                .query_row(
                    "SELECT id, host, serial, not_after, chain_pem, created_at
                     FROM certs WHERE host = ?1",
                    params![host],
                    |row| {
                        Ok(CertRecord {
                            id: row.get(0)?,
                            host: row.get(1)?,
                            serial: row.get(2)?,
                            not_after: row.get(3)?,
                            chain_pem: row.get(4)?,
                            created_at: row.get(5)?,
                        })
                    },
                )
                .optional()?;
            Ok(c)
        })
    }

    /// All issued certificates.
    pub fn list_certs(&self) -> Result<Vec<CertRecord>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, host, serial, not_after, chain_pem, created_at FROM certs ORDER BY host",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(CertRecord {
                    id: row.get(0)?,
                    host: row.get(1)?,
                    serial: row.get(2)?,
                    not_after: row.get(3)?,
                    chain_pem: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
    }

    /// Remove a certificate record.
    pub fn delete_cert(&self, host: &str) -> Result<bool> {
        self.with(|conn| {
            let n = conn.execute("DELETE FROM certs WHERE host = ?1", params![host])?;
            Ok(n > 0)
        })
    }

    /// Count rows in a table, for diagnostics.
    pub fn count(&self, table: &str) -> Result<i64> {
        let allowed = [
            "audit_log",
            "sites",
            "runtimes",
            "services",
            "tasks",
            "snapshots",
            "repos",
            "certs",
        ];
        if !allowed.contains(&table) {
            return Err(crate::error::DevdError::invalid(format!(
                "unknown table `{table}`"
            )));
        }
        self.with(|conn| {
            let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
            Ok(n)
        })
    }
}

fn map_site(row: &rusqlite::Row<'_>) -> rusqlite::Result<Site> {
    Ok(Site {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        root: row.get(3)?,
        web_server: row.get(4)?,
        php: row.get(5)?,
        tls_mode: row.get(6)?,
        db_name: row.get(7)?,
        upstream: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn map_service(row: &rusqlite::Row<'_>) -> rusqlite::Result<Service> {
    Ok(Service {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        runtime_id: row.get(3)?,
        argv: row.get(4)?,
        port: row.get::<_, Option<i64>>(5)?.map(|v| v as u16),
        health: row.get(6)?,
        desired_state: row.get(7)?,
        restart_policy: row.get(8)?,
        enabled: row.get::<_, i32>(9)? != 0,
        created_at: row.get(10)?,
    })
}

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        capability: row.get(1)?,
        state: row.get(2)?,
        progress: row.get(3)?,
        message: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        cancel_requested: row.get::<_, i32>(7)? != 0,
        result: row.get(8)?,
    })
}

fn map_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        remote_url: row.get(3)?,
        default_branch: row.get(4)?,
        last_seen_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.migrate().unwrap();
        s
    }

    #[test]
    fn the_unused_receipts_table_is_gone_and_a_database_that_had_it_migrates() {
        let s = store();
        let tables: Vec<String> = s
            .with(|conn| {
                let mut stmt =
                    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                Ok(rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap();
        assert!(!tables.iter().any(|t| t == "receipts"), "{tables:?}");
        assert!(tables.iter().any(|t| t == "sites"), "{tables:?}");

        // An installation made before the table was dropped has it, and its
        // data, and must still open: migrate a version-6 database.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            for (_, sql) in &MIGRATIONS[..6] {
                conn.execute_batch(sql).unwrap();
            }
            conn.execute_batch(
                "PRAGMA user_version = 6;
                 INSERT INTO receipts (id, capability, created_at)
                 VALUES ('r1', 'site.create', '2026-10-21T00:00:00Z');",
            )
            .unwrap();
        }
        let upgraded = Store::open(&path).unwrap();
        upgraded.migrate().unwrap();
        assert_eq!(upgraded.schema_version().unwrap(), MIGRATIONS.len() as i32);
        assert!(upgraded.count("sites").is_ok());
    }

    #[test]
    fn migrations_set_schema_version() {
        let s = store();
        assert!(s.schema_version().unwrap() >= 1);
        assert_eq!(s.integrity_check().unwrap(), "ok");
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let s = store();
        let v1 = s.schema_version().unwrap();
        s.migrate().unwrap();
        assert_eq!(s.schema_version().unwrap(), v1);
    }

    #[test]
    fn site_round_trip_and_delete() {
        let s = store();
        let site = Site {
            id: "1".into(),
            name: "blog".into(),
            host: "blog.test".into(),
            root: "C:/devd/www/blog".into(),
            web_server: "nginx".into(),
            php: Some("8.3.14".into()),
            tls_mode: "internal".into(),
            db_name: Some("blog".into()),
            upstream: None,
            created_at: util::now(),
        };
        s.upsert_site(&site).unwrap();
        assert_eq!(s.get_site("blog").unwrap().unwrap(), site);
        assert_eq!(s.list_sites().unwrap().len(), 1);
        assert!(s.delete_site("blog").unwrap());
        assert!(s.get_site("blog").unwrap().is_none());
    }

    #[test]
    fn audit_secrets_are_redacted_by_the_caller_not_the_store() {
        let s = store();
        s.record_audit(
            "human:cli",
            "cli",
            "demo",
            "read",
            "read",
            false,
            true,
            "{}",
            "ok",
        )
        .unwrap();
        let rows = s.list_audit(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].ok);
    }

    #[test]
    fn runtimes_upsert_by_kind_and_version() {
        let s = store();
        let mut rt = Runtime {
            id: "a".into(),
            kind: "php".into(),
            version: "8.3.14".into(),
            path: "C:/devd/runtimes/php/8.3.14".into(),
            digest: Some("deadbeef".into()),
            pinned: false,
            installed_at: util::now(),
        };
        s.upsert_runtime(&rt).unwrap();
        rt.path = "C:/other".into();
        s.upsert_runtime(&rt).unwrap();
        let all = s.list_runtimes().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].path, "C:/other");
    }

    #[test]
    fn tasks_transition_and_cancel() {
        let s = store();
        let id = s.create_task("runtime.install", "installing php").unwrap();
        s.update_task(&id, "running", 0.5, "halfway").unwrap();
        assert_eq!(s.get_task(&id).unwrap().unwrap().state, "running");
        assert!(s.request_task_cancel(&id).unwrap());
        assert!(s.get_task(&id).unwrap().unwrap().cancel_requested);
        s.finish_task(&id, "cancelled", "{}").unwrap();
        assert_eq!(s.get_task(&id).unwrap().unwrap().state, "cancelled");
    }

    #[test]
    fn document_root_prefers_public_dir() {
        let dir = std::env::temp_dir().join(format!("devd-site-{}", std::process::id()));
        let public = dir.join("public");
        std::fs::create_dir_all(&public).unwrap();
        let site = Site {
            id: "1".into(),
            name: "app".into(),
            host: "app.test".into(),
            root: dir.to_string_lossy().to_string(),
            web_server: "nginx".into(),
            php: None,
            tls_mode: "off".into(),
            db_name: None,
            upstream: None,
            created_at: util::now(),
        };
        assert!(site.document_root().ends_with("public"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_table_count_is_rejected() {
        let s = store();
        assert!(s.count("users; DROP TABLE sites").is_err());
    }
}
