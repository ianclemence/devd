//! Cron jobs for a site, run by devd while it is open.
//!
//! A job is a five-field cron expression and a command, run in the site's folder with the site's
//! PHP first on PATH, so `php artisan schedule:run` is the PHP the site is served with. Windows has
//! no cron and a laptop sleeps, so devd runs the jobs itself, and a job that was due while devd was
//! closed is skipped rather than run late in a burst.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, Timelike};
use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::store_ops::OpsItem;
use devd_core::{util, DevdError, Result};

/// The kind under which jobs are stored.
pub const KIND: &str = "cron";

/// A parsed cron expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minute: [bool; 60],
    hour: [bool; 24],
    dom: [bool; 32],
    month: [bool; 13],
    dow: [bool; 7],
    dom_any: bool,
    dow_any: bool,
}

fn field(text: &str, lo: u32, hi: u32, names: &[&str], name: &str) -> Result<(Vec<u32>, bool)> {
    let bad = |why: String| DevdError::invalid(format!("the {name} field `{text}` is not valid: {why}"));
    let value = |s: &str| -> Result<u32> {
        if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(s)) {
            return Ok(i as u32 + if name == "month" { 1 } else { 0 });
        }
        s.parse::<u32>().map_err(|_| bad(format!("`{s}` is not a number")))
    };
    let mut out = Vec::new();
    let mut any = false;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| bad(format!("`{s}` is not a step")))?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(bad("a step of 0".to_string()));
        }
        let (a, b) = if range == "*" {
            any |= step == 1;
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let v = value(range)?;
            (v, if step > 1 { hi } else { v })
        };
        if a < lo || b > hi || a > b {
            return Err(bad(format!("it must be between {lo} and {hi}")));
        }
        let mut v = a;
        while v <= b {
            out.push(v);
            v += step;
        }
    }
    Ok((out, any))
}

/// Read a cron expression: five fields, or `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`.
pub fn parse(expr: &str) -> Result<Cron> {
    let text = expr.trim();
    let text = match text {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        other => other,
    };
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(DevdError::invalid(format!(
            "`{expr}` is not a cron expression: it needs five fields (minute hour day-of-month month day-of-week), like `* * * * *` or `0 3 * * 1`"
        )));
    }
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let days = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let mut c = Cron { minute: [false; 60], hour: [false; 24], dom: [false; 32], month: [false; 13], dow: [false; 7], dom_any: false, dow_any: false };
    let (m, _) = field(parts[0], 0, 59, &[], "minute")?;
    m.iter().for_each(|v| c.minute[*v as usize] = true);
    let (h, _) = field(parts[1], 0, 23, &[], "hour")?;
    h.iter().for_each(|v| c.hour[*v as usize] = true);
    let (d, dom_any) = field(parts[2], 1, 31, &[], "day-of-month")?;
    d.iter().for_each(|v| c.dom[*v as usize] = true);
    c.dom_any = dom_any;
    let (mo, _) = field(parts[3], 1, 12, &months, "month")?;
    mo.iter().for_each(|v| c.month[*v as usize] = true);
    let (w, dow_any) = field(parts[4], 0, 7, &days, "day-of-week")?;
    w.iter().for_each(|v| c.dow[(*v % 7) as usize] = true);
    c.dow_any = dow_any;
    Ok(c)
}

