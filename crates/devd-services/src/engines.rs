//! PostgreSQL, MongoDB and Memcached: services devd runs but does not download.
//!
//! None of the three publishes a checksum for its Windows build that devd could
//! check (EnterpriseDB's PostgreSQL zips and the community Memcached builds publish
//! none; MongoDB's digests are on a download page that is not machine readable), so
//! devd does not fetch them. It adopts a folder a person already has, records it,
//! writes its own configuration and data directory under the devd home, and runs
//! the program from there. Nothing in the adopted folder is changed.
//!
//! All three bind to 127.0.0.1 only. PostgreSQL is initialised with trust
//! authentication for local connections, the same stated default as MariaDB's
//! passwordless root: a development server that cannot be reached from the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use devd_core::capability::Ctx;
use devd_core::util;
use devd_core::{DevdError, Result};

/// What devd needs to know about one of these programs.
pub struct Engine {
    /// Service name.
    pub name: &'static str,
    /// The server program's name without an extension.
    pub program: &'static str,
    /// Other programs that must sit beside it.
    pub needs: &'static [&'static str],
    /// The folder name portable stacks keep it under (`bin/<tool>/<version>`).
    pub tool: &'static str,
    /// Arguments that make it print its version.
    pub version_args: &'static [&'static str],
    /// Reads the version out of that output.
    pub version_regex: &'static str,
    /// The port it listens on unless told otherwise.
    pub default_port: u16,
}

/// The engines.
pub const ENGINES: &[Engine] = &[
    Engine {
        name: "postgres",
        program: "postgres",
        needs: &["initdb", "pg_ctl", "psql", "pg_dump"],
        tool: "postgresql",
        version_args: &["--version"],
        version_regex: r"(\d+(?:\.\d+){1,2})",
        default_port: 5432,
    },
    Engine {
        name: "mongodb",
        program: "mongod",
        needs: &[],
        tool: "mongodb",
        version_args: &["--version"],
        version_regex: r"v(\d+\.\d+\.\d+)",
        default_port: 27017,
    },
    Engine {
        name: "memcached",
        program: "memcached",
        needs: &[],
        tool: "memcached",
        version_args: &["-V"],
        version_regex: r"memcached (\d+\.\d+\.\d+)",
        default_port: 11211,
    },
];

/// One engine by service name.
pub fn get(name: &str) -> Option<&'static Engine> {
    ENGINES.iter().find(|e| e.name == name)
}

fn with_ext(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Where a program is inside a root: `bin/<name>` or beside the root.
fn locate(root: &Path, stem: &str) -> Option<PathBuf> {
    let name = with_ext(stem);
    [root.join("bin").join(&name), root.join(&name)].into_iter().find(|p| p.is_file())
}

/// The engine's root inside `folder`: the folder itself, or one level down (a
/// version folder, or the `pgsql` folder the PostgreSQL zip unpacks to).
pub fn root_of(engine: &Engine, folder: &Path) -> Option<PathBuf> {
    let complete = |root: &Path| locate(root, engine.program).is_some() && engine.needs.iter().all(|n| locate(root, n).is_some());
    if complete(folder) {
        return Some(folder.to_path_buf());
    }
    let mut kids: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir() && complete(p)).collect();
    kids.sort();
    kids.pop()
}

/// The server program.
pub fn exe(engine: &Engine, root: &Path) -> Result<PathBuf> {
    locate(root, engine.program).ok_or_else(|| DevdError::not_found(format!("{} is not in {}", engine.program, root.display())))
}

/// The version the program prints.
pub fn version(engine: &Engine, root: &Path) -> Option<String> {
    let program = locate(root, engine.program)?;
    let args: Vec<String> = engine.version_args.iter().map(|a| a.to_string()).collect();
    let out = util::run(&program, &args, program.parent(), &[], Some(Duration::from_secs(15))).ok()?;
    util::extract_version(&format!("{}\n{}", out.stdout, out.stderr), Some(engine.version_regex))
}

