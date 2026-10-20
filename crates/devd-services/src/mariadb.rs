//! MariaDB: install, initialize, and create databases.
//!
//! MariaDB is the one service with real state of its own. Three decisions follow
//! from that:
//!
//! * **The data directory lives under `etc/`, never inside the install
//!   directory.** Reinstalling or removing the server therefore cannot delete
//!   databases as a side effect, and `service.remove mariadb` says where the data
//!   still is.
//! * **`mysql_install_db.exe` is run once**, when the data directory is empty. It
//!   creates the system tables and nothing else: no Windows service is
//!   registered, so MariaDB runs as a child of the devd supervisor exactly like
//!   nginx, and starts and stops when the stack does.
//! * **The root password comes from the environment or not at all.** A local
//!   development server bound to `127.0.0.1` with a passwordless root is the
//!   normal default, and it is stated plainly instead of being hidden. If
//!   `DEVD_MYSQL_ROOT_PASSWORD` is set when the server is initialized, that
//!   password is hashed into the database and is passed to the client through the
//!   environment on every later call, so it never appears in `argv`, in a log, in
//!   `settings.toml` or in a capability result.
//!
//! The server is bound to the loopback address and cannot be reached from the
//! network, which is what makes the passwordless default defensible for a
//! development machine and not for anything else. `docs/security.md` says so in
//! the same words.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::state::Service;
use devd_core::util;
use devd_core::{DevdError, Result};

/// Environment variable holding the root password, when one is wanted.
pub const ENV_ROOT_PASSWORD: &str = "DEVD_MYSQL_ROOT_PASSWORD";

/// How long initialization may take. Creating the system tables on Windows with
/// a cold page cache takes tens of seconds, not the two a service start takes.
const INIT_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a client command may take.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(60);

/// The MariaDB installation directory.
pub fn directory(ctx: &Ctx) -> PathBuf {
    ctx.paths.services().join("mariadb")
}

/// The `bin` directory of the installation.
pub fn bin_dir(ctx: &Ctx) -> PathBuf {
    directory(ctx).join("bin")
}

/// The server executable.
pub fn server_exe(ctx: &Ctx) -> PathBuf {
    bin_dir(ctx).join(exe("mysqld.exe", "mariadbd"))
}

/// The client executable.
pub fn client_exe(ctx: &Ctx) -> PathBuf {
    bin_dir(ctx).join(exe("mysql.exe", "mysql"))
}

/// The bootstrap executable.
pub fn install_db_exe(ctx: &Ctx) -> PathBuf {
    bin_dir(ctx).join(exe("mysql_install_db.exe", "mariadb-install-db"))
}

/// Where the databases live. Outside the install directory, on purpose.
pub fn data_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.etc().join("mariadb").join("data")
}

/// The generated server configuration.
pub fn config_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.etc().join("mariadb").join("my.ini")
}

/// The server's error log.
pub fn error_log(ctx: &Ctx) -> PathBuf {
    ctx.paths.logs().join("mariadb-error.log")
}

fn exe(windows: &'static str, unix: &'static str) -> &'static str {
    if cfg!(windows) {
        windows
    } else {
        unix
    }
}

/// The result of preparing a data directory.
#[derive(Debug, Clone, Serialize)]
pub struct Provision {
    /// Where the data lives.
    pub data_dir: String,
    /// Where the configuration was written.
    pub config: String,
    /// Whether this call created the system tables.
    pub initialized: bool,
    /// The port the server will listen on.
    pub port: u16,
    /// Whether a root password was set from the environment.
    pub root_password_set: bool,
    /// Anything the caller should know.
    pub warnings: Vec<String>,
}

/// Whether the system tables are present.
pub fn is_initialized(ctx: &Ctx) -> bool {
    // `mysql` is the schema mysql_install_db creates first; its absence means the
    // bootstrap either never ran or did not finish.
    data_dir(ctx).join("mysql").is_dir()
}

