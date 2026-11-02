//! A project's own rules for starting a run.
//!
//! In the super-agent picture an event that is worth acting on is decided by two
//! layers: rules in code, then a small model. A schedule is only a clock; it has
//! no opinion about whether the thing it was written for is actually happening.
//! An intake rule is a project's written statement of the second half: *when this
//! happens, this many times, at most this often, consider a run that does this.*
//!
//! Three promises keep it from becoming a way to run unattended agents on
//! someone's machine:
//!
//! * **Nothing runs without a record.** In `observe` mode - the default - a rule
//!   *observes* and records a suggestion ("devd would have started a run that
//!   does this"); no model is called and nothing changes.
//! * **Plan approval is never waived.** In `auto` mode the rule starts a run,
//!   and that run still waits for a person to approve its plan, exactly as any
//!   other. A rule cannot make the plan approval optional.
//! * **The rule is data a person wrote.** It lives in `intake_rules`, is
//!   human-only to write, and the run it starts carries the same rules as any
//!   other run, including production mode's stricter ones.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{IntakeRule, Result};

/// The events a rule may watch.
pub const TRIGGERS: &[(&str, &str)] = &[
    ("health.down", "a watched site is down"),
    ("check.failed", "a project's own check (test, lint, build) failed"),
    ("schedule.missed", "a schedule was due while devd was not running"),
];

/// Whether `trigger` is one a rule may name.
pub fn known_trigger(trigger: &str) -> bool {
    TRIGGERS.iter().any(|(t, _)| *t == trigger)
}

/// What one evaluation decided. Pure data, so it can be tested without a store.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// The event matched, but not the number of times required yet.
    Counting {
        /// How many matches so far, including this one.
        count: i64,
        /// How many are needed.
        required: i64,
    },
    /// The event matched the count, but the rule fired recently.
    Cooling {
        /// Minutes still to wait.
        minutes: i64,
    },
    /// The rule wants to act.
    Fire,
}

/// Judge one event against one rule.
///
/// `seen` is how many times in a row the event has already matched. The decision
/// is split out of the store so the counting, the cooldown and the off switch can
/// be tested as arithmetic rather than through a database.
pub fn judge(rule: &IntakeRule, seen: i64, now: DateTime<Utc>) -> Decision {
    if !rule.enabled {
        return Decision::Counting { count: seen, required: i64::MAX };
    }
    let required = rule.min_failures();
    let count = seen + 1;
    if count < required {
        return Decision::Counting { count, required };
    }
    if !rule.cooled_down(now) {
        let fired = rule
            .fired_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| now.signed_duration_since(t.with_timezone(&Utc)).num_minutes())
            .unwrap_or(0);
        return Decision::Cooling {
            minutes: (rule.cooldown_minutes() - fired).max(0),
        };
    }
    Decision::Fire
}

/// The run a rule asks for, in `observe` mode: a suggestion a person can turn
/// into a run, recorded without calling a model.
///
/// This is not a dry run of the agent: nothing is asked of a provider, so
/// observing costs nothing and cannot go wrong. It is a row a person reads.
pub fn observe(ctx: &Ctx, rule: &IntakeRule, detail: &str) -> Result<Option<String>> {
    let title = format!("{}: {}", rule.name, rule.goal.chars().take(60).collect::<String>());
    // The same rule proposing the same thing twice fills the inbox with noise;
    // one open suggestion per title is enough.
    if let Some(existing) = ctx.store.open_suggestion_titled(Some(&rule.project), &title)? {
        return Ok(Some(existing.id));
    }
    let steps = plan_steps(rule).unwrap_or_else(|| vec![rule.goal.clone()]);
    let accepts = ["the project's own checks pass after the change".to_string()];
    let s = ctx.store.add_suggestion(
        Some(&rule.project),
        &rule.role,
        &title,
        &format!("The rule `{}` matched: {detail}", rule.name),
        &steps,
        &accepts,
        "intake",
        Some(&rule.name),
    )?;
    ctx.events.emit_with(
        "team.suggestion",
        s.title.clone(),
        json!({ "id": s.id, "project": rule.project, "mode": "observe" }),
    );
    Ok(Some(s.id))
}

/// The steps a person reads as part of the suggestion.
fn plan_steps(rule: &IntakeRule) -> Option<Vec<String>> {
    rule.condition_object()
        .get("steps")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
}

/// The condition written as a sentence, for the team room.
pub fn condition_text(rule: &IntakeRule) -> String {
    let mut parts = Vec::new();
    let n = rule.min_failures();
    if n > 1 {
        parts.push(format!("after {n} times in a row"));
    }
    if rule.cooldown_minutes() > 0 {
        parts.push(format!("at most once every {} minutes", rule.cooldown_minutes()));
    }
    parts.sort_by_key(|p| if p.starts_with("after") { 0 } else { 1 });
    if parts.is_empty() {
        "as soon as it happens".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> IntakeRule {
        IntakeRule {
            project: "D:\\work\\shop".into(),
            name: "nightly".into(),
            trigger: "check.failed".into(),
            condition: json!({ "min_failures": 3, "cooldown_minutes": 60 }),
            goal: "Make the nightly check pass again".into(),
            template: None,
            role: "implementer".into(),
            mode: "observe".into(),
            countdown: 0,
            fired_at: None,
            enabled: true,
        }
    }

    #[test]
    fn a_rule_counts_before_it_acts() {
        let r = rule();
        let now = Utc::now();
        assert_eq!(judge(&r, 0, now), Decision::Counting { count: 1, required: 3 });
        assert_eq!(judge(&r, 1, now), Decision::Counting { count: 2, required: 3 });
        assert_eq!(judge(&r, 2, now), Decision::Fire);
    }

    #[test]
    fn a_rule_that_fired_recently_waits() {
        let mut r = rule();
        r.fired_at = Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339());
        match judge(&r, 5, Utc::now()) {
            Decision::Cooling { minutes } => assert!((49..=51).contains(&minutes), "{minutes}"),
            other => panic!("expected cooling, got {other:?}"),
        }
        // A rule without a cooldown fires again straight away.
        r.condition = json!({ "min_failures": 1, "cooldown_minutes": 0 });
        assert_eq!(judge(&r, 0, Utc::now()), Decision::Fire);
    }

    #[test]
    fn a_switched_off_rule_never_fires() {
        let mut r = rule();
        r.enabled = false;
        assert!(matches!(judge(&r, 99, Utc::now()), Decision::Counting { .. }));
    }

    #[test]
    fn a_default_rule_fires_on_the_first_match() {
        let mut r = rule();
        r.condition = json!({});
        assert_eq!(r.min_failures(), 1);
        assert_eq!(judge(&r, 0, Utc::now()), Decision::Fire);
        assert_eq!(condition_text(&r), "at most once every 60 minutes");
    }

    #[test]
    fn the_condition_reads_as_one_sentence() {
        let r = rule();
        assert_eq!(condition_text(&r), "after 3 times in a row, at most once every 60 minutes");
    }

    #[test]
    fn only_known_triggers_are_accepted() {
        assert!(known_trigger("check.failed") && known_trigger("health.down"));
        assert!(!known_trigger("vibes"), "a rule may not invent an event");
        assert_eq!(TRIGGERS.len(), 3);
    }
}
