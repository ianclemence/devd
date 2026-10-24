//! The operational health model.
//!
//! One question, answered the same way on every surface: *can I rely on devd
//! right now, and if not, what is wrong and what do I do?*
//!
//! Each component reports one of seven states, and says whether devd needs it
//! for what the person is doing (`required`). Optional components that are not
//! set up never make devd look unhealthy, they become setup steps. Detection is
//! never health: a service that merely exists is not "healthy" until it answers.
//!
//! | state              | meaning                                                         |
//! |--------------------|-----------------------------------------------------------------|
//! | `healthy`          | checked, and doing what it should                               |
//! | `degraded`         | working, but something around it is wrong                       |
//! | `starting`         | on its way up; not an error yet                                 |
//! | `unavailable`      | a dependency devd needs cannot be reached right now             |
//! | `critical`         | devd cannot do its job until this is fixed                      |
//! | `setup_incomplete` | not configured yet; nothing is broken                           |
//! | `unknown`          | could not be determined                                         |

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::doctor::{self, Level};
use devd_core::{util, Result};

use crate::secrets;

#[derive(Debug)]
struct Component {
    id: String,
    label: String,
    group: &'static str,
    state: &'static str,
    required: bool,
    detail: String,
}

impl Component {
    fn new(id: &str, label: &str, group: &'static str, state: &'static str, required: bool, detail: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), group, state, required, detail: detail.into() }
    }
}

fn label_of(name: &str) -> &'static str {
    match name {
        "home" => "devd data directory",
        "home.conflict" => "Two devd data directories",
        "home.other" => "Another devd data directory",
        "layout" => "Directory layout",
        "settings" => "Settings",
        "state" => "State database",
        "www_root" => "Web root",
        "git" => "Git",
        "port.http" => "HTTP port",
        "port.https" => "HTTPS port",
        _ => "Check",
    }
}

fn explain(id: &str, state: &str, detail: &str) -> (String, Value) {
    let (text, fix): (String, Value) = match id {
        "git" => ("Git is needed for every repository feature, and for the isolated workspaces the agent team uses.".into(), json!({ "label": "Install Git", "capability": "toolchain.install", "args": { "tool": "git" } })),
        "port.http" | "port.https" => (
            "Nothing is wrong if the program named above is devd's own nginx or Apache: it is expected to hold this port. If it is another program, start your sites on another port or stop that program.".into(),
            json!({ "label": "See what is using it", "view": "overview" }),
        ),
        "state" => (format!("devd's database could not be opened or is damaged ({detail}). Nothing is saved until this is fixed. Check free disk space and permissions on the devd directory."), json!({ "label": "Open diagnostics", "view": "system" })),
        "settings" => ("The settings file is invalid. Fix the file named in the detail.".into(), json!({ "label": "Open diagnostics", "view": "system" })),
        "layout" | "home" => ("devd cannot use its data directory. Check that it exists and is writable.".into(), json!({ "label": "Open diagnostics", "view": "system" })),
        "www_root" => ("The folder devd serves sites from is missing or not writable.".into(), json!({ "label": "Open diagnostics", "view": "system" })),
        "ai.provider" => match state {
            "setup_incomplete" => ("The built-in assistant and agent team need a model provider and an API key. Nothing else in devd depends on it.".into(), json!({ "label": "Set up a provider", "view": "providers" })),
            "unavailable" => ("The model provider did not accept the last request. Agents cannot work until it answers. Check the key, the account's credit and the provider's status page.".into(), json!({ "label": "Test the provider", "view": "providers" })),
            _ => ("Whether the provider works has not been checked yet.".into(), json!({ "label": "Test the provider", "view": "providers" })),
        },
        i if i.starts_with("service.") => (
            if state == "starting" { "The process is up but its port is not answering yet. Give it a moment; if it stays like this, read its log.".to_string() } else { "You asked for this service to be running but it is not. It may have crashed or been stopped from outside devd.".to_string() },
            json!({ "label": "Open services", "view": "services" }),
        ),
        _ => (if state == "healthy" { "Working.".into() } else { "See the detail.".into() }, json!({ "label": "Open diagnostics", "view": "system" })),
    };
    (text, fix)
}

/// Compute the health of everything devd depends on.
/// Whether anything is running the scheduled work, and whether there is any to run.
///
/// Cron jobs, database backups, scheduled deploys and health checks are run by a loop inside a
/// long-lived devd process. With none open the jobs simply do not happen, and nothing else says so.
fn background_jobs(ctx: &Ctx) -> Component {
    let count = |kind: &str| ctx.store.ops_list(kind, None).map(|rows| rows.len()).unwrap_or(0);
    let jobs = count("cron") + count("db_backup") + count("deploy_schedule") + count("health");
    let holder = devd_core::process::live_devd_holder(&ctx.paths.root().join("ops-host.pid"));
    let label = "Scheduled jobs";
    match (holder, jobs) {
        (Some(pid), _) => Component::new("jobs", label, "core", "healthy", false, format!("{jobs} job(s); devd (pid {pid}) is running them")),
        (None, 0) => Component::new("jobs", label, "core", "healthy", false, "none scheduled; cron jobs, backups, scheduled deploys and health checks run only while the window, `devd ui` or `devd serve` is open"),
        (None, n) => Component::new(
            "jobs",
            label,
            "core",
            "degraded",
            false,
            format!("{n} job(s) are scheduled but nothing is running them: cron jobs, backups, scheduled deploys and health checks run only while the window, `devd ui` or `devd serve` is open, and none is"),
        ),
    }
}

