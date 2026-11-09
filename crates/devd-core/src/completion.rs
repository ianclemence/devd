//! The canonical completion verdict.
//!
//! A run is not finished because every task says `done`. It is finished when an
//! independent gatekeeper approved a specific revision and every acceptance
//! criterion it judged passed. This module owns that rule so the scheduler, the
//! store and every interface read one answer.
//!
//! The text classifier exists because the models write verdicts in prose; where
//! a structured verdict is supplied it always wins. A sentence that names a
//! failing check can never be read as an approval, which is the failure this
//! module was written to make impossible.

use serde::{Deserialize, Serialize};

use crate::store_team::{TeamRun, TeamTask};

/// The only three answers an independent check may give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The work may be accepted.
    Approve,
    /// The work needs another pass.
    ChangesNeeded,
    /// The work cannot continue without something outside the team.
    Blocked,
}

impl Verdict {
    /// The database and wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::ChangesNeeded => "changes_needed",
            Self::Blocked => "blocked",
        }
    }

    /// Read a verdict, accepting the ways a person or a model writes it.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
            "approve" | "approved" | "lgtm" | "accept" | "passed" => Some(Self::Approve),
            "changes_needed" | "changes" | "reject" | "rejected" | "not_approved" => {
                Some(Self::ChangesNeeded)
            }
            "blocked" | "block" => Some(Self::Blocked),
            _ => None,
        }
    }
}

/// What a run's finished state is, and why.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Completion {
    /// The canonical verdict, when one exists.
    pub verdict: Option<Verdict>,
    /// The run was completed because a person explicitly accepted the gatekeeper
    /// task without machine proof. Never the same thing as `approve`.
    pub by_hand: bool,
    /// Everything that still stops completion, in the words the person will read.
    pub reasons: Vec<String>,
    /// The gatekeeper task the verdict came from.
    pub gatekeeper: Option<String>,
    /// The commit that verdict applies to.
    pub reviewed_commit: Option<String>,
}

impl Completion {
    /// Whether the run may enter `done`.
    pub fn approved(&self) -> bool {
        self.verdict == Some(Verdict::Approve) && self.reasons.is_empty()
    }

    /// The verdict string stored on the run (`accepted_by_hand` when a person
    /// overrode a non-approving check).
    pub fn stored_verdict(&self) -> Option<&'static str> {
        if self.approved() {
            Some(if self.by_hand { "accepted_by_hand" } else { "approve" })
        } else {
            self.verdict.map(Verdict::as_str)
        }
    }
}

/// Whether a role is allowed to judge finished work.
pub fn is_gatekeeper(role: &str) -> bool {
    matches!(role, "reviewer" | "security" | "qa")
}

/// Whether a task is the run's final check.
fn is_final_check(t: &TeamTask) -> bool {
    t.key == "final-verify" || t.key.ends_with("-verify")
}

/// Read a gatekeeper's prose and say what it concluded.
///
/// The classifier is deliberately conservative in one direction only: a line
/// that names a failure is a failure. A zero/negated failure (`0 failed`,
/// `no failures`) and a plain pass statement are not.
pub fn derive_verdict(role: &str, summary: &str) -> Option<Verdict> {
    if !is_gatekeeper(role) {
        return None;
    }
    let s = summary.trim();
    if s.is_empty() {
        return Some(Verdict::Blocked);
    }
    let lower = s.to_lowercase();
    let first = lower.lines().take(3).collect::<Vec<_>>().join(" ");

    // An explicit verdict in the opening lines is the author's own answer.
    if first.contains("verdict: approve")
        || first.contains("verdict - approve")
        || first.contains("verdict: approved")
        || first.contains("verdict: lgtm")
        || first.contains("verdict: pass")
    {
        return Some(Verdict::Approve);
    }
    if first.contains("verdict: changes")
        || first.contains("verdict: reject")
        || first.contains("verdict: blocked")
    {
        return Some(if first.contains("blocked") {
            Verdict::Blocked
        } else {
            Verdict::ChangesNeeded
        });
    }

    // A named, explicit blocker outranks everything except an explicit approval.
    if ["i am blocked", "blocked on", "blocked:", "cannot continue", "cannot proceed"]
        .iter()
        .any(|m| first.contains(m))
    {
        return Some(Verdict::Blocked);
    }

    if lower.lines().any(line_names_failure) {
        return Some(Verdict::ChangesNeeded);
    }

    if [
        "all tests pass",
        "tests pass",
        "suite passes",
        "suite is green",
        "all checks pass",
        "no must-fix",
        "no blocking",
        "meets the definition of done",
        "approved",
        "approve",
    ]
    .iter()
    .any(|m| lower.contains(m))
    {
        return Some(Verdict::Approve);
    }

    // A gatekeeper that reported without naming a failure and without an
    // explicit approval is treated as approving; the evidence gate has already
    // checked that the review actually read or ran something.
    Some(Verdict::Approve)
}