/// Folders on this machine that hold the engine, with their versions.
///
/// Portable stacks keep them at `X:\<stack>\bin\<tool>\<version>`; a person may have
/// unpacked one anywhere under `C:\` (`C:\pgsql`); on Linux and macOS the system
/// package's folders and the directories on `PATH` are looked in.
pub fn find_local(engine: &Engine) -> Vec<(PathBuf, String)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for drive in 'A'..='Z' {
            let root = PathBuf::from(format!("{drive}:\\"));
            if !root.is_dir() {
                continue;
            }
            candidates.push(root.join(engine.tool));
            candidates.push(root.join("pgsql"));
            candidates.push(root.join("Program Files").join(engine.tool));
            candidates.push(root.join("Program Files").join("PostgreSQL"));
            candidates.push(root.join("Program Files").join("MongoDB").join("Server"));
            let Ok(tops) = std::fs::read_dir(&root) else { continue };
            for top in tops.flatten().filter(|e| e.path().is_dir()) {
                candidates.push(top.path().join("bin").join(engine.tool));
            }
        }
    } else {
        for base in ["/usr/lib/postgresql", "/opt/homebrew/opt", "/usr/local/opt", "/usr/local/bin", "/usr/bin", "/opt/homebrew/bin"] {
            candidates.push(PathBuf::from(base));
            if let Ok(kids) = std::fs::read_dir(base) {
                candidates.extend(kids.flatten().map(|k| k.path()).filter(|p| p.is_dir()));
            }
        }
        if let Some(paths) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&paths));
        }
    }
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    for candidate in candidates {
        let Some(root) = root_of(engine, &candidate) else { continue };
        if out.iter().any(|(p, _)| p == &root) {
            continue;
        }
        if let Some(v) = version(engine, &root) {
            out.push((root, v));
        }
    }
    out
}

/// Why an engine is adopted rather than downloaded.
pub fn download_refusal(engine: &Engine) -> DevdError {
    DevdError::unsupported(format!(
        "devd does not download {}: its Windows builds publish no checksum devd could verify. Use the one you already have: `devd service find {}`, then `devd service adopt {} <folder>`",
        engine.name, engine.name, engine.name
    ))
}

/// Where the engine keeps its data. Outside the adopted folder, on purpose.
pub fn data_dir(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.paths.etc().join(name).join("data")
}

fn slashy(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The argument array that runs the engine.
pub fn argv(ctx: &Ctx, engine: &Engine, root: &Path, port: u16) -> Result<Vec<String>> {
    let program = exe(engine, root)?.display().to_string();
    let data = data_dir(ctx, engine.name);
    let mut args = vec![program];
    match engine.name {
        "postgres" => {
            args.extend(["-D".into(), data.display().to_string(), "-p".into(), port.to_string(), "-c".into(), "listen_addresses=127.0.0.1".into()]);
            if !cfg!(windows) {
                // A package's default socket directory is not writable by an ordinary user.
                args.extend(["-c".into(), format!("unix_socket_directories={}", slashy(&ctx.paths.etc().join("postgres").join("run")))]);
            }
        }
        "mongodb" => args.extend(["--dbpath".into(), data.display().to_string(), "--bind_ip".into(), "127.0.0.1".into(), "--port".into(), port.to_string()]),
        "memcached" => args.extend(["-l".into(), "127.0.0.1".into(), "-p".into(), port.to_string(), "-m".into(), "64".into()]),
        other => return Err(DevdError::unsupported(format!("no engine named {other}"))),
    }
    Ok(args)
}

/// Make the data directory the engine needs. Returns what the caller should say.
///
/// Idempotent: a directory that already holds a cluster is left alone.
pub fn prepare(ctx: &Ctx, engine: &Engine, root: &Path) -> Result<Vec<String>> {
    let data = data_dir(ctx, engine.name);
    std::fs::create_dir_all(&data)?;
    let mut notes = Vec::new();
    if engine.name == "postgres" {
        if !cfg!(windows) {
            std::fs::create_dir_all(ctx.paths.etc().join("postgres").join("run"))?;
        }
        if !data.join("PG_VERSION").is_file() {
            let initdb = locate(root, "initdb").ok_or_else(|| DevdError::not_found("initdb is not beside postgres"))?;
            let args = [
                "-D".to_string(),
                data.display().to_string(),
                "-U".to_string(),
                "postgres".to_string(),
                "--auth=trust".to_string(),
                "-E".to_string(),
                "UTF8".to_string(),
                "--no-locale".to_string(),
            ];
            let out = util::run(&initdb, &args, initdb.parent(), &[], Some(Duration::from_secs(300)))?;
            if !data.join("PG_VERSION").is_file() {
                return Err(DevdError::Command {
                    program: "initdb".to_string(),
                    code: out.code,
                    stderr: util::truncate(format!("{}{}", out.stdout, out.stderr).trim(), 1500),
                });
            }
            notes.push("PostgreSQL was initialised with trust authentication for connections from this machine and listens on 127.0.0.1 only".to_string());
        }
    }
    Ok(notes)
}

/// Ask the engine to stop cleanly before its process is ended. Best effort: a
/// failure here falls through to the ordinary stop.
pub fn graceful_stop(ctx: &Ctx, name: &str, root: &Path) {
    if name != "postgres" {
        return;
    }
    if let Some(pg_ctl) = locate(root, "pg_ctl") {
        let args = ["-D".to_string(), data_dir(ctx, "postgres").display().to_string(), "stop".to_string(), "-m".to_string(), "fast".to_string(), "-w".to_string(), "-t".to_string(), "20".to_string()];
        let _ = util::run(&pg_ctl, &args, pg_ctl.parent(), &[], Some(Duration::from_secs(40)));
    }
}

// --------------------------------------------------------------- PostgreSQL databases

/// Run one statement through `psql` against devd's PostgreSQL.
fn psql(ctx: &Ctx, root: &Path, port: u16, sql: &str) -> Result<String> {
    let _ = ctx;
    let program = locate(root, "psql").ok_or_else(|| DevdError::not_found("psql is not installed beside postgres"))?;
    let args = [
        "-h".to_string(),
        "127.0.0.1".to_string(),
        "-p".to_string(),
        port.to_string(),
        "-U".to_string(),
        "postgres".to_string(),
        "-d".to_string(),
        "postgres".to_string(),
        "-X".to_string(),
        "-A".to_string(),
        "-t".to_string(),
        "-v".to_string(),
        "ON_ERROR_STOP=1".to_string(),
        "-c".to_string(),
        sql.to_string(),
    ];
    let out = util::run(&program, &args, program.parent(), &[], Some(Duration::from_secs(60)))?;
    if out.code != 0 {
        return Err(DevdError::Command { program: "psql".to_string(), code: out.code, stderr: util::truncate(out.stderr.trim(), 1500) });
    }
    Ok(out.stdout)
}

/// A PostgreSQL database name: letters, digits and underscores, not starting with a digit.
pub fn valid_pg_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(DevdError::invalid(format!("`{name}` is not a PostgreSQL database name: letters, digits and _ only, not starting with a digit")))
    }
}

