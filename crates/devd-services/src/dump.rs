//! Moving databases: dump one to a file, restore a file into one, or copy one from
//! another server on this machine into devd's MariaDB.
//!
//! The last is what a move from another local stack needs: its MySQL holds the
//! databases, devd's MariaDB is where they are going, and a hand-typed `mysqldump | mysql` is the part
//! nobody wants to remember. Both ends must be on this machine; a dump that can name
//! a remote host is a way to send a database somewhere.
//!
//! Passwords reach the clients through `MYSQL_PWD`, never `argv`. The SQL a restore
//! runs is the dump file itself, fed on standard input.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::util;
use devd_core::{DevdError, Result};

use crate::mariadb;

/// How long a dump or a restore may take.
const LIMIT: Duration = Duration::from_secs(60 * 60);

/// A MySQL-protocol server on this machine.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Always a loopback address.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Account.
    pub user: String,
    /// Password, when there is one.
    pub password: Option<String>,
}

impl Endpoint {
    /// devd's own MariaDB.
    pub fn devd(port: u16) -> Self {
        Self { host: "127.0.0.1".into(), port, user: "root".into(), password: mariadb::root_password() }
    }

    /// Another server on this machine. Refuses anything that is not loopback.
    pub fn local(host: Option<&str>, port: u16, user: Option<&str>, password: Option<String>) -> Result<Self> {
        let host = host.unwrap_or("127.0.0.1").trim().to_string();
        if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
            return Err(DevdError::denied(
                "a database can only be copied from a server on this machine (127.0.0.1)",
            ));
        }
        let user = user.unwrap_or("root").trim().to_string();
        if user.is_empty() || !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(DevdError::invalid("the user name may hold letters, digits, - and _"));
        }
        Ok(Self { host, port, user, password })
    }
}

/// A database name as MySQL allows it, for a source that was named by someone
/// else. devd's own names are stricter ([`util::validate_name`]).
pub fn valid_source_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || name.starts_with('-') || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$')) {
        return Err(DevdError::invalid(format!(
            "`{name}` is not a database name: letters, digits, _, - and $ only"
        )));
    }
    Ok(())
}

/// Where dumps go unless the caller names a place.
pub fn backups_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("backups")
}

fn dump_exe(ctx: &Ctx) -> Result<PathBuf> {
    let bin = mariadb::bin_dir(ctx);
    let names: &[&str] = if cfg!(windows) { &["mariadb-dump.exe", "mysqldump.exe"] } else { &["mariadb-dump", "mysqldump"] };
    names
        .iter()
        .map(|n| bin.join(n))
        .find(|p| p.is_file())
        .ok_or_else(|| DevdError::not_found("the MariaDB dump tool is not installed; run `devd service install mariadb`"))
}

fn connection_args(at: &Endpoint) -> Vec<String> {
    vec![
        "--protocol=TCP".into(),
        format!("--host={}", at.host),
        format!("--port={}", at.port),
        format!("--user={}", at.user),
    ]
}

fn envs(at: &Endpoint) -> Vec<(String, String)> {
    at.password.iter().map(|p| ("MYSQL_PWD".to_string(), p.clone())).collect()
}

/// Run a command to completion within [`LIMIT`], with an optional file as its input.
fn run(program: &Path, args: &[String], envs: &[(String, String)], stdin: Option<&Path>, what: &str) -> Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args).envs(envs.iter().cloned()).stdout(Stdio::null()).stderr(Stdio::piped());
    match stdin {
        Some(file) => cmd.stdin(Stdio::from(std::fs::File::open(file)?)),
        None => cmd.stdin(Stdio::null()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| DevdError::Command { program: what.to_string(), code: -1, stderr: e.to_string() })?;
    let started = Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                if status.success() {
                    return Ok(());
                }
                return Err(DevdError::Command {
                    program: what.to_string(),
                    code: status.code().unwrap_or(-1),
                    stderr: util::truncate(stderr.trim(), 2000),
                });
            }
            None if started.elapsed() > LIMIT => {
                let _ = child.kill();
                return Err(DevdError::Command { program: what.to_string(), code: -1, stderr: "it took longer than an hour and was stopped".into() });
            }
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Write one database to `out` as SQL.
pub fn dump(ctx: &Ctx, from: &Endpoint, database: &str, out: &Path) -> Result<Value> {
    valid_source_name(database)?;
    let exe = dump_exe(ctx)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut args = vec!["--no-defaults".to_string()];
    args.extend(connection_args(from));
    args.extend([
        "--single-transaction".to_string(),
        "--routines".to_string(),
        "--triggers".to_string(),
        "--events".to_string(),
        "--default-character-set=utf8mb4".to_string(),
        format!("--result-file={}", out.display()),
        database.to_string(),
    ]);
    run(&exe, &args, &envs(from), None, "mysqldump")?;
    let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    Ok(json!({ "database": database, "file": out.display().to_string(), "bytes": bytes }))
}

/// Read SQL from `file` into a database on devd's MariaDB, creating the database
/// when it is not there.
pub fn restore(ctx: &Ctx, to: &Endpoint, database: &str, file: &Path) -> Result<Value> {
    valid_source_name(database)?;
    if !file.is_file() {
        return Err(DevdError::not_found(format!("{} is not a file", file.display())));
    }
    let client = mariadb::client_exe(ctx);
    if !client.is_file() {
        return Err(DevdError::not_found("the MariaDB client is not installed; run `devd service install mariadb`"));
    }
    mariadb::create_database_unchecked(ctx, to.port, database)?;
    let mut args = vec!["--no-defaults".to_string()];
    args.extend(connection_args(to));
    args.extend(["--connect-timeout=15".to_string(), "--default-character-set=utf8mb4".to_string(), database.to_string()]);
    run(&client, &args, &envs(to), Some(file), "mysql")?;
    Ok(json!({ "database": database, "file": file.display().to_string() }))
}

/// A timestamped file name inside the backups directory.
pub fn default_file(ctx: &Ctx, database: &str) -> PathBuf {
    let stamp = util::now().replace([':', '-'], "").replace('T', "-");
    backups_dir(ctx).join(format!("{database}-{}.sql", &stamp[..stamp.len().min(15)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_on_this_machine_can_be_a_source() {
        assert!(Endpoint::local(None, 3306, None, None).is_ok());
        assert!(Endpoint::local(Some("localhost"), 3306, Some("root"), None).is_ok());
        for host in ["10.0.0.5", "db.example.com", "0.0.0.0", "127.0.0.1.evil.test"] {
            assert_eq!(Endpoint::local(Some(host), 3306, None, None).unwrap_err().code(), "permission_denied", "{host}");
        }
        assert!(Endpoint::local(None, 3306, Some("root; DROP"), None).is_err());
    }

    #[test]
    fn a_source_database_name_cannot_carry_a_second_argument() {
        assert!(valid_source_name("Blog_2024").is_ok());
        assert!(valid_source_name("wp-site$1").is_ok());
        // A name that starts with a dash would be read as an option by the dump tool.
        for bad in ["", "a b", "a;b", "--all-databases", "-x", "x`y", &"a".repeat(65)] {
            assert!(valid_source_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_password_is_passed_in_the_environment_not_the_arguments() {
        let at = Endpoint { host: "127.0.0.1".into(), port: 3306, user: "root".into(), password: Some("s3cret".into()) };
        assert!(connection_args(&at).iter().all(|a| !a.contains("s3cret")));
        assert_eq!(envs(&at), vec![("MYSQL_PWD".to_string(), "s3cret".to_string())]);
    }
}
