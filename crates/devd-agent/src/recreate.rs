//! Recreating a program or a site from what a user can see.
//!
//! A recreation is a *study*, a *build* and a *check*, and the check is a measurement, never a
//! verdict. This module keeps the record that makes that distinction possible: what the target
//! was, which mode the person allowed, what was captured at which width, and what the comparison
//! found. The record lives beside the run (`<devd home>/recreate/<run>.json`) so a person can
//! read it, and every capture and comparison is also written into the run's evidence, where a
//! reviewer sees it next to everything else the run did.
//!
//! Two rules are enforced here rather than hoped for:
//!
//! * **Clean room is the default.** `begin` accepts only `clean_room`; switching a project to
//!   `deep` is a separate, human-only capability, so reading a target's code is never a side
//!   effect of starting a recreation.
//! * **Similarity is not verification.** A comparison is recorded as `read` evidence with
//!   `passed: false`. The completion gate in `devd_core::completion` only accepts `verify`
//!   evidence, so a rebuild that is 98% similar and untested cannot be called done.

use std::path::{Path, PathBuf};

use devd_core::runs::RunMode;
use devd_core::{NewEvidence, RunSpec};
use devd_core::{DevdError, Paths, Result, Store, util};
use serde::{Deserialize, Serialize};

/// The three widths every screen is captured at. One width is not the layout.
pub const WIDTHS: &[(&str, u32)] = &[("phone", 390), ("tablet", 834), ("desktop", 1440)];

/// The modes a recreation may run in.
pub const MODES: &[&str] = &["clean_room", "deep"];

/// One picture of one screen at one width.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    /// The screen's name, as the person would say it ("home", "search results").
    pub screen: String,
    /// `phone`, `tablet`, `desktop`, or a pixel width as text.
    pub width: String,
    /// Where the picture is.
    pub path: String,
    /// What it shows, in a sentence.
    pub note: String,
    /// When it was taken.
    pub at: String,
}

/// One comparison of the rebuild against the original.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// The screen compared.
    pub screen: String,
    /// The width compared.
    pub width: String,
    /// How alike the screen looked, 0 to 100. A measurement, not a pass.
    pub similarity: Option<f64>,
    /// What does not match yet, most noticeable first.
    pub gaps: Vec<String>,
    /// A report file, when there is one.
    pub report: Option<String>,
    /// When it was compared.
    pub at: String,
}

/// A recreation's record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The run this belongs to.
    pub run: String,
    /// What is being recreated: an address, or a program's name.
    pub target: String,
    /// `clean_room` or `deep`.
    pub mode: String,
    /// The person's statement that they may recreate this. Their words, not ours.
    pub right: String,
    /// Where the rebuild lives, once there is one.
    pub app: Option<String>,
    /// The unified run evidence hangs off, when the record is keyed by a team run whose id is
    /// different. Absent for a recreation that started its own run.
    #[serde(default)]
    pub evidence_run: Option<String>,
    /// When it started.
    pub created_at: String,
    /// Every picture taken.
    pub captures: Vec<Capture>,
    /// Every comparison made.
    pub checks: Vec<Check>,
}

impl Record {
    /// Whether this recreation may only use what a visitor can see.
    pub fn is_clean_room(&self) -> bool {
        self.mode == "clean_room"
    }

    /// The run that evidence rows hang off: the team's own run, or this record's run.
    pub fn evidence_id(&self) -> &str {
        self.evidence_run.as_deref().unwrap_or(&self.run)
    }

    /// The screens that have been captured, in the order they were first seen.
    pub fn screens(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in &self.captures {
            if !out.contains(&c.screen) {
                out.push(c.screen.clone());
            }
        }
        out
    }

    /// The screens captured at all three widths, and the ones that are not.
    pub fn incomplete(&self) -> Vec<String> {
        self.screens()
            .into_iter()
            .filter(|screen| {
                let widths: Vec<&str> = self
                    .captures
                    .iter()
                    .filter(|c| &c.screen == screen)
                    .map(|c| c.width.as_str())
                    .collect();
                WIDTHS.iter().any(|(name, _)| !widths.contains(name))
            })
            .collect()
    }

