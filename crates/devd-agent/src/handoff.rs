//! What one task passes to the next.
//!
//! A task that finishes leaves a report in its own words. The task that depends on it needs more than a paragraph cut off
//! at a character count: what was done, which files were touched, what was decided and why, what was left open. This
//! keeps those four things in a shape, builds them from what the harness saw as well as from what the agent said (the
//! files come from the changes it made, not from its claim), and writes them out for the next agent and for a progress
//! file the whole team reads. The agent that starts next is given this, not the conversation that produced it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a finished task hands on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    /// What was done, in a few sentences.
    pub summary: String,
    /// The files it changed, by path.
    #[serde(default)]
    pub files: Vec<String>,
    /// What it decided, one line each.
    #[serde(default)]
    pub decisions: Vec<String>,
    /// What it left open, one line each: questions for the person or for a later task.
    #[serde(default)]
    pub open_questions: Vec<String>,
    /// The checks it ran and whether each passed: `(name, ok)`.
    #[serde(default)]
    pub checks: Vec<(String, bool)>,
}

/// The lines of a bulleted section that follows a heading in prose: `Decisions:` or `## Open questions` and the `-` or `*`
/// lines under it, up to the first line that is not one.
fn section(text: &str, headings: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let bare = line
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_end_matches(':')
            .trim()
            .trim_matches('*')
            .to_ascii_lowercase();
        if !inside {
            if headings.iter().any(|h| bare == *h) {
                inside = true;
            }
            continue;
        }
        let item = line.trim();
        if let Some(rest) = item.strip_prefix("- ").or_else(|| item.strip_prefix("* ")) {
            out.push(rest.trim().to_string());
        } else if item.is_empty() && out.is_empty() {
            continue;
        } else {
            break;
        }
    }
    out
}

/// The paths in a list of evidence that were changed, in the order they first were.
pub fn changed_files(evidence: &[Value], root: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for e in evidence {
        if e["kind"] != "change" || e["ok"] != true {
            continue;
        }
        let path = e["detail"]
            .as_str()
            .and_then(|d| serde_json::from_str::<Value>(d).ok())
            .and_then(|d| d["data"]["path"].as_str().map(str::to_string))
            .or_else(|| {
                e["summary"].as_str().map(|s| {
                    s.split_once(": ")
                        .filter(|(head, _)| head.starts_with("fs."))
                        .map(|(_, p)| p)
                        .unwrap_or(s)
                        .trim()
                        .to_string()
                })
            });
        let Some(path) = path else { continue };
        let normal = path.replace('\\', "/");
        let root = root.replace('\\', "/");
        let relative = normal
            .strip_prefix(root.trim_end_matches('/'))
            .map(|r| r.trim_start_matches('/'))
            .unwrap_or(&normal)
            .to_string();
        if !relative.is_empty() && seen.insert(relative.clone()) {
            out.push(relative);
        }
    }
    out
}

/// The checks in a list of evidence: what ran, and whether it passed.
pub fn checks_run(evidence: &[Value]) -> Vec<(String, bool)> {
    evidence
        .iter()
        .filter(|e| e["kind"] == "verify")
        .map(|e| {
            (
                e["summary"].as_str().unwrap_or("a check").to_string(),
                e["ok"] == true,
            )
        })
        .collect()
}

