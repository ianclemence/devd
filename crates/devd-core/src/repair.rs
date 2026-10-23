//! Rebuild a damaged state database from whatever can still be read.
//!
//! `devd doctor` reports a database SQLite itself calls corrupt. The way out is not to delete
//! it: a new database is built with the current schema, every table is copied across, and
//! what could not be read is counted and reported. The damaged files are kept, whole, in a
//! `corrupt-state-<time>` folder beside the new one.

use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{params, Connection};

use crate::error::{DevdError, Result};
use crate::paths::Paths;
use crate::state::Store;

/// What happened to one table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TableReport {
    /// The table.
    pub name: String,
    /// Rows now in the new database.
    pub kept: i64,
    /// Rows that could not be read from the damaged one.
    pub lost: i64,
}

/// The outcome of a repair.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RepairReport {
    /// Where the damaged files were put.
    pub backup: PathBuf,
    /// Every table, with how many rows survived.
    pub tables: Vec<TableReport>,
}

fn sidecars(db: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|s| PathBuf::from(format!("{}{s}", db.display())))
        .collect()
}


/// The names of a table's columns.
fn columns(c: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = c.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let rows = st.query_map([], |r| r.get::<_, String>(1))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Copy one table across. The damaged database is read through its own connection, so a page that
/// cannot be read never disturbs the one being written: first the whole table in one pass, and
/// where that stops at damage, row by row, counting the rows that cannot be read.
fn copy_table(new: &Connection, old: &Connection, table: &str) -> Result<TableReport> {
    let common: Vec<String> = columns(new, table)?.into_iter().filter(|n| columns(old, table).unwrap_or_default().contains(n)).collect();
    if common.is_empty() {
        return Ok(TableReport { name: table.into(), kept: 0, lost: 0 });
    }
    let list = common.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(",");
    let marks = vec!["?"; common.len()].join(",");
    let insert = format!("INSERT OR IGNORE INTO \"{table}\" ({list}) VALUES ({marks})");
    let width = common.len();
    let read_row = |r: &rusqlite::Row| -> rusqlite::Result<Vec<Value>> { (0..width).map(|k| r.get::<_, Value>(k + 1)).collect() };

    let mut rows: Vec<(i64, Vec<Value>)> = Vec::new();
    let mut stopped = false;
    match old.prepare(&format!("SELECT rowid,{list} FROM \"{table}\"")) {
        Ok(mut st) => match st.query([]) {
            Ok(mut it) => loop {
                match it.next() {
                    Ok(Some(r)) => match (r.get::<_, i64>(0), read_row(r)) {
                        (Ok(id), Ok(v)) => rows.push((id, v)),
                        _ => stopped = true,
                    },
                    Ok(None) => break,
                    Err(_) => {
                        stopped = true;
                        break;
                    }
                }
            },
            Err(_) => stopped = true,
        },
        Err(_) => stopped = true,
    }
    let mut lost = 0i64;
    if stopped {
        // The pass stopped at damage. Probe the row ids one at a time, a little beyond the last one seen.
        let have: std::collections::HashSet<i64> = rows.iter().map(|(id, _)| *id).collect();
        let bound = rows.iter().map(|(id, _)| *id).max().unwrap_or(0) + 3000;
        let one = format!("SELECT rowid,{list} FROM \"{table}\" WHERE rowid=?1");
        for id in 1..=bound {
            if have.contains(&id) {
                continue;
            }
            match old.query_row(&one, params![id], |r| Ok((r.get::<_, i64>(0)?, read_row(r)?))) {
                Ok((rid, v)) => rows.push((rid, v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(_) => lost += 1,
            }
        }
    }
    let tx = new.unchecked_transaction()?;
    for (_, values) in &rows {
        tx.execute(&insert, rusqlite::params_from_iter(values.iter()))?;
    }
    tx.commit()?;
    let kept: i64 = new.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| r.get(0))?;
    Ok(TableReport { name: table.into(), kept, lost })
}

/// Rebuild the state database. No devd process may have it open: the damaged files are moved, and a
/// process holding them makes the move fail, which stops the repair before anything is changed.
pub fn repair_state(paths: &Paths) -> Result<RepairReport> {
    let db = paths.state_db();
    if !db.is_file() {
        return Err(DevdError::not_found(format!("there is no database at {}", db.display())));
    }
    let work = std::env::temp_dir().join(format!("devd-repair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = rebuild(&db, &work);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn rebuild(db: &Path, work: &Path) -> Result<RepairReport> {
    // A working copy of the damaged files, so reading them never changes them.
    let copy = work.join("damaged.db");
    for (from, to) in sidecars(db).into_iter().zip(sidecars(&copy)) {
        if from.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    let fresh = db.with_file_name("state.db.repairing");
    for f in sidecars(&fresh) {
        let _ = std::fs::remove_file(f);
    }
    let tables = {
        let store = Store::open(&fresh)?;
        let old = Connection::open(&copy).map_err(|e| DevdError::internal(format!("opening the damaged database: {e}")))?;
        store.with(|new| {
            let names: Vec<String> = {
                let mut st = new.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            let mut out = Vec::new();
            for name in names {
                let exists_old: i64 = old.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", params![name], |r| r.get(0)).unwrap_or(0);
                if exists_old == 0 {
                    out.push(TableReport { name, kept: 0, lost: 0 });
                    continue;
                }
                out.push(copy_table(new, &old, &name).map_err(|e| DevdError::internal(format!("copying {name}: {e}")))?);
            }
            Ok(out)
        })?
    };
    // The connection that read the damaged copy is closed; the new database is checked on its own,
    // and folded into one file.
    let checked = Store::open(&fresh)?;
    let verdict = checked.integrity_check()?;
    if verdict != "ok" {
        return Err(DevdError::internal(format!("the rebuilt database did not pass its own check: {verdict}")));
    }
    checked.with(|c| {
        c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    })?;
    drop(checked);

    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = db.with_file_name(format!("corrupt-state-{stamp}"));
    std::fs::create_dir_all(&backup)?;
    for from in sidecars(db) {
        if from.is_file() {
            let to = backup.join(from.file_name().unwrap_or_default());
            std::fs::rename(&from, &to).map_err(|e| {
                DevdError::conflict(format!("could not move {} aside ({e}); close devd and try again", from.display()))
            })?;
        }
    }
    std::fs::rename(&fresh, db)?;
    for f in sidecars(&fresh) {
        let _ = std::fs::remove_file(f);
    }
    Ok(RepairReport { backup, tables })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_database_is_rebuilt_from_what_can_be_read() {
        let dir = std::env::temp_dir().join(format!("devd-repair-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        {
            let s = Store::open(&paths.state_db()).unwrap();
            s.create_thread("keep me", None, None, None).unwrap();
            for i in 0..3000 {
                s.with(|c| {
                    c.execute(
                        "INSERT INTO audit_log (at,actor,surface,capability,tier,effect,dry_run,ok,args,message) VALUES ('2026-01-01','human:x','cli',?1,'read','read',0,1,'{}','ok')",
                        params![format!("cap.{i}")],
                    )?;
                    Ok(())
                })
                .unwrap();
            }
            s.with(|c| {
                c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
                Ok(())
            })
            .unwrap();
        }
        // Damage the last part of the file, where the audit rows live (the schema is near the start).
        let bytes = std::fs::read(paths.state_db()).unwrap();
        let mut damaged = bytes.clone();
        let mid = damaged.len() * 3 / 4;
        for b in &mut damaged[mid..mid + 6000] {
            *b = 0xA5;
        }
        std::fs::write(paths.state_db(), &damaged).unwrap();

        let report = repair_state(&paths).unwrap();
        let s = Store::open(&paths.state_db()).unwrap();
        assert_eq!(s.integrity_check().unwrap(), "ok");
        assert_eq!(s.list_threads(10).unwrap().len(), 1, "the small tables came through whole: {:?}", report.tables);
        assert!(report.backup.join("state.db").is_file(), "the damaged file is kept");
        let audit = report.tables.iter().find(|t| t.name == "audit_log").unwrap();
        assert!(audit.kept > 0 && audit.kept < 3000, "some audit rows survive and some do not: {audit:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
