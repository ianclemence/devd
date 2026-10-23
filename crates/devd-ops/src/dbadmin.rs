//! Database users and scheduled backups, for devd's own MariaDB.
//!
//! Users are made with a generated password that is shown once; backups are the same dump `db.dump` writes,
//! taken on a cron schedule while devd is open, kept per database, and pruned to the newest `keep`.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Local};
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, DevdError, Result};
use devd_services::{dump, mariadb};

use crate::cron;

/// The kind under which backup schedules are stored.
pub const BACKUP: &str = "db_backup";

fn port(ctx: &Ctx) -> Result<u16> {
    devd_services::mariadb_service(ctx).map(|(p, _)| p).ok_or_else(|| DevdError::not_found("MariaDB is not installed; `devd service install mariadb` adds it"))
}

/// Run one SQL statement and return what it printed.
fn sql(ctx: &Ctx, statement: &str) -> Result<String> {
    let r = mariadb::query(ctx, port(ctx)?, statement)?;
    let code = r.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let why = r.get("stderr").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        return Err(DevdError::Command { program: "mysql".to_string(), code: code as i32, stderr: why });
    }
    Ok(r.get("stdout").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// A name for a database user or a database: letters, digits and underscores.
pub fn valid_ident(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The hosts a user may connect from. MariaDB listens on loopback only, so `%` is no wider than `localhost`
/// from the network's point of view, but it still says what is meant.
pub const HOSTS: &[&str] = &["localhost", "127.0.0.1", "%"];

/// A password that is safe to put inside single quotes in SQL.
pub fn valid_password(p: &str) -> bool {
    (8..=128).contains(&p.len()) && p.chars().all(|c| c.is_ascii_graphic() && c != '\'' && c != '\\' && c != '"' && c != '`')
}

fn generated_password() -> String {
    let a = util::new_id().replace('-', "");
    let b = util::new_id().replace('-', "");
    format!("{a}{b}")[..24].to_string()
}

/// The users that are not the system's own.
pub fn users(ctx: &Ctx) -> Result<Vec<Value>> {
    let out = sql(ctx, "SELECT user, host FROM mysql.user WHERE user NOT IN ('root','mariadb.sys','mysql','') ORDER BY user, host")?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            Some(json!({ "user": p.next()?.trim(), "host": p.next()?.trim() }))
        })
        .collect())
}

/// Make a user, optionally with every privilege on one database. Returns the password (generated when not given).
pub fn create_user(ctx: &Ctx, name: &str, host: &str, password: Option<&str>, database: Option<&str>) -> Result<String> {
    if !valid_ident(name) {
        return Err(DevdError::invalid("a user name is letters, digits and underscores, up to 32 characters"));
    }
    if !HOSTS.contains(&host) {
        return Err(DevdError::invalid(format!("the host is one of {}", HOSTS.join(", "))));
    }
    let pw = match password {
        Some(p) if valid_password(p) => p.to_string(),
        Some(_) => return Err(DevdError::invalid("a password is 8 to 128 printable characters without quotes, backslashes or backticks")),
        None => generated_password(),
    };
    if let Some(db) = database {
        if !valid_ident(db) || !mariadb::databases(ctx, port(ctx)?)?.iter().any(|d| d == db) {
            return Err(DevdError::not_found(format!("there is no database named `{db}`")));
        }
    }
    sql(ctx, &format!("CREATE USER '{name}'@'{host}' IDENTIFIED BY '{pw}'"))?;
    if let Some(db) = database {
        sql(ctx, &format!("GRANT ALL PRIVILEGES ON `{db}`.* TO '{name}'@'{host}'"))?;
    }
    sql(ctx, "FLUSH PRIVILEGES")?;
    Ok(pw)
}

/// Remove a user.
pub fn drop_user(ctx: &Ctx, name: &str, host: &str) -> Result<()> {
    if !valid_ident(name) || !HOSTS.contains(&host) {
        return Err(DevdError::invalid("name a user made with db.user_create, and its host"));
    }
    sql(ctx, &format!("DROP USER '{name}'@'{host}'"))?;
    sql(ctx, "FLUSH PRIVILEGES")?;
    Ok(())
}