impl Cron {
    fn day_matches(&self, d: NaiveDateTime) -> bool {
        let dom = self.dom[d.day() as usize];
        let dow = self.dow[d.weekday().num_days_from_sunday() as usize];
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            // Both restricted: cron runs on either.
            (false, false) => dom || dow,
        }
    }

    /// The first minute strictly after `from` that the expression matches, within the next two years.
    pub fn next_after(&self, from: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut t = from.naive_local().with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        for _ in 0..(2 * 366 * 24 * 60) {
            if !self.month[t.month() as usize] {
                // Jump to the first of next month.
                let (y, m) = if t.month() == 12 { (t.year() + 1, 1) } else { (t.year(), t.month() + 1) };
                t = chrono::NaiveDate::from_ymd_opt(y, m, 1)?.and_hms_opt(0, 0, 0)?;
                continue;
            }
            if !self.day_matches(t) {
                t = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            if !self.hour[t.hour() as usize] {
                t = t.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if self.minute[t.minute() as usize] {
                if let Some(local) = t.and_local_timezone(Local).earliest() {
                    return Some(local);
                }
            }
            t += Duration::minutes(1);
        }
        None
    }
}

/// A job, as the capabilities show it.
#[derive(Debug, Clone, Serialize)]
pub struct Job {
    /// Id.
    pub id: String,
    /// The site.
    pub site: String,
    /// The cron expression.
    pub schedule: String,
    /// The command line.
    pub command: String,
    /// Whether devd runs it.
    pub enabled: bool,
    /// When it is next due (RFC 3339).
    pub next_run_at: Option<String>,
    /// When it last ran.
    pub last_run_at: Option<String>,
    /// Its last exit code.
    pub last_exit: Option<i64>,
    /// How long it last took, in seconds.
    pub last_seconds: Option<f64>,
    /// Where its output goes.
    pub log: String,
}

fn job_of(ctx: &Ctx, item: &OpsItem) -> Job {
    let d = &item.data;
    Job {
        id: item.id.clone(),
        site: item.site.clone(),
        schedule: d["schedule"].as_str().unwrap_or("").to_string(),
        command: d["command"].as_str().unwrap_or("").to_string(),
        enabled: d["enabled"].as_bool().unwrap_or(true),
        next_run_at: d["next_run_at"].as_str().map(str::to_string),
        last_run_at: d["last_run_at"].as_str().map(str::to_string),
        last_exit: d["last_exit"].as_i64(),
        last_seconds: d["last_seconds"].as_f64(),
        log: log_path(ctx, &item.site, &item.id).display().to_string(),
    }
}

fn log_path(ctx: &Ctx, site: &str, id: &str) -> std::path::PathBuf {
    crate::log_dir(ctx).join(format!("cron-{site}-{id}.log"))
}

fn valid_command(command: &str) -> Result<()> {
    let c = command.trim();
    if c.is_empty() || c.len() > 2000 || c.contains(['\0', '\n', '\r']) {
        return Err(DevdError::invalid("the command must be one line of at most 2000 characters"));
    }
    Ok(())
}

fn next_of(schedule: &str) -> Result<Option<String>> {
    Ok(parse(schedule)?.next_after(Local::now()).map(|t| t.to_rfc3339()))
}

/// Add a job.
pub fn add(ctx: &Ctx, site: &str, schedule: &str, command: &str) -> Result<Job> {
    crate::site_named(ctx, site)?;
    valid_command(command)?;
    let next = next_of(schedule)?;
    let id = util::new_id()[..8].to_string();
    ctx.store.ops_put(KIND, &id, site, &json!({ "schedule": schedule.trim(), "command": command.trim(), "enabled": true, "next_run_at": next }))?;
    let item = ctx.store.ops_get(KIND, &id)?.ok_or_else(|| DevdError::internal("the job was not saved"))?;
    Ok(job_of(ctx, &item))
}

/// The jobs, optionally one site's.
pub fn list(ctx: &Ctx, site: Option<&str>) -> Result<Vec<Job>> {
    Ok(ctx.store.ops_list(KIND, site)?.iter().map(|i| job_of(ctx, i)).collect())
}

fn get(ctx: &Ctx, id: &str) -> Result<OpsItem> {
    ctx.store.ops_get(KIND, id)?.ok_or_else(|| DevdError::not_found(format!("no cron job `{id}`; `devd call cron.list` shows them")))
}

/// One job.
pub fn show(ctx: &Ctx, id: &str) -> Result<Job> {
    Ok(job_of(ctx, &get(ctx, id)?))
}

/// The last `lines` lines a job printed.
pub fn log(ctx: &Ctx, id: &str, lines: usize) -> Result<Vec<String>> {
    let item = get(ctx, id)?;
    let text = crate::tail_bytes(&log_path(ctx, &item.site, id), 200_000);
    let mut all: Vec<String> = text.lines().map(str::to_string).collect();
    let lines = lines.clamp(1, 1000);
    if all.len() > lines {
        all.drain(0..all.len() - lines);
    }
    Ok(all)
}

/// Remove a job.
pub fn remove(ctx: &Ctx, id: &str) -> Result<()> {
    get(ctx, id)?;
    ctx.store.ops_delete(KIND, id)?;
    Ok(())
}

/// Turn a job on or off.
pub fn set_enabled(ctx: &Ctx, id: &str, enabled: bool) -> Result<Job> {
    let item = get(ctx, id)?;
    let mut d = item.data.clone();
    d["enabled"] = json!(enabled);
    d["next_run_at"] = match (enabled, next_of(d["schedule"].as_str().unwrap_or(""))?) {
        (true, Some(n)) => json!(n),
        _ => Value::Null,
    };
    ctx.store.ops_put(KIND, id, &item.site, &d)?;
    show(ctx, id)
}

/// Run a job now, wait for it, and record how it went.
pub fn run_now(ctx: &Ctx, id: &str) -> Result<(Job, crate::Ran)> {
    let item = get(ctx, id)?;
    let command = item.data["command"].as_str().unwrap_or("").to_string();
    let (site, root) = crate::site_root(ctx, &item.site)?;
    let log = log_path(ctx, &item.site, id);
    if std::fs::metadata(&log).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        // Keep one old copy and start again, so a job that runs every minute does not fill the disk.
        let _ = std::fs::rename(&log, log.with_extension("log.1"));
    }
    let ran = crate::run_line(ctx, &site, &root, &command, &log);
    let mut d = item.data.clone();
    d["last_run_at"] = json!(util::now());
    d["last_exit"] = json!(ran.exit);
    d["last_seconds"] = json!(ran.seconds);
    if d["enabled"].as_bool().unwrap_or(true) {
        d["next_run_at"] = match next_of(d["schedule"].as_str().unwrap_or("")) {
            Ok(Some(n)) => json!(n),
            _ => Value::Null,
        };
    }
    ctx.store.ops_put(KIND, id, &item.site, &d)?;
    if ran.exit != 0 {
        crate::notify::send(ctx, "cron.failed", &item.site, &format!("Cron job `{command}` on {} exited with {}", item.site, ran.exit), json!({ "job": id, "exit": ran.exit }));
    }
    Ok((show(ctx, id)?, ran))
}

