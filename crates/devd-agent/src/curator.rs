//! A curator for skills: usage, pins, locks, archive, and a ledger of every change.
//!
//! A skill library rots in two directions at once: it grows near-duplicates nobody dares delete,
//! and it loses the one skill that mattered because nothing recorded that it mattered. The
//! curator is the small amount of bookkeeping that prevents both:
//!
//! * **Usage.** Every `skills.get` records that the skill was read, so "is this used?" has an
//!   answer rather than a feeling.
//! * **Pin and lock.** A pinned skill is never archived as stale; a locked skill cannot be edited
//!   or deleted at all. Both are a person's decision.
//! * **Archive, not delete.** A stale user skill is moved into the curator's folder with a
//!   snapshot; `skills.restore` puts it back. Nothing is destroyed to tidy up.
//! * **Snapshot before a change.** Any curator change copies the file first, so "what did this
//!   say before" is answerable.
//! * **A ledger.** Every change is one JSON line, so a person can read the history and a test can
//!   assert it.
//! * **Dry run.** `skills.curate` plans; `skills.curate_apply` is the human-only step.
//!
//! Built-in skills are compiled in and cannot be archived: the curator manages the person's own
//! folder only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use devd_core::{DevdError, Paths, Result};

use crate::skills;

/// A skill nobody has read in this many days is stale, unless it is pinned or locked.
pub const STALE_DAYS: i64 = 90;

/// What the curator has been told about a skill by a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    /// Never archive this as stale.
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Never edit or delete this, even by a person's tool, until it is unlocked.
    #[serde(default)]
    pub locked: Vec<String>,
    /// Skills moved out of the library by the curator.
    #[serde(default)]
    pub archived: Vec<String>,
}

impl Flags {
    fn has(list: &[String], name: &str) -> bool {
        list.iter().any(|n| n == name)
    }

    /// Whether this skill is pinned.
    pub fn pinned(&self, name: &str) -> bool {
        Self::has(&self.pinned, name)
    }

    /// Whether this skill is locked.
    pub fn locked(&self, name: &str) -> bool {
        Self::has(&self.locked, name)
    }

    /// Whether this skill is archived.
    pub fn archived(&self, name: &str) -> bool {
        Self::has(&self.archived, name)
    }
}

/// How often a skill has been read and when.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    /// Times read through `skills.get`.
    #[serde(default)]
    pub uses: u64,
    /// The first time it was read.
    #[serde(default)]
    pub first_used: String,
    /// The last time it was read.
    #[serde(default)]
    pub last_used: String,
}

/// One usage record as a screen reads it.
#[derive(Debug, Clone, Serialize)]
pub struct UsageRow {
    /// The skill.
    pub name: String,
    /// `builtin` or `user`.
    pub source: String,
    /// Times read.
    pub uses: u64,
    /// Last read, empty when never.
    pub last_used: String,
    /// Days since it was last read, when it has been.
    pub idle_days: Option<i64>,
    /// Whether a person pinned it.
    pub pinned: bool,
    /// Whether a person locked it.
    pub locked: bool,
    /// Whether the curator archived it.
    pub archived: bool,
}

fn root(paths: &Paths) -> PathBuf {
    skills::user_dir(paths).join(".curator")
}

fn flags_file(paths: &Paths) -> PathBuf {
    root(paths).join("state.json")
}

fn usage_file(paths: &Paths) -> PathBuf {
    root(paths).join("usage.json")
}

fn ledger_file(paths: &Paths) -> PathBuf {
    root(paths).join("ledger.jsonl")
}

fn archive_file(paths: &Paths, name: &str) -> PathBuf {
    root(paths).join("archive").join(format!("{name}.md"))
}

fn snapshot_dir(paths: &Paths, name: &str) -> PathBuf {
    root(paths).join("snapshots").join(name)
}

fn skill_file(paths: &Paths, name: &str) -> PathBuf {
    skills::user_dir(paths).join(format!("{name}.md"))
}

