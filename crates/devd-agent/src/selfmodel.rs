//! devd's description of itself.
//!
//! The agent should not infer its own version, paths, state or surface list from
//! the request or from a model's memory. This module is the one place those
//! facts are assembled: the prompt gets a short live brief, and `devd.self`
//! returns the full object for anything the brief leaves out.
//!
//! The static architecture belongs in the prompt. Everything here is either
//! process state or a fact that changes between builds, machines and modes.

use devd_core::capability::Ctx;
use devd_core::config::AgentProvider;
use devd_core::Registry;
use serde_json::{json, Value};

/// The places in the native window, in the order the rail shows them.
///
/// `app.report` and `app.go` use the first element as the route segment. The
/// descriptions are for the agent, not for the rail.
#[rustfmt::skip]
const SCREENS: &[(&str, &str, &str)] = &[
    ("home", "Home", "what needs a decision, what is running, recent work"),
    ("inbox", "Inbox", "approvals, questions and proposals waiting for a person"),
    ("assistant", "Assistant", "conversations with the built-in agent"),
    ("runs", "Runs", "agent runs, their steps, evidence and verdicts"),
    ("team", "Team", "the roster, the team room, schedules and skills"),
    ("intelligence", "Intelligence", "providers, models, role assignments and spend"),
    ("projects", "Projects", "the projects on this machine and their files, git and tasks"),
    ("sites", "Sites", "local sites, their hosts, certificates and deploys"),
    ("server", "Server", "managed services, runtimes, databases and tools"),
    ("activity", "Activity", "the audit trail: what changed, by whom"),
    ("settings", "Settings", "preferences and diagnostics"),
];

/// The ways a person or another program can reach devd.
#[rustfmt::skip]
const SURFACES: &[(&str, &str)] = &[
    ("cli", "the `devd` command line"),
    ("tui", "the terminal dashboard, `devd tui`"),
    ("app", "the native GPUI window, drawn without a webview"),
    ("http", "the loopback HTTP API and its live event stream"),
    ("mcp", "the Model Context Protocol server, for other agents"),
    ("acp", "the Agent Client Protocol builder, `devd acp`, for editors"),
    ("desktop", "the Tauri shell around the same server and UI"),
];

fn provider_name(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::None => "none",
        AgentProvider::Anthropic => "anthropic",
        AgentProvider::OpenAiCompatible => "openai-compatible",
    }
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn executable() -> Value {
    std::env::current_exe()
        .ok()
        .map(|path| Value::String(path.display().to_string()))
        .unwrap_or(Value::Null)
}