/// The root password, when the operator supplied one.
///
/// Read at the moment of use and never stored. An empty value is treated as
/// unset, because an empty password is the same as no password and pretending
/// otherwise would imply a protection that is not there.
pub fn root_password() -> Option<String> {
    match std::env::var(ENV_ROOT_PASSWORD) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

/// Write `my.ini`.
///
/// Every path in it is absolute: the server is not started from its own
/// directory by every surface, so nothing may depend on the working directory.
pub fn write_config(ctx: &Ctx, port: u16) -> Result<PathBuf> {
    let basedir = slashy(&directory(ctx));
    let datadir = slashy(&data_dir(ctx));
    let log = slashy(&error_log(ctx));
    let pid = slashy(&ctx.paths.etc().join("mariadb").join("mariadb.pid"));
    let text = format!(
        "# Generated by devd. Do not edit.\n\
         # The server is bound to the loopback address: this is a development database,\n\
         # not one to expose. `devd service install mariadb --port N` changes the port.\n\n\
         [mysqld]\n\
         basedir=\"{basedir}\"\n\
         datadir=\"{datadir}\"\n\
         port={port}\n\
         bind-address=127.0.0.1\n\
         skip-name-resolve\n\
         log-error=\"{log}\"\n\
         pid-file=\"{pid}\"\n\
         character-set-server=utf8mb4\n\
         collation-server=utf8mb4_unicode_ci\n\
         # Strict by default, with the one leniency every modern framework expects.\n\
         sql_mode=\"STRICT_TRANS_TABLES,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION\"\n\
         max_allowed_packet=64M\n\
         # A development profile: small buffer pool, and a commit policy that trades a\n\
         # fraction of durability for the speed of test suites that insert in a loop.\n\
         innodb_buffer_pool_size=256M\n\
         innodb_flush_log_at_trx_commit=2\n\
         innodb_file_per_table=1\n\
         explicit_defaults_for_timestamp=1\n\
         slow_query_log=0\n\n\
         [client]\n\
         port={port}\n\
         default-character-set=utf8mb4\n"
    );
    util::atomic_write(&config_path(ctx), &text)?;
    Ok(config_path(ctx))
}

/// Write the configuration and create the system tables when they are missing.
pub fn provision(ctx: &Ctx, port: u16) -> Result<Provision> {
    write_config(ctx, port)?;
    std::fs::create_dir_all(data_dir(ctx))?;

    let mut warnings = Vec::new();
    let password = root_password();
    let initialized = if is_initialized(ctx) {
        false
    } else {
        bootstrap(ctx, port, password.as_deref())?;
        true
    };
    if !initialized {
        warnings.push(format!(
            "the data directory at {} already contained databases; it was left untouched",
            data_dir(ctx).display()
        ));
    } else {
        // `mysql_install_db` creates a `test` schema that any local user can
        // write to. It is removed here, while the server is still stopped, so
        // that a fresh devd installation does not ship a world-writable
        // database. Only a directory this bootstrap just created is touched.
        let test_schema = data_dir(ctx).join("test");
        if test_schema.is_dir() {
            let _ = std::fs::remove_dir_all(&test_schema);
        }
    }
    if password.is_none() {
        warnings.push(format!(
            "root has no password and the server binds to 127.0.0.1 only. Set {ENV_ROOT_PASSWORD} \
             before installing to give root a password; see docs/security.md"
        ));
    }

    Ok(Provision {
        data_dir: data_dir(ctx).display().to_string(),
        config: config_path(ctx).display().to_string(),
        initialized,
        port,
        root_password_set: password.is_some(),
        warnings,
    })
}

/// Run `mysql_install_db` to create the system tables.
fn bootstrap(ctx: &Ctx, port: u16, password: Option<&str>) -> Result<()> {
    let exe = install_db_exe(ctx);
    if !exe.is_file() {
        return Err(DevdError::not_found(format!(
            "{} does not contain {}, so the data directory cannot be created",
            directory(ctx).display(),
            exe.file_name().unwrap_or_default().to_string_lossy()
        )));
    }

    let mut args = vec![
        // `mysql_install_db.exe` takes no `--basedir`: it works that out from
        // where it is running, and rejects the flag as an unknown variable.
        format!("--datadir={}", data_dir(ctx).display()),
        format!("--port={port}"),
        "--default-user".to_string(),
    ];
    // The password is passed on the bootstrap command line only, where the
    // operating system shows it to the same user who typed it. It is hashed into
    // the database by the bootstrap and is never written to a file devd owns.
    if let Some(password) = password {
        args.push(format!("--password={password}"));
    }

    let out = util::run(&exe, &args, Some(&directory(ctx)), &[], Some(INIT_TIMEOUT))?;
    if !is_initialized(ctx) {
        return Err(DevdError::Command {
            program: "mysql_install_db".to_string(),
            code: out.code,
            stderr: format!(
                "the system tables were not created.\nstdout:\n{}\nstderr:\n{}",
                util::truncate(out.stdout.trim(), 2000),
                util::truncate(out.stderr.trim(), 2000)
            ),
        });
    }
    Ok(())
}

/// Run a statement through the client.
///
/// The password reaches the client through `MYSQL_PWD` rather than `argv`, so it
/// is not visible to another user listing processes, and the SQL is a fixed
/// statement built from a validated identifier.
fn query(ctx: &Ctx, port: u16, sql: &str) -> Result<Value> {
    let exe = client_exe(ctx);
    if !exe.is_file() {
        return Err(DevdError::not_found(format!(
            "the MariaDB client is not installed at {}",
            exe.display()
        )));
    }
    let mut args = vec![
        // --defaults-file must come first: everything after it is an override.
        format!("--defaults-file={}", config_path(ctx).display()),
        "--protocol=TCP".to_string(),
        "--host=127.0.0.1".to_string(),
        format!("--port={port}"),
        "--user=root".to_string(),
        "--connect-timeout=15".to_string(),
        "--batch".to_string(),
        // Rows only. Without this the first line of a `SELECT` is the column
        // heading, which reads as a database called `schema_name`.
        "--skip-column-names".to_string(),
        format!("--execute={sql}"),
    ];
    if root_password().is_none() {
        // An explicit empty password stops the client from looking for a .my.cnf.
        args.push("--password=".to_string());
    }

    let envs: Vec<(String, String)> = match root_password() {
        Some(password) => vec![("MYSQL_PWD".to_string(), password)],
        None => Vec::new(),
    };

    let out = util::run(
        &exe,
        &args,
        Some(&bin_dir(ctx)),
        &envs,
        Some(CLIENT_TIMEOUT),
    )?;
    Ok(json!({
        "stdout": out.stdout.trim(),
        "stderr": out.stderr.trim(),
        "code": out.code,
    }))
}

/// Create a database if it does not exist.
///
/// The name is validated as a plain identifier before it is interpolated, so a
/// database name can never carry a second statement.
pub fn create_database(ctx: &Ctx, port: u16, name: &str) -> Result<Value> {
    util::validate_name(name)?;
    let sql = format!(
        "CREATE DATABASE IF NOT EXISTS `{name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    );
    let result = query(ctx, port, &sql)?;
    let code = result.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        return Err(DevdError::Command {
            program: "mysql".to_string(),
            code: code as i32,
            stderr: result
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(json!({ "database": name, "created": true }))
}

/// Remove a database.
pub fn drop_database(ctx: &Ctx, port: u16, name: &str) -> Result<Value> {
    util::validate_name(name)?;
    // `DROP DATABASE` is the one destructive statement here, and it is only ever
    // reached through the `db.drop` capability, whose tier requires approval.
    let sql = format!("DROP DATABASE IF EXISTS `{name}`");
    let result = query(ctx, port, &sql)?;
    let code = result.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        return Err(DevdError::Command {
            program: "mysql".to_string(),
            code: code as i32,
            stderr: result
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(json!({ "database": name, "dropped": true }))
}

/// The databases on the server, excluding the system schemas.
pub fn databases(ctx: &Ctx, port: u16) -> Result<Vec<String>> {
    let sql = "SELECT schema_name FROM information_schema.schemata \
               WHERE schema_name NOT IN \
               ('mysql','information_schema','performance_schema','sys') ORDER BY schema_name";
    let result = query(ctx, port, sql)?;
    let code = result.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        return Err(DevdError::Command {
            program: "mysql".to_string(),
            code: code as i32,
            stderr: result
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(result
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// The port a service row listens on, with the driver's default.
pub fn port_of(service: &Service) -> u16 {
    service.port.unwrap_or(3306)
}

/// The server version, from the installed files rather than from a live server.
pub fn version_from_files(ctx: &Ctx) -> Option<String> {
    // `mariadb.exe`/`mysqld.exe` are the same version; the directory name in the
    // install manifest is authoritative and cheap to read.
    let text = std::fs::read_to_string(directory(ctx).join(".devd-install.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn slashy(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::paths::Paths;
    use devd_core::state::Store;
    use std::sync::Arc;

    fn ctx(tag: &str) -> Ctx {
        let dir = std::env::temp_dir().join(format!("devd-maria-{tag}-{}", std::process::id()));
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

    #[test]
    fn the_configuration_is_absolute_bound_to_loopback_and_portable() {
        let c = ctx("config");
        write_config(&c, 3307).unwrap();
        let text = std::fs::read_to_string(config_path(&c)).unwrap();
        assert!(text.contains("port=3307"), "{text}");
        assert!(text.contains("bind-address=127.0.0.1"), "{text}");
        assert!(text.contains("datadir=\""), "{text}");
        assert!(text.contains("basedir=\""), "{text}");
        assert!(text.contains("utf8mb4"), "{text}");
        assert!(
            !text.contains('\\'),
            "no backslashes may reach the server: {text}"
        );
        // The data directory must not be inside the install directory, or
        // removing the server would take the databases with it.
        assert!(!data_dir(&c).starts_with(directory(&c)));
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_port_is_the_only_thing_that_changes_between_configurations() {
        let c = ctx("ports");
        write_config(&c, 3306).unwrap();
        let one = std::fs::read_to_string(config_path(&c)).unwrap();
        write_config(&c, 3308).unwrap();
        let two = std::fs::read_to_string(config_path(&c)).unwrap();
        assert_ne!(one, two);
        assert!(two.contains("port=3308"));
        assert!(!two.contains("port=3306"), "{two}");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn an_empty_data_directory_is_not_initialized() {
        let c = ctx("init");
        assert!(!is_initialized(&c));
        std::fs::create_dir_all(data_dir(&c)).unwrap();
        assert!(!is_initialized(&c));
        std::fs::create_dir_all(data_dir(&c).join("mysql")).unwrap();
        assert!(is_initialized(&c));
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_database_name_that_could_carry_a_second_statement_is_refused() {
        let c = ctx("sqli");
        let err = create_database(&c, 3306, "blog`; DROP DATABASE mysql; --").unwrap_err();
        assert_eq!(err.code(), "invalid_input", "{err}");
        // It fails before the client is even looked for, which is what makes the
        // validation load-bearing rather than decorative.
        assert!(
            !err.to_string().contains("client is not installed"),
            "{err}"
        );
        assert_eq!(
            drop_database(&c, 3306, "MySQL").unwrap_err().code(),
            "invalid_input"
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_root_password_is_read_from_the_environment_and_never_defaulted() {
        let previous = std::env::var(ENV_ROOT_PASSWORD).ok();
        std::env::remove_var(ENV_ROOT_PASSWORD);
        assert_eq!(root_password(), None);
        std::env::set_var(ENV_ROOT_PASSWORD, "");
        assert_eq!(root_password(), None, "an empty password is no password");
        std::env::set_var(ENV_ROOT_PASSWORD, "s3cret");
        assert_eq!(root_password().as_deref(), Some("s3cret"));
        match previous {
            Some(value) => std::env::set_var(ENV_ROOT_PASSWORD, value),
            None => std::env::remove_var(ENV_ROOT_PASSWORD),
        }
    }

    #[test]
    fn the_port_falls_back_to_the_servers_default() {
        let service = Service {
            id: util::new_id(),
            name: "mariadb".into(),
            kind: "mariadb".into(),
            runtime_id: None,
            argv: "[]".into(),
            port: None,
            health: None,
            desired_state: "stopped".into(),
            restart_policy: "on-failure".into(),
            enabled: true,
            created_at: util::now(),
        };
        assert_eq!(port_of(&service), 3306);
        let mut with_port = service;
        with_port.port = Some(3307);
        assert_eq!(port_of(&with_port), 3307);
    }
}