/// The curator's flags for this installation.
pub fn flags(paths: &Paths) -> Flags {
    std::fs::read_to_string(flags_file(paths))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_flags(paths: &Paths, flags: &Flags) -> Result<()> {
    let file = flags_file(paths);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(flags).map_err(|e| DevdError::internal(e.to_string()))?,
    )?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

/// Every skill's use, by name.
pub fn usage(paths: &Paths) -> BTreeMap<String, Use> {
    std::fs::read_to_string(usage_file(paths))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_usage(paths: &Paths, usage: &BTreeMap<String, Use>) -> Result<()> {
    let file = usage_file(paths);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(usage).map_err(|e| DevdError::internal(e.to_string()))?,
    )?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

/// Append one line to the ledger. Best effort: a ledger that fails must not break the work it
/// was recording, but the caller can decide to surface the error.
pub fn note_event(paths: &Paths, name: &str, kind: &str, detail: &str) {
    let file = ledger_file(paths);
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = json!({
        "at": devd_core::util::now(),
        "skill": name,
        "event": kind,
        "detail": detail,
    });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
        let _ = writeln!(f, "{line}");
    }
}

/// Record that a skill was read. Called by `skills.get`.
pub fn note_use(paths: &Paths, name: &str) {
    let mut all = usage(paths);
    let now = devd_core::util::now();
    let entry = all.entry(name.to_string()).or_default();
    entry.uses = entry.uses.saturating_add(1);
    if entry.first_used.is_empty() {
        entry.first_used = now.clone();
    }
    entry.last_used = now;
    let _ = save_usage(paths, &all);
}

/// The number of days since a timestamp, when it can be read.
fn idle_days(created: &str) -> Option<i64> {
    let then = chrono::DateTime::parse_from_rfc3339(created).ok()?;
    Some((chrono::Utc::now() - then.with_timezone(&chrono::Utc)).num_days())
}

/// Every skill with its use and flags, built-ins included but never archiveable.
pub fn usage_rows(paths: &Paths) -> Vec<UsageRow> {
    let flags = flags(paths);
    let all = usage(paths);
    let live = skills::all(paths);
    let mut out: Vec<UsageRow> = live
        .iter()
        .map(|s| {
            let used = all.get(&s.name);
            UsageRow {
                uses: used.map(|u| u.uses).unwrap_or(0),
                last_used: used.map(|u| u.last_used.clone()).unwrap_or_default(),
                idle_days: used.and_then(|u| idle_days(&u.last_used)),
                pinned: flags.pinned(&s.name),
                locked: flags.locked(&s.name),
                archived: flags.archived(&s.name),
                name: s.name.clone(),
                source: s.source.clone(),
            }
        })
        .collect();
    // An archived skill is not in the library, but a person still has to be able to see it and
    // restore it, so it is a row too.
    for name in &flags.archived {
        if out.iter().any(|r| &r.name == name) {
            continue;
        }
        let used = all.get(name);
        out.push(UsageRow {
            uses: used.map(|u| u.uses).unwrap_or(0),
            last_used: used.map(|u| u.last_used.clone()).unwrap_or_default(),
            idle_days: used.and_then(|u| idle_days(&u.last_used)),
            pinned: flags.pinned(name),
            locked: flags.locked(name),
            archived: true,
            name: name.clone(),
            source: "archived".to_string(),
        });
    }
    out
}

/// A skill that may be edited or deleted. Locked and archived skills may not, and the refusal
/// says which one and why.
pub fn ensure_editable(paths: &Paths, name: &str) -> Result<()> {
    let flags = flags(paths);
    if flags.locked(name) {
        return Err(DevdError::denied(format!(
            "the skill `{name}` is locked; unlock it before changing it"
        )));
    }
    if flags.archived(name) {
        return Err(DevdError::conflict(format!(
            "the skill `{name}` is archived; restore it before changing it"
        )));
    }
    Ok(())
}

fn set_flag(paths: &Paths, name: &str, which: &str, on: bool) -> Result<Flags> {
    if !skills::all(paths).iter().any(|s| s.name == name) {
        return Err(DevdError::not_found(format!("no skill `{name}`")));
    }
    let mut flags = flags(paths);
    let list = match which {
        "pinned" => &mut flags.pinned,
        "locked" => &mut flags.locked,
        "archived" => &mut flags.archived,
        _ => return Err(DevdError::internal("unknown curator flag")),
    };
    list.retain(|n| n != name);
    if on {
        list.push(name.to_string());
    }
    save_flags(paths, &flags)?;
    let event = match (which, on) {
        ("pinned", true) => "pinned",
        ("pinned", false) => "unpinned",
        ("locked", true) => "locked",
        ("locked", false) => "unlocked",
        ("archived", true) => "archived",
        _ => "changed",
    };
    note_event(paths, name, event, "by a person");
    Ok(flags)
}

/// Pin a skill so the curator never archives it as stale.
pub fn pin(paths: &Paths, name: &str, on: bool) -> Result<Flags> {
    set_flag(paths, name, "pinned", on)
}

/// Lock a skill so it cannot be edited or deleted.
pub fn lock(paths: &Paths, name: &str, on: bool) -> Result<Flags> {
    set_flag(paths, name, "locked", on)
}

/// Copy a skill's file into the curator's snapshots before a change.
pub fn snapshot(paths: &Paths, name: &str) -> Result<Option<PathBuf>> {
    let file = if skill_file(paths, name).is_file() {
        skill_file(paths, name)
    } else if archive_file(paths, name).is_file() {
        archive_file(paths, name)
    } else {
        return Ok(None);
    };
    let body = std::fs::read_to_string(&file)?;
    let dir = snapshot_dir(paths, name);
    std::fs::create_dir_all(&dir)?;
    let at = devd_core::util::now().replace([':', '.'], "-");
    let out = dir.join(format!("{at}.md"));
    std::fs::write(&out, body)?;
    Ok(Some(out))
}

/// Move a user skill out of the library, keeping a snapshot. Built-ins cannot be archived.
pub fn archive(paths: &Paths, name: &str) -> Result<Flags> {
    if skills::builtin_description(name).is_some() {
        return Err(DevdError::conflict(format!(
            "`{name}` is built in; a built-in skill is compiled in and cannot be archived"
        )));
    }
    let flags = flags(paths);
    if flags.locked(name) {
        return Err(DevdError::denied(format!(
            "the skill `{name}` is locked; unlock it before archiving it"
        )));
    }
    if flags.pinned(name) {
        return Err(DevdError::conflict(format!(
            "the skill `{name}` is pinned; unpin it before archiving it"
        )));
    }
    if flags.archived(name) {
        return Ok(flags);
    }
    let file = skill_file(paths, name);
    if !file.is_file() {
        return Err(DevdError::not_found(format!("no skill `{name}`")));
    }
    snapshot(paths, name)?;
    let out = archive_file(paths, name);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&file, &out)?;
    // A skill's reference pages travel with it.
    let pages = skills::resource_dir(paths, name);
    if pages.is_dir() {
        let _ = std::fs::rename(&pages, root(paths).join("archive").join(name));
    }
    let mut flags = flags;
    if !flags.archived(name) {
        flags.archived.push(name.to_string());
    }
    save_flags(paths, &flags)?;
    note_event(paths, name, "archived", "moved out of the library; the file is kept");
    Ok(flags)
}