    /// Every gap named by every comparison, in the order they were found.
    pub fn gaps(&self) -> Vec<String> {
        self.checks.iter().flat_map(|c| c.gaps.clone()).collect()
    }
}

fn invalid(why: impl Into<String>) -> DevdError {
    DevdError::invalid(why.into())
}

/// Where recreation records live.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.root().join("recreate")
}

fn record_file(paths: &Paths, run: &str) -> PathBuf {
    dir(paths).join(format!("{run}.json"))
}

/// The recreation for a run, when it is one.
///
/// A team run that is also a recreation keeps the same run id, so the gate can ask this without
/// knowing which kind of run it is looking at.
pub fn find(paths: &Paths, run: &str) -> Option<Record> {
    std::fs::read_to_string(record_file(paths, run))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// Read one recreation.
pub fn load(paths: &Paths, run: &str) -> Result<Record> {
    let file = record_file(paths, run);
    let text = std::fs::read_to_string(&file).map_err(|e| {
        DevdError::not_found(format!("there is no recreation recorded for run {run}: {e}"))
    })?;
    serde_json::from_str(&text).map_err(|e| DevdError::internal(format!("{} is not a recreation: {e}", file.display())))
}

/// Write one recreation. Written beside the old one and moved, so a crash cannot leave half.
fn save(paths: &Paths, record: &Record) -> Result<()> {
    let file = record_file(paths, &record.run);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(record).map_err(|e| DevdError::internal(e.to_string()))?)?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

/// Every recreation, newest first.
pub fn list(paths: &Paths) -> Vec<Record> {
    let mut out: Vec<Record> = std::fs::read_dir(dir(paths))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str::<Record>(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Every recreation, as the list screen reads it: the record's own fields plus the two values
/// that need evidence to answer (`proven` and `ready`).
pub fn summaries(paths: &Paths, store: &Store) -> Vec<serde_json::Value> {
    list(paths)
        .into_iter()
        .map(|r| {
            let proven = store
                .list_evidence(r.evidence_id(), 500)
                .map(|all| all.iter().any(|e| e.kind == "verify" && e.passed))
                .unwrap_or(false);
            let missing = r.incomplete();
            serde_json::json!({
                "run": r.run,
                "target": r.target,
                "mode": r.mode,
                "right": r.right,
                "app": r.app,
                "created_at": r.created_at,
                "captures": r.captures.len(),
                "checks": r.checks.len(),
                "screens": r.screens(),
                "screens_missing_a_width": missing,
                "gaps": r.gaps(),
                "proven": proven,
                "ready": proven && missing.is_empty() && !r.checks.is_empty(),
                "clean_room": r.is_clean_room(),
            })
        })
        .collect()
}

/// Read a width as a name or a pixel count, and answer the name and the pixel count.
///
/// A pixel count that is one of the three canonical widths is stored under the name a person
/// uses, so "390" and "phone" are one capture rather than two, and the three-width check cannot
/// be passed twice at the same size.
fn width_name(width: &str) -> Option<(String, u32)> {
    let trimmed = width.trim();
    if let Some((name, px)) = WIDTHS.iter().find(|(name, _)| *name == trimmed) {
        return Some(((*name).to_string(), *px));
    }
    let px = trimmed.parse::<u32>().ok().filter(|n| (240..=7680).contains(n))?;
    let named = WIDTHS.iter().find(|(_, w)| *w == px).map(|(name, _)| (*name).to_string());
    Some((named.unwrap_or_else(|| px.to_string()), px))
}

/// The pixel count a stored width means, for a report.
pub fn width_px(width: &str) -> Option<u32> {
    width_name(width).map(|(_, px)| px)
}

/// Start a recreation: the target, the mode and the person's declaration.
///
/// Only `clean_room` starts here. Reading a target's code is a separate decision a person makes
/// with `recreate.deep`, so beginning a recreation can never quietly read what it should not.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    paths: &Paths,
    store: &Store,
    target: &str,
    mode: &str,
    right: &str,
    app: Option<&str>,
    by: &str,
    attach: Option<&str>,
) -> Result<Record> {
    let target = target.trim();
    if target.is_empty() {
        return Err(invalid("name the target: the address of the site, or the program, that is being recreated"));
    }
    if right.trim().len() < 10 {
        return Err(invalid(
            "record the person's statement that they may recreate this: `right` is their words, for example \"my own copy of an app I pay for\"",
        ));
    }
    let mode = mode.trim();
    if !MODES.contains(&mode) {
        return Err(invalid(format!("`mode` is one of {}; clean room is the default", MODES.join(", "))));
    }
    if mode != "clean_room" {
        return Err(DevdError::denied(
            "a recreation starts in clean room; switching it to deep is its own decision, made by a person with recreate.deep",
        ));
    }
    // A recreation is its own run: captures and comparisons are evidence, and evidence hangs
    // off a run. When the person already started a team run, the recreation attaches to that
    // run instead, so the team's completion is judged against this record rather than beside it.
    let (run_id, evidence_run) = match attach.map(str::trim).filter(|r| !r.is_empty()) {
        Some(run) => {
            let team = store
                .get_run(run)?
                .ok_or_else(|| DevdError::not_found(format!("there is no run `{run}` to attach the recreation to")))?;
            // A team run keeps its own id; its evidence lives on the unified run it mirrors to.
            let unified = store.ensure_run_for_team(
                &team.id,
                &team.goal,
                team.project.as_deref(),
                by,
                &[],
            )?;
            (team.id, Some(unified.id))
        }
        None => {
            let run = store.start_run(RunSpec {
                goal: &format!("Recreate {target}: study what a user sees, build an independent version, and check it against the original."),
                mode: RunMode::Api,
                project: app.and_then(|a| Path::new(a).parent()).map(|p| p.display().to_string()).as_deref(),
                actor: by,
                thread_id: None,
                team_run_id: None,
                acceptance: &[],
            })?;
            (run.id, None)
        }
    };
    let record = Record {
        run: run_id,
        evidence_run,
        target: target.to_string(),
        mode: mode.to_string(),
        right: right.trim().to_string(),
        app: app.map(str::to_string),
        created_at: util::now(),
        captures: Vec::new(),
        checks: Vec::new(),
    };
    save(paths, &record)?;
    store.add_evidence(NewEvidence {
        run_id: record.evidence_id(),
        step_id: None,
        tool_call_id: None,
        artifact_id: None,
        kind: "read",
        capability: "recreate.begin",
        passed: true,
        claim: Some("the recreation has a target, a mode and the person's right to recreate it"),
        summary: &format!("recreation of {target} started in {} mode", mode.replace('_', " ")),
        detail: Some(&serde_json::json!({ "target": target, "mode": mode, "right": right.trim() }).to_string()),
    })?;
    Ok(record)
}

/// Allow looking inside the target. Human only: it is the difference between recreating what a
/// user sees and reading somebody's code.
pub fn set_mode(paths: &Paths, store: &Store, run: &str, mode: &str, reason: &str) -> Result<Record> {
    let mut record = load(paths, run)?;
    let mode = mode.trim();
    if !MODES.contains(&mode) {
        return Err(invalid(format!("`mode` is one of {}", MODES.join(", "))));
    }
    if reason.trim().len() < 10 {
        return Err(invalid("say why the target's code has to be read: `reason` is recorded beside the mode"));
    }
    record.mode = mode.to_string();
    save(paths, &record)?;
    store.add_evidence(NewEvidence {
        run_id: record.evidence_id(),
        step_id: None,
        tool_call_id: None,
        artifact_id: None,
        kind: "read",
        capability: "recreate.deep",
        passed: true,
        claim: Some("a person allowed reading inside the target"),
        summary: &format!("recreation switched to {} mode: {reason}", mode.replace('_', " ")),
        detail: Some(&serde_json::json!({ "mode": mode, "reason": reason.trim() }).to_string()),
    })?;
    Ok(record)
}

/// Record a picture of one screen at one width.
pub fn capture(paths: &Paths, store: &Store, run: &str, screen: &str, width: &str, file: &str, note: &str) -> Result<Record> {
    let mut record = load(paths, run)?;
    let screen = screen.trim();
    if screen.is_empty() {
        return Err(invalid("name the screen the picture shows, for example \"home\""));
    }
    let Some((canonical, px)) = width_name(width) else {
        let known = WIDTHS.iter().map(|(n, w)| format!("{n} ({w}px)")).collect::<Vec<_>>().join(", ");
        return Err(invalid(format!("`width` is one of {known}, or a pixel count between 240 and 7680")));
    };
    let path = Path::new(file.trim());
    if !path.is_file() {
        return Err(DevdError::not_found(format!("there is no picture at `{}`: take it first, then record it", path.display())));
    }
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if bytes == 0 {
        return Err(invalid(format!("`{}` is empty; a picture with nothing in it is not a capture", path.display())));
    }
    let capture = Capture {
        screen: screen.to_string(),
        width: canonical,
        path: path.display().to_string(),
        note: note.trim().to_string(),
        at: util::now(),
    };
    record.captures.retain(|c| !(c.screen == capture.screen && c.width == capture.width));
    record.captures.push(capture.clone());
    save(paths, &record)?;
    store.add_evidence(NewEvidence {
        run_id: record.evidence_id(),
        step_id: None,
        tool_call_id: None,
        artifact_id: None,
        kind: "read",
        capability: "recreate.capture",
        passed: true,
        claim: Some("a screen was seen at a width"),
        summary: &format!("captured {} at {} ({px}px)", capture.screen, capture.width),
        detail: Some(&serde_json::json!({ "screen": capture.screen, "width": capture.width, "path": capture.path, "note": capture.note, "bytes": bytes }).to_string()),
    })?;
    Ok(record)
}

/// Record a comparison of the rebuild against the original.
///
/// The result is stored as an observation. It cannot complete a run on its own: the completion
/// gate accepts `verify` evidence, and this is not it.
pub fn check(paths: &Paths, store: &Store, run: &str, asked: Check) -> Result<Record> {
    let mut record = load(paths, run)?;
    let screen = asked.screen.trim();
    if screen.is_empty() {
        return Err(invalid("name the screen that was compared"));
    }
    let Some((canonical_width, _)) = width_name(&asked.width) else {
        return Err(invalid("`width` is one of the three widths, or a pixel count between 240 and 7680"));
    };
    if let Some(s) = asked.similarity {
        if !(0.0..=100.0).contains(&s) || s.is_nan() {
            return Err(invalid("`similarity` is a percentage between 0 and 100"));
        }
    }
    let report = asked.report.clone().filter(|r| !r.trim().is_empty());
    // The report becomes a run artifact: a path in a folder someone may tidy is not a record.
    let report = match report {
        Some(original) => {
            let source = Path::new(original.trim());
            if !source.is_file() {
                return Err(DevdError::not_found(format!("there is no report at `{original}`")));
            }
            let safe = |s: &str| -> String {
                s.chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                    .collect::<String>()
                    .trim_matches('-')
                    .to_string()
            };
            let folder = dir(paths).join(&record.run).join("checks");
            std::fs::create_dir_all(&folder)?;
            let name = format!(
                "{}-{}-{}.md",
                safe(screen),
                safe(&canonical_width),
                util::now().replace([':', '.'], "-")
            );
            let kept = folder.join(name);
            std::fs::copy(source, &kept).map_err(|e| {
                DevdError::internal(format!("could not keep the check report: {e}"))
            })?;
            Some(kept.display().to_string())
        }
        None => None,
    };
    let found = Check {
        screen: screen.to_string(),
        width: canonical_width,
        similarity: asked.similarity,
        gaps: asked.gaps.iter().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect(),
        report,
        at: util::now(),
    };
    let summary = match found.similarity {
        Some(s) => format!("{} at {} measured {s:.0}% alike, {} gap(s) named", found.screen, found.width, found.gaps.len()),
        None => format!("{} at {} compared, {} gap(s) named", found.screen, found.width, found.gaps.len()),
    };
    record.checks.push(found.clone());
    save(paths, &record)?;
    store.add_evidence(NewEvidence {
        run_id: record.evidence_id(),
        step_id: None,
        tool_call_id: None,
        artifact_id: None,
        kind: "read",
        capability: "recreate.check",
        passed: false,
        claim: None,
        summary: &summary,
        detail: Some(&serde_json::json!({ "screen": found.screen, "width": found.width, "similarity": found.similarity, "gaps": found.gaps, "report": found.report }).to_string()),
    })?;
    Ok(record)
}

/// What the run has proved, and what it has only measured.
pub fn report(paths: &Paths, store: &Store, run: &str) -> Result<serde_json::Value> {
    let record = load(paths, run)?;
    let evidence = store.list_evidence(record.evidence_id(), 500)?;
    let verified: Vec<&str> = evidence
        .iter()
        .filter(|e| e.kind == "verify" && e.passed)
        .map(|e| e.summary.as_str())
        .collect();
    let measured = evidence.iter().filter(|e| e.capability == "recreate.check").count();
    let missing = record.incomplete();
    let proven = !verified.is_empty();
    Ok(serde_json::json!({
        "run": record.run,
        "target": record.target,
        "mode": record.mode,
        "right": record.right,
        "app": record.app,
        "captures": record.captures.len(),
        "capture_files": record.captures.iter().map(|c| serde_json::json!({ "screen": c.screen, "width": c.width, "path": c.path, "note": c.note })).collect::<Vec<_>>(),
        "screens": record.screens(),
        "screens_missing_a_width": missing,
        "checks": record.checks.len(),
        "check_rows": record.checks.clone(),
        "check_reports": record.checks.iter().filter_map(|c| c.report.clone()).collect::<Vec<_>>(),
        "gaps": record.gaps(),
        "measured": measured,
        "verified": verified,
        // A recreation is done when something was *verified*, never because it looked alike.
        "proven": proven,
        // What the completion gate reads: a check report exists, every screen was seen at all
        // three widths, and the project's own checks passed. A similarity never enters here.
        "ready": proven && missing.is_empty() && !record.checks.is_empty(),
        "clean_room": record.is_clean_room(),
    }))
}

/// Why a recreation may not finish yet. Empty for a run that is not a recreation.
///
/// The completion gate calls this, so a run that began with `recreate.begin` cannot be marked
/// done on a similarity figure, a screenshot, or a task that says it is finished.
pub fn gate(paths: &Paths, store: &Store, run: &str) -> Result<Vec<String>> {
    if find(paths, run).is_none() {
        return Ok(Vec::new());
    }
    let reported = report(paths, store, run)?;
    let mut out = Vec::new();
    let missing: Vec<String> = reported["screens_missing_a_width"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if reported["captures"].as_u64().unwrap_or(0) == 0 {
        out.push("the recreation has no capture of the target at all".to_string());
    } else if !missing.is_empty() {
        out.push(format!(
            "the recreation has not seen every screen at three widths; still missing a width: {}",
            missing.join(", ")
        ));
    }
    if reported["checks"].as_u64().unwrap_or(0) == 0 {
        out.push("no comparison of the rebuild against the original was recorded".to_string());
    } else if reported["check_reports"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0)
        == 0
    {
        out.push(
            "the comparison named gaps but kept no check report, so a reviewer has nothing to read"
                .to_string(),
        );
    }
    if !reported["proven"].as_bool().unwrap_or(false) {
        out.push("the rebuild's own checks have not passed; a similar screen is a measurement, not a verdict".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        (d, p)
    }

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn begun(paths: &Paths, store: &Store) -> Record {
        begin(paths, store, "https://example.com", "clean_room", "my own copy of a site I use", None, "human:test", None).unwrap()
    }

    #[test]
    fn a_recreation_starts_in_clean_room_and_never_in_deep() {
        let (_d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        assert!(r.is_clean_room());
        assert_eq!(r.target, "https://example.com");
        let refused = begin(&p, &s, "https://example.com", "deep", "my own copy of a site I use", None, "human:test", None).unwrap_err();
        assert_eq!(refused.code(), "permission_denied", "deep is a separate decision: {refused}");
    }

    #[test]
    fn a_recreation_can_join_a_team_run_so_the_team_gate_reads_it() {
        let (_d, p) = temp_paths();
        let s = store();
        let team = s
            .create_run("Recreate the site", "Recreate the site", None, 1, "human:test")
            .unwrap();
        let r = begin(
            &p,
            &s,
            "https://example.com",
            "clean_room",
            "my own copy of a site I use",
            None,
            "human:test",
            Some(&team.id),
        )
        .unwrap();
        assert_eq!(r.run, team.id, "the record hangs off the run the team drives");
        assert_eq!(
            gate(&p, &s, &team.id).unwrap().len(),
            3,
            "the team's completion now reads the recreation's record"
        );
    }

    #[test]
    fn a_recreation_without_a_declaration_is_refused() {
        let (_d, p) = temp_paths();
        let s = store();
        let e = begin(&p, &s, "https://example.com", "clean_room", "yes", None, "human:test", None).unwrap_err();
        assert!(e.to_string().contains("may recreate"), "{e}");
    }

    #[test]
    fn a_capture_must_be_a_real_picture_and_replaces_the_same_screen_at_the_same_width() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let shot = d.path().join("home-390.png");
        std::fs::write(&shot, b"not really a png, but bytes").unwrap();
        let after = capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "the signed-out home").unwrap();
        assert_eq!(after.captures.len(), 1);
        let again = capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "the signed-out home, retaken").unwrap();
        assert_eq!(again.captures.len(), 1, "the same screen at the same width is replaced, not stacked");
        assert_eq!(again.captures[0].note, "the signed-out home, retaken");
        let missing = capture(&p, &s, &r.run, "home", "phone", "C:/nope/missing.png", "").unwrap_err();
        assert_eq!(missing.code(), "not_found");
        let bad_width = capture(&p, &s, &r.run, "home", "watch", &shot.display().to_string(), "").unwrap_err();
        assert!(bad_width.to_string().contains("width"), "{bad_width}");
    }

    #[test]
    fn three_widths_are_required_and_the_missing_ones_are_named() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let shot = d.path().join("home.png");
        std::fs::write(&shot, b"bytes").unwrap();
        capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "").unwrap();
        capture(&p, &s, &r.run, "home", "desktop", &shot.display().to_string(), "").unwrap();
        let record = load(&p, &r.run).unwrap();
        assert_eq!(record.incomplete(), vec!["home".to_string()], "the tablet width is missing");
        capture(&p, &s, &r.run, "home", "tablet", &shot.display().to_string(), "").unwrap();
        assert!(load(&p, &r.run).unwrap().incomplete().is_empty());
    }

    #[test]
    fn a_comparison_is_recorded_as_an_observation_and_never_as_verification() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let shot = d.path().join("home.png");
        std::fs::write(&shot, b"bytes").unwrap();
        capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "").unwrap();
        check(
            &p,
            &s,
            &r.run,
            Check {
                screen: "home".into(),
                width: "phone".into(),
                similarity: Some(96.0),
                gaps: vec!["the search field is 8px lower".to_string()],
                report: None,
                at: String::new(),
            },
        )
        .unwrap();

        let evidence = s.list_evidence(&r.run, 100).unwrap();
        let compared = evidence.iter().find(|e| e.capability == "recreate.check").expect("the comparison is evidence");
        assert_eq!(compared.kind, "read", "a similarity is an observation");
        assert!(!compared.passed, "looking alike does not pass anything");

        let reported = report(&p, &s, &r.run).unwrap();
        assert_eq!(reported["measured"], 1);
        assert_eq!(reported["proven"], false, "nothing was verified, so nothing is proven");
        assert_eq!(reported["gaps"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_passing_check_is_what_makes_a_recreation_proven() {
        let (_d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        s.add_evidence(NewEvidence {
            run_id: &r.run,
            step_id: None,
            tool_call_id: None,
            artifact_id: None,
            kind: "verify",
            capability: "project.verify",
            passed: true,
            claim: Some("the rebuild's own checks pass"),
            summary: "project.verify: 42 passed",
            detail: None,
        })
        .unwrap();
        assert_eq!(report(&p, &s, &r.run).unwrap()["proven"], true);
    }

    #[test]
    fn a_check_report_is_kept_as_a_run_artifact() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let report = d.path().join("compare.md");
        std::fs::write(&report, "# Comparison\n\nHome at phone: the search field is 8px low.\n").unwrap();
        check(
            &p,
            &s,
            &r.run,
            Check {
                screen: "home".into(),
                width: "phone".into(),
                similarity: Some(80.0),
                gaps: vec!["the search field is 8px low".into()],
                report: Some(report.display().to_string()),
                at: String::new(),
            },
        )
        .unwrap();
        let kept = load(&p, &r.run).unwrap().checks[0].report.clone().expect("kept");
        assert!(Path::new(&kept).is_file(), "the report is copied beside the run: {kept}");
        assert!(std::fs::read_to_string(&kept).unwrap().contains("8px low"));
        assert!(kept.contains(&r.run), "the copy belongs to the run: {kept}");
    }

    #[test]
    fn a_pixel_count_that_is_a_named_width_is_the_same_capture() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let shot = d.path().join("home.png");
        std::fs::write(&shot, b"bytes").unwrap();
        capture(&p, &s, &r.run, "home", "390", &shot.display().to_string(), "").unwrap();
        let after = capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "").unwrap();
        assert_eq!(after.captures.len(), 1, "390 and phone are one width, not two");
        assert_eq!(after.captures[0].width, "phone");
        assert_eq!(width_px("390"), Some(390));
        assert_eq!(width_px("500"), Some(500));
        assert_eq!(width_px("thimble"), None);
    }

    #[test]
    fn the_completion_gate_names_every_missing_part_and_is_silent_for_other_runs() {
        let (d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let reasons = gate(&p, &s, &r.run).unwrap();
        assert!(reasons.iter().any(|x| x.contains("no capture")), "{reasons:?}");
        assert!(reasons.iter().any(|x| x.contains("no comparison")), "{reasons:?}");
        assert!(reasons.iter().any(|x| x.contains("not a verdict")), "{reasons:?}");
        assert!(gate(&p, &s, "not-a-recreation").unwrap().is_empty());

        let shot = d.path().join("home.png");
        std::fs::write(&shot, b"bytes").unwrap();
        capture(&p, &s, &r.run, "home", "phone", &shot.display().to_string(), "").unwrap();
        let reasons = gate(&p, &s, &r.run).unwrap();
        assert!(reasons.iter().any(|x| x.contains("three widths") && x.contains("home")), "{reasons:?}");
    }

    #[test]
    fn deep_mode_is_its_own_decision_and_needs_a_reason() {
        let (_d, p) = temp_paths();
        let s = store();
        let r = begun(&p, &s);
        let refused = set_mode(&p, &s, &r.run, "deep", "short").unwrap_err();
        assert!(refused.to_string().contains("reason"), "{refused}");
        let switched = set_mode(&p, &s, &r.run, "deep", "the site is mine and I need its source").unwrap();
        assert!(!switched.is_clean_room());
        assert!(report(&p, &s, &r.run).unwrap()["clean_room"] == false);
    }
}
