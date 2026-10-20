//! The composition root.
//!
//! There is exactly one way to assemble devd: resolve the layout, open the
//! store, discover Git, create the process-owning workspace, and register every
//! skill's capabilities against that same workspace. The CLI and the desktop
//! shell both call this function. If they assembled their own registries, an
//! agent connected over MCP and a button in the window could end up talking to
//! different session managers - a bug that would look like a missing terminal.

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

        let git = Arc::new(GitService::discover(settings.git_executable.as_deref())?);
        let workspace = Workspace::new(Arc::new(store), Arc::new(paths.clone()))?;

        let mut registry = Registry::new();
        git_caps::register(&mut registry, git.clone())?;
        runtime_caps::register(&mut registry)?;
        devd_services::caps::register(&mut registry)?;
        devd_sites::caps::register(&mut registry)?;
        devd_workspace::caps::register(&mut registry, workspace.clone())?;
        devd_stacks::caps::register(&mut registry)?;

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
        })
    }
}
