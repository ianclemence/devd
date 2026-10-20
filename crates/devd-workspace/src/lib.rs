//! `devd-workspace` - the durable, process-owning half of a development
//! workspace.
//!
//! Three things live here because they share one lifetime model:
//!
//! * **Agent sessions.** A real PTY per session, owned by the backend, with
//!   durable metadata and a bounded transcript.
//! * **Delegated tasks.** A real child process, not a metadata record, with
//!   spawn/wait/cancel/result semantics and enforced nesting and concurrency
//!   limits.
//! * **Project processes and shared state.** Supervised `devd.yml` processes,
//!   plus project-scoped scratchpads and todos that both people and agents use.
//!
//! The crate is deliberately separate from `devd-server`: the desktop shell,
//! the MCP server and the tests all need the runtime without needing an HTTP
//! socket.

pub mod caps;
pub mod manifest;
pub mod orchestrate;
pub mod project;
pub mod session;

use std::sync::{Arc, Mutex};

use devd_core::error::Result;
use devd_core::state::Store;
use devd_core::Paths;

pub use manifest::{Manifest, ProcessSpec};
pub use orchestrate::{Orchestrator, SpawnRequest};
pub use project::{ProjectManager, ProjectMonitor};
pub use session::{SessionManager, SessionView, StartOptions};

/// The process-owning runtime shared by the API, the desktop shell and MCP.
pub struct Workspace {
    store: Arc<Store>,
    paths: Arc<Paths>,
    sessions: Arc<SessionManager>,
    orchestrator: Arc<Orchestrator>,
    projects: Arc<ProjectManager>,
    monitor: Mutex<Option<ProjectMonitor>>,
}

impl Workspace {
    /// Assemble a workspace and start its process monitor.
    pub fn new(store: Arc<Store>, paths: Arc<Paths>) -> Result<Arc<Self>> {
        paths.ensure()?;
        let sessions = SessionManager::new(store.clone(), paths.clone())?;
        let orchestrator = Arc::new(Orchestrator::new(sessions.clone()));
        let projects = ProjectManager::new(store.clone(), paths.clone());
        let monitor = projects.start_monitor();
        Ok(Arc::new(Self {
            store,
            paths,
            sessions,
            orchestrator,
            projects,
            monitor: Mutex::new(Some(monitor)),
        }))
    }

    /// The durable store.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// The filesystem layout.
    pub fn paths(&self) -> &Arc<Paths> {
        &self.paths
    }

    /// Session ownership and PTY transport.
    pub fn sessions(&self) -> &Arc<SessionManager> {
        &self.sessions
    }

    /// Delegated task orchestration.
    pub fn orchestrator(&self) -> &Arc<Orchestrator> {
        &self.orchestrator
    }

    /// `devd.yml` process supervision.
    pub fn projects(&self) -> &Arc<ProjectManager> {
        &self.projects
    }

    /// Stop every process this workspace owns.
    pub fn shutdown(&self) {
        if let Ok(mut monitor) = self.monitor.lock() {
            if let Some(monitor) = monitor.take() {
                monitor.stop();
            }
        }
        self.projects.shutdown();
        self.sessions.shutdown();
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.shutdown();
    }
}
