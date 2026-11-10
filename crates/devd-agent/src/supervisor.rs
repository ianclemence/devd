//! Keeps the machine's services up without being asked.
//!
//! A project that needs a database or a PHP pool should not stop working
//! because the process behind it went away while nobody was looking. This loop
//! watches the services devd manages and is expected to keep running (one a
//! project needs, or one a person set to start always), starts a stopped one,
//! and records what it did. It never fights a person's decision to only observe
//! a service, and it never loops: three attempts inside fifteen minutes, then
//! it stops and leaves the failure visible rather than restarting forever.
//!
//! The restart goes through the registry as the system actor, so it is checked
//! by the same permission code and recorded in the same audit trail as any other
//! operation; the capability's own `control_allowed` check is the boundary that
//! keeps an "observe" decision an "observe" decision.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::{Approval, Ctx, InvokeRequest};
use devd_core::{util, DispatchOptions, Registry};

/// How often the machine is looked at. A minute is frequent enough to notice a
/// dead service and cheap enough to leave the process mostly asleep between
/// probes.
const TICK: Duration = Duration::from_secs(60);
/// Attempts for one service before it is left alone.
const MAX_ATTEMPTS: u32 = 3;
/// The window those attempts are counted in.
const COOLDOWN: Duration = Duration::from_secs(15 * 60);
/// How many incidents are kept for `health.status`.
const KEEP: usize = 40;

#[derive(Default)]
struct Watch {
    attempts: u32,
    since: Option<Instant>,
}

static INCIDENTS: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();

fn log() -> &'static Mutex<Vec<Value>> {
    INCIDENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// What self-healing has done recently, newest last. Read by `health.status`.
pub fn incidents() -> Vec<Value> {
    log().lock().map(|v| v.clone()).unwrap_or_default()
}

fn record(ctx: &Ctx, kind: &str, service: &str, detail: &str) {
    let row = json!({ "at": util::now(), "kind": kind, "service": service, "detail": detail });
    if let Ok(mut v) = log().lock() {
        v.push(row.clone());
        if v.len() > KEEP {
            v.remove(0);
        }
    }
    ctx.events.emit_with(kind, format!("{kind}: {service}"), row);
}

/// Start the loop. Only the process that owns the agents calls this.
pub fn spawn(ctx: Ctx, registry: Arc<Registry>) {
    let _ = std::thread::Builder::new()
        .name("devd-supervisor".into())
        .spawn(move || {
            let mut watches: HashMap<String, Watch> = HashMap::new();
            loop {
                std::thread::sleep(TICK);
                tick(&ctx, &registry, &mut watches);
            }
        });
}

fn tick(ctx: &Ctx, registry: &Registry, watches: &mut HashMap<String, Watch>) {
    let Ok(services) = devd_services::list(ctx) else {
        return;
    };
    let now = Instant::now();
    for s in services {
        if !s.installed || s.running {
            watches.remove(&s.name);
            continue;
        }
        // Only what devd is expected to keep up, not every installed program.
        if !(s.required || s.startup_mode == "always") {
            continue;
        }
        // "Observe" is a person's decision, not a failure to repair.
        if devd_stacks::reconcile::control_allowed(ctx, &s.name).is_err() {
            continue;
        }
        let w = watches.entry(s.name.clone()).or_default();
        if w.since
            .map(|t| now.duration_since(t) > COOLDOWN)
            .unwrap_or(true)
        {
            w.attempts = 0;
            w.since = Some(now);
        }
        if w.attempts >= MAX_ATTEMPTS {
            continue;
        }
        w.attempts += 1;
        record(
            ctx,
            "service.restarting",
            &s.name,
            &format!("attempt {} of {MAX_ATTEMPTS}", w.attempts),
        );
        let request = InvokeRequest {
            id: "toolchain.service".into(),
            args: json!({ "tool": s.name, "action": "start" }),
            dry_run: false,
        };
        match registry.dispatch_with(ctx, &request, Approval::Granted, &DispatchOptions::default()) {
            Ok(result) => {
                let outcome = result.outcome;
                let running = outcome
                    .as_ref()
                    .map(|o| o.data["running"].as_bool().unwrap_or(true))
                    .unwrap_or(true);
                if running {
                    record(ctx, "service.recovered", &s.name, "started and answering");
                    watches.remove(&s.name);
                } else {
                    let why = outcome.map(|o| o.message).unwrap_or_default();
                    record(ctx, "service.restart_failed", &s.name, &why);
                }
            }
            Err(e) => record(ctx, "service.restart_failed", &s.name, &e.to_string()),
        }
    }
}
