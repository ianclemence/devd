//! What devd does between projects: work it promised itself, and the rules a
//! project wrote for what it would like to be told about.
//!
//! Two things live here, and both run only in the process that owns the agents
//! (see [`crate::host`]) because both can start work:
//!
//! * **The effects the team owns.** A background review and a scheduled start
//!   are rows in the durable queue. `devd-ops` drains the ones that reach nothing
//!   but a webhook or an inbox; these two need the agent runtime, so they are
//!   drained here, with the same backoff and the same health entry.
//! * **Intake rules.** A project's written rule - "when the nightly check has
//!   failed three times, consider a run that fixes it" - is evaluated against
//!   what devd already observed. In `observe` mode it records a suggestion and
//!   stops there. In `auto` it starts a run, which still waits for plan approval,
//!   and never in production without a person's answer.
//!
//! Both are ordinary code: no model is called to decide whether to look, so the
//! cost of watching nothing happen is nothing.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, IntakeRule, Result};

use crate::chat::Hub;
use crate::intake;
use crate::team::{StartRequest, Team};

/// How often the guardian looks. It is cheap: a handful of database reads.
const TICK: Duration = Duration::from_secs(30);

/// The effect kinds the agent runtime owns.
const OWNED: &[&str] = &["review.start", "team.schedule"];

/// Ask the guardian to run now, instead of at its next tick.
///
/// A review a person just asked for should not wait up to half a minute for the
/// loop. The pass is still the same pass: nothing about it is special because it
/// was asked for early.
pub fn nudge(ctx: Ctx) {
    std::thread::spawn(move || {
        let Ok(queued) = ctx.store.due_effects(&util::now(), 10) else {
            return;
        };
        for e in queued.into_iter().filter(|e| OWNED.contains(&e.kind.as_str())) {
            if e.kind == "review.start" {
                let _ = run_review(&ctx, &e.payload).map(|()| ctx.store.settle_effect(&e.kind, &e.key));
            }
        }
    });
}

/// Start the guardian, once per process.
pub fn spawn(team: Arc<Team>, hub: Arc<Hub>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let ctx = team.context().clone();
        // A conversation stopped by a provider limit continues by itself once
        // the reset time has passed. Nothing else about it is special.
        hub.resume_limited();
        drain_effects(&team);
        let _ = evaluate_intake(&ctx, &team);
    });
}

/// Run one pass of the guardian, for `devd doctor` and for a test.
pub fn tick_once(team: &Arc<Team>, hub: &Arc<Hub>) {
    hub.resume_limited();
    drain_effects(team);
    let _ = evaluate_intake(team.context(), team);
}

