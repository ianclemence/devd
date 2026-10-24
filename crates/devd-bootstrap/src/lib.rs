//! The composition root.
//!
//! There is exactly one way to assemble devd: resolve the layout, open the
//! store, discover Git, create the process-owning workspace, and register every
//! skill's capabilities against that same workspace. The CLI and the desktop
//! shell both call this function. If they assembled their own registries, an
//! agent connected over MCP and a button in the window could end up talking to
//! different registries - a bug that would look like a missing capability.

use std::sync::Arc;

use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::{Registry, Result};
use devd_git::{caps as git_caps, GitService};
use devd_runtime::caps as runtime_caps;
use devd_workspace::Workspace;

/// Everything a surface needs to run.
pub struct Bootstrap {
    /// Resolved filesystem layout.
    pub paths: Paths,
    /// The durable store.
    pub store: Arc<Store>,
    /// Effective settings.
    pub settings: Arc<Settings>,
    /// Event bus shared by all capabilities.
    pub events: EventBus,
    /// Git backend.
    pub git: Arc<GitService>,
    /// Every capability, including sessions, tasks, processes and shared state.
    pub registry: Arc<Registry>,
    /// The process-owning runtime the new capabilities refer to.
    pub workspace: Arc<Workspace>,
    /// The built-in agent and the team, which a long-lived process starts with [`Bootstrap::resume_agents`].
    pub agents: Agents,
}

/// The built-in agent and the team that drives it.
#[derive(Clone)]
pub struct Agents {
    hub: Arc<devd_agent::Hub>,
    team: Arc<devd_agent::team::Team>,
}

impl Agents {
    /// Recover the conversations and runs a previous process left half done, if
    /// this process is the one that owns the agents. A one-shot command beside a
    /// running application is not, and leaves the application's work alone.
    pub fn resume(&self, paths: &Paths) {
        if devd_agent::host::claim(paths) {
            self.hub.reconcile();
            self.team.recover();
        }
    }
}

/// Start the loop that runs cron jobs, in this process, if no other devd process already runs it.
/// Only a long-lived command calls this (the application window, `devd ui`, `devd serve`): a
/// one-shot command must not start every due job. Returns whether this process runs them.
pub fn start_ops(paths: &Paths, store: &Arc<Store>, settings: &Arc<Settings>, events: &EventBus) -> bool {
    devd_ops::runner::start(devd_core::capability::Ctx {
        store: store.clone(),
        paths: Arc::new(paths.clone()),
        settings: settings.clone(),
        events: events.clone(),
        actor: devd_core::capability::Actor::system(),
        dry_run: false,
    })
}

/// Watch what sites serve, and keep the web server's configuration in step.
///
/// What a site serves is read from its folder, so another program creating or removing an entry file
/// changes the answer without devd being told. This loop compares the effective answer every few
/// seconds; when it changes, the configuration is rendered again, checked with the server's own
/// `-t`, and only then reloaded. It runs only in the process that already owns the background work
/// (see [`start_ops`]), so two open devd processes cannot fight over the reload.
///
/// A failure is reported as an event and retried, never as a success, and the running server keeps
/// the configuration it already had until the replacement passes its check.
pub fn start_reconcile(paths: &Paths, store: &Arc<Store>, settings: &Arc<Settings>, events: &EventBus) {
    let ctx = devd_core::capability::Ctx {
        store: store.clone(),
        paths: Arc::new(paths.clone()),
        settings: settings.clone(),
        events: events.clone(),
        actor: devd_core::capability::Actor::system(),
        dry_run: false,
    };
    let events = events.clone();
    let _ = std::thread::Builder::new().name("devd-serve-watch".into()).spawn(move || {
        let mut last: Option<String> = None;
        let mut last_error = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let Ok(fingerprint) = web_fingerprint(&ctx) else { continue };
            if last.as_deref() == Some(fingerprint.as_str()) {
                continue;
            }
            match devd_services::reconcile_web(&ctx) {
                Ok(services) => {
                    // The first pass is the render a fresh process does anyway. Only a later change
                    // is news worth telling the window about.
                    if last.is_some() && !services.is_empty() {
                        events.emit_with(
                            "web.reconciled",
                            format!("reloaded {}", services.join(", ")),
                            serde_json::json!({ "services": services, "reason": "a site's effective serving changed" }),
                        );
                    }
                    last = Some(fingerprint);
                    last_error.clear();
                }
                Err(e) => {
                    let message = e.to_string();
                    if message != last_error {
                        events.emit_with(
                            "web.reconcile_failed",
                            "the web server configuration could not be updated",
                            serde_json::json!({ "error": message }),
                        );
                        last_error = message;
                    }
                }
            }
        }
    });
}

