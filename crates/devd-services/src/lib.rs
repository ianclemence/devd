//! Managed services: nginx or Apache, the PHP FastCGI listener, and MariaDB.
//!
//! A service is the piece that turns installed files into a running thing. The
//! shape here follows from one decision: **`devd` does not require a background
//! daemon**.
//!
//! `devd service start nginx` spawns nginx detached, records its process id, and
//! exits. The pid file plus the process image name are the record of what is
//! running, so `devd service stop nginx` in a later shell finds it, and so does
//! the application window. When a long-lived surface *does* hold the process - the
//! application, or `devd serve` - the shared supervisor is used instead, and it
//! waits for a clean exit rather than killing by pid.
//!
//! Every start is verified rather than assumed: the port has to open, or the
//! service is killed and the failure is reported with the tail of its own log. A
//! service that is "started" but not listening is the single most useless state a
//! development environment can report.

pub mod apache;
pub mod share;
pub mod autostart;
pub mod caps;
pub mod catalogue;
pub mod dump;
pub mod engines;
pub mod extras;
pub mod mariadb;
pub mod phpcaps;
pub mod redirects;
pub mod siteconf;
pub mod nginx;
pub mod system;
pub mod tls;
pub mod web;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::doctor::Check;
use devd_core::download::{Artifact, ProgressFn};
use devd_core::paths::Paths;
use devd_core::platform::{self, Host, Os, SystemHost};
use devd_core::process::{self, SpawnSpec, Supervisor};
use devd_core::state::{Service, Store};
use devd_core::util;
use devd_core::{archive, DevdError, Result};
use devd_runtime::catalogue::{resolve, Available};

/// What `devd` knows how to install as a service.
pub struct Definition {
    /// Service name, also the directory under `services/`.
    pub name: &'static str,
    /// Driver kind.
    pub kind: &'static str,
    /// One-line description for discovery.
    pub summary: &'static str,
    /// Executable on Windows, relative to the install directory.
    pub windows_exe: &'static str,
    /// Executable on Linux and macOS, relative to the install directory.
    pub unix_exe: &'static str,
    /// The port it listens on by default.
    pub default_port: u16,
    /// How long to wait for the port to open before calling the start a failure.
    pub startup: Duration,
}

/// The installable services.
pub const DEFINITIONS: &[Definition] = &[
    Definition {
        name: "nginx",
        kind: "nginx",
        summary: "nginx, the web server that fronts every site",
        windows_exe: "nginx.exe",
        unix_exe: "sbin/nginx",
        default_port: 0,
        startup: Duration::from_secs(10),
    },
    Definition {
        name: "apache",
        kind: "apache",
        summary: "Apache httpd, for the sites that depend on .htaccess",
        windows_exe: "Apache24/bin/httpd.exe",
        unix_exe: "bin/httpd",
        default_port: 0,
        startup: Duration::from_secs(20),
    },
    Definition {
        name: "redis",
        kind: "redis",
        summary: "Redis, for cache, queues and sessions",
        windows_exe: "redis-server.exe",
        unix_exe: "bin/redis-server",
        default_port: 6379,
        startup: Duration::from_secs(10),
    },
    Definition {
        name: "postgres",
        kind: "postgres",
        summary: "PostgreSQL, for projects that need it",
        windows_exe: "bin/postgres.exe",
        unix_exe: "bin/postgres",
        default_port: 5432,
        startup: Duration::from_secs(40),
    },
    Definition {
        name: "mongodb",
        kind: "mongodb",
        summary: "MongoDB, a document database",
        windows_exe: "bin/mongod.exe",
        unix_exe: "bin/mongod",
        default_port: 27017,
        startup: Duration::from_secs(40),
    },
    Definition {
        name: "memcached",
        kind: "memcached",
        summary: "Memcached, an in-memory cache",
        windows_exe: "bin/memcached.exe",
        unix_exe: "memcached",
        default_port: 11211,
        startup: Duration::from_secs(10),
    },
    Definition {
        name: "mailpit",
        kind: "mailpit",
        summary: "Mailpit, which catches the mail your applications send",
        windows_exe: "mailpit.exe",
        unix_exe: "mailpit",
        default_port: extras::MAILPIT_UI_PORT,
        startup: Duration::from_secs(10),
    },
    Definition {
        name: "mariadb",
        kind: "mariadb",
        summary: "MariaDB, the database for new projects",
        windows_exe: "bin/mysqld.exe",
        unix_exe: "bin/mariadbd",
        default_port: 3306,
        startup: Duration::from_secs(60),
    },
];

impl Definition {
    /// The executable on this platform, relative to the install directory.
    pub fn executable(&self) -> &'static str {
        if cfg!(windows) {
            self.windows_exe
        } else {
            self.unix_exe
        }
    }
}

/// Look up a service definition.
pub fn definition(name: &str) -> Option<&'static Definition> {
    DEFINITIONS
        .iter()
        .find(|d| d.name == name || d.kind == name)
}

/// The PHP FastCGI listener that nginx hands PHP requests to.
///
/// It is not "installed" like the others: it comes from a PHP runtime, so its
/// definition is synthesized from the runtime row.
pub const PHP_SERVICE: &str = "php-cgi";

/// The port the PHP listener uses.
pub const PHP_PORT: u16 = 9000;

/// The name of the listener for one PHP series.
///
/// A site can be pinned to a PHP version, and each version needs its own
/// listener: two listeners cannot share a port. The listener is named after
/// the *series* - 8.5, not 8.5.11 - because a patch release is not a
/// different thing to a site, and two patch releases of one series would
/// fight over the same port.
pub fn php_service_name(series: &str) -> String {
    format!("{PHP_SERVICE}-{series}")
}

/// The series of a PHP version: `8.5.11` is the series `8.5`.
///
/// A site records the version it was created against, which may be a full
/// version, a series, or nothing at all. The series is what a listener is
/// per, so that is what everything else is resolved through.
pub fn php_series(version: &str) -> String {
    let trimmed = version.trim();
    let mut parts = trimmed.split('.');
    let major = parts.next().unwrap_or("");
    let minor = parts.next().unwrap_or("");
    if major.is_empty() || minor.is_empty() {
        return trimmed.to_string();
    }
    format!("{major}.{minor}")
}

/// The port the listener for a PHP series uses.
///
/// `9000 + major * 10 + minor`, so 8.3 is 9083 and 8.5 is 9085. The scheme
/// is the one Laravel Herd uses, so the numbers are recognisable.
pub fn php_port_for_series(series: &str) -> u16 {
    let mut parts = series.split('.');
    let major: u16 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor: u16 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let offset = major.saturating_mul(10).saturating_add(minor);
    // The offset is bounded so that a nonsense version cannot produce a port
    // outside the range the supervisor and the firewall rules expect.
    PHP_PORT.saturating_add(offset.min(99))
}

/// How many PHP workers each series has, as the settings file says now (the `Ctx` holds a snapshot
/// from start-up, and a change made through `php.configure` must show at once).
pub fn php_workers(ctx: &Ctx) -> u8 {
    devd_core::config::Settings::load(&ctx.paths.settings_file())
        .map(|s| s.php_workers)
        .unwrap_or(ctx.settings.php_workers)
        .clamp(1, 16)
}

/// The port of worker `k` (counted from 1) in the pool for a PHP series.
///
/// `php-cgi` answers one request at a time, so a series is served by a small pool of them. Worker 1 is
/// the series' own port; the others are 100 apart (9085, 9185, 9285), clear of every other series.
pub fn php_worker_port(series: &str, k: u8) -> u16 {
    php_port_for_series(series).saturating_add(100u16.saturating_mul(u16::from(k.max(1)) - 1))
}

/// The service name of worker `k` in the pool for a PHP series: `php-cgi-8.4`, `php-cgi-8.4-2`, ...
pub fn php_worker_name(series: &str, k: u8) -> String {
    if k <= 1 {
        php_service_name(series)
    } else {
        format!("{}-{k}", php_service_name(series))
    }
}

/// The series and pool of a PHP listener's service name: `php-cgi-8.4-3` is (`8.4`, false), `php-cgi-8.4-alt` is
/// (`8.4`, true). `None` for any other service, including the legacy single `php-cgi`.
pub fn php_pool_of(name: &str) -> Option<(String, bool)> {
    let rest = name.strip_prefix(&format!("{PHP_SERVICE}-"))?;
    let (series, alt, _) = split_pool(rest);
    series.chars().next().filter(char::is_ascii_digit).map(|_| (series, alt))
}

/// The port of worker `k` in the pool that runs the *other* PHP profile: 50 above the main pool.
///
/// A site can run with a limits profile different from the machine's (`php.site_profile`). Those sites
/// are served by a second pool of PHP processes started with the other profile's ini.
pub fn php_alt_port(series: &str, k: u8) -> u16 {
    php_worker_port(series, k).saturating_add(50)
}

/// The service name of worker `k` in the other-profile pool: `php-cgi-8.4-alt`, `php-cgi-8.4-alt-2`, ...
pub fn php_alt_name(series: &str, k: u8) -> String {
    if k <= 1 {
        format!("{}-alt", php_service_name(series))
    } else {
        format!("{}-alt-{k}", php_service_name(series))
    }
}

/// Split what follows `php-cgi-` into the series, whether it is the other-profile pool, and the worker
/// number: `8.4-3` is (`8.4`, false, 3) and `8.4-alt-2` is (`8.4`, true, 2).
fn split_pool(rest: &str) -> (String, bool, u8) {
    let (head, k) = match rest.rsplit_once('-') {
        Some((h, k)) if k.parse::<u8>().is_ok() => (h, k.parse::<u8>().unwrap_or(1).max(1)),
        _ => (rest, 1),
    };
    match head.strip_suffix("-alt") {
        Some(series) => (series.to_string(), true, k),
        None => (head.to_string(), false, k),
    }
}

/// The limits profile the machine runs, from the settings file as it is now.
fn global_profile(ctx: &Ctx) -> String {
    devd_core::config::Settings::load(&ctx.paths.settings_file()).map(|s| s.php_profile).unwrap_or_else(|_| ctx.settings.php_profile.clone())
}

/// The profile that is not the machine's: what the second pool runs.
pub fn alt_profile(ctx: &Ctx) -> &'static str {
    if global_profile(ctx) == "production" {
        "local"
    } else {
        "production"
    }
}