/// Run the due effects that need the team.
fn drain_effects(team: &Arc<Team>) {
    let ctx = team.context();
    let Ok(due) = ctx.store.due_effects(&util::now(), 10) else {
        return;
    };
    for e in due.into_iter().filter(|e| OWNED.contains(&e.kind.as_str())) {
        let outcome = match e.kind.as_str() {
            "review.start" => run_review(ctx, &e.payload),
            "team.schedule" => start_schedule(team, &e.payload),
            _ => Ok(()),
        };
        match outcome {
            Ok(()) => {
                let _ = ctx.store.settle_effect(&e.kind, &e.key);
            }
            Err(why) => {
                let secs = 2i64.saturating_mul(1 << e.attempts.min(8)).min(600);
                let at = (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339();
                let _ = ctx.store.fail_effect(&e.kind, &e.key, &why, &at);
                ctx.events.emit_with(
                    "effect.failed",
                    format!("{} could not run: {why}", e.kind),
                    json!({ "kind": e.kind, "key": e.key, "attempts": e.attempts + 1 }),
                );
            }
        }
    }
}

fn run_review(ctx: &Ctx, payload: &Value) -> std::result::Result<(), String> {
    let id = payload["id"].as_str().unwrap_or("");
    let project = payload["project"].as_str().unwrap_or("");
    let args: crate::workflows::ReviewRun = serde_json::from_value(payload["args"].clone())
        .map_err(|e| format!("the stored review no longer matches what a review needs: {e}"))?;
    let mut c = ctx.clone();
    // The daemon is acting as itself, not as whoever started the review.
    c.actor = devd_core::capability::Actor::system();
    crate::workflows::finish_background_review(&c, &args, id, project);
    Ok(())
}

fn start_schedule(team: &Arc<Team>, payload: &Value) -> std::result::Result<(), String> {
    let name = payload["name"].as_str().unwrap_or("");
    let store = &team.context().store;
    let Some(schedule) = store.get_schedule(name).map_err(|e| e.to_string())? else {
        return Err(format!("the schedule `{name}` no longer exists"));
    };
    if !schedule.enabled {
        return Err(format!("the schedule `{name}` was switched off"));
    }
    crate::schedule::start_run(team, &schedule)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// What devd has observed for a project, which is what a rule looks at.
///
/// Only facts devd itself recorded are offered: a site is down because a health
/// check said so, a check failed because a project task exited non-zero. Nothing
/// here is a model's opinion, so a rule cannot be talked into firing.
fn observed(ctx: &Ctx, trigger: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    match trigger {
        "health.down" => {
            for item in ctx.store.ops_list("health", None).unwrap_or_default() {
                if item.data["state"].as_str() == Some("down") {
                    out.push((
                        item.data["site"].as_str().unwrap_or(&item.site).to_string(),
                        item.data["last_error"]
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| "it is not answering".into()),
                    ));
                }
            }
        }
        "check.failed" => {
            // The latest run of each project says whether its checks passed.
            for run in ctx.store.list_runs(60).unwrap_or_default() {
                if !matches!(run.state.as_str(), "failed" | "unverified") {
                    continue;
                }
                let Some(project) = run.project.clone() else { continue };
                let failed = run.error.clone().unwrap_or_else(|| format!("the run ended {}", run.state));
                out.push((project, failed));
            }
        }
        "schedule.missed" => {
            for s in ctx.store.list_schedules().unwrap_or_default() {
                if let Some(err) = s.last_error.as_deref() {
                    if err.starts_with("skipped") {
                        out.push((s.project.clone(), err.to_string()));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Figure out which project a fact belongs to. A site is not a project, so the
/// site's folder is used, and a fact that names no folder matches no rule.
fn project_of(ctx: &Ctx, trigger: &str, subject: &str) -> Option<String> {
    match trigger {
        "health.down" => ctx
            .store
            .list_sites()
            .ok()?
            .into_iter()
            .find(|s| s.name == subject)
            .map(|s| s.root),
        _ => Some(subject.to_string()),
    }
}

/// Evaluate every rule against what was observed.
///
/// Returns how many rules matched. A rule in `observe` mode records a suggestion;
/// a rule in `auto` starts a run, unless the machine is in production or the same
/// rule is already showing an open proposal.
pub fn evaluate_intake(ctx: &Ctx, team: &Arc<Team>) -> Result<usize> {
    let mut fired = 0;
    for trigger in intake::TRIGGERS.iter().map(|(t, _)| *t) {
        let rules = ctx.store.intake_rules_for(trigger)?;
        if rules.is_empty() {
            continue;
        }
        let facts = observed(ctx, trigger);
        for rule in rules {
            if !rule.enabled {
                continue;
            }
            let matched = facts
                .iter()
                .find(|(subject, _)| project_of(ctx, trigger, subject).as_deref() == Some(rule.project.as_str()));
            let Some((subject, detail)) = matched else {
                // The condition is no longer true: the count starts again, so a
                // rule that has to see three failures in a row sees three in a row.
                if rule.countdown != 0 {
                    let _ = ctx.store.record_intake(&rule.project, &rule.name, 0, false);
                }
                continue;
            };
            match intake::judge(&rule, rule.countdown, chrono::Utc::now()) {
                intake::Decision::Counting { count, .. } => {
                    let _ = ctx.store.record_intake(&rule.project, &rule.name, count, false);
                }
                intake::Decision::Cooling { .. } => {}
                intake::Decision::Fire => {
                    fired += 1;
                    // One observation is one match, whatever it is: the count
                    // starts again after a rule acts, so a rule that needs three
                    // in a row sees three separate events rather than one event
                    // counted three times.
                    let _ = ctx.store.record_intake(&rule.project, &rule.name, 0, true);
                    if trigger == "check.failed" && subject == &rule.project {
                        // A failed run is the run itself; acting on it again would
                        // loop. Only a health check or a clock can start work.
                        continue;
                    }
                    if rule.mode == "auto" && ctx.store.environment_mode() != "production" {
                        match team.start(
                            ctx,
                            &StartRequest {
                                goal: rule.goal.clone(),
                                project: Some(rule.project.clone()),
                                new_project: None,
                                parallel: 2,
                                auto_approve: false,
                                done_when: Vec::new(),
                                max_rounds: 2,
                            },
                        ) {
                            Ok(run) => {
                                ctx.events.emit_with(
                                    "team.intake",
                                    format!("{}: a run was started", rule.name),
                                    json!({ "rule": rule.name, "run": run.id, "trigger": trigger }),
                                );
                            }
                            Err(e) => {
                                let _ = ctx.store.fail_effect(
                                    "team.suggestion",
                                    &format!("{}:{}", rule.project, rule.name),
                                    &e.to_string(),
                                    &(chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339(),
                                );
                            }
                        }
                    } else {
                        // Observe, or production: say what would have happened and
                        // let a person decide.
                        intake::observe(ctx, &rule, detail)?;
                    }
                }
            }
        }
    }
    Ok(fired)
}

/// The rules written for a project, in the words the team room shows.
pub fn rules_for(ctx: &Ctx, project: &str) -> Vec<Value> {
    ctx.store
        .list_intake_rules(Some(project))
        .unwrap_or_default()
        .iter()
        .map(|r: &IntakeRule| {
            json!({
                "name": r.name,
                "trigger": r.trigger,
                "trigger_text": intake::TRIGGERS.iter().find(|(t, _)| *t == r.trigger.as_str()).map(|(_, s)| *s).unwrap_or("an unknown event"),
                "condition_text": intake::condition_text(r),
                "goal": r.goal,
                "mode": r.mode,
                "enabled": r.enabled,
                "countdown": r.countdown,
                "min_failures": r.min_failures(),
            })
        })
        .collect()
}
