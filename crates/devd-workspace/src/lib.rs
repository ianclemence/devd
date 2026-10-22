//! `devd-workspace` - the durable, process-owning half of a development
//! workspace.
//!
//! Two things live here because they share one lifetime model: supervised
//! `devd.yml` project processes, and the project-scoped scratchpads and todos
//! that both people and devd's own agents use.
//!
//! The crate is deliberately separate from `devd-server`: the desktop shell,
//! the MCP server and the tests all need the runtime without needing an HTTP
//! socket.

pub mod caps;
pub mod manifest;
pub mod project;

use std::sync::{Arc, Mutex};

use devd_core::error::Result;
use devd_core::state::Store;
use devd_core::Paths;

pub use manifest::{Manifest, ProcessSpec};
pub use project::{ProjectManager, ProjectMonitor};

/// The process-owning runtime shared by the API, the desktop shell and MCP.
pub struct Workspace {
    store: Arc<Store>,
    paths: Arc<Paths>,
    projects: Arc<ProjectManager>,
    monitor: Mutex<Option<ProjectMonitor>>,
}

impl Workspace {
    /// Assemble a workspace and start its process monitor.
    pub fn new(store: Arc<Store>, paths: Arc<Paths>) -> Result<Arc<Self>> {
        paths.ensure()?;
        let projects = ProjectManager::new(store.clone(), paths.clone());
        let monitor = projects.start_monitor();
        Ok(Arc::new(Self {
            store,
            paths,
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
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.shutdown();
    }
}