/// Everything devd knows about this process, this machine and its interface.
pub fn facts(ctx: &Ctx) -> Value {
    let settings = &ctx.settings;
    let database = ctx.paths.state_db();
    let schema = ctx.store.schema_version().unwrap_or(-1);
    let mode = ctx.store.environment_mode();
    let www = settings.effective_www_root(&ctx.paths.www());
    let current_run = ctx
        .run
        .as_ref()
        .and_then(|run| ctx.store.run_by_id(&run.run_id).ok().flatten());

    let screens: Vec<Value> = SCREENS
        .iter()
        .map(|(route, title, what)| {
            json!({
                "route": format!("/{route}"),
                "title": title,
                "what": what,
            })
        })
        .collect();
    let surfaces: Vec<Value> = SURFACES
        .iter()
        .map(|(id, what)| json!({ "id": id, "what": what }))
        .collect();

    json!({
        "name": "devd",
        "version": env!("CARGO_PKG_VERSION"),
        "build": {
            "profile": profile(),
            "target": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            "executable": executable(),
        },
        "runtime": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
            "pid": std::process::id(),
        },
        "home": ctx.paths.root().display().to_string(),
        "paths": {
            "www": www.display().to_string(),
            "runtimes": ctx.paths.runtimes().display().to_string(),
            "services": ctx.paths.services().display().to_string(),
            "etc": ctx.paths.etc().display().to_string(),
            "certs": ctx.paths.certs().display().to_string(),
            "logs": ctx.paths.logs().display().to_string(),
            "mcp": ctx.paths.mcp().display().to_string(),
            "projects": ctx.paths.projects().display().to_string(),
            "cache": ctx.paths.cache().display().to_string(),
            "snapshots": ctx.paths.snapshots().display().to_string(),
            "mods": ctx.paths.mods().display().to_string(),
            "uploads": ctx.paths.root().join("uploads").display().to_string(),
            "roles": ctx.paths.root().join("roles").display().to_string(),
            "skills": ctx.paths.root().join("skills").display().to_string(),
            "templates": ctx.paths.root().join("templates").display().to_string(),
            "settings": ctx.paths.settings_file().display().to_string(),
        },
        "state": {
            "database": database.display().to_string(),
            "exists": database.is_file(),
            "engine": "sqlite",
            "journal": "wal",
            "schema": schema,
            "latest_schema": devd_core::state::LATEST_SCHEMA,
        },
        "environment": {
            "mode": mode,
            "development_tld": settings.tld,
            "http_port": settings.http_port,
            "https_port": settings.https_port,
            "www_root": www.display().to_string(),
            "agent_auto_approve_write": settings.agent_auto_approve_write,
        },
        "model": {
            "provider": provider_name(settings.agent_provider),
            "model": settings.agent_model,
            "note": "A conversation may choose its own model; `models.roles` is the live assignment table.",
        },
        "surfaces": surfaces,
        "window": {
            "screens": screens,
            "link_scheme": "devd://<screen>/<id>",
            "note": "Call `app.where` before `app.go`; it says what the person is looking at and how long ago the window reported it.",
        },
        "capabilities": {
            "registry": "The live capability registry is the only authority for what devd can do.",
            "tiers": ["read", "write", "exec", "destructive", "remote", "admin"],
            "discovery": ["devd_groups", "devd_capabilities", "devd_describe", "devd_invoke", "devd_doctor", "devd.self"],
            "approval": "A person approves destructive, remote and admin operations. An agent cannot supply its own approval, and production mode raises every change to approval.",
        },
        "read_also": {
            "projects": "project.list",
            "conversations": "chat.list or chat.get",
            "skills": "skills.list",
            "roles": "team.roles",
            "models": "models.roles",
            "providers": "provider.list",
            "health": "health.status",
            "flags": "flags.list",
            "activity": "activity.list",
            "window": "app.where",
        },
        "current": {
            "actor": ctx.actor.kind,
            "surface": ctx.actor.surface.as_str(),
            "client": ctx.actor.name,
            "run": ctx.run.as_ref().map(|run| run.run_id.clone()),
            "step": ctx.run.as_ref().and_then(|run| run.step_id.clone()),
            "project": current_run.as_ref().and_then(|run| run.project.clone()),
            "thread": current_run.as_ref().and_then(|run| run.thread_id.clone()),
            "dry_run": ctx.dry_run,
        },
        "limits": [
            "Secrets in `.env` files, key files and provider credentials are not readable by an agent; a secret is never written into code, a commit, a log or a message.",
            "Another project is read-only unless the conversation is scoped to it.",
            "Destructive, remote and admin capabilities need a person's approval; an agent cannot grant it to itself.",
            "In production mode every change needs a person's approval.",
            "`project.exec` passes an argv array to one program. It is not a shell: no pipes, redirects, `&&`, `cd` or built-ins.",
            "Completion is decided from recorded tool calls and evidence, not from a model saying it finished.",
        ],
    })
}