/// Put an archived skill back exactly where it was.
pub fn restore(paths: &Paths, name: &str) -> Result<Flags> {
    let mut flags = flags(paths);
    if !flags.archived(name) {
        return Err(DevdError::conflict(format!("the skill `{name}` is not archived")));
    }
    let file = archive_file(paths, name);
    let out = skill_file(paths, name);
    if !file.is_file() {
        return Err(DevdError::not_found(format!(
            "the archived copy of `{name}` is missing ({})",
            file.display()
        )));
    }
    if out.is_file() {
        return Err(DevdError::conflict(format!(
            "a skill named `{name}` already exists; remove or rename it before restoring the archived one"
        )));
    }
    snapshot(paths, name)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&file, &out)?;
    let pages = root(paths).join("archive").join(name);
    if pages.is_dir() {
        let _ = std::fs::rename(pages, skills::resource_dir(paths, name));
    }
    flags.archived.retain(|n| n != name);
    save_flags(paths, &flags)?;
    note_event(paths, name, "restored", "back in the library");
    Ok(flags)
}

/// One skill the curator would do something about.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// The skill.
    pub name: String,
    /// `stale`, `duplicate` or both.
    pub reasons: Vec<String>,
    /// Days since it was last read; `None` when it never was.
    pub idle_days: Option<i64>,
    /// The skill it looks like, for a duplicate.
    pub looks_like: Option<String>,
    /// Why it will not be touched automatically.
    pub kept_because: Vec<String>,
}

