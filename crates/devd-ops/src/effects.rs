//! Work that must happen outside the transaction that decided it.
//!
//! A health check that flips to `down`, a review nobody is waiting for, a
//! schedule that fires: each of them announced its result by calling out to the
//! network, writing an inbox row, or starting a run, from inside the pass that
//! noticed it. A failure there was a line in a log file that scrolls away, and the
//! person who should have been told was not.
//!
//! An **effect** is the same work as a row: engine and key, a payload, attempts,
//! the last error and when to try again. The pass writes the row and commits; the
//! runner picks it up, calls the handler, and settles it or backs off. So a dead
//! webhook is a visible row that recovers when the webhook comes back, and the
//! health screen can say which kind of work is not going out.
//!
//! Three rules keep it from becoming a job system of its own:
//!
//! * **One row per kind and key.** Queuing the same thing again replaces the
//!   payload instead of stacking a second copy, so a rule that fires every minute
//!   does not fill the queue.
//! * **Bounded backoff.** Two seconds, doubling to ten minutes. After a week of
//!   failing it is dropped with the reason, so a permanently broken target does
//!   not retry forever.
//! * **The health entry clears itself.** A kind reports an error only after one of
//!   its effects failed three times in a row, and stops reporting on the next
//!   success. A one-off network blip is not an unhealthy instance.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, Result};

/// The kinds this process knows how to handle.
pub const KINDS: &[&str] = &["notify.send", "review.start", "team.schedule", "team.suggestion"];

/// The first retry, and the ceiling of the doubling.
const FIRST_BACKOFF: i64 = 2;
const MAX_BACKOFF: i64 = 600;

fn retry_at(attempts: i64) -> String {
    let secs = FIRST_BACKOFF.saturating_mul(1i64 << attempts.min(16)).min(MAX_BACKOFF);
    (chrono::Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339()
}

/// Queue a message for the notification targets.
///
/// The row is written by `notify.send` even when there are no targets, so
/// "nobody asked to hear about this" is a fact the health screen can show rather
/// than a silent no-op.
pub fn queue_notify(ctx: &Ctx, event: &str, site: &str, message: &str, data: &Value) -> Result<()> {
    ctx.store.enqueue_effect(
        "notify.send",
        &format!("{event}:{site}:{}", util::now()),
        &json!({ "event": event, "site": site, "message": message, "data": data }),
    )
}

/// Queue a background review of a project. The payload is what `review.start`
/// was called with.
pub fn queue_review(ctx: &Ctx, id: &str, args: &Value, project: &str) -> Result<()> {
    ctx.store
        .enqueue_effect("review.start", id, &json!({ "id": id, "args": args, "project": project }))
}

/// Queue the start of a scheduled run, so a schedule that was due is not lost
/// when the process is busy or restarting.
pub fn queue_schedule(ctx: &Ctx, name: &str) -> Result<()> {
    ctx.store.enqueue_effect("team.schedule", name, &json!({ "name": name }))
}

/// What a handler did.
pub enum Handled {
    /// Done: settle the row.
    Done,
    /// Not done, and worth trying again later.
    Retry(String),
    /// Not done, and never will be as queued: drop the row with the reason.
    Drop(String),
}

/// Run every effect that is due. Returns how many were attempted.
///
/// Each kind is handled by the module that owns it, through a match here rather
/// than a registry, because the daemon must be able to work with no model
/// configured and no registry built.
///
/// `start_schedule` is the crate's own path to starting a scheduled run, supplied
/// by the caller so the queue does not need to know about the agent team.
pub fn tick(ctx: &Ctx, start_schedule: &dyn Fn(&Ctx, &devd_core::Schedule) -> std::result::Result<(), String>) -> usize {
    let now = util::now();
    let Ok(due) = ctx.store.due_effects(&now, 25) else {
        return 0;
    };
    let mut attempted = 0;
    for e in due {
        attempted += 1;
        let outcome = match e.kind.as_str() {
            "notify.send" => deliver_notify(ctx, &e.payload),
            "team.schedule" => start_scheduled(ctx, &e.payload, start_schedule),
            // The review is run by the agent crate, which this crate does not
            // depend on. It reads the row itself (see `workflows::tick_effects`),
            // so an unknown kind here is left for its owner rather than dropped.
            "review.start" | "team.suggestion" => continue,
            other => Handled::Drop(format!("nothing in this process handles `{other}`")),
        };
        match outcome {
            Handled::Done => {
                let _ = ctx.store.settle_effect(&e.kind, &e.key);
            }
            Handled::Retry(why) => {
                if e.attempts >= 40 {
                    let _ = ctx.store.drop_effect(&e.kind, &e.key, &format!("gave up after {} attempts: {why}", e.attempts + 1));
                } else {
                    let _ = ctx.store.fail_effect(&e.kind, &e.key, &why, &retry_at(e.attempts));
                }
            }
            Handled::Drop(why) => {
                let _ = ctx.store.drop_effect(&e.kind, &e.key, &why);
            }
        }
    }
    attempted
}

fn deliver_notify(ctx: &Ctx, payload: &Value) -> Handled {
    let event = payload["event"].as_str().unwrap_or("");
    let site = payload["site"].as_str().unwrap_or("");
    let message = payload["message"].as_str().unwrap_or("");
    let data = payload.get("data").cloned().unwrap_or(Value::Null);
    let Ok(items) = ctx.store.ops_list(crate::notify::KIND, None) else {
        return Handled::Retry("the notification targets could not be read".into());
    };
    let mut errors = Vec::new();
    let mut delivered = 0;
    for item in items {
        let wants = item.data["events"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(event)));
        let scoped = item.data["site"].as_str().is_none_or(|s| s == site);
        if !wants || !scoped {
            continue;
        }
        match crate::notify::deliver(ctx, &item, event, site, message, &data) {
            Ok(()) => delivered += 1,
            Err(why) => errors.push(format!("{} ({}): {why}", item.id, item.data["channel"].as_str().unwrap_or(""))),
        }
    }
    if errors.is_empty() {
        let _ = delivered;
        Handled::Done
    } else {
        // One target failing must not stop the others, but the row is not done
        // until none is failing.
        Handled::Retry(errors.join("; "))
    }
}