/// The limits profile a site was given with `php.site_profile`, when it was given one.
pub fn site_profile(ctx: &Ctx, site: &str) -> Option<String> {
    ctx.store.ops_get("site_profile", site).ok().flatten().and_then(|i| i.data["profile"].as_str().map(str::to_string))
}

/// The sites that are served by the other-profile pool: those whose own profile is not the machine's.
pub fn alt_sites(ctx: &Ctx) -> std::collections::BTreeSet<String> {
    let alt = alt_profile(ctx);
    ctx.store
        .ops_list("site_profile", None)
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.data["profile"].as_str() == Some(alt))
        .map(|i| i.site)
        .collect()
}

/// The file name of the other-profile ini, beside `php.ini`.
fn alt_ini_name(ctx: &Ctx) -> String {
    format!("php.{}.ini", alt_profile(ctx))
}

/// Write the other-profile ini for a series: the current `php.ini` with the other profile's limits on top.
fn write_alt_ini(ctx: &Ctx, series: &str) -> Result<()> {
    let php = php_executable_for_series(ctx, series)?;
    let dir = php.parent().ok_or_else(|| DevdError::internal("a PHP with no folder"))?;
    let main = dir.join("php.ini");
    if !main.is_file() {
        return Ok(());
    }
    let alt = dir.join(alt_ini_name(ctx));
    // The other profile's file from an earlier machine profile is stale now.
    for stale in ["php.local.ini", "php.production.ini"] {
        if stale != alt_ini_name(ctx) {
            let _ = std::fs::remove_file(dir.join(stale));
        }
    }
    std::fs::copy(&main, &alt)?;
    devd_runtime::phpconf::apply_profile(&alt, alt_profile(ctx))
}

/// The PHP listeners, as series and port.
///
/// One listener per installed series. A site pinned to a version is sent to
/// the listener for that version's series; a site that is not pinned is sent
/// to the primary.
pub fn php_listeners(ctx: &Ctx) -> Result<Vec<(String, u16)>> {
    Ok(installed_php_series(ctx)?
        .into_iter()
        .map(|series| {
            let port = php_port_for_series(&series);
            (series, port)
        })
        .collect())
}

/// The port the listener for a series uses, when that series is installed.
pub fn php_port_for(ctx: &Ctx, series: &str) -> Option<u16> {
    php_listeners(ctx)
        .ok()?
        .into_iter()
        .find(|(s, _)| s == series)
        .map(|(_, port)| port)
}

/// The series the primary PHP version belongs to.
pub fn primary_php_series(ctx: &Ctx) -> Option<String> {
    let primary = ctx.settings.primary_php.clone()?;
    if primary.trim().is_empty() {
        return None;
    }
    Some(php_series(&primary))
}

/// The PHP series that are installed, newest first.
///
/// A series appears once even when several patch releases of it are
/// installed: the newest is the one a site would use.
pub fn installed_php_series(ctx: &Ctx) -> Result<Vec<String>> {
    let mut series: Vec<(String, String)> = Vec::new();
    for runtime in ctx.store.list_runtimes()? {
        if runtime.kind != "php" {
            continue;
        }
        let key = php_series(&runtime.version);
        // The version strings sort lexicographically, which is right for
        // "8.5.11" and "8.5.9" only by luck, so the comparison is by parts.
        match series.iter_mut().find(|(s, _)| s == &key) {
            Some((_, current)) if version_parts(&runtime.version) <= version_parts(current) => {}
            Some((_, current)) => *current = runtime.version.clone(),
            None => series.push((key, runtime.version.clone())),
        }
    }
    series.sort_by_key(|s| std::cmp::Reverse(version_parts(&s.1)));
    Ok(series.into_iter().map(|(s, _)| s).collect())
}

/// A version as a list of numbers, so that versions compare by value.
fn version_parts(version: &str) -> Vec<u32> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Where a service is installed.
pub fn install_dir(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.paths.services().join(name)
}

/// The pid file for a service.
fn pid_file(ctx: &Ctx, name: &str) -> PathBuf {
    install_dir(ctx, name).join("devd.pid")
}

