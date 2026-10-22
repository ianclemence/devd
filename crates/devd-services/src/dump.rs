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
    let clean = file.with_extension("mariadb.sql");
    let rewrites = sanitize_for_mariadb(file, &clean)?;
    let ran = run(&client, &args, &envs(to), Some(&clean), "mysql");
    let _ = std::fs::remove_file(&clean);
    ran?;
    Ok(json!({
        "database": database,
        "file": file.display().to_string(),
        "rewritten": rewrites.iter().map(|(what, n)| json!({ "what": what, "times": n })).collect::<Vec<_>>(),
    }))
}

/// What differs between MySQL 8 and MariaDB in a dump, and what each becomes.
///
/// MySQL 8's default collations (`utf8mb4_0900_ai_ci` and its siblings) and the
/// `utf8mb3` charset name do not exist in MariaDB, and a dump that names them stops
/// at the first table. They are rewritten to the nearest MariaDB equivalent, and the
/// count of each is reported so nothing is changed silently.
const REWRITES: &[(&str, &str)] = &[
    ("utf8mb4_0900_ai_ci", "utf8mb4_unicode_ci"),
    ("utf8mb4_0900_as_ci", "utf8mb4_unicode_ci"),
    ("utf8mb4_0900_as_cs", "utf8mb4_bin"),
    ("utf8mb4_0900_bin", "utf8mb4_bin"),
    ("utf8mb3_general_ci", "utf8_general_ci"),
    ("utf8mb3_unicode_ci", "utf8_unicode_ci"),
    ("utf8mb3_bin", "utf8_bin"),
    ("utf8mb3", "utf8"),
];

/// Copy `source` to `target` with MySQL 8-only names replaced. Returns each rewrite
/// that happened and how many times. The file is read and written a line at a time,
/// so a large dump is not held in memory.
pub fn sanitize_for_mariadb(source: &Path, target: &Path) -> Result<Vec<(String, usize)>> {
    use std::io::{BufRead, Write};
    let mut counts = vec![0usize; REWRITES.len()];
    let reader = std::io::BufReader::new(std::fs::File::open(source)?);
    let mut writer = std::io::BufWriter::new(std::fs::File::create(target)?);
    for line in reader.split(b'\n') {
        let line = line?;
        match String::from_utf8(line) {
            // Rows are data, not schema: a value that happens to contain a collation name stays as it is.
            Ok(mut text) if !text.starts_with("INSERT ") => {
                for (i, (from, to)) in REWRITES.iter().enumerate() {
                    if text.contains(from) {
                        counts[i] += text.matches(from).count();
                        text = text.replace(from, to);
                    }
                }
                writer.write_all(text.as_bytes())?;
            }
            // Rows and binary data in a dump are passed through untouched.
            Ok(text) => writer.write_all(text.as_bytes())?,
            Err(bytes) => writer.write_all(bytes.as_bytes())?,
        }
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(REWRITES.iter().zip(counts).filter(|(_, n)| *n > 0).map(|((from, to), n)| (format!("{from} -> {to}"), n)).collect())
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
    fn mysql_8_names_become_the_mariadb_equivalents_and_are_counted() {
        let dir = std::env::temp_dir().join(format!("devd-sanitize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dump = dir.join("in.sql");
        let out = dir.join("out.sql");
        std::fs::write(
            &dump,
            "CREATE TABLE a (n varchar(9) COLLATE utf8mb4_0900_ai_ci) DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;\nCREATE TABLE b (n text CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci);\nINSERT INTO a VALUES ('utf8mb4_0900_ai_ci is data here');\n-- plain line\n",
        )
        .unwrap();
        let done = sanitize_for_mariadb(&dump, &out).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(!text.contains("COLLATE utf8mb4_0900") && !text.contains("utf8mb3"), "{text}");
        assert!(text.contains("COLLATE=utf8mb4_unicode_ci") && text.contains("CHARACTER SET utf8 COLLATE utf8_general_ci"));
        assert!(text.contains("VALUES ('utf8mb4_0900_ai_ci is data here')"), "row data is never rewritten: {text}");
        assert!(text.ends_with("-- plain line\n"));
        let by_name: std::collections::HashMap<&str, usize> = done.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        assert_eq!(by_name["utf8mb4_0900_ai_ci -> utf8mb4_unicode_ci"], 2);
        assert_eq!(by_name["utf8mb3_general_ci -> utf8_general_ci"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