fn line_names_failure(line: &str) -> bool {
    let l = line.to_lowercase();
    // These phrases contain "fail" but mean the opposite.
    let negated = [
        "0 failed",
        "0 failures",
        "0 failing",
        "zero failed",
        "zero failures",
        "no failures",
        "no failed",
        "no failing",
        "not a failure",
        "without failures",
        "all tests pass",
        "all checks pass",
        "tests pass",
        "suite passes",
        "suite is green",
    ];
    if negated.iter().any(|n| l.contains(n)) {
        return false;
    }
    [
        "failed",
        "fails",
        "failing",
        "failure",
        "not pass",
        "did not pass",
        "does not pass",
        "regression",
        "not fixed",
        "still broken",
        "defects remain",
        "blocking finding",
        "blocking issue",
        "not met",
        "unmet",
        "incomplete",
        "cannot sign off",
        "could not sign off",
        "changes needed",
        "changes requested",
        "request changes",
        "not approved",
        "do not approve",
    ]
    .iter()
    .any(|m| l.contains(m))
}

/// Whether a task recorded a change to the project, whatever its role.
///
/// A designer can write the deliverable, an architect can write the decision
/// record, and a researcher can write the report. Completion must not depend on
/// the role's name; it depends on whether the work changed anything.
fn task_changed(t: &TeamTask) -> bool {
    t.evidence
        .as_array()
        .map(|rows| rows.iter().any(|f| f["kind"] == "change" && f["ok"] == true))
        .unwrap_or(false)
}

/// Whether finished work needs an independent final check.
///
/// Any task that wrote files needs one, and so do the roles whose whole job is
/// to leave a change. A pure answer or a read-only research note does not.
pub fn needs_final_verification(real: &[&TeamTask]) -> bool {
    real.iter().any(|t| {
        task_changed(t)
            || matches!(
                t.role.as_str(),
                "implementer"
                    | "backend"
                    | "frontend"
                    | "mobile"
                    | "data"
                    | "designer"
                    | "docs"
                    | "infra"
            )
    })
}