/// How late a due job may be and still run. Later than this, devd was closed (or the machine asleep),
/// and the job waits for its next time.
const STALE: Duration = Duration::minutes(2);

/// Start every job that is due, each on its own thread. Returns how many were started.
pub fn tick(ctx: &Ctx) -> usize {
    let now = Local::now();
    let mut started = 0;
    let Ok(items) = ctx.store.ops_list(KIND, None) else { return 0 };
    for item in items {
        let d = &item.data;
        if !d["enabled"].as_bool().unwrap_or(true) {
            continue;
        }
        let Some(due) = d["next_run_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { continue };
        if due.with_timezone(&Local) > now {
            continue;
        }
        if now.signed_duration_since(due.with_timezone(&Local)) > STALE {
            // Missed while devd was not running: schedule the next time without running now.
            let mut nd = d.clone();
            nd["next_run_at"] = match next_of(d["schedule"].as_str().unwrap_or("")) {
                Ok(Some(n)) => json!(n),
                _ => Value::Null,
            };
            let _ = ctx.store.ops_put(KIND, &item.id, &item.site, &nd);
            continue;
        }
        // Claim this run by moving the next time forward before the thread starts, so a slow job is not started twice.
        let mut claimed = d.clone();
        claimed["next_run_at"] = match next_of(d["schedule"].as_str().unwrap_or("")) {
            Ok(Some(n)) => json!(n),
            _ => Value::Null,
        };
        if ctx.store.ops_put(KIND, &item.id, &item.site, &claimed).is_err() {
            continue;
        }
        let ctx = ctx.clone();
        let id = item.id.clone();
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
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).earliest().unwrap()
    }

    #[test]
    fn every_minute_is_the_next_minute() {
        let c = parse("* * * * *").unwrap();
        assert_eq!(c.next_after(at(2026, 3, 4, 10, 15)).unwrap(), at(2026, 3, 4, 10, 16));
    }

    #[test]
    fn steps_ranges_and_lists_are_read() {
        let c = parse("*/15 9-17 * * 1-5").unwrap();
        // Saturday 2026-03-07 10:00 -> Monday 2026-03-09 09:00
        assert_eq!(c.next_after(at(2026, 3, 7, 10, 0)).unwrap(), at(2026, 3, 9, 9, 0));
        assert_eq!(c.next_after(at(2026, 3, 9, 9, 0)).unwrap(), at(2026, 3, 9, 9, 15));
        let l = parse("0,30 * * * *").unwrap();
        assert_eq!(l.next_after(at(2026, 3, 4, 10, 5)).unwrap(), at(2026, 3, 4, 10, 30));
    }

    #[test]
    fn the_shortcuts_and_names_work() {
        assert_eq!(parse("@daily").unwrap(), parse("0 0 * * *").unwrap());
        let c = parse("30 4 1 jan *").unwrap();
        assert_eq!(c.next_after(at(2026, 3, 4, 0, 0)).unwrap(), at(2027, 1, 1, 4, 30));
        let w = parse("0 8 * * mon").unwrap();
        assert_eq!(w.next_after(at(2026, 3, 4, 0, 0)).unwrap().weekday().num_days_from_sunday(), 1);
    }

    #[test]
    fn day_of_month_and_day_of_week_are_either_when_both_are_set() {
        let c = parse("0 0 13 * fri").unwrap();
        // 2026-03-06 is a Friday, before the 13th.
        assert_eq!(c.next_after(at(2026, 3, 4, 0, 0)).unwrap(), at(2026, 3, 6, 0, 0));
    }

    #[test]
    fn nonsense_is_refused_with_the_field_named() {
        for bad in ["", "* * * *", "61 * * * *", "* 24 * * *", "*/0 * * * *", "a b c d e"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("* * * * *").is_ok());
        assert!(parse("99 * * * *").unwrap_err().to_string().contains("minute"));
    }
}