/// The folder a service runs from when it was adopted rather than installed.
pub(crate) fn manifest_root(ctx: &Ctx, name: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(manifest(ctx, name)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("root").and_then(|v| v.as_str()).map(PathBuf::from)
}

/// The install manifest written by `service.install`.
fn manifest(ctx: &Ctx, name: &str) -> PathBuf {
    install_dir(ctx, name).join(".devd-install.json")
}

/// The log a supervised service writes to.
pub fn log_path(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.paths.logs().join(format!("{name}.log"))
}

/// The executable of an installed service.
pub fn executable_path(_ctx: &Ctx, service: &Service) -> Result<PathBuf> {
    let program = program_of(service)?;
    if !program.is_file() {
        return Err(DevdError::not_found(format!(
            "`{}` is not installed at {}; run `devd service install {}`",
            service.name,
            program.display(),
            service.name
        )));
    }
    Ok(program)
}

/// The program from a service's argv, which is always an argument array.
fn program_of(service: &Service) -> Result<PathBuf> {
    let argv: Vec<String> = serde_json::from_str(&service.argv).map_err(|e| {
        DevdError::internal(format!(
            "the stored command for `{}` is unreadable: {e}",
            service.name
        ))
    })?;
    argv.first()
        .map(PathBuf::from)
        .ok_or_else(|| DevdError::internal(format!("`{}` has no program", service.name)))
}

/// The version recorded at install time, when there is a manifest.
pub fn installed_version(ctx: &Ctx, name: &str) -> Option<String> {
    if name == PHP_SERVICE {
        // The PHP listener has no package of its own: it runs the primary PHP
        // runtime's `php-cgi.exe`, which lives in `runtimes/php/<version>/`.
        // Without this the one service whose version users care about most
        // would be reported as unknown.
        let exe = devd_runtime::executable(ctx, "php", None).ok()?;
        return exe
            .parent()
            .and_then(|dir| dir.file_name())
            .map(|name| name.to_string_lossy().into_owned());
    }
    let text = std::fs::read_to_string(manifest(ctx, name)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// What a service looks like right now.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    /// Service name.
    pub name: String,
    /// Driver kind.
    pub kind: String,
    /// Version, read from the install manifest.
    pub version: Option<String>,
    /// Whether the files are on disk.
    pub installed: bool,
    /// Whether a process is running.
    pub running: bool,
    /// Process id, when running.
    pub pid: Option<u32>,
    /// Port the service listens on.
    pub port: Option<u16>,
    /// Whether the port accepts a connection.
    pub listening: bool,
    /// What the user asked for: `running` or `stopped`.
    pub desired_state: String,
    /// Whether the service takes part in `stack.up`.
    pub enabled: bool,
    /// Install directory.
    pub install_dir: String,
    /// Log file.
    pub log: String,
    /// A sentence the surfaces can print without interpretation.
    pub message: String,
}

/// Whether the recorded pid is alive *and* still the right program.
///
/// Process ids are reused by the operating system. Without the image check,
/// stopping a service whose recorded pid had been recycled would terminate an
/// unrelated program, so the name is confirmed before anything is killed.
pub fn running_pid(ctx: &Ctx, service: &Service) -> Option<u32> {
    // A supervisor in this process is authoritative.
    let shared = process::shared();
    let supervised = shared
        .list()
        .into_iter()
        .find(|p| p.name == service.name && p.running)
        .map(|p| p.pid);
    if supervised.is_some() {
        return supervised;
    }

    let text = std::fs::read_to_string(pid_file(ctx, &service.name)).ok()?;
    let pid: u32 = text.trim().parse().ok()?;
    if !process::pid_alive(pid) {
        return None;
    }
    let expected = program_stem(service).ok()?;
    match process::process_image(pid) {
        Some(image) if image.eq_ignore_ascii_case(&expected) => Some(pid),
        _ => None,
    }
}

/// The image name a service's program runs under, e.g. `nginx` or `mysqld`.
fn program_stem(service: &Service) -> Result<String> {
    let program = program_of(service)?;
    Ok(program
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default())
}

/// A TCP connect that does not wait for the operating system's full timeout.
pub fn local_tcp_open(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok()
}

/// The current status of a service.
pub fn status(ctx: &Ctx, name: &str) -> Result<Status> {
    let service = ctx.store.get_service(name)?.ok_or_else(|| {
        DevdError::not_found(format!(
            "`{name}` is not a devd service; run `devd service install {name}`"
        ))
    })?;

    let dir = install_dir(ctx, &service.name);
    let program = program_of(&service).unwrap_or_default();
    let installed = program.is_file();
    let pid = running_pid(ctx, &service);
    let listening = service.port.map(local_tcp_open).unwrap_or(false);
    let running = pid.is_some();

    let message = if !installed {
        format!("{} is not installed at {}", service.name, dir.display())
    } else if running && listening {
        format!(
            "{name} is running (pid {}) on port {}",
            pid.unwrap_or(0),
            service.port.unwrap_or(0)
        )
    } else if running {
        format!(
            "{name} is running (pid {}) but nothing is answering on port {}",
            pid.unwrap_or(0),
            service.port.unwrap_or(0)
        )
    } else if service.desired_state == "running" {
        format!(
            "{name} should be running but no process was found; run `devd service start {name}`"
        )
    } else {
        format!("{name} is stopped")
    };

    Ok(Status {
        name: service.name.clone(),
        kind: service.kind.clone(),
        version: installed_version(ctx, &service.name),
        installed,
        running,
        pid,
        port: service.port,
        listening,
        desired_state: service.desired_state.clone(),
        enabled: service.enabled,
        install_dir: dir.display().to_string(),
        log: log_path(ctx, &service.name).display().to_string(),
        message,
    })
}

/// Every service `devd` knows about.
pub fn list(ctx: &Ctx) -> Result<Vec<Status>> {
    let mut out = Vec::new();
    for service in ctx.store.list_services()? {
        match status(ctx, &service.name) {
            Ok(s) => out.push(s),
            Err(e) => out.push(Status {
                name: service.name.clone(),
                kind: service.kind.clone(),
                version: installed_version(ctx, &service.name),
                installed: false,
                running: false,
                pid: None,
                port: service.port,
                listening: false,
                desired_state: service.desired_state.clone(),
                enabled: service.enabled,
                install_dir: install_dir(ctx, &service.name).display().to_string(),
                log: log_path(ctx, &service.name).display().to_string(),
                message: format!("could not be inspected: {e}"),
            }),
        }
    }
    Ok(out)
}

/// The available versions upstream, for `service.available`.
pub fn available(kind: &str) -> Result<(Vec<Available>, Vec<String>)> {
    let mut notes = Vec::new();
    if !cfg!(windows) {
        notes.push(
            "devd downloads web server and database binaries on Windows only; on this \
             platform `service install` adopts the ones your system package manager has \
             installed, after checking them against the package"
                .to_string(),
        );
    }
    Ok((catalogue::for_kind(kind)?, notes))
}

/// What an install produced.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledService {
    /// The recorded service.
    pub service: Service,
    /// Installed version.
    pub version: String,
    /// Install directory.
    pub path: String,
    /// Digest of the downloaded artifact, when the vendor published one.
    pub digest: Option<String>,
    /// Bytes downloaded.
    pub bytes: u64,
    /// Whether the download came from the cache.
    pub from_cache: bool,
    /// Anything the caller should know.
    pub warnings: Vec<String>,
    /// Whatever the install learned that only its own kind can report - a
    /// MariaDB data directory and whether it was created, for example.
    pub details: serde_json::Value,
}

/// Download, unpack and record a service.
pub fn install(
    ctx: &Ctx,
    name: &str,
    spec: &str,
    port: Option<u16>,
    force: bool,
    progress: Option<&ProgressFn<'_>>,
) -> Result<InstalledService> {
    let def = definition(name).ok_or_else(|| {
        DevdError::unsupported(format!(
            "`{name}` is not a service devd can install; installable services are {}",
            DEFINITIONS
                .iter()
                .map(|d| d.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    if def.name == "apache" && Os::current().uses_system_packages() {
        return Err(DevdError::unsupported(
            "devd drives Apache on Windows. On Linux and macOS it serves sites with nginx; \
             set web_server back to \"nginx\"",
        ));
    }

    // Redis and memcached have Windows builds whose digests GitHub records, so they are downloaded and
    // checked like Mailpit. PostgreSQL and MongoDB still have none devd can verify and are adopted.
    let downloadable = matches!(def.name, "redis" | "memcached") && cfg!(windows);
    if let Some(engine) = engines::get(def.name).filter(|_| !downloadable) {
        return Err(engines::download_refusal(engine));
    }

    // Off Windows the program comes from the system's package manager, not from
    // a download devd verifies itself. Mailpit is the exception: it is published
    // for every platform with a digest, so it is downloaded and checked everywhere.
    if def.name != "mailpit" && Os::current().uses_system_packages() {
        return install_system(ctx, def, port, &SystemHost);
    }

    let (list, notes) = available(def.kind)?;
    if list.is_empty() {
        return Err(DevdError::unsupported(if notes.is_empty() {
            format!("no {} builds are published for this platform", def.name)
        } else {
            notes.join("; ")
        }));
    }

    // A running service must be stopped before its files are replaced; swapping
    // the binary under a live process is how you get a web server that is half
    // one version and half another.
    if let Ok(Some(existing)) = ctx.store.get_service(def.name) {
        if running_pid(ctx, &existing).is_some() {
            return Err(DevdError::conflict(format!(
                "{} is running; stop it first with `devd service stop {}`",
                def.name, def.name
            )));
        }
    }

    // "latest" means the newest build devd can verify, not the newest published:
    // nginx.org publishes no checksum, so only versions whose digest devd has
    // recorded can be installed, and failing setup because a release came out
    // yesterday helps nobody. An explicit version that cannot be verified is
    // still refused below, with its own message.
    let installable = verifiable(def.name, &list);
    let chosen = resolve(&installable, spec, None).or_else(|_| resolve(&list, spec, None))?;
    let newer_unverified = list
        .first()
        .filter(|newest| {
            newest.version != chosen.version
                && installable.first().map(|a| a.version.as_str()) == Some(chosen.version.as_str())
                && !installable.iter().any(|a| a.version == newest.version)
        })
        .map(|newest| newest.version.clone());
    let dest = install_dir(ctx, def.name);
    let already = installed_version(ctx, def.name);
    if dest.join(def.executable()).is_file() && !force {
        match already.as_deref() {
            Some(v) if v == chosen.version => {
                return Err(DevdError::conflict(format!(
                    "{} {} is already installed; pass force to reinstall it",
                    def.name, chosen.version
                )))
            }
            Some(v) => {
                return Err(DevdError::conflict(format!(
                    "{} {} is installed; pass force to replace it with {}",
                    def.name, v, chosen.version
                )))
            }
            None => {
                return Err(DevdError::conflict(format!(
                    "{} is already installed at {}; pass force to replace it",
                    def.name,
                    dest.display()
                )))
            }
        }
    }

    let mut artifact = Artifact::new(chosen.url.clone());
    artifact.sha256 = chosen.sha256.clone();
    artifact.bytes = chosen.bytes;
    if artifact.sha256.is_none() && def.name == "mariadb" {
        if let Ok(Some(digest)) = catalogue::mariadb_digest(&chosen.version, &artifact.filename) {
            artifact.sha256 = Some(digest);
        }
    }
    // A service devd cannot verify is a service it does not install. The
    // archive is the program that will run with the machine's privileges;
    // "it looked like the right file" is not evidence.
    if artifact.sha256.is_none() {
        return Err(DevdError::unsupported(format!(
            "{} {} is published without a checksum devd can check, so devd will not install it",
            def.name, chosen.version
        )));
    }

    if let Some(report) = progress {
        report(&devd_core::download::Progress {
            received: 0,
            total: chosen.bytes,
        });
    }
    let fetched = devd_core::download::fetch(&artifact, &ctx.paths.cache(), progress)?;

    let mut warnings = Vec::new();
    if let Some(newest) = newer_unverified {
        warnings.push(format!(
            "{} {newest} is newer, but devd has no recorded checksum for it, so {} was installed",
            def.name, chosen.version
        ));
    }
    if artifact.sha256.is_none() {
        warnings.push(format!(
            "{} publishes no checksum file, so this download could only be checked for length \
             ({} bytes); it came over HTTPS from {}. See docs/limitations.md.",
            def.name, fetched.bytes, chosen.url
        ));
    }

    archive::extract(&fetched.path, &dest, true)?;

    let mut record = json!({
        "url": chosen.url,
        "sha256": fetched.sha256,
        "bytes": fetched.bytes,
    });
    if def.name == "apache" {
        // The Apache Lounge archive holds an `Apache24` folder beside its notes.
        record["root"] = json!(dest.join("Apache24").display().to_string());
    }
    if engines::get(def.name).is_some() {
        record["root"] = json!(dest.display().to_string());
    }
    let (service, extra) = finish_install(
        ctx,
        def,
        port,
        &chosen.version,
        record,
        &dest,
        &mut warnings,
    )?;

    Ok(InstalledService {
        service,
        version: chosen.version,
        path: dest.display().to_string(),
        digest: artifact.sha256.clone().or(Some(fetched.sha256.clone())),
        bytes: fetched.bytes,
        from_cache: fetched.from_cache,
        warnings,
        details: serde_json::Value::Object(extra),
    })
}

/// Everything an install does once the program is in place: provision what the
/// program needs (a database's data directory, the web server's configuration),
/// write the install manifest and record the service.
///
/// Shared by both ways a program arrives - downloaded from its publisher on
/// Windows, adopted from the system's package manager elsewhere - so the two
/// cannot drift apart in what "installed" means.
fn finish_install(
    ctx: &Ctx,
    def: &Definition,
    port: Option<u16>,
    version: &str,
    record: serde_json::Value,
    dest: &Path,
    warnings: &mut Vec<String>,
) -> Result<(Service, serde_json::Map<String, serde_json::Value>)> {
    let mut extra = serde_json::Map::new();
    match def.name {
        "mariadb" => {
            let port = port.unwrap_or(def.default_port);
            let report = mariadb::provision(ctx, port)?;
            extra.insert("port".to_string(), json!(port));
            extra.insert("data_dir".to_string(), json!(report.data_dir));
            extra.insert("initialized".to_string(), json!(report.initialized));
            extra.insert(
                "root_password_set".to_string(),
                json!(report.root_password_set),
            );
            // The provisioning warnings are the ones a user most needs to read -
            // "root has no password" among them - so they are merged into the
            // install's warnings rather than left in a field nobody prints.
            warnings.extend(report.warnings);
        }
        "nginx" => {
            // The configuration is rendered from the store, which is empty until
            // a site exists; rendering here means `nginx -t` has something to
            // check the moment nginx is installed.
            nginx::render(ctx)?;
        }
        "redis" => {
            let port = port.unwrap_or(def.default_port);
            extras::redis_write_config(ctx, port)?;
            extra.insert("port".to_string(), json!(port));
            extra.insert("config".to_string(), json!(extras::redis_config_path(ctx).display().to_string()));
            warnings.push("Redis listens on 127.0.0.1 only and keeps a snapshot under the devd data folder".to_string());
        }
        other if engines::get(other).is_some() => {
            let engine = engines::get(other).expect("checked");
            let root = record.get("root").and_then(|v| v.as_str()).map(PathBuf::from).ok_or_else(|| DevdError::internal("an engine was recorded without its folder"))?;
            warnings.extend(engines::prepare(ctx, engine, &root)?);
            extra.insert("port".to_string(), json!(port.unwrap_or(def.default_port)));
            extra.insert("data_dir".to_string(), json!(engines::data_dir(ctx, other).display().to_string()));
            warnings.push(format!("{other} listens on 127.0.0.1 only; its data lives in {}", engines::data_dir(ctx, other).display()));
        }
        "mailpit" => {
            std::fs::create_dir_all(ctx.paths.etc().join("mailpit"))?;
            let port = port.unwrap_or(def.default_port);
            extra.insert("inbox".to_string(), json!(format!("http://127.0.0.1:{port}")));
            extra.insert("smtp".to_string(), json!(format!("127.0.0.1:{}", extras::SMTP_PORT)));
            let pointed = extras::point_php_at_mailpit(ctx)?;
            if !pointed.is_empty() {
                warnings.push(format!(
                    "PHP {} now sends mail to Mailpit; restart the php-cgi service to apply it",
                    pointed.join(", ")
                ));
            }
        }
        "apache" => {
            // The manifest that names the Apache root is written below, so the
            // render has to look where it will be, not where it is.
            if let serde_json::Value::Object(fields) = &record {
                if let Some(root) = fields.get("root").and_then(|v| v.as_str()) {
                    std::fs::create_dir_all(install_dir(ctx, def.name))?;
                    util::atomic_write(
                        &manifest(ctx, def.name),
                        &json!({ "name": def.name, "version": version, "root": root }).to_string(),
                    )?;
                }
            }
            apache::render(ctx)?;
        }
        _ => {}
    }

    let mut manifest_doc = serde_json::Map::new();
    manifest_doc.insert("name".to_string(), json!(def.name));
    manifest_doc.insert("version".to_string(), json!(version));
    manifest_doc.insert("installed_at".to_string(), json!(util::now()));
    if let serde_json::Value::Object(fields) = record {
        manifest_doc.extend(fields);
    }
    util::atomic_write(
        &manifest(ctx, def.name),
        &serde_json::to_string_pretty(&serde_json::Value::Object(manifest_doc))
            .unwrap_or_default(),
    )?;

    let service = ctx.store.get_service(def.name)?.unwrap_or(Service {
        id: util::new_id(),
        name: def.name.to_string(),
        kind: def.kind.to_string(),
        runtime_id: None,
        argv: String::new(),
        port: None,
        health: None,
        desired_state: "stopped".to_string(),
        restart_policy: "on-failure".to_string(),
        enabled: true,
        created_at: util::now(),
    });

    let mut service = service;
    service.kind = def.kind.to_string();
    service.argv = serde_json::to_string(&service_argv(ctx, def.name, port)?).unwrap_or_default();
    service.port = Some(port.unwrap_or(def.default_port));
    if def.name == "nginx" || def.name == "apache" {
        service.port = Some(ctx.settings.http_port);
    }
    service.health = Some(json!({ "tcp": service.port }).to_string());
    ctx.store.upsert_service(&service)?;

    // Installing nginx is the moment a PHP listener becomes meaningful, so it is
    // created (stopped) if a PHP runtime exists.
    if (def.name == "nginx" || def.name == "apache")
        && devd_runtime::executable(ctx, "php", None).is_ok() {
            ensure_php_service(ctx)?;
        }

    ctx.events.emit_with(
        format!("service.{}.installed", def.name),
        format!("installed {} {}", def.name, version),
        json!({ "name": def.name, "version": version, "path": dest.display().to_string() }),
    );

    Ok((service, extra))
}

/// Adopt the program the system's package manager installed.
///
/// On Linux and macOS nginx and MariaDB are built and signed by the
/// distribution, not published by the project as an archive devd could check.
/// devd therefore does not fetch a second copy. It finds the installed program,
/// asks the package manager which package owns it and whether the file still
/// matches that package, refuses a file that does not, and links the program
/// into the layout the rest of devd uses. Configuration and data stay devd's
/// own: nothing under `/etc` is read or written.
pub fn install_system(
    ctx: &Ctx,
    def: &Definition,
    port: Option<u16>,
    host: &dyn Host,
) -> Result<InstalledService> {
    if let Ok(Some(existing)) = ctx.store.get_service(def.name) {
        if running_pid(ctx, &existing).is_some() {
            return Err(DevdError::conflict(format!(
                "{} is running; stop it first with `devd service stop {}`",
                def.name, def.name
            )));
        }
    }

    let adoption = system::adopt(host, def.kind)?;
    let version = system::version_of(def.kind, &adoption.banner).ok_or_else(|| {
        DevdError::invalid(format!(
            "could not read a {} version from `{}`",
            def.name,
            adoption.banner.trim()
        ))
    })?;

    let dest = install_dir(ctx, def.name);
    platform::link_into(&dest, &adoption.found)?;
    system::prepare(def.kind, &dest, &adoption)?;

    let mut warnings = adoption.warnings.clone();
    let record = json!({
        "source": "system",
        "prefix": adoption.prefix.display().to_string(),
        "programs": adoption.found,
        "origin": adoption.origin,
    });
    let (service, extra) = finish_install(ctx, def, port, &version, record, &dest, &mut warnings)?;

    Ok(InstalledService {
        service,
        version,
        path: dest.display().to_string(),
        digest: None,
        bytes: 0,
        from_cache: false,
        warnings,
        details: serde_json::Value::Object(extra),
    })
}

/// Folders where a portable stack keeps `tool` (`X:\<stack>\bin\<tool>\<version>`) or a
/// program is usually unpacked, on every drive. Only Windows is searched.
fn portable_candidates(tool: &str, fixed: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !cfg!(windows) {
        return out;
    }
    for drive in 'A'..='Z' {
        let root = PathBuf::from(format!("{drive}:\\"));
        if !root.is_dir() {
            continue;
        }
        out.extend(fixed.iter().map(|f| root.join(f)));
        let Ok(tops) = std::fs::read_dir(&root) else { continue };
        for top in tops.flatten().filter(|e| e.path().is_dir()) {
            let bin = top.path().join("bin").join(tool);
            if let Ok(kids) = std::fs::read_dir(&bin) {
                out.extend(kids.flatten().map(|k| k.path()));
            }
            out.push(bin);
        }
    }
    out
}

/// Redis folders already on this machine, as (path, version).
pub fn find_local_redis() -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for candidate in portable_candidates("redis", &["Redis", "Program Files\\Redis"]) {
        let Some(root) = extras::redis_root_of(&candidate) else { continue };
        if out.iter().any(|(p, _)| p == &root) {
            continue;
        }
        if let Some(version) = extras::redis_version(&root) {
            out.push((root, version));
        }
    }
    out
}

/// Apache folders already on this machine, as (path, version): portable stacks,
/// XAMPP, a plain `C:\Apache24`, the Apache Software Foundation's own folder.
/// Nothing is read beyond `httpd -v`.
pub fn find_local_apache() -> Vec<(PathBuf, String)> {
    let mut candidates = portable_candidates("apache", &["Apache24", "Apache", "xampp\\apache"]);
    if cfg!(windows) {
        for drive in 'A'..='Z' {
            for parent in ["Program Files", "Program Files\\Apache Software Foundation"] {
                let Ok(entries) = std::fs::read_dir(PathBuf::from(format!("{drive}:\\")).join(parent)) else { continue };
                candidates.extend(entries.flatten().filter(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("apache")).map(|e| e.path()));
            }
        }
    }
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for candidate in candidates {
        let Some(root) = apache_root_of(&candidate) else { continue };
        if out.iter().any(|(p, _)| p == &root) {
            continue;
        }
        if let Some(version) = apache::version(&root) {
            out.push((root, version));
        }
    }
    out
}

/// The Apache root inside a folder: the folder itself, or its `Apache24` child.
fn apache_root_of(folder: &Path) -> Option<PathBuf> {
    [folder.to_path_buf(), folder.join("Apache24")]
        .into_iter()
        .find(|p| p.join(apache::exe_name()).is_file() && p.join("modules").is_dir())
}

/// Use an Apache that is already on disk, without copying or changing it.
///
/// The folder is recorded as the Apache root. devd writes its own configuration
/// under its own directory and starts `httpd -f <that>`, so the owner's
/// `httpd.conf` and the rest of the folder are never touched. It is a person's
/// decision, because the program in that folder is what devd will then run.
pub fn adopt(ctx: &Ctx, name: &str, path: &Path, port: Option<u16>) -> Result<InstalledService> {
    let def = definition(name)
        .filter(|d| matches!(d.name, "apache" | "redis") || engines::get(d.name).is_some())
        .ok_or_else(|| DevdError::unsupported("only Apache, Redis, PostgreSQL, MongoDB and Memcached can be adopted from a folder"))?;
    let engine = engines::get(def.name);
    if !path.is_absolute() {
        return Err(DevdError::invalid(format!("give the absolute path of the {} folder", def.name)));
    }
    let found_root = match (engine, def.name) {
        (Some(e), _) => engines::root_of(e, path),
        (None, "redis") => extras::redis_root_of(path),
        _ => apache_root_of(path),
    };
    let root = found_root.ok_or_else(|| {
        DevdError::not_found(format!(
            "{} is not a {} folder: it needs {}",
            path.display(),
            def.name,
            match engine {
                Some(e) => format!("{} (and {})", e.program, e.needs.join(", ")),
                None if def.name == "redis" => extras::redis_exe_name().to_string(),
                None => apache::exe_name().to_string(),
            }
        ))
    })?;
    if let Ok(Some(existing)) = ctx.store.get_service(def.name) {
        if running_pid(ctx, &existing).is_some() {
            return Err(DevdError::conflict(
                format!("{} is running; stop it first with `devd service stop {}`", def.name, def.name),
            ));
        }
    }
    let read_version = match (engine, def.name) {
        (Some(e), _) => engines::version(e, &root),
        (None, "redis") => extras::redis_version(&root),
        _ => apache::version(&root),
    };
    let version = read_version.ok_or_else(|| {
        DevdError::invalid(format!("could not read a {} version from {}", def.name, root.display()))
    })?;
    let config = match (engine, def.name) {
        (Some(_), _) => engines::data_dir(ctx, def.name),
        (None, "redis") => extras::redis_config_path(ctx),
        _ => apache::config_path(ctx),
    };
    let mut warnings = vec![format!(
        "{} is used as it is. devd does not change it; devd's own files for it are in {}",
        root.display(),
        config.display()
    )];
    let record = json!({ "source": "adopted", "root": root.display().to_string() });
    let dest = install_dir(ctx, def.name);
    std::fs::create_dir_all(&dest)?;
    let (service, extra) = finish_install(ctx, def, port, &version, record, &dest, &mut warnings)?;
    Ok(InstalledService {
        service,
        version,
        path: root.display().to_string(),
        digest: None,
        bytes: 0,
        from_cache: false,
        warnings,
        details: serde_json::Value::Object(extra),
    })
}

/// The builds devd can verify before it installs them.
///
/// nginx publishes a signature and no checksum, so only the versions whose digest
/// devd has recorded qualify. The others (MariaDB, whose digest comes from its
/// release API when a build is chosen) are all candidates.
fn verifiable(name: &str, list: &[Available]) -> Vec<Available> {
    if name == "nginx" {
        list.iter().filter(|a| a.sha256.is_some()).cloned().collect()
    } else {
        list.to_vec()
    }
}

/// The argument array for a service's process. Never a shell string.
pub fn service_argv(ctx: &Ctx, name: &str, port: Option<u16>) -> Result<Vec<String>> {
    let dir = install_dir(ctx, name);
    match name {
        "nginx" => {
            let exe = dir.join(nginx::exe_name());
            // These arguments are passed to the process as an array, not through a
            // shell, so native separators need no escaping here. Only the
            // *configuration file contents* require forward slashes, because nginx
            // treats a backslash as an escape character inside a configuration
            // value.
            Ok(vec![
                native(&exe),
                "-p".to_string(),
                nginx::prefix(&dir),
                "-c".to_string(),
                native(&nginx::config_path(ctx)),
            ])
        }
        other if engines::get(other).is_some() => {
            let engine = engines::get(other).expect("checked");
            let root = manifest_root(ctx, other).ok_or_else(|| DevdError::not_found(format!("{other} has no recorded folder; adopt it again")))?;
            // The port being installed with, else the one already recorded, else the engine's own.
            let port = match port {
                Some(p) => p,
                None => ctx.store.get_service(other)?.and_then(|s| s.port).unwrap_or(engine.default_port),
            };
            engines::argv(ctx, engine, &root, port)
        }
        "redis" => {
            let exe = extras::redis_directory(ctx).join(extras::redis_exe_name());
            Ok(vec![native(&exe), native(&extras::redis_config_path(ctx))])
        }
        "mailpit" => {
            let exe = dir.join(extras::mailpit_exe_name());
            let port = ctx.store.get_service("mailpit")?.and_then(|s| s.port).unwrap_or(extras::MAILPIT_UI_PORT);
            let mut argv = vec![native(&exe)];
            argv.extend(extras::mailpit_args(ctx, port));
            Ok(argv)
        }
        "apache" => {
            let root = apache::directory(ctx);
            Ok(vec![
                native(&root.join(apache::exe_name())),
                "-d".to_string(),
                apache::root_arg(&root),
                "-f".to_string(),
                native(&apache::config_path(ctx)),
            ])
        }
        "mariadb" => {
            let exe = dir.join(if cfg!(windows) {
                "bin/mysqld.exe"
            } else {
                "bin/mariadbd"
            });
            // Writing the configuration is a side effect and belongs to install
            // and start, not to computing an argument array that a plan will show.
            Ok(vec![
                native(&exe),
                format!("--defaults-file={}", native(&mariadb::config_path(ctx))),
            ])
        }
        PHP_SERVICE => {
            let php = php_executable(ctx)?;
            Ok(php_cgi_argv(ctx, &php, PHP_PORT, None))
        }
        // A listener for one PHP series: `php-cgi-8.3`.
        other if other.starts_with(&format!("{PHP_SERVICE}-")) => {
            let (series, alt, k) = split_pool(other.trim_start_matches(&format!("{PHP_SERVICE}-")));
            let php = php_executable_for_series(ctx, &series)?;
            Ok(if alt {
                php_cgi_argv(ctx, &php, php_alt_port(&series, k), Some(alt_ini_name(ctx)))
            } else {
                php_cgi_argv(ctx, &php, php_worker_port(&series, k), None)
            })
        }
        other => Err(DevdError::unsupported(format!(
            "there is no driver for the service `{other}`"
        ))),
    }
}

/// The argument array for a PHP FastCGI listener.
fn php_cgi_argv(_ctx: &Ctx, php_exe: &Path, port: u16, ini_name: Option<String>) -> Vec<String> {
    let dir = php_exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| php_exe.to_path_buf());
    let cgi = dir.join(if cfg!(windows) { "php-cgi.exe" } else { "php-cgi" });
    let ini = dir.join(ini_name.unwrap_or_else(|| "php.ini".to_string()));
    let mut argv = vec![
        native(&cgi),
        "-b".to_string(),
        format!("127.0.0.1:{port}"),
    ];
    // A downloaded PHP carries the php.ini devd generated beside it. A system
    // PHP keeps its configuration under /etc, so there is no file here to point
    // at, and a `-c` that names nothing is worse than none.
    if cfg!(windows) || ini.is_file() {
        argv.push("-c".to_string());
        argv.push(native(&ini));
    }
    argv
}

/// The PHP interpreter of the newest patch release of a series.
fn php_executable_for_series(ctx: &Ctx, series: &str) -> Result<PathBuf> {
    let wanted = php_series(series);
    let mut best: Option<String> = None;
    for runtime in ctx.store.list_runtimes()? {
        if runtime.kind != "php" || php_series(&runtime.version) != wanted {
            continue;
        }
        match &best {
            Some(current) if version_parts(&runtime.version) <= version_parts(current) => {}
            _ => best = Some(runtime.version.clone()),
        }
    }
    let version = best.ok_or_else(|| {
        DevdError::not_found(format!(
            "no PHP {wanted} is installed; `devd runtime install php@{wanted}` adds one"
        ))
    })?;
    devd_runtime::executable(ctx, "php", Some(&version))
}

/// A path in the form the operating system prefers, for a command-line argument.
///
/// Arguments are passed as an array and never through a shell, so a backslash is
/// a literal character rather than an escape; the native separator is therefore
/// both correct and unambiguous.
fn native(path: &Path) -> String {
    path.display().to_string()
}

/// The argument array a service would be started with, from its stored row.
///
/// Used by a plan so that a person or an agent sees the exact command before
/// approving it, and so that a plan does not have to reproduce the driver logic
/// that produced it.
pub fn planned_argv(ctx: &Ctx, name: &str) -> Result<Vec<String>> {
    let service = ctx.store.get_service(name)?.ok_or_else(|| {
        DevdError::not_found(format!(
            "`{name}` is not a devd service; run `devd service install {name}`"
        ))
    })?;
    serde_json::from_str(&service.argv).map_err(|e| {
        DevdError::internal(format!(
            "the stored command for `{name}` is unreadable: {e}"
        ))
    })
}

fn php_executable(ctx: &Ctx) -> Result<PathBuf> {
    devd_runtime::executable(ctx, "php", None)
}

/// Create or refresh the `php-cgi` service row from an installed PHP runtime.
pub fn ensure_php_service(ctx: &Ctx) -> Result<Service> {
    let runtime_id = ctx
        .store
        .list_runtimes()?
        .into_iter()
        .find(|r| {
            r.kind == "php"
                && devd_runtime::executable_relative("php")
                    .map(|rel| Path::new(&r.path).join(rel).is_file())
                    .unwrap_or(false)
        })
        .map(|r| r.id);

    let existing = ctx.store.get_service(PHP_SERVICE)?;
    let mut service = existing.unwrap_or(Service {
        id: util::new_id(),
        name: PHP_SERVICE.to_string(),
        kind: "php-cgi".to_string(),
        runtime_id: None,
        argv: String::new(),
        port: Some(PHP_PORT),
        health: None,
        desired_state: "stopped".to_string(),
        restart_policy: "on-failure".to_string(),
        enabled: true,
        created_at: util::now(),
    });
    service.kind = "php-cgi".to_string();
    service.runtime_id = runtime_id;
    service.argv =
        serde_json::to_string(&service_argv(ctx, PHP_SERVICE, None)?).unwrap_or_default();
    service.port = Some(PHP_PORT);
    service.health = Some(json!({ "tcp": PHP_PORT }).to_string());
    ctx.store.upsert_service(&service)?;
    Ok(service)
}

/// Create or refresh one listener per installed PHP series.
///
/// A site can be pinned to a PHP version, and each version needs its own
/// listener, so a machine with three PHP versions has three listeners. The
/// legacy single `php-cgi` row is replaced by these: it pointed at whichever
/// interpreter happened to be primary, which is exactly the limitation this
/// removes.
pub fn ensure_php_services(ctx: &Ctx) -> Result<Vec<Service>> {
    let mut out = Vec::new();
    let workers = php_workers(ctx);
    let alt_needed = !alt_sites(ctx).is_empty();
    let mut pool: Vec<(String, u8, u16, String)> = Vec::new();
    for (series, _) in php_listeners(ctx)? {
        for k in 1..=workers {
            pool.push((series.clone(), k, php_worker_port(&series, k), php_worker_name(&series, k)));
            if alt_needed {
                pool.push((series.clone(), k, php_alt_port(&series, k), php_alt_name(&series, k)));
            }
        }
        if alt_needed {
            write_alt_ini(ctx, &series)?;
        }
    }
    for (series, _k, port, name) in pool {
        let argv = service_argv(ctx, &name, Some(port))?;
        let runtime_id = ctx
            .store
            .list_runtimes()?
            .into_iter()
            .find(|r| r.kind == "php" && php_series(&r.version) == series)
            .map(|r| r.id);
        let existing = ctx.store.get_service(&name)?;
        let mut service = existing.unwrap_or(Service {
            id: util::new_id(),
            name: name.clone(),
            kind: "php-cgi".to_string(),
            runtime_id: None,
            argv: String::new(),
            port: Some(port),
            health: None,
            desired_state: "stopped".to_string(),
            restart_policy: "on-failure".to_string(),
            enabled: true,
            created_at: util::now(),
        });
        service.kind = "php-cgi".to_string();
        service.runtime_id = runtime_id;
        service.argv = serde_json::to_string(&argv).unwrap_or_default();
        service.port = Some(port);
        service.health = Some(json!({ "tcp": port }).to_string());
        ctx.store.upsert_service(&service)?;
        out.push(service);
    }
    Ok(out)
}

/// Remove the listener rows for PHP series that are no longer installed.
///
/// A runtime that is uninstalled must not leave a listener behind that points
/// at an interpreter that is gone.
pub fn prune_php_services(ctx: &Ctx) -> Result<Vec<String>> {
    let workers = php_workers(ctx);
    let alt_needed = !alt_sites(ctx).is_empty();
    let wanted: Vec<String> = installed_php_series(ctx)?
        .into_iter()
        .flat_map(|s| {
            (1..=workers).flat_map(move |k| {
                let mut v = vec![php_worker_name(&s, k)];
                if alt_needed {
                    v.push(php_alt_name(&s, k));
                }
                v
            })
        })
        .collect();
    let mut removed = Vec::new();
    for service in ctx.store.list_services()? {
        // The single `php-cgi` of before there was a listener per version is replaced by those listeners:
        // it points at whichever interpreter was primary and nothing is sent to it any more.
        let legacy = service.name == PHP_SERVICE && !wanted.is_empty();
        if !legacy && !service.name.starts_with(&format!("{PHP_SERVICE}-")) {
            continue;
        }
        if wanted.contains(&service.name) {
            continue;
        }
        if running_pid(ctx, &service).is_some() {
            let _ = stop(ctx, &service.name);
        }
        ctx.store.delete_service(&service.name)?;
        removed.push(service.name);
    }
    Ok(removed)
}

/// Build the spawn specification for a service.
fn spawn_spec(ctx: &Ctx, service: &Service) -> Result<SpawnSpec> {
    let argv: Vec<String> = serde_json::from_str(&service.argv)
        .map_err(|e| DevdError::internal(format!("unreadable command: {e}")))?;
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| DevdError::internal(format!("`{}` has an empty command", service.name)))?;
    let program = PathBuf::from(program);
    // The working directory is the program's own directory, which is what makes
    // relative paths inside a generated `php.ini` (`extension_dir = "ext"`) and
    // nginx's prefix-relative includes resolve.
    let cwd = program.parent().map(Path::to_path_buf);

    Ok(SpawnSpec {
        name: service.name.clone(),
        kind: service.kind.clone(),
        program,
        args: args.to_vec(),
        cwd,
        envs: Vec::new(),
        clear_env: false,
        log_path: log_path(ctx, &service.name),
        restarts: 0,
    })
}

/// The HTTP port `settings.toml` currently states.
///
/// Read from disk rather than from `ctx.settings` so that a change made by
/// another process - or by `web.ports` in a long-lived one - is seen without a
/// restart. Rendering and starting both use it, so the generated configuration
/// and the port devd verifies cannot disagree.
pub fn http_port(ctx: &Ctx) -> u16 {
    devd_core::config::Settings::load(&ctx.paths.settings_file())
        .map(|s| s.http_port)
        .unwrap_or(ctx.settings.http_port)
}

/// The HTTPS port `settings.toml` currently states.
pub fn https_port(ctx: &Ctx) -> u16 {
    devd_core::config::Settings::load(&ctx.paths.settings_file())
        .map(|s| s.https_port)
        .unwrap_or(ctx.settings.https_port)
}

/// Write new web ports to `settings.toml` and to the web server's row.
///
/// The row is how `service status`, `doctor` and the start verification know
/// which port to expect, so it moves with the setting; a row left at the
/// install-time value is what made a port changed by hand fail to start.
pub fn set_web_ports(ctx: &Ctx, http: u16, https: u16) -> Result<()> {
    let file = ctx.paths.settings_file();
    let mut settings = devd_core::config::Settings::load(&file)?;
    settings.http_port = http;
    settings.https_port = https;
    settings.validate()?;
    settings.save(&file)?;
    for name in ["nginx", "apache"] {
        if let Some(mut service) = ctx.store.get_service(name)? {
            service.port = Some(http);
            service.health = Some(json!({ "tcp": http }).to_string());
            ctx.store.upsert_service(&service)?;
        }
    }
    Ok(())
}

/// Start a service and confirm it is answering.
pub fn start(ctx: &Ctx, name: &str) -> Result<Status> {
    // The PHP listener has no installer of its own: it comes from a PHP runtime.
    // Asking for it to start before anything has created the row is a reasonable
    // thing to do, so the row is created here rather than reported as missing.
    if name == PHP_SERVICE && ctx.store.get_service(PHP_SERVICE)?.is_none() {
        php_executable(ctx)?;
        ensure_php_service(ctx)?;
    }

    let mut service = ctx.store.get_service(name)?.ok_or_else(|| {
        DevdError::not_found(format!(
            "`{name}` is not a devd service; run `devd service install {name}`"
        ))
    })?;
    executable_path(ctx, &service)?;

    if let Some(pid) = running_pid(ctx, &service) {
        let mut current = status(ctx, name)?;
        current.message = format!("{name} was already running (pid {pid})");
        return Ok(current);
    }

    // A port changed in settings.toml applies from here on. Without this the row
    // keeps the port from install time, so the start is verified against a port
    // nothing is listening on and reported as a failure while the server is up.
    if service.name == "nginx" || service.name == "apache" {
        let http = http_port(ctx);
        if service.port != Some(http) {
            service.port = Some(http);
            service.health = Some(json!({ "tcp": http }).to_string());
            ctx.store.upsert_service(&service)?;
        }
    }

    // For nginx the configuration is checked before the process is started, so a
    // bad render is reported as a configuration error with nginx's own message
    // rather than as a service that exits immediately after it starts.
    if service.name == "nginx" {
        if !nginx::config_path(ctx).is_file() {
            nginx::render(ctx)?;
        }
        nginx::test(ctx)?;
    }

    if service.name == "redis" {
        extras::redis_write_config(ctx, service.port.unwrap_or(6379))?;
    }

    if let Some(engine) = engines::get(&service.name) {
        let root = manifest_root(ctx, &service.name).ok_or_else(|| DevdError::not_found(format!("{} has no recorded folder; adopt it again", service.name)))?;
        engines::prepare(ctx, engine, &root)?;
    }

    if service.name == "apache" {
        if !apache::config_path(ctx).is_file() {
            apache::render(ctx)?;
        }
        apache::test(ctx)?;
    }

    // MariaDB needs its configuration on disk and its system tables created
    // before it will start at all. Both are reported as themselves rather than as
    // a server that mysteriously exits.
    if service.name == "mariadb" {
        let port = mariadb::port_of(&service);
        if !mariadb::is_initialized(ctx) {
            return Err(DevdError::invalid(format!(
                "the MariaDB data directory at {} has no system tables; reinstall with \
                 `devd service install mariadb --force` to create them",
                mariadb::data_dir(ctx).display()
            )));
        }
        mariadb::write_config(ctx, port)?;
    }

    let def = definition(&service.name);
    let startup = def.map(|d| d.startup).unwrap_or(Duration::from_secs(10));
    let spec = spawn_spec(ctx, &service)?;
    let log = log_path(ctx, &service.name);

    let supervisor: std::sync::Arc<Supervisor> = process::shared();
    let pid = supervisor.start(spec, util::new_id())?;
    util::atomic_write(&pid_file(ctx, &service.name), &pid.to_string())?;
    let run_id = ctx.store.start_service_run(&service.id, Some(pid), 0)?;

    let port = service.port.unwrap_or(0);
    let deadline = Instant::now() + startup;
    let mut listening = port == 0 || local_tcp_open(port);
    while !listening && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
        if !process::pid_alive(pid) {
            break;
        }
        listening = local_tcp_open(port);
    }

    service.desired_state = "running".to_string();
    ctx.store.upsert_service(&service)?;

    if !listening {
        let tail = tail(&log, 25).unwrap_or_default();
        let _ = supervisor.stop(&service.name);
        let _ = ctx.store.finish_service_run(&run_id, -1);
        let _ = std::fs::remove_file(pid_file(ctx, &service.name));
        service.desired_state = "stopped".to_string();
        ctx.store.upsert_service(&service)?;
        return Err(DevdError::Command {
            program: format!("{} start", service.name),
            code: -1,
            stderr: format!(
                "{} did not begin answering on port {port} within {} seconds. Its log ({}) says:\n{}",
                service.name,
                startup.as_secs(),
                log.display(),
                if tail.is_empty() {
                    "  (the log is empty)".to_string()
                } else {
                    tail.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
                }
            ),
        });
    }

    ctx.events.emit_with(
        format!("service.{}.started", service.name),
        format!("started {} (pid {pid})", service.name),
        json!({ "name": service.name, "pid": pid, "port": port }),
    );

    status(ctx, name)
}

/// Stop a service.
pub fn stop(ctx: &Ctx, name: &str) -> Result<Status> {
    let mut service = ctx
        .store
        .get_service(name)?
        .ok_or_else(|| DevdError::not_found(format!("`{name}` is not a devd service")))?;

    let pid = running_pid(ctx, &service);
    let supervisor = process::shared();
    let mut exit_code = 0;

    // A database is asked to finish what it is doing before its process is ended.
    if pid.is_some() {
        if let Some(root) = manifest_root(ctx, name).filter(|_| engines::get(name).is_some()) {
            engines::graceful_stop(ctx, name, &root);
        }
    }

    if supervisor.list().iter().any(|p| p.name == service.name) {
        // The supervisor owns the handle in this process, so it can wait for a
        // clean exit and report the real code.
        if let Some((code, _)) = supervisor.stop(&service.name)? {
            exit_code = code;
        }
    } else if let Some(pid) = pid {
        process::kill_tree(pid);
        // Wait for the port to close, so a start immediately afterwards does not
        // race the old process for the listener.
        let deadline = Instant::now() + Duration::from_secs(15);
        while process::pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    if let Ok(runs) = ctx.store.list_service_runs(&service.id, 5) {
        if let Some(open) = runs.into_iter().find(|r| r.stopped_at.is_none()) {
            let _ = ctx.store.finish_service_run(&open.id, exit_code);
        }
    }
    let _ = std::fs::remove_file(pid_file(ctx, name));

    service.desired_state = "stopped".to_string();
    ctx.store.upsert_service(&service)?;

    ctx.events.emit_with(
        format!("service.{}.stopped", service.name),
        format!("stopped {}", service.name),
        json!({ "name": service.name, "pid": pid, "exit_code": exit_code }),
    );

    status(ctx, name)
}

/// Stop, then start.
pub fn restart(ctx: &Ctx, name: &str) -> Result<Status> {
    stop(ctx, name)?;
    start(ctx, name)
}

/// The last lines of a service's log.
pub fn logs(ctx: &Ctx, name: &str, lines: usize) -> Result<Vec<String>> {
    // A service that does not exist is a not-found; a service that exists but has
    // never run is an empty log, which is a different thing.
    ctx.store
        .get_service(name)?
        .ok_or_else(|| DevdError::not_found(format!("`{name}` is not a devd service")))?;
    tail(&log_path(ctx, name), lines)
}

fn tail(path: &Path, lines: usize) -> Result<Vec<String>> {
    let text = util::read_to_string_opt(path)?.unwrap_or_default();
    let mut all: Vec<String> = text.lines().map(str::to_string).collect();
    if all.len() > lines {
        all.drain(0..all.len() - lines);
    }
    Ok(all)
}

/// Start every enabled service, in dependency order.
///
/// Everything is attempted before anything is reported as failed, because
/// "php-cgi did not start, so nginx was not attempted" tells the user less than
/// both results at once.
pub fn stack_up(ctx: &Ctx) -> Result<Vec<Status>> {
    // The PHP listeners have no archive of their own: they come from the
    // installed PHP runtimes. A stack that refused to start because nobody had
    // created those rows would be a worse answer than one that creates them, so
    // the rows are made here when a runtime is present and simply skipped when
    // none is.
    //
    // One listener per installed PHP version, so that a site pinned to a
    // version is served by that version.
    if !installed_php_series(ctx)?.is_empty() {
        prune_php_services(ctx)?;
        ensure_php_services(ctx)?;
    } else if ctx.store.get_service(PHP_SERVICE)?.is_none()
        && php_executable(ctx).is_ok() {
            ensure_php_service(ctx)?;
        }

    // The listeners come first: nginx needs a FastCGI upstream to answer with.
    let services: Vec<Service> = ctx
        .store
        .list_services()?
        .into_iter()
        .filter(|s| s.enabled)
        .collect();

    let mut started = Vec::new();
    let mut failures = Vec::new();

    let mut ordered: Vec<&Service> = Vec::new();
    // Every PHP listener, whichever version it is for, before anything else.
    ordered.extend(
        services
            .iter()
            .filter(|s| s.name == "php-cgi" || s.name.starts_with("php-cgi-")),
    );
    // Only the web server the settings name is started: two of them would fight
    // over the same ports.
    let web = web::service_name(ctx);
    for name in ["mariadb", web] {
        ordered.extend(services.iter().filter(|s| s.name == name));
    }
    ordered.extend(services.iter().filter(|s| {
        s.name != "php-cgi"
            && !s.name.starts_with("php-cgi-")
            && s.name != "mariadb"
            && s.name != "nginx"
            && s.name != "apache"
    }));

    for service in ordered {
        // A service whose files are missing was removed deliberately; skipping it
        // is right, failing the whole stack because of it is not.
        if executable_path(ctx, service).is_err() {
            continue;
        }
        match start(ctx, &service.name) {
            Ok(s) => started.push(s),
            Err(e) => failures.push(format!("{}: {e}", service.name)),
        }
    }

    if !failures.is_empty() {
        return Err(DevdError::Command {
            program: "devd up".to_string(),
            code: 1,
            stderr: format!(
                "{} service(s) started; these did not:\n  {}",
                started.len(),
                failures.join("\n  ")
            ),
        });
    }
    Ok(started)
}

/// Stop every running service, in reverse dependency order.
pub fn stack_down(ctx: &Ctx) -> Result<Vec<Status>> {
    let order = ["nginx", "apache", "mariadb", "php-cgi"];
    let mut stopped = Vec::new();
    let names: Vec<String> = ctx
        .store
        .list_services()?
        .into_iter()
        .map(|s| s.name)
        .collect();
    for name in order.iter().map(|s| s.to_string()).chain(
        names
            .iter()
            .filter(|n| !order.contains(&n.as_str()))
            .cloned(),
    ) {
        if ctx.store.get_service(&name)?.is_none() {
            continue;
        }
        match stop(ctx, &name) {
            Ok(s) => stopped.push(s),
            Err(e) => {
                return Err(DevdError::Command {
                    program: "devd down".to_string(),
                    code: 1,
                    stderr: format!("stopping `{name}` failed: {e}"),
                })
            }
        }
    }
    Ok(stopped)
}

/// Remove a service's files and its record.
///
/// For MariaDB the data directory under `etc/` is deliberately kept: uninstalling
/// the server is not the same decision as discarding every database, and the
/// result says where the data still is.
pub fn remove(ctx: &Ctx, name: &str) -> Result<serde_json::Value> {
    let service = ctx
        .store
        .get_service(name)?
        .ok_or_else(|| DevdError::not_found(format!("`{name}` is not a devd service")))?;
    if running_pid(ctx, &service).is_some() {
        stop(ctx, name)?;
    }

    let dir = install_dir(ctx, name);
    let bytes = util::directory_size(&dir);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    ctx.store.delete_service(name)?;

    ctx.events.emit(
        format!("service.{name}.removed"),
        format!("removed the {name} installation"),
    );

    Ok(json!({
        "name": name,
        "path": dir.display().to_string(),
        "bytes_removed": bytes,
        "data_kept": if name == "mariadb" {
            Some(mariadb::data_dir(ctx).display().to_string())
        } else if engines::get(name).is_some() {
            Some(engines::data_dir(ctx, name).display().to_string())
        } else {
            None
        },
    }))
}

/// The MariaDB service row's port and whether it is running.
///
/// Used by the site flow, which creates a database only when there is a running
/// server to create it on rather than starting one as a side effect.
pub fn mariadb_service(ctx: &Ctx) -> Option<(u16, bool)> {
    let service = ctx.store.get_service("mariadb").ok()??;
    let port = mariadb::port_of(&service);
    let running = running_pid(ctx, &service).is_some();
    Some((port, running))
}

/// The service component of the environment check.
///
/// The check that earns its place is the third one: a service recorded as
/// `desired_state = running` with no process behind it is a real state - a crash,
/// a machine restart, a task manager kill - and reporting it as healthy would be
/// a lie the user would only discover by opening a browser.
pub fn doctor_check(paths: &Paths) -> Vec<Check> {
    let Ok(store) = Store::open(&paths.state_db()) else {
        return vec![Check::fail("services", "the state store could not be read")];
    };
    let Ok(services) = store.list_services() else {
        return vec![Check::fail(
            "services",
            "the service table could not be read",
        )];
    };
    if services.is_empty() {
        return vec![Check::info(
            "services",
            "none installed; `devd service install nginx` installs the web server",
        )];
    }

    let mut checks = Vec::new();
    let mut lines = Vec::new();
    let mut stale = Vec::new();
    let mut missing = Vec::new();

    for service in &services {
        let program = serde_json::from_str::<Vec<String>>(&service.argv)
            .ok()
            .and_then(|argv| argv.first().map(PathBuf::from))
            .unwrap_or_default();
        if !program.is_file() {
            missing.push(service.name.clone());
            lines.push(format!("{} (files missing)", service.name));
            continue;
        }
        let version = if service.name == PHP_SERVICE {
            // The listener runs the PHP runtime's own binary, so its version is
            // the runtime directory it lives in - there is no manifest of its
            // own to read.
            program
                .parent()
                .and_then(|dir| dir.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown version".to_string())
        } else {
            std::fs::read_to_string(
                paths
                    .services()
                    .join(&service.name)
                    .join(".devd-install.json"),
            )
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| {
                v.get("version")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "unknown version".to_string())
        };
        let running = store
            .get_service(&service.name)
            .ok()
            .flatten()
            .and_then(|s| running_pid_for_check(paths, &s))
            .is_some();
        if service.desired_state == "running" && !running {
            stale.push(service.name.clone());
        }
        lines.push(format!(
            "{} {version} ({})",
            service.name,
            if running { "running" } else { "stopped" }
        ));
    }

    if missing.is_empty() && stale.is_empty() {
        checks.push(Check::ok("services", lines.join(", ")));
    } else if !stale.is_empty() {
        checks.push(Check::fail(
            "services",
            format!(
                "{} should be running but is not: {}",
                if stale.len() == 1 {
                    "one service"
                } else {
                    "these services"
                },
                stale.join(", ")
            ),
        ));
    } else {
        checks.push(Check::fail(
            "services",
            format!(
                "{} is recorded but its files are gone; reinstall it with `devd service install`",
                missing.join(", ")
            ),
        ));
    }
    checks
}

/// A pid check that needs only the layout, for the doctor, which has no `Ctx`.
fn running_pid_for_check(paths: &Paths, service: &Service) -> Option<u32> {
    let text =
        std::fs::read_to_string(paths.services().join(&service.name).join("devd.pid")).ok()?;
    let pid: u32 = text.trim().parse().ok()?;
    if !process::pid_alive(pid) {
        return None;
    }
    let expected = serde_json::from_str::<Vec<String>>(&service.argv)
        .ok()
        .and_then(|argv| argv.first().map(PathBuf::from))
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))?;
    match process::process_image(pid) {
        Some(image) if image.eq_ignore_ascii_case(&expected) => Some(pid),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_pool_has_one_port_per_worker_and_names_that_round_trip() {
        assert_eq!(php_worker_port("8.4", 1), 9084);
        assert_eq!(php_worker_port("8.4", 3), 9284);
        assert_eq!(php_worker_name("8.4", 1), "php-cgi-8.4");
        assert_eq!(php_worker_name("8.4", 3), "php-cgi-8.4-3");
        assert_eq!(split_pool("8.4"), ("8.4".to_string(), false, 1));
        assert_eq!(split_pool("8.4-3"), ("8.4".to_string(), false, 3));
        assert_eq!(split_pool("8.4-alt"), ("8.4".to_string(), true, 1));
        assert_eq!(split_pool("8.4-alt-2"), ("8.4".to_string(), true, 2));
        assert_eq!(php_alt_port("8.4", 1), 9134);
        assert_eq!(php_alt_port("8.4", 3), 9334);
        assert_eq!(php_alt_name("8.4", 1), "php-cgi-8.4-alt");
        assert_eq!(php_alt_name("8.4", 2), "php-cgi-8.4-alt-2");
        assert_eq!(php_pool_of("php-cgi-8.4"), Some(("8.4".to_string(), false)));
        assert_eq!(php_pool_of("php-cgi-8.4-3"), Some(("8.4".to_string(), false)));
        assert_eq!(php_pool_of("php-cgi-8.5-alt-2"), Some(("8.5".to_string(), true)));
        assert_eq!(php_pool_of("php-cgi"), None, "the legacy single listener is not part of a pool");
        assert_eq!(php_pool_of("nginx"), None);
    }

    #[test]
    fn a_port_changed_in_settings_is_used_by_the_next_start() {
        let ctx = ctx("ports");
        let file = ctx.paths.settings_file();
        let booted_http = ctx.settings.http_port;
        let http = if booted_http == 8081 { 8082 } else { 8081 };
        let https = if http == 8443 { 8444 } else { 8443 };
        let mut settings = devd_core::config::Settings::load(&file).unwrap();
        settings.http_port = http;
        settings.https_port = https;
        settings.save(&file).unwrap();

        // The long-lived context still holds the values it booted with; the port
        // is read from disk, so a change made after boot is not lost.
        assert_eq!(http_port(&ctx), http);
        assert_eq!(https_port(&ctx), https);

        // The web server's row moves with the setting: it is what a start is
        // verified against, so a stale row is what fails a correct start.
        let row = Service {
            id: util::new_id(),
            name: "nginx".to_string(),
            kind: "web".to_string(),
            runtime_id: None,
            argv: "[]".to_string(),
            port: Some(booted_http),
            health: Some(json!({ "tcp": booted_http }).to_string()),
            desired_state: "stopped".to_string(),
            restart_policy: "on-failure".to_string(),
            enabled: true,
            created_at: util::now(),
        };
        ctx.store.upsert_service(&row).unwrap();
        set_web_ports(&ctx, http, https).unwrap();

        let stored = ctx.store.get_service("nginx").unwrap().unwrap();
        assert_eq!(stored.port, Some(http));
        assert_eq!(stored.health, Some(json!({ "tcp": http }).to_string()));
        let reloaded = devd_core::config::Settings::load(&file).unwrap();
        assert_eq!((reloaded.http_port, reloaded.https_port), (http, https));

        if let Some(dir) = file.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// A forward-slash path, for asserting on stored argument arrays.
    fn slashy(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn ctx(tag: &str) -> Ctx {
        let dir = std::env::temp_dir().join(format!("devd-svc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(dir);
        paths.ensure().unwrap();
        Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(devd_core::config::Settings::default()),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: false,
        }
    }

    /// A machine whose programs are the files given, with a fixed version banner.
    #[cfg(unix)]
    struct FakeHost {
        programs: Vec<(&'static str, std::path::PathBuf)>,
        banner: &'static str,
    }

    #[cfg(unix)]
    impl devd_core::platform::Commands for FakeHost {
        fn run(&self, _program: &str, _args: &[&str]) -> Option<(i32, String)> {
            None
        }
    }

    #[cfg(unix)]
    impl Host for FakeHost {
        fn find(&self, names: &[&str]) -> Option<std::path::PathBuf> {
            names.iter().find_map(|n| {
                self.programs
                    .iter()
                    .find(|(name, _)| name == n)
                    .map(|(_, p)| p.clone())
            })
        }
        fn banner(&self, _exe: &Path, _args: &[&str]) -> Result<String> {
            Ok(self.banner.to_string())
        }
    }

    #[cfg(unix)]
    #[test]
    fn installing_nginx_on_unix_links_the_system_program_and_records_where_it_came_from() {
        let c = ctx("adopt-nginx");
        let bin = c.paths.root().join("system/sbin");
        std::fs::create_dir_all(&bin).unwrap();
        let real = bin.join("nginx");
        std::fs::write(&real, b"#!/bin/sh\n").unwrap();
        let host = FakeHost {
            programs: vec![("nginx", real.clone())],
            banner: "nginx version: nginx/1.24.0 (Ubuntu)\n",
        };

        let def = definition("nginx").unwrap();
        let done = install_system(&c, def, None, &host).expect("install");

        assert_eq!(done.version, "1.24.0");
        assert!(done.digest.is_none(), "a system install has no download digest");
        // The layout the rest of devd expects: the program is where it is
        // looked for, as a link, and the files nginx needs beside it exist.
        let dir = install_dir(&c, "nginx");
        let linked = dir.join("sbin/nginx");
        assert!(std::fs::symlink_metadata(&linked).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&linked).unwrap(), real);
        assert!(dir.join("conf/fastcgi_params").is_file());
        assert!(dir.join("conf/mime.types").is_file());
        assert!(dir.join("temp/client_body").is_dir());
        // The manifest says the program was adopted, and from where.
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest(&c, "nginx")).unwrap())
                .unwrap();
        assert_eq!(manifest["source"], "system");
        assert_eq!(manifest["version"], "1.24.0");
        // The service is recorded and its command line starts the linked program.
        let service = c.store.get_service("nginx").unwrap().expect("service row");
        let argv: Vec<String> = serde_json::from_str(&service.argv).unwrap();
        assert!(argv[0].ends_with("sbin/nginx"), "{argv:?}");
        // The configuration keeps nginx's temporary files out of /var/lib.
        let conf = std::fs::read_to_string(nginx::config_path(&c)).unwrap();
        assert!(conf.contains("client_body_temp_path"), "{conf}");
    }

    #[cfg(unix)]
    #[test]
    fn a_system_mariadb_that_is_really_mysql_is_refused() {
        let c = ctx("adopt-mysql");
        let bin = c.paths.root().join("system/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["mysqld", "mysql", "mysql_install_db"] {
            std::fs::write(bin.join(name), b"x").unwrap();
        }
        let host = FakeHost {
            programs: ["mysqld", "mysql", "mysql_install_db"]
                .iter()
                .map(|n| (*n, bin.join(n)))
                .collect(),
            banner: "mysqld  Ver 8.0.36 for Linux on x86_64 (MySQL Community Server - GPL)\n",
        };
        let err = install_system(&c, definition("mariadb").unwrap(), None, &host).unwrap_err();
        assert!(err.to_string().contains("not MariaDB"), "{err}");
        assert!(c.store.get_service("mariadb").unwrap().is_none());
    }

    fn build(version: &str, sha: Option<&str>) -> Available {
        Available {
            kind: "nginx".to_string(),
            version: version.to_string(),
            series: devd_runtime::catalogue::series_of(version),
            channel: "mainline".to_string(),
            url: format!("https://example.invalid/nginx-{version}.zip"),
            sha256: sha.map(str::to_string),
            bytes: None,
            published: None,
            executable: "nginx.exe".to_string(),
        }
    }

    #[test]
    fn latest_nginx_means_the_newest_build_with_a_recorded_checksum() {
        // 1.31.8 is out, but no digest for it is recorded.
        let list = vec![
            build("1.31.8", None),
            build("1.31.7", Some("aa")),
            build("1.30.6", Some("bb")),
        ];
        let installable = verifiable("nginx", &list);
        assert_eq!(installable.len(), 2);
        let chosen = resolve(&installable, "latest", None).unwrap();
        assert_eq!(chosen.version, "1.31.7");
        // Another service's builds are all candidates: MariaDB's digest is read
        // from its release API once a build is chosen.
        assert_eq!(verifiable("mariadb", &list).len(), 3);
        // Asking for the unverifiable version by name still finds it, so the
        // install can refuse it with a message that names it.
        let exact = resolve(&installable, "1.31.8", None).or_else(|_| resolve(&list, "1.31.8", None));
        assert_eq!(exact.unwrap().version, "1.31.8");
    }

    #[test]
    fn definitions_are_consistent() {
        for def in DEFINITIONS {
            assert_eq!(definition(def.name).map(|d| d.name), Some(def.name));
            assert_eq!(definition(def.kind).map(|d| d.name), Some(def.name));
            assert!(!def.windows_exe.is_empty() && !def.unix_exe.is_empty());
        }
        assert!(definition("lighttpd").is_none());
    }

    #[test]
    fn a_status_is_readable_for_an_installed_but_stopped_service() {
        let c = ctx("status");
        let m = nginx::directory(&c);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("nginx.exe"), b"binary").unwrap();
        c.store
            .upsert_service(&Service {
                id: util::new_id(),
                name: "nginx".into(),
                kind: "nginx".into(),
                runtime_id: None,
                argv: serde_json::to_string(&vec![slashy(&m.join("nginx.exe"))]).unwrap(),
                port: Some(8080),
                health: None,
                desired_state: "stopped".into(),
                restart_policy: "on-failure".into(),
                enabled: true,
                created_at: util::now(),
            })
            .unwrap();

        let s = status(&c, "nginx").unwrap();
        assert!(s.installed);
        assert!(!s.running);
        assert_eq!(s.port, Some(8080));
        assert!(s.message.contains("stopped"), "{}", s.message);
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_missing_install_is_reported_as_not_installed() {
        let c = ctx("missing");
        c.store
            .upsert_service(&Service {
                id: util::new_id(),
                name: "nginx".into(),
                kind: "nginx".into(),
                runtime_id: None,
                argv: serde_json::to_string(&vec![slashy(&nginx::directory(&c).join("nginx.exe"))])
                    .unwrap(),
                port: Some(80),
                health: None,
                desired_state: "stopped".into(),
                restart_policy: "never".into(),
                enabled: false,
                created_at: util::now(),
            })
            .unwrap();
        let s = status(&c, "nginx").unwrap();
        assert!(!s.installed);
        assert!(s.message.contains("is not installed"), "{}", s.message);
        assert!(start(&c, "nginx").is_err());
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn an_unknown_service_is_not_found_and_says_how_to_install_one() {
        let c = ctx("unknown");
        let err = status(&c, "redis").unwrap_err();
        assert_eq!(err.code(), "not_found");
        assert!(err.to_string().contains("service install"), "{err}");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_pid_file_for_an_unrelated_process_is_not_treated_as_running() {
        let c = ctx("recycled");
        let m = nginx::directory(&c);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("nginx.exe"), b"binary").unwrap();
        // This process is alive, but it is not nginx, so it must not be adopted.
        util::atomic_write(&pid_file(&c, "nginx"), &std::process::id().to_string()).unwrap();
        let service = Service {
            id: util::new_id(),
            name: "nginx".into(),
            kind: "nginx".into(),
            runtime_id: None,
            argv: serde_json::to_string(&vec![slashy(&m.join("nginx.exe"))]).unwrap(),
            port: Some(80),
            health: None,
            desired_state: "stopped".into(),
            restart_policy: "never".into(),
            enabled: true,
            created_at: util::now(),
        };
        assert_eq!(running_pid(&c, &service), None, "the image name must match");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_doctor_reports_an_empty_install_as_information() {
        let dir = std::env::temp_dir().join(format!("devd-svc-doctor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let checks = doctor_check(&paths);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].level, devd_core::Level::Info);
        assert!(checks[0].detail.contains("service install"), "{checks:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_doctor_flags_a_service_that_should_be_running() {
        let dir = std::env::temp_dir().join(format!("devd-svc-doctor2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let store = Store::open(&paths.state_db()).unwrap();
        let exe = paths.services().join("nginx").join("nginx.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"binary").unwrap();
        store
            .upsert_service(&Service {
                id: util::new_id(),
                name: "nginx".into(),
                kind: "nginx".into(),
                runtime_id: None,
                argv: serde_json::to_string(&vec![slashy(&exe)]).unwrap(),
                port: Some(80),
                health: None,
                desired_state: "running".into(),
                restart_policy: "on-failure".into(),
                enabled: true,
                created_at: util::now(),
            })
            .unwrap();
        drop(store);
        let checks = doctor_check(&paths);
        assert_eq!(checks[0].level, devd_core::Level::Fail);
        assert!(checks[0].detail.contains("should be running"), "{checks:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