/// Judge a run from its persisted tasks. Pure: the caller supplies the pending
/// approval count, because approvals live in another store surface.
pub fn check(run: &TeamRun, tasks: &[TeamTask], pending_approvals: usize) -> Completion {
    let real: Vec<&TeamTask> = tasks.iter().filter(|t| t.role != "planner").collect();
    let mut reasons = Vec::new();

    // Work that is not in a terminal state stops completion regardless of what
    // any other task says.
    for t in &real {
        if !matches!(
            t.state.as_str(),
            "done" | "skipped" | "cancelled"
        ) {
            reasons.push(format!(
                "{} is still {}",
                t.key,
                t.state.replace('_', " ")
            ));
        }
        if t.state != "skipped" {
            if let Some(kind) = &t.failure_kind {
                if matches!(kind.as_str(), "policy" | "environment" | "pre-existing") {
                    reasons.push(format!("{} stopped: {}", t.key, t.error.clone().unwrap_or_else(|| kind.clone())));
                }
            }
        }
    }
    if pending_approvals > 0 {
        reasons.push(format!(
            "{pending_approvals} approval(s) still waiting for a person"
        ));
    }

    // The latest independent check is authoritative; older reviews are history.
    let gatekeeper = real
        .iter()
        .filter(|t| is_gatekeeper(&t.role) || is_final_check(t))
        .max_by_key(|t| t.created_at.clone());
    let Some(gate) = gatekeeper else {
        if needs_final_verification(&real) {
            reasons.push("no independent verification was recorded".to_string());
        }
        return Completion {
            verdict: if reasons.is_empty() {
                Some(Verdict::Approve)
            } else {
                Some(Verdict::Blocked)
            },
            by_hand: false,
            reasons,
            gatekeeper: None,
            reviewed_commit: run.reviewed_commit.clone(),
        };
    };

    let by_hand = gate.state == "skipped";
    // A person accepting the gatekeeper task is an explicit override: it is not
    // machine approval, but it does allow the run to finish, visibly marked.
    let verdict = if by_hand {
        Some(Verdict::Approve)
    } else {
        gate.verdict
            .as_deref()
            .and_then(Verdict::parse)
            .or_else(|| derive_verdict(&gate.role, gate.result.as_deref().unwrap_or("")))
    };

    if !by_hand {
        if let Some(kind) = &gate.failure_kind {
            if kind == "policy" || kind == "environment" {
                reasons.push(format!(
                    "the final check stopped: {}",
                    gate.error.clone().unwrap_or_else(|| kind.clone())
                ));
            }
        }

        // A structured criterion result is stronger than the prose verdict.
        if let Some(data) = &gate.verdict_data {
            if let Some(criteria) = data.get("criteria").and_then(|c| c.as_array()) {
                for c in criteria {
                    let status = c.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    if !matches!(status, "passed" | "not_applicable" | "skipped") {
                        let id = c
                            .get("id")
                            .and_then(|s| s.as_str())
                            .or_else(|| c.get("text").and_then(|s| s.as_str()))
                            .unwrap_or("a criterion");
                        let why = c.get("reason").and_then(|s| s.as_str()).unwrap_or("");
                        reasons.push(format!("{id} is {status}{}", if why.is_empty() { String::new() } else { format!(": {why}") }));
                    }
                }
            }
        }
    }

    let verdict = match verdict {
        Some(Verdict::Approve) if reasons.is_empty() => Some(Verdict::Approve),
        Some(Verdict::Approve) => Some(Verdict::ChangesNeeded),
        Some(v) => Some(v),
        None => Some(Verdict::Blocked),
    };
    if verdict != Some(Verdict::Approve) {
        let said = gate.result.as_deref().unwrap_or("").trim();
        if !said.is_empty() {
            let short: String = said.chars().take(240).collect();
            reasons.push(format!("{}: {}", gate.key, short));
        }
    }
    if verdict != Some(Verdict::Approve) && reasons.is_empty() {
        reasons.push(match verdict {
            Some(Verdict::ChangesNeeded) => format!("the final check on {} asked for changes", gate.key),
            Some(Verdict::Blocked) => format!("the final check on {} is blocked", gate.key),
            _ => "there is no completion verdict".to_string(),
        });
    }

    Completion {
        verdict,
        by_hand,
        reasons,
        gatekeeper: Some(gate.key.clone()),
        reviewed_commit: gate
            .reviewed_commit
            .clone()
            .or_else(|| run.reviewed_commit.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store_team::{TeamRun, TeamTask};

    fn task(key: &str, role: &str, state: &str, result: &str) -> TeamTask {
        TeamTask {
            id: format!("id-{key}"),
            run_id: "run".into(),
            key: key.into(),
            role: role.into(),
            title: key.into(),
            instructions: String::new(),
            depends_on: vec![],
            scope: vec![],
            state: state.into(),
            attempts: 1,
            workdir: None,
            branch: None,
            thread_id: None,
            result: Some(result.into()),
            evidence: serde_json::json!([]),
            error: None,
            created_at: format!("2026-01-01T00:00:{:02}Z", key.len()),
            started_at: None,
            finished_at: None,
            mandate: serde_json::Value::Null,
            last_step_at: None,
            stalls: 0,
            criteria: vec![],
            verdict: None,
            verdict_data: None,
            reviewed_commit: None,
            checkpoint: None,
            failure_kind: None,
        }
    }

    fn run() -> TeamRun {
        TeamRun {
            id: "run".into(),
            title: "t".into(),
            goal: "g".into(),
            project: None,
            state: "running".into(),
            plan_approved: true,
            summary: None,
            error: None,
            parallel: 2,
            thread_id: None,
            created_by: "human".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            verdict: None,
            verdict_at: None,
            reviewed_commit: None,
            base_commit: None,
        }
    }

    #[test]
    fn a_named_test_failure_is_never_an_approval() {
        for summary in [
            "Final verification: `php artisan test` FAILS, exit 1 - 3 failed, 814 passed.",
            "TESTS: FAIL — 3 failed, 806 passed.",
            "VERDICT: changes needed. The blocking finding is not fixed.",
            "I could not sign off: the suite is red.",
        ] {
            assert_eq!(
                derive_verdict("qa", summary),
                Some(Verdict::ChangesNeeded),
                "{summary}"
            );
        }
    }

    #[test]
    fn a_zero_failure_count_is_a_pass() {
        for summary in [
            "Tests: 142 passed (499 assertions), 0 failed.",
            "All 817 tests pass; no failures.",
            "VERDICT: approve. No must-fix findings.",
        ] {
            assert_eq!(derive_verdict("qa", summary), Some(Verdict::Approve), "{summary}");
        }
    }

    #[test]
    fn only_gatekeepers_have_verdicts() {
        assert_eq!(derive_verdict("implementer", "tests failed"), None);
        assert_eq!(derive_verdict("reviewer", "tests failed"), Some(Verdict::ChangesNeeded));
    }

    #[test]
    fn a_run_with_a_changes_needed_gatekeeper_cannot_complete() {
        let mut t = task("final-verify", "qa", "done", "3 failed, 806 passed");
        t.verdict = Some("changes_needed".into());
        let c = check(&run(), &[t], 0);
        assert!(!c.approved());
        assert_eq!(c.verdict, Some(Verdict::ChangesNeeded));
        assert!(c.reasons.iter().any(|r| r.contains("failed")));
    }

    #[test]
    fn a_run_with_an_approving_gatekeeper_completes() {
        let mut t = task("final-verify", "qa", "done", "817 passed, 0 failed");
        t.verdict = Some("approve".into());
        t.reviewed_commit = Some("abc".into());
        let c = check(&run(), &[t], 0);
        assert!(c.approved());
        assert_eq!(c.reviewed_commit.as_deref(), Some("abc"));
    }

    #[test]
    fn a_hand_accepted_gatekeeper_is_a_visible_override() {
        let t = task("final-verify", "qa", "skipped", "I accept the existing failures");
        let c = check(&run(), &[t], 0);
        assert!(c.approved());
        assert!(c.by_hand);
        assert_eq!(c.stored_verdict(), Some("accepted_by_hand"));
    }

    #[test]
    fn a_designer_deliverable_without_an_independent_check_is_not_approved() {
        // The live failure: a one-task designer run wrote and committed a
        // document and verified its own work with lint. With no gatekeeper the
        // run was marked done. A task that changed files is not its own check.
        let mut t = task("work", "designer", "done", "wrote and committed the design");
        t.evidence = serde_json::json!([{ "kind": "change", "ok": true }]);
        let c = check(&run(), &[t.clone()], 0);
        assert!(!c.approved());
        assert!(c.reasons.iter().any(|r| r.contains("independent verification")));
        assert!(needs_final_verification(&[&t]));
    }

    #[test]
    fn a_read_only_answer_does_not_need_a_gatekeeper() {
        // A research note or an answer changed nothing; making it wait for a
        // final check would turn every question into a two-agent run.
        let mut t = task("answer", "researcher", "done", "the answer is 42");
        t.evidence = serde_json::json!([{ "kind": "read", "ok": true }]);
        let c = check(&run(), &[t.clone()], 0);
        assert!(c.approved());
        assert!(!needs_final_verification(&[&t]));
    }

    #[test]
    fn a_pending_approval_blocks_completion() {
        let mut t = task("final-verify", "qa", "done", "817 passed, 0 failed");
        t.verdict = Some("approve".into());
        let c = check(&run(), &[t], 1);
        assert!(!c.approved());
        assert!(c.reasons.iter().any(|r| r.contains("approval")));
    }
}