// --------------------------------------------------------------------------------------------- backups

fn folder(ctx: &Ctx, database: &str) -> PathBuf {
    ctx.paths.root().join("backups").join(database)
}

fn file_name_ok(database: &str, name: &str) -> bool {
    // <database>-YYYYMMDD-HHMMSS.sql
    let Some(rest) = name.strip_prefix(&format!("{database}-")).and_then(|r| r.strip_suffix(".sql")) else { return false };
    rest.len() == 15 && rest.chars().enumerate().all(|(i, c)| if i == 8 { c == '-' } else { c.is_ascii_digit() })
}

/// One backup file.
pub fn files(ctx: &Ctx, database: &str) -> Result<Vec<Value>> {
    if !valid_ident(database) {
        return Err(DevdError::invalid("that is not a database name"));
    }
    let mut v: Vec<(String, u64, String)> = std::fs::read_dir(folder(ctx, database))
        .map(|d| {
            d.flatten()
                .filter(|e| file_name_ok(database, &e.file_name().to_string_lossy()))
                .map(|e| {
                    let m = e.metadata().ok();
                    (e.file_name().to_string_lossy().to_string(), m.as_ref().map(|m| m.len()).unwrap_or(0), m.and_then(|m| m.modified().ok()).map(|t| DateTime::<Local>::from(t).to_rfc3339()).unwrap_or_default())
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(v.into_iter().map(|(name, size, at)| json!({ "file": folder(ctx, database).join(&name).display().to_string(), "name": name, "bytes": size, "at": at })).collect())
}

/// Keep the newest `keep` backups of a database and remove the older ones (only files this module named).
fn prune(ctx: &Ctx, database: &str, keep: usize) -> usize {
    let all = files(ctx, database).unwrap_or_default();
    let mut removed = 0;
    for old in all.iter().skip(keep) {
        if let Some(p) = old["file"].as_str() {
            if std::fs::remove_file(p).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

fn next_of(schedule: &str) -> Option<String> {
    cron::parse(schedule).ok().and_then(|c| c.next_after(Local::now())).map(|t| t.to_rfc3339())
}

/// Add a backup schedule for a database.
pub fn add(ctx: &Ctx, database: &str, schedule: &str, keep: usize) -> Result<Value> {
    if !valid_ident(database) || !mariadb::databases(ctx, port(ctx)?)?.iter().any(|d| d == database) {
        return Err(DevdError::not_found(format!("there is no database named `{database}`; `devd call db.list` shows them")));
    }
    cron::parse(schedule)?;
    if keep == 0 || keep > 365 {
        return Err(DevdError::invalid("keep between 1 and 365 backups"));
    }
    let id = util::new_id()[..8].to_string();
    ctx.store.ops_put(BACKUP, &id, "", &json!({ "database": database, "schedule": schedule.trim(), "keep": keep, "enabled": true, "next_run_at": next_of(schedule) }))?;
    show(ctx, &id)
}

fn describe(item: &devd_core::store_ops::OpsItem) -> Value {
    let mut v = item.data.clone();
    v["id"] = json!(item.id);
    v
}

/// One schedule.
pub fn show(ctx: &Ctx, id: &str) -> Result<Value> {
    ctx.store.ops_get(BACKUP, id)?.map(|i| describe(&i)).ok_or_else(|| DevdError::not_found(format!("no backup schedule `{id}`; `devd call backup.list` shows them")))
}

/// The schedules.
pub fn list(ctx: &Ctx) -> Result<Vec<Value>> {
    Ok(ctx.store.ops_list(BACKUP, None)?.iter().map(describe).collect())
}

/// Remove a schedule. The backup files stay.
pub fn remove(ctx: &Ctx, id: &str) -> Result<()> {
    show(ctx, id)?;
    ctx.store.ops_delete(BACKUP, id)?;
    Ok(())
}

/// Take the backup now, prune, and record how it went.
pub fn run_now(ctx: &Ctx, id: &str) -> Result<Value> {
    let item = ctx.store.ops_get(BACKUP, id)?.ok_or_else(|| DevdError::not_found(format!("no backup schedule `{id}`")))?;
    let database = item.data["database"].as_str().unwrap_or("").to_string();
    let keep = item.data["keep"].as_u64().unwrap_or(7) as usize;
    let dir = folder(ctx, &database);
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{database}-{}.sql", Local::now().format("%Y%m%d-%H%M%S")));
    let result = dump::dump(ctx, &dump::Endpoint::devd(port(ctx)?), &database, &file);
    let mut d = item.data.clone();
    d["last_run_at"] = json!(util::now());
    if d["enabled"].as_bool().unwrap_or(true) {
        d["next_run_at"] = json!(next_of(d["schedule"].as_str().unwrap_or("")));
    }
    let out = match result {
        Ok(report) => {
            let pruned = prune(ctx, &database, keep);
            d["last_ok"] = json!(true);
            d["last_error"] = Value::Null;
            d["last_file"] = json!(file.display().to_string());
            json!({ "database": database, "file": file.display().to_string(), "report": report, "pruned": pruned })
        }
        Err(e) => {
            d["last_ok"] = json!(false);
            d["last_error"] = json!(e.to_string());
            let _ = std::fs::remove_file(&file);
            crate::notify::send(ctx, "backup.failed", &database, &format!("The backup of database {database} failed: {e}"), json!({ "schedule": id }));
            ctx.store.ops_put(BACKUP, id, "", &d)?;
            return Err(e);
        }
    };
    ctx.store.ops_put(BACKUP, id, "", &d)?;
    Ok(out)
}

/// A schedule that was due this long ago and was not run was missed (devd was closed): it waits for its next time.
const STALE: Duration = Duration::minutes(5);

/// Start every backup that is due. Returns how many were started.
pub fn tick(ctx: &Ctx) -> usize {
    let now = Local::now();
    let mut started = 0;
    let Ok(items) = ctx.store.ops_list(BACKUP, None) else { return 0 };
    for item in items {
        if !item.data["enabled"].as_bool().unwrap_or(true) {
            continue;
        }
        let Some(due) = item.data["next_run_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Local)) else { continue };
        if due > now {
            continue;
        }
        let mut claimed = item.data.clone();
        claimed["next_run_at"] = json!(next_of(item.data["schedule"].as_str().unwrap_or("")));
        if ctx.store.ops_put(BACKUP, &item.id, "", &claimed).is_err() {
            continue;
        }
        if now.signed_duration_since(due) > STALE {
            continue;
        }
        let (ctx, id) = (ctx.clone(), item.id.clone());
        std::thread::spawn(move || {
            let _ = run_now(&ctx, &id);
        });
        started += 1;
    }
    started
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_hosts_and_passwords_are_checked_before_any_sql_is_written() {
        assert!(valid_ident("shop_app1") && !valid_ident("shop app") && !valid_ident("a;b") && !valid_ident("") && !valid_ident(&"x".repeat(33)));
        assert!(valid_password("S3cure-Pass!") && !valid_password("short") && !valid_password("it's-quoted-1") && !valid_password("back\\slash-1") && !valid_password("with space 1234"));
        assert_eq!(generated_password().len(), 24);
        assert!(valid_password(&generated_password()));
        assert_ne!(generated_password(), generated_password());
    }

    #[test]
    fn only_files_named_by_the_backup_are_ever_listed_or_pruned() {
        assert!(file_name_ok("shop", "shop-20261023-153000.sql"));
        for bad in ["shop-20261023-153000.sql.bak", "shop-2026102-153000.sql", "other-20261023-153000.sql", "shop-latest.sql", "shop-20261023_153000.sql", "../shop-20261023-153000.sql"] {
            assert!(!file_name_ok("shop", bad), "{bad}");
        }
    }
}
