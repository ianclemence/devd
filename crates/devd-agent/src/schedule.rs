//! Schedules: team runs devd starts on its own, on a clock.
//!
//! A schedule is a standing instruction a person gave: "every weekday at 09:00,
//! run the security review on this project". Every twenty seconds the process that
//! owns the agents (see [`crate::host`]) starts the ones that are due.
//!
//! Three rules keep a clock from becoming a runaway bill:
//!
//! * A schedule never starts a run while the last run it started is still going.
//! * An occurrence missed because devd was not running is skipped when it is more
//!   than two hours old, and the schedule says so, rather than firing a stale run.
//! * Plan approval is waived only when the person who made the schedule said so,
//!   and never in production mode; otherwise each run waits for a person, and the
//!   inbox says a plan is ready.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};

use devd_core::schedule::{next_after, parse};
use devd_core::{Result, Schedule, TeamRun};

use crate::team::{StartRequest, Team};
use crate::templates;

/// How often due schedules are looked for.
const TICK: Duration = Duration::from_secs(20);

/// An occurrence older than this when devd notices it is skipped.
const STALE_AFTER: chrono::Duration = chrono::Duration::hours(2);

fn active(state: &str) -> bool {
    matches!(state, "planning" | "running" | "awaiting_plan" | "paused")
}

/// When a schedule is next due after `from`, as an RFC 3339 UTC string.
pub fn next_due(spec: &str, from: DateTime<Local>) -> Result<Option<String>> {
    Ok(next_after(&parse(spec)?, from).map(|d| d.with_timezone(&Utc).to_rfc3339()))
}

/// Start the run a schedule describes.
pub fn start_run(team: &Arc<Team>, schedule: &Schedule) -> Result<TeamRun> {
    let ctx = team.context();
    let (goal, parallel, mut done_when) = match (&schedule.template, &schedule.goal) {
        (Some(id), _) => {
            let template = templates::get(&ctx.paths.root().join("templates"), id)?;
            let values = schedule.params.as_object().cloned().unwrap_or_default();
            (template.goal(&values)?, template.parallel, template.done_when.clone())
        }
        (None, Some(goal)) => (goal.clone(), 2, Vec::new()),
        (None, None) => return Err(devd_core::DevdError::invalid("the schedule has neither a template nor a goal")),
    };
    done_when.extend(schedule.done_when.iter().cloned());
    let production = ctx.store.environment_mode() == "production";
    team.start(
        ctx,
        &StartRequest {
            goal,
            project: Some(schedule.project.clone()),
            new_project: None,
            parallel,
            auto_approve: schedule.auto_approve && !production,
            done_when,
            max_rounds: 2,
        },
    )
}

/// Start every schedule that is due. Returns how many runs started.
pub fn tick(team: &Arc<Team>) -> Result<usize> {
    let store = team.context().store.clone();
    let now = Utc::now();
    let mut started = 0;
    for schedule in store.due_schedules(&now.to_rfc3339())? {
        let next = match next_due(&schedule.spec, Local::now()) {
            Ok(next) => next,
            Err(e) => {
                // A schedule that cannot be read again is switched off, not retried every tick.
                store.set_schedule_enabled(&schedule.name, false, None)?;
                store.record_schedule_attempt(&schedule.name, None, Some(&format!("switched off: {e}")), None)?;
                continue;
            }
        };
        let overdue = schedule
            .next_run_at
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|due| now.signed_duration_since(due.with_timezone(&Utc)));
        if overdue.map(|o| o > STALE_AFTER).unwrap_or(false) {
            store.record_schedule_attempt(&schedule.name, None, Some("skipped: devd was not running when it was due"), next.as_deref())?;
            continue;
        }
        let still_going = schedule
            .last_run_id
            .as_deref()
            .and_then(|id| store.get_run(id).ok().flatten())
            .map(|run| active(&run.state))
            .unwrap_or(false);
        if still_going {
            store.record_schedule_attempt(&schedule.name, None, Some("skipped: the run it started last time is still going"), next.as_deref())?;
            continue;
        }
        match start_run(team, &schedule) {
            Ok(run) => {
                store.add_team_event(&run.id, None, "note", &serde_json::json!({ "text": format!("Started by the schedule `{}` ({}).", schedule.name, schedule.spec) }))?;
                store.record_schedule_attempt(&schedule.name, Some(&run.id), None, next.as_deref())?;
                started += 1;
            }
            Err(e) => store.record_schedule_attempt(&schedule.name, None, Some(&e.to_string()), next.as_deref())?,
        }
    }
    Ok(started)
}

/// Look for due schedules for as long as the process lives. Once per process.
pub fn spawn_loop(team: Arc<Team>) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let _ = tick(&team);
    });
}