/// The databases on devd's PostgreSQL, excluding the templates and `postgres`.
pub fn pg_databases(ctx: &Ctx, root: &Path, port: u16) -> Result<Vec<String>> {
    let out = psql(ctx, root, port, "SELECT datname FROM pg_database WHERE NOT datistemplate AND datname <> 'postgres' ORDER BY 1")?;
    Ok(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// Create a database (UTF-8, from template0) if it is not there.
pub fn pg_create(ctx: &Ctx, root: &Path, port: u16, name: &str) -> Result<()> {
    valid_pg_name(name)?;
    if pg_databases(ctx, root, port)?.iter().any(|d| d == name) {
        return Ok(());
    }
    psql(ctx, root, port, &format!("CREATE DATABASE \"{name}\" ENCODING 'UTF8' TEMPLATE template0")).map(|_| ())
}

/// Drop a database.
pub fn pg_drop(ctx: &Ctx, root: &Path, port: u16, name: &str) -> Result<()> {
    valid_pg_name(name)?;
    psql(ctx, root, port, &format!("DROP DATABASE IF EXISTS \"{name}\"")).map(|_| ())
}

/// How many tables a database has.
pub fn pg_table_count(root: &Path, port: u16, name: &str) -> Result<u64> {
    valid_pg_name(name)?;
    let program = locate(root, "psql").ok_or_else(|| DevdError::not_found("psql is not installed beside postgres"))?;
    let args = ["-h".to_string(), "127.0.0.1".to_string(), "-p".to_string(), port.to_string(), "-U".to_string(), "postgres".to_string(), "-d".to_string(), name.to_string(), "-X".to_string(), "-A".to_string(), "-t".to_string(), "-c".to_string(), "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema')".to_string()];
    let out = util::run(&program, &args, program.parent(), &[], Some(Duration::from_secs(60)))?;
    Ok(if out.code == 0 { out.stdout.trim().parse().unwrap_or(0) } else { 0 })
}

/// Write a database to a plain SQL file with `pg_dump`.
pub fn pg_dump(root: &Path, host_port: u16, user: &str, password: Option<&str>, database: &str, out: &Path) -> Result<()> {
    valid_pg_name(database)?;
    let program = locate(root, "pg_dump").ok_or_else(|| DevdError::not_found("pg_dump is not installed beside postgres"))?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let args = [
        "-h".to_string(),
        "127.0.0.1".to_string(),
        "-p".to_string(),
        host_port.to_string(),
        "-U".to_string(),
        user.to_string(),
        "--no-owner".to_string(),
        "--no-privileges".to_string(),
        "-f".to_string(),
        out.display().to_string(),
        database.to_string(),
    ];
    let envs: Vec<(String, String)> = password.map(|p| vec![("PGPASSWORD".to_string(), p.to_string())]).unwrap_or_default();
    let res = util::run(&program, &args, program.parent(), &envs, Some(Duration::from_secs(3600)))?;
    if res.code != 0 {
        return Err(DevdError::Command { program: "pg_dump".to_string(), code: res.code, stderr: util::truncate(res.stderr.trim(), 1500) });
    }
    Ok(())
}

/// Run a SQL file against a database on devd's PostgreSQL.
pub fn pg_restore(ctx: &Ctx, root: &Path, port: u16, database: &str, file: &Path) -> Result<()> {
    pg_create(ctx, root, port, database)?;
    let program = locate(root, "psql").ok_or_else(|| DevdError::not_found("psql is not installed beside postgres"))?;
    let args = [
        "-h".to_string(),
        "127.0.0.1".to_string(),
        "-p".to_string(),
        port.to_string(),
        "-U".to_string(),
        "postgres".to_string(),
        "-d".to_string(),
        database.to_string(),
        "-X".to_string(),
        "-q".to_string(),
        "-v".to_string(),
        "ON_ERROR_STOP=1".to_string(),
        "-f".to_string(),
        file.display().to_string(),
    ];
    let res = util::run(&program, &args, program.parent(), &[], Some(Duration::from_secs(3600)))?;
    if res.code != 0 {
        return Err(DevdError::Command { program: "psql".to_string(), code: res.code, stderr: util::truncate(res.stderr.trim(), 1500) });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_root(dir: &Path, programs: &[&str]) {
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        for p in programs {
            std::fs::write(dir.join("bin").join(with_ext(p)), b"x").unwrap();
        }
    }

    #[test]
    fn a_folder_is_an_engine_root_only_when_everything_it_needs_is_there() {
        let base = std::env::temp_dir().join(format!("devd-engines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pg = get("postgres").unwrap();
        fake_root(&base.join("full"), &["postgres", "initdb", "pg_ctl", "psql", "pg_dump"]);
        fake_root(&base.join("partial"), &["postgres"]);
        fake_root(&base.join("zip").join("pgsql"), &["postgres", "initdb", "pg_ctl", "psql", "pg_dump"]);
        assert_eq!(root_of(pg, &base.join("full")), Some(base.join("full")));
        assert_eq!(root_of(pg, &base.join("partial")), None, "a server without initdb cannot make a cluster");
        assert_eq!(root_of(pg, &base.join("zip")), Some(base.join("zip").join("pgsql")), "the zip's own folder is found one level down");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn each_engine_listens_on_loopback_and_keeps_its_data_under_the_devd_home() {
        let dir = std::env::temp_dir().join(format!("devd-engine-argv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("prog");
        fake_root(&root, &["postgres", "initdb", "pg_ctl", "psql", "pg_dump", "mongod", "memcached"]);
        let paths = devd_core::Paths::rooted(dir.join("home"));
        let ctx = Ctx {
            store: std::sync::Arc::new(devd_core::state::Store::open_in_memory().unwrap()),
            paths: std::sync::Arc::new(paths),
            settings: std::sync::Arc::new(devd_core::config::Settings::default()),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: false,
        };
        let pg = argv(&ctx, get("postgres").unwrap(), &root, 5433).unwrap();
        assert!(pg.contains(&"listen_addresses=127.0.0.1".to_string()) && pg.contains(&"5433".to_string()));
        assert!(pg.iter().any(|a| a.contains("postgres") && a.contains("data")), "{pg:?}");
        let mongo = argv(&ctx, get("mongodb").unwrap(), &root, 27018).unwrap();
        assert!(mongo.windows(2).any(|w| w[0] == "--bind_ip" && w[1] == "127.0.0.1"));
        let mem = argv(&ctx, get("memcached").unwrap(), &root, 11212).unwrap();
        assert!(mem.windows(2).any(|w| w[0] == "-l" && w[1] == "127.0.0.1") && mem.contains(&"11212".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn database_names_cannot_carry_a_second_statement() {
        assert!(valid_pg_name("shop_2024").is_ok());
        for bad in ["", "1shop", "a-b", "a;drop", "a b", "a\"b", &"a".repeat(64)] {
            assert!(valid_pg_name(bad).is_err(), "{bad}");
        }
    }
}