/// The short, live self brief that goes into the assistant's system prompt.
pub fn prompt(ctx: &Ctx, registry: Option<&Registry>) -> String {
    let database = ctx.paths.state_db();
    let schema = ctx.store.schema_version().unwrap_or(-1);
    let mode = ctx.store.environment_mode();
    let www = ctx.settings.effective_www_root(&ctx.paths.www());
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "the devd binary".to_string());

    let mut brief = format!(
        "## This devd, right now\n\nYou are devd {} ({} build) on {}/{}, running from `{}`. \
         The devd home is `{}`, projects live under `{}`, and state is SQLite at `{}` \
         (schema {}/{}, {} mode).",
        env!("CARGO_PKG_VERSION"),
        profile(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        exe,
        ctx.paths.root().display(),
        www.display(),
        database.display(),
        schema,
        devd_core::state::LATEST_SCHEMA,
        mode,
    );

    if let Some(registry) = registry {
        let groups = registry.groups();
        let domains: Vec<String> = groups
            .iter()
            .map(|(group, caps)| format!("{group} ({})", caps.len()))
            .collect();
        brief.push_str(&format!(
            "\n\nYou have {} capabilities in {} domains: {}.",
            registry.len(),
            groups.len(),
            domains.join(", ")
        ));
    }

    brief.push_str(
        "\n\nThe registry is the only authority for what you can do: `devd_groups` lists the domains, \
         `devd_capabilities` searches them, `devd_describe` returns one exact schema, and `devd_invoke` runs it. \
         For the rest of your own state - exact paths, build, current actor, run, project and conversation, screens and limits - invoke `devd.self` through `devd_invoke`. \
         Do not guess your version, paths or schema when you can read them here or with that call.",
    );
    brief
}

/// The route names in the native window, for tests and callers that need them
/// without the rest of the self model.
pub fn screen_routes() -> Vec<&'static str> {
    SCREENS.iter().map(|(route, _, _)| *route).collect()
}

/// A one-line summary for logs and diagnostics.
pub fn summary(ctx: &Ctx) -> String {
    format!(
        "devd {} on {}/{}; home {}; schema {}/{}; {} mode",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        ctx.paths.root().display(),
        ctx.store.schema_version().unwrap_or(-1),
        devd_core::state::LATEST_SCHEMA,
        ctx.store.environment_mode(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use devd_core::capability::{Actor, Effect, Outcome, Plan, Surface, Tier};
    use devd_core::config::Settings;
    use devd_core::events::EventBus;
    use devd_core::paths::Paths;
    use devd_core::state::Store;

    fn ctx() -> Ctx {
        let store = Store::open_in_memory().unwrap();
        Ctx {
            store: Arc::new(store),
            paths: Arc::new(Paths::rooted("C:/tmp/devd-self")),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::agent("test"),
            run: None,
            dry_run: false,
        }
    }

    #[test]
    fn facts_name_the_build_and_its_state() {
        let facts = facts(&ctx());
        assert_eq!(facts["name"], "devd");
        assert_eq!(facts["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(facts["state"]["schema"], devd_core::state::LATEST_SCHEMA);
        assert_eq!(facts["current"]["surface"], Surface::Mcp.as_str());
        assert_eq!(facts["window"]["screens"].as_array().unwrap().len(), 11);
    }

    #[test]
    fn prompt_says_where_the_process_lives() {
        let brief = prompt(&ctx(), None);
        assert!(brief.contains("## This devd, right now"));
        assert!(brief.contains(env!("CARGO_PKG_VERSION")));
        assert!(brief.contains("C:/tmp/devd-self"));
        assert!(brief.contains("`devd.self`"));
        assert!(!brief.contains("{{SELF}}"));
    }

    #[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
    struct Nothing {}

    #[test]
    fn prompt_names_the_live_registry() {
        let mut registry = Registry::new();
        registry
            .define::<Nothing>("sample.one", "A sample")
            .group("sample")
            .tier(Tier::Read)
            .effect(Effect::Read)
            .agent_safe()
            .idempotent()
            .handlers(
                |_, _| Ok(Plan::new("sample.one", "read the sample")),
                |_, _| Ok(Outcome::ok("read")),
            )
            .unwrap();

        let brief = prompt(&ctx(), Some(&registry));
        assert!(brief.contains("1 capabilities in 1 domains: sample (1)"));
    }

    #[test]
    fn every_screen_is_a_route() {
        assert_eq!(screen_routes().first(), Some(&"home"));
        assert!(screen_routes().contains(&"settings"));
    }
}