/// Build a handoff from a report and the evidence behind it.
pub fn build(report: &str, evidence: &[Value], root: &str) -> Handoff {
    // The summary is the report before its own sections, so what was decided is not said twice.
    let summary = report
        .lines()
        .take_while(|l| {
            let bare = l
                .trim()
                .trim_start_matches('#')
                .trim()
                .trim_end_matches(':')
                .trim()
                .trim_matches('*')
                .to_ascii_lowercase();
            !matches!(
                bare.as_str(),
                "decisions" | "decided" | "open questions" | "questions" | "left open"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    Handoff {
        summary: if summary.is_empty() {
            report.trim().to_string()
        } else {
            summary
        },
        files: changed_files(evidence, root),
        decisions: section(report, &["decisions", "decided"]),
        open_questions: section(report, &["open questions", "questions", "left open"]),
        checks: checks_run(evidence),
    }
}

/// A report with its decisions and open questions written as sections, so a report made of separate fields reads, and
/// is read back, the same as one written in prose.
pub fn with_sections(summary: &str, decisions: &[String], open_questions: &[String]) -> String {
    let mut out = summary.trim().to_string();
    if !decisions.is_empty() {
        out.push_str("\n\nDecisions:\n");
        out.push_str(
            &decisions
                .iter()
                .map(|d| format!("- {d}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !open_questions.is_empty() {
        out.push_str("\n\nOpen questions:\n");
        out.push_str(
            &open_questions
                .iter()
                .map(|d| format!("- {d}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    out
}

/// A handoff as the next agent reads it.
pub fn render(h: &Handoff, limit: usize) -> String {
    let clip = |s: &str, n: usize| -> String {
        let mut it = s.chars();
        let head: String = it.by_ref().take(n).collect();
        if it.next().is_some() {
            format!("{}…", head.trim_end())
        } else {
            head
        }
    };
    let mut out = clip(&h.summary, limit);
    if !h.files.is_empty() {
        let shown: Vec<&str> = h.files.iter().take(25).map(String::as_str).collect();
        out.push_str(&format!(
            "\nFiles changed: {}{}",
            shown.join(", "),
            if h.files.len() > 25 {
                format!(" and {} more", h.files.len() - 25)
            } else {
                String::new()
            }
        ));
    }
    if !h.decisions.is_empty() {
        out.push_str("\nDecided:\n");
        out.push_str(
            &h.decisions
                .iter()
                .take(10)
                .map(|d| format!("  - {}", clip(d, 240)))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !h.open_questions.is_empty() {
        out.push_str("\nLeft open:\n");
        out.push_str(
            &h.open_questions
                .iter()
                .take(10)
                .map(|d| format!("  - {}", clip(d, 240)))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !h.checks.is_empty() {
        let passed = h.checks.iter().filter(|(_, ok)| *ok).count();
        out.push_str(&format!(
            "\nChecks it ran: {passed} of {} passed",
            h.checks.len()
        ));
    }
    out
}

/// One entry of the team's progress file.
pub fn progress_entry(key: &str, role: &str, title: &str, h: &Handoff, at: &str) -> String {
    let mut out = format!("## {title} ({key}, {role}) — {at}\n\n{}\n", render(h, 900));
    out.push('\n');
    out
}

/// The last `max` characters of the progress file, cut at an entry so a half entry is not given to anyone.
pub fn tail(progress: &str, max: usize) -> String {
    if progress.chars().count() <= max {
        return progress.trim().to_string();
    }
    let skip = progress.chars().count() - max;
    let from: usize = progress
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let rest = &progress[from..];
    match rest.find("\n## ") {
        Some(at) => rest[at + 1..].trim().to_string(),
        None => rest.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(path: &str) -> Value {
        json!({ "kind": "change", "ok": true, "capability": "fs.write", "summary": format!("fs.write: {path}"), "detail": json!({ "data": { "path": path } }).to_string() })
    }

    #[test]
    fn the_files_come_from_the_changes_the_agent_made_not_from_its_claim_and_are_relative() {
        let evidence = vec![
            change("D:\\work\\shop\\store\\store.go"),
            change("D:\\work\\shop\\store\\store.go"),
            change("D:\\work\\shop\\go.mod"),
            json!({ "kind": "read", "ok": true, "summary": "fs.read: x" }),
            json!({ "kind": "change", "ok": false, "summary": "fs.write: failed.go" }),
        ];
        assert_eq!(
            changed_files(&evidence, "D:\\work\\shop"),
            vec!["store/store.go".to_string(), "go.mod".to_string()],
            "once each, in order, only what succeeded"
        );
    }

    #[test]
    fn decisions_and_open_questions_are_read_from_the_report_and_the_summary_stops_before_them() {
        let report = "Store written and verified.\n\nDecisions:\n- one state.json, rewritten atomically\n- 200 runs kept per job\n\nOpen questions:\n- should output be encrypted?\n";
        let h = build(report, &[], "/p");
        assert_eq!(h.summary, "Store written and verified.");
        assert_eq!(
            h.decisions,
            vec![
                "one state.json, rewritten atomically",
                "200 runs kept per job"
            ]
        );
        assert_eq!(h.open_questions, vec!["should output be encrypted?"]);
    }

    #[test]
    fn a_report_made_of_fields_reads_back_as_the_same_handoff() {
        let report = with_sections(
            "Done.",
            &["use argv, never a shell".to_string()],
            &["Windows process groups?".to_string()],
        );
        let h = build(&report, &[], "/p");
        assert_eq!(
            (
                h.summary.as_str(),
                h.decisions.len(),
                h.open_questions.len()
            ),
            ("Done.", 1, 1)
        );
    }

    #[test]
    fn a_report_with_no_sections_is_all_summary() {
        let h = build("It works.", &[], "/p");
        assert_eq!(h.summary, "It works.");
        assert!(h.decisions.is_empty() && h.open_questions.is_empty());
    }

    #[test]
    fn what_the_next_agent_reads_names_the_files_the_decisions_and_what_is_open() {
        let h = Handoff {
            summary: "Parser done.".into(),
            files: vec!["cron/parse.go".into()],
            decisions: vec!["Vixie day-of-month/day-of-week OR".into()],
            open_questions: vec!["L and W support?".into()],
            checks: vec![("go test".into(), true), ("go vet".into(), false)],
        };
        let text = render(&h, 500);
        assert!(
            text.contains("Files changed: cron/parse.go")
                && text.contains("Decided:")
                && text.contains("Left open:")
                && text.contains("1 of 2 passed"),
            "{text}"
        );
    }

    #[test]
    fn the_progress_tail_starts_at_an_entry_and_a_short_file_is_whole() {
        let file =
            "## a (1, x) — t\n\nfirst\n\n## b (2, x) — t\n\nsecond\n\n## c (3, x) — t\n\nthird\n";
        assert_eq!(tail(file, 10_000), file.trim());
        let cut = tail(file, 40);
        assert!(cut.starts_with("## "), "{cut}");
        assert!(cut.contains("third") && !cut.contains("first"));
    }
}