/// What would change the web server's configuration: each site and what it currently serves.
fn web_fingerprint(ctx: &devd_core::capability::Ctx) -> Result<String> {
    let sites = ctx.store.list_sites()?;
    let rows: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| {
            serde_json::json!({
                "site": s,
                "effective": s.effective_serving(),
                "http": devd_services::http_port(ctx),
                "https": devd_services::https_port(ctx),
            })
        })
        .collect();
    Ok(devd_core::util::sha256_hex(serde_json::to_string(&rows)?.as_bytes()))
}

impl Bootstrap {
    /// Assemble from the environment.
    pub fn discover() -> Result<Self> {
        Self::at(Paths::discover()?)
    }

    /// Assemble against an explicit layout, for tests and portable installs.
    pub fn at(paths: Paths) -> Result<Self> {
        paths.ensure()?;

        let settings = Settings::load(&paths.settings_file())?;
        settings.validate()?;

        let store = Store::open(&paths.state_db())?;
        store.migrate()?;

        // The launchers follow what is installed: written when PHP or Composer
        // arrives, rewritten if devd moved, removed when what they run is gone.
        // A failure here must never stop devd from starting.
        if let Ok(exe) = devd_core::util::cli_exe() {
            let _ = devd_core::shims::ensure(&paths, &exe);
        }

        let git = Arc::new(GitService::discover(settings.git_executable.as_deref())?);
        let workspace = Workspace::new(Arc::new(store), Arc::new(paths.clone()))?;

        let mut registry = Registry::new();
        git_caps::register(&mut registry, git.clone())?;
        runtime_caps::register(&mut registry)?;
        devd_services::caps::register(&mut registry)?;
        devd_sites::caps::register(&mut registry)?;
        devd_workspace::caps::register(&mut registry, workspace.clone())?;
        devd_stacks::caps::register(&mut registry)?;
        devd_ops::caps::register(&mut registry)?;
        devd_mcp::caps::register(&mut registry)?;

        // The built-in agent needs the shared store, settings and event bus, and
        // - once it exists - the registry it acts through.
        let events = EventBus::new();
        let settings = Arc::new(settings);
        let hub = devd_agent::Hub::new(devd_core::capability::Ctx {
            store: workspace.store().clone(),
            paths: Arc::new(paths.clone()),
            settings: settings.clone(),
            events: events.clone(),
            actor: devd_core::capability::Actor::system(),
            dry_run: false,
        });
        let team = devd_agent::team::Team::new(hub.clone(), devd_core::capability::Ctx {
            store: workspace.store().clone(),
            paths: Arc::new(paths.clone()),
            settings: settings.clone(),
            events: events.clone(),
            actor: devd_core::capability::Actor::system(),
            dry_run: false,
        });
        devd_agent::caps::register(&mut registry, hub.clone(), team.clone())?;
        let registry = Arc::new(registry);
        hub.attach(registry.clone());
        team.attach(registry.clone());

        Ok(Self {
            paths,
            store: workspace.store().clone(),
            settings,
            events,
            git,
            registry,
            workspace,
            agents: Agents { hub, team },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::capability::Actor;
    use devd_core::state::Site;

    fn ctx(tag: &str) -> (devd_core::capability::Ctx, Paths) {
        let dir = std::env::temp_dir().join(format!("devd-reconcile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let ctx = devd_core::capability::Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths.clone()),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::system(),
            dry_run: false,
        };
        (ctx, paths)
    }

    #[test]
    fn a_file_another_program_creates_changes_what_the_web_server_must_do() {
        let (ctx, paths) = ctx("fingerprint");
        let root = paths.www().join("shop");
        std::fs::create_dir_all(&root).unwrap();
        ctx.store
            .upsert_site(&Site {
                id: "site-shop".into(),
                name: "shop".into(),
                host: "shop.test".into(),
                root: root.display().to_string(),
                web_server: "nginx".into(),
                php: None,
                serving: None,
                tls_mode: "off".into(),
                db_name: None,
                upstream: None,
                created_at: devd_core::util::now(),
            })
            .unwrap();

        let empty = web_fingerprint(&ctx).unwrap();
        // An editor or a scaffold writes a static entry: the site now serves files.
        std::fs::write(root.join("index.html"), "<!doctype html>").unwrap();
        let static_site = web_fingerprint(&ctx).unwrap();
        assert_ne!(empty, static_site, "a new index.html changes what is served");

        // A tool adds a PHP entry point: the same folder must switch to PHP.
        std::fs::write(root.join("index.php"), "<?php").unwrap();
        let php = web_fingerprint(&ctx).unwrap();
        assert_ne!(static_site, php, "a new index.php changes what is served");

        // Reading it again is stable, so the watcher does not reload on every pass.
        assert_eq!(php, web_fingerprint(&ctx).unwrap());

        let _ = std::fs::remove_dir_all(ctx.paths.root());
    }
}