/// Start a scheduled run through the path the caller supplies. It is reached here
/// so a schedule whose start failed is tried again rather than lost.
fn start_scheduled(ctx: &Ctx, payload: &Value, start: &dyn Fn(&Ctx, &devd_core::Schedule) -> std::result::Result<(), String>) -> Handled {
    let name = payload["name"].as_str().unwrap_or("");
    let Ok(Some(schedule)) = ctx.store.get_schedule(name) else {
        return Handled::Drop(format!("the schedule `{name}` no longer exists"));
    };
    if !schedule.enabled {
        return Handled::Drop(format!("the schedule `{name}` was switched off"));
    }
    match start(ctx, &schedule) {
        Ok(()) => Handled::Done,
        Err(e) => Handled::Retry(e),
    }
}

/// How each kind of effect is doing, for the health screen.
pub fn health(ctx: &Ctx) -> Vec<Value> {
    ctx.store
        .effect_health()
        .unwrap_or_default()
        .into_iter()
        .map(|h| {
            json!({
                "kind": h.kind,
                "pending": h.pending,
                "failing": h.failing,
                "dropped": h.dropped,
                "last_error": h.last_error,
                // A kind is unwell when something has failed repeatedly; a
                // backlog that is moving is ordinary traffic.
                "state": if h.failing > 0 { "error" } else if h.pending > 0 { "working" } else { "ok" },
            })
        })
        .collect()
}

/// How long a run of the queue takes, so the pass can be bounded like the others.
pub fn timed(ctx: &Ctx, start: &dyn Fn(&Ctx, &devd_core::Schedule) -> std::result::Result<(), String>) -> (usize, Duration) {
    let started = Instant::now();
    let n = tick(ctx, start);
    (n, started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_and_stops_at_the_ceiling() {
        let at = |n: i64| {
            chrono::DateTime::parse_from_rfc3339(&retry_at(n))
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let gap = |n: i64| (at(n) - chrono::Utc::now()).num_seconds();
        assert!((gap(0) - 2).abs() <= 2);
        assert!((gap(3) - 16).abs() <= 2);
        assert!(gap(30) <= 600 + 2, "it never waits longer than the ceiling");
        // A long history does not overflow the shift.
        assert!(gap(64) <= 600 + 2);
    }

    #[test]
    fn an_unknown_effect_is_dropped_rather_than_retried_forever() {
        let store = std::sync::Arc::new(devd_core::Store::open_in_memory().unwrap());
        store.enqueue_effect("nobody.handles.this", "k", &json!({})).unwrap();
        let dir = std::env::temp_dir().join(format!("devd-eff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = std::sync::Arc::new(devd_core::Paths::rooted(&dir));
        let ctx = Ctx {
            store: store.clone(),
            paths,
            settings: std::sync::Arc::new(devd_core::Settings::default()),
            events: devd_core::EventBus::new(),
            actor: devd_core::Actor::system(),
            run: None,
            dry_run: false,
        };
        let never = |_: &Ctx, _: &devd_core::Schedule| Err("not used".to_string());
        tick(&ctx, &never);
        let later = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
        assert!(
            store.due_effects(&later, 10).unwrap().iter().all(|e| e.kind != "nobody.handles.this"),
            "an effect nothing handles is dropped, not retried"
        );
    }
}