fn words(text: &str) -> std::collections::BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_string)
        .collect()
}

fn similarity(a: &str, b: &str) -> f64 {
    let a = words(a);
    let b = words(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(&b).count() as f64;
    let all = a.union(&b).count() as f64;
    shared / all
}

/// Plan what the curator would do. Writes nothing.
pub fn curate(paths: &Paths, stale_days: i64) -> Vec<Finding> {
    let flags = flags(paths);
    let all = usage(paths);
    let owned: Vec<skills::Skill> = skills::all(paths)
        .into_iter()
        .filter(|s| s.source != "builtin")
        .collect();
    let mut out = Vec::new();
    for s in &owned {
        let used = all.get(&s.name);
        let idle = used.and_then(|u| idle_days(&u.last_used));
        let mut reasons = Vec::new();
        let mut kept = Vec::new();
        if used.is_none() || idle.is_some_and(|d| d >= stale_days) {
            reasons.push("stale".to_string());
        }
        let mut looks_like = None;
        for other in &owned {
            if other.name == s.name {
                continue;
            }
            if similarity(&s.body, &other.body) >= 0.8
                || (!s.description.is_empty()
                    && s.description.eq_ignore_ascii_case(&other.description))
            {
                looks_like = Some(other.name.clone());
                reasons.push("duplicate".to_string());
                break;
            }
        }
        if reasons.is_empty() {
            continue;
        }
        if skills::builtin_description(&s.name).is_some() {
            kept.push("built in, never archived".to_string());
        }
        if flags.pinned(&s.name) {
            kept.push("pinned".to_string());
        }
        if flags.locked(&s.name) {
            kept.push("locked".to_string());
        }
        out.push(Finding {
            name: s.name.clone(),
            reasons,
            idle_days: idle,
            looks_like,
            kept_because: kept,
        });
    }
    out
}

/// Do what [`curate`] planned for stale skills. Snapshots first, ledger lines after, and only
/// the skills nobody pinned or locked.
pub fn curate_apply(paths: &Paths, stale_days: i64) -> Result<Vec<String>> {
    let actions = curate(paths, stale_days);
    let mut archived = Vec::new();
    for finding in actions {
        if !finding.reasons.iter().any(|r| r == "stale") || !finding.kept_because.is_empty() {
            continue;
        }
        archive(paths, &finding.name)?;
        archived.push(finding.name);
    }
    Ok(archived)
}

/// The ledger, newest last, at most `limit` lines.
pub fn ledger(paths: &Paths, limit: usize) -> Vec<Value> {
    let text = std::fs::read_to_string(ledger_file(paths)).unwrap_or_default();
    let mut rows: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if rows.len() > limit {
        rows = rows.split_off(rows.len() - limit);
    }
    rows
}

/// The archive folder, for diagnostics.
pub fn archive_dir(paths: &Paths) -> PathBuf {
    root(paths).join("archive")
}

/// A summary for the team screen: counts a person can act on.
pub fn summary(paths: &Paths) -> Value {
    let flags = flags(paths);
    let rows = usage_rows(paths);
    let user = rows.iter().filter(|r| r.source != "builtin").count();
    let stale = curate(paths, STALE_DAYS)
        .into_iter()
        .filter(|f| f.reasons.iter().any(|r| r == "stale") && f.kept_because.is_empty())
        .count();
    json!({
        "skills": rows.len(),
        "yours": user,
        "pinned": flags.pinned.len(),
        "locked": flags.locked.len(),
        "archived": flags.archived.len(),
        "stale": stale,
        "unused": rows.iter().filter(|r| r.uses == 0 && r.source != "builtin").count(),
    })
}

/// The file a skill is in, so a test can assert it did not move.
pub fn path_of(paths: &Paths, name: &str) -> PathBuf {
    skill_file(paths, name)
}

/// Whether a path is the curator's own folder (used by tests and diagnostics).
pub fn is_curator_dir(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == ".curator")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        (d, p)
    }

    fn add(paths: &Paths, name: &str, body: &str) {
        skills::save(paths, name, "a skill for the test", "build", &[], body).unwrap();
    }

    #[test]
    fn reading_a_skill_is_counted_once_per_read_and_the_last_one_is_kept() {
        let (_d, p) = temp();
        add(&p, "one", "A body that is long enough to be saved and read again.");
        note_use(&p, "one");
        note_use(&p, "one");
        let rows = usage_rows(&p);
        let row = rows.iter().find(|r| r.name == "one").unwrap();
        assert_eq!(row.uses, 2);
        assert!(row.idle_days.is_some());
        assert_eq!(ledger(&p, 10).len(), 0, "reads are usage, not curator changes");
    }

    #[test]
    fn a_pinned_skill_is_not_archived_and_a_locked_one_cannot_be_edited() {
        let (_d, p) = temp();
        add(&p, "mine", "A body that is long enough to be saved and read again.");
        pin(&p, "mine", true).unwrap();
        let err = archive(&p, "mine").unwrap_err();
        assert!(err.to_string().contains("pinned"), "{err}");
        pin(&p, "mine", false).unwrap();

        lock(&p, "mine", true).unwrap();
        assert!(ensure_editable(&p, "mine").is_err());
        assert!(archive(&p, "mine").is_err());
        lock(&p, "mine", false).unwrap();
        assert!(ensure_editable(&p, "mine").is_ok());
    }

    #[test]
    fn archiving_keeps_the_file_and_a_snapshot_and_restoring_brings_it_back() {
        let (_d, p) = temp();
        add(&p, "mine", "A body that is long enough to be saved and read again.");
        let before = std::fs::read_to_string(path_of(&p, "mine")).unwrap();
        archive(&p, "mine").unwrap();
        assert!(skills::get(&p, "mine").is_err(), "archived skills are not in the library");
        assert!(!path_of(&p, "mine").is_dir() && !path_of(&p, "mine").is_file());
        assert!(archive_file(&p, "mine").is_file(), "the file is kept");
        assert_eq!(
            std::fs::read_to_string(archive_file(&p, "mine")).unwrap(),
            before
        );
        assert!(!snapshot_dir(&p, "mine").read_dir().unwrap().next().is_none());
        assert!(ledger(&p, 10).iter().any(|e| e["event"] == "archived"));

        restore(&p, "mine").unwrap();
        assert!(skills::get(&p, "mine").is_ok());
        assert_eq!(std::fs::read_to_string(path_of(&p, "mine")).unwrap(), before);
        assert!(ledger(&p, 10).iter().any(|e| e["event"] == "restored"));
    }

    #[test]
    fn a_builtin_is_never_archived_and_a_stale_skill_is_planned_before_it_is_moved() {
        let (_d, p) = temp();
        assert!(archive(&p, "work").is_err(), "a built-in cannot be archived");
        add(&p, "old", "A body about a workflow nobody uses any more, saved long ago.");
        // Never read: it is stale. Planning changes nothing.
        let plan = curate(&p, STALE_DAYS);
        assert!(plan.iter().any(|f| f.name == "old"));
        assert!(skills::get(&p, "old").is_ok(), "the plan does not move anything");
        // Pinned stale skills stay, and the plan says why.
        pin(&p, "old", true).unwrap();
        let plan = curate(&p, STALE_DAYS);
        let row = plan.iter().find(|f| f.name == "old").unwrap();
        assert!(row.kept_because.iter().any(|k| k == "pinned"), "{row:?}");
        assert_eq!(curate_apply(&p, STALE_DAYS).unwrap(), Vec::<String>::new());
        pin(&p, "old", false).unwrap();
        assert_eq!(curate_apply(&p, STALE_DAYS).unwrap(), vec!["old".to_string()]);
        assert!(skills::get(&p, "old").is_err());
    }

    #[test]
    fn two_skills_that_say_the_same_thing_are_reported_as_duplicates() {
        let (_d, p) = temp();
        add(&p, "one", "Always run the migration before the suite, and never skip the linter.");
        add(&p, "two", "Always run the migration before the suite, and never skip the linter.");
        let plan = curate(&p, STALE_DAYS);
        let two = plan.iter().find(|f| f.name == "two").unwrap();
        assert!(two.reasons.iter().any(|r| r == "duplicate") || plan.iter().find(|f| f.name == "one").is_some_and(|f| f.reasons.iter().any(|r| r == "duplicate")));
    }
}