pub fn status(ctx: &Ctx) -> Result<Value> {
    let started = std::time::Instant::now();
    let mut comps: Vec<Component> = Vec::new();

    // ---- devd itself and the tools it needs
    let mut checks = doctor::core_checks_for(&ctx.paths, &ctx.settings, Some(&ctx.store));
    checks.push(devd_git::doctor_check(ctx.settings.git_executable.as_deref()));
    for c in &checks {
        let (state, required, group) = match (c.name.as_str(), c.level) {
            ("git", Level::Fail) => ("unavailable", true, "tools"),
            ("git", Level::Info) => ("degraded", true, "tools"),
            ("git", Level::Ok) => ("healthy", true, "tools"),
            (n, Level::Info) if n.starts_with("home.") => ("degraded", false, "core"),
            (n, Level::Info) if n.starts_with("port.") => ("degraded", false, "server"),
            (n, Level::Ok) if n.starts_with("port.") => ("healthy", false, "server"),
            (n, Level::Fail) if n.starts_with("port.") => ("unknown", false, "server"),
            (_, Level::Fail) => ("critical", true, "core"),
            (_, Level::Info) => ("degraded", true, "core"),
            (_, Level::Ok) => ("healthy", true, "core"),
        };
        if c.name == "home" {
            continue;
        }
        comps.push(Component::new(&c.name, label_of(&c.name), group, state, required, c.detail.clone()));
    }

    // ---- services devd manages
    for s in devd_services::list(ctx).unwrap_or_default() {
        if !s.installed {
            continue;
        }
        let wanted = s.desired_state == "running";
        let (state, detail) = if s.running && s.listening {
            ("healthy", format!("running (pid {}), answering on port {}", s.pid.unwrap_or(0), s.port.unwrap_or(0)))
        } else if s.running {
            ("starting", format!("process is up (pid {}) but port {} is not answering yet", s.pid.unwrap_or(0), s.port.unwrap_or(0)))
        } else if wanted {
            ("degraded", "should be running but no process was found".to_string())
        } else {
            ("healthy", "stopped, as you left it".to_string())
        };
        comps.push(Component::new(&format!("service.{}", s.name), &s.name, "server", state, wanted, detail));
    }

    // ---- the loop that runs scheduled work
    comps.push(background_jobs(ctx));

    // ---- AI provider
    let providers = ctx.store.list_providers()?;
    let busy = ctx.store.list_threads(50)?.iter().any(|t| t.state == "running" || t.state == "waiting_approval")
        || ctx.store.list_runs(20)?.iter().any(|r| matches!(r.state.as_str(), "running" | "planning"));
    match providers.iter().find(|p| p.is_default && p.enabled).or_else(|| providers.iter().find(|p| p.enabled)) {
        None => comps.push(Component::new("ai.provider", "AI provider", "ai", "setup_incomplete", false, "none configured")),
        Some(p) => {
            let has_key = secrets::read_key(&ctx.paths, p).is_some() || p.kind == "openai-compatible";
            if !has_key {
                comps.push(Component::new("ai.provider", "AI provider", "ai", "setup_incomplete", false, format!("{} has no API key", p.label)));
            } else {
                match ctx.store.get_provider_check(&p.id)? {
                    None => comps.push(Component::new("ai.provider", "AI provider", "ai", "unknown", false, format!("{} has a key but was never tested", p.label))),
                    Some(c) if c.ok => comps.push(Component::new("ai.provider", "AI provider", "ai", "healthy", false, format!("{} answered ({})", p.label, util::truncate(&c.message, 80)))),
                    Some(c) => comps.push(Component::new("ai.provider", "AI provider", "ai", "unavailable", busy, format!("{}: {}", p.label, util::truncate(&c.message, 160)))),
                }
            }
        }
    }

    if ctx.store.environment_mode() == "production" {
        comps.push(Component::new("environment", "Environment", "core", "healthy", false, "production mode: agents need approval for every change"));
    }

    // ---- roll up
    let required: Vec<&Component> = comps.iter().filter(|c| c.required).collect();
    let any = |states: &[&str]| required.iter().any(|c| states.contains(&c.state));
    let state = if any(&["critical"]) {
        "critical"
    } else if any(&["unavailable", "degraded"]) {
        "degraded"
    } else if comps.iter().any(|c| c.state == "starting") {
        "starting"
    } else if any(&["unknown"]) {
        "unknown"
    } else if any(&["setup_incomplete"]) {
        "setup_incomplete"
    } else {
        "healthy"
    };
    let notes = comps.iter().filter(|c| !c.required && c.state != "healthy").count();
    let setup: Vec<Value> = comps.iter().filter(|c| c.state == "setup_incomplete").map(|c| {
        let (meaning, fix) = explain(&c.id, c.state, &c.detail);
        json!({ "id": c.id, "label": c.label, "detail": c.detail, "meaning": meaning, "fix": fix })
    }).collect();
    let bad: Vec<&&Component> = required.iter().filter(|c| !matches!(c.state, "healthy" | "starting")).collect();
    let headline = match state {
        "critical" => format!("{} broken: {}", if bad.len() == 1 { "1 thing is".to_string() } else { format!("{} things are", bad.len()) }, bad.first().map(|c| c.label.clone()).unwrap_or_default()),
        "degraded" => format!("Working, with {} to fix: {}", if bad.len() == 1 { "1 problem".to_string() } else { format!("{} problems", bad.len()) }, bad.first().map(|c| c.label.clone()).unwrap_or_default()),
        "starting" => "Starting up".to_string(),
        "unknown" => "Could not check everything".to_string(),
        "setup_incomplete" => "Setup is not finished".to_string(),
        _ if notes > 0 => format!("Healthy, {} optional note{}", notes, if notes == 1 { "" } else { "s" }),
        _ => "Healthy".to_string(),
    };

    // ---- what is waiting for a person (attention, not health)
    let approvals = ctx.store.list_requests(true, None, 200)?.len();
    let questions = ctx.store.list_inbox(true, 200)?.iter().filter(|m| m.needs_reply).count();
    let mut attention = Vec::new();
    if approvals > 0 {
        attention.push(json!({ "id": "approvals", "count": approvals, "label": format!("{approvals} operation(s) waiting for your approval"), "view": "inbox" }));
    }
    if questions > 0 {
        attention.push(json!({ "id": "questions", "count": questions, "label": format!("{questions} agent question(s) waiting for your reply"), "view": "inbox" }));
    }

    let rows: Vec<Value> = comps
        .iter()
        .map(|c| {
            let (meaning, fix) = explain(&c.id, c.state, &c.detail);
            let level = match c.state { "healthy" => "ok", "critical" | "unavailable" | "degraded" if c.required => "fail", _ => "info" };
            json!({ "name": c.id, "label": c.label, "group": c.group, "state": c.state, "required": c.required, "level": level, "detail": c.detail, "meaning": meaning, "fix": fix })
        })
        .collect();
    let level = match state { "healthy" => "ok", "critical" | "degraded" => "problem", _ => "attention" };
    Ok(json!({
        "state": state, "level": level, "headline": headline, "notes": notes,
        "setup": setup, "attention": attention, "checks": rows,
        "checked_at": util::now(), "took_ms": started.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (Ctx, std::path::PathBuf) {
        use devd_core::capability::Actor;
        use devd_core::config::Settings;
        use devd_core::events::EventBus;
        use devd_core::paths::Paths;
        use devd_core::state::Store;
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("devd-jobs-{}-{}", std::process::id(), util::new_id()));
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let store = Store::open_in_memory().unwrap();
        (Ctx { store: Arc::new(store), paths: Arc::new(paths), settings: Arc::new(Settings::default()), events: EventBus::new(), actor: Actor::system(), dry_run: false }, dir)
    }

    #[test]
    fn scheduled_jobs_with_nothing_running_them_are_reported_not_assumed_to_run() {
        let (c, dir) = ctx();
        let none = background_jobs(&c);
        assert_eq!(none.state, "healthy", "no jobs, nothing to run: {none:?}");

        c.store.ops_put("cron", "j1", "shop", &json!({ "schedule": "* * * * *", "command": "echo hi" })).unwrap();
        let waiting = background_jobs(&c);
        assert_eq!(waiting.state, "degraded", "{waiting:?}");
        assert!(waiting.detail.contains("nothing is running them") && waiting.detail.contains("devd serve"), "{}", waiting.detail);
        assert!(!waiting.required, "a stopped scheduler does not make devd itself unhealthy");

        // This test process is a devd program holding the claim, so the jobs are being run.
        std::fs::write(dir.join("ops-host.pid"), std::process::id().to_string()).unwrap();
        let running = background_jobs(&c);
        assert_eq!(running.state, "healthy", "{running:?}");
        assert!(running.detail.contains(&std::process::id().to_string()));

        // A claim naming a process that is gone is nobody.
        std::fs::write(dir.join("ops-host.pid"), "4294967").unwrap();
        assert_eq!(background_jobs(&c).state, "degraded");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_port_note_does_not_call_devd_its_own_web_server_another_program() {
        let (meaning, fix) = explain(
            "port.http",
            "degraded",
            "80 is held by nginx.exe (pid 1) on 0.0.0.0:80",
        );
        assert!(!meaning.contains("80 is held"), "the detail is shown beside the note, not repeated in it: {meaning}");
        assert!(meaning.contains("expected"), "{meaning}");
        assert!(!meaning.contains("Another program"), "{meaning}");
        assert_eq!(fix["view"], "overview");
    }
}
