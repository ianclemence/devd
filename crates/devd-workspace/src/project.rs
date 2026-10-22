//! Supervised project processes declared in `devd.yml`.
//!
//! The supervisor is real: each process is spawned with an argv array, owns a
//! pid, writes to a log file, is health-checked and is stopped as a process
//! tree. A manifest is executable configuration, so starting anything requires
//! an approval recorded against the manifest's semantic hash. Editing the
//! commands invalidates the approval.
//!
//! # Limitations, stated rather than hidden
//!
//! * Supervision is owned by the devd process that started the processes. If
//!   that process dies without running its shutdown path, the children may
//!   survive as orphans; on the next run they are reported as unsupervised
//!   rather than silently adopted.
//! * `.cmd` and `.bat` shims (the normal way npm installs a CLI on Windows)
//!   must be started through `cmd.exe /c`. The argv array is still never
//!   concatenated into a shell string; `cmd` receives one shim path and the
//!   arguments as its own argument vector.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::error::{DevdError, Result};
use devd_core::process::{self, SpawnSpec, Supervisor};
use devd_core::state::Store;
use devd_core::store_work::{ProcessUpdate, ProjectProcess};
use devd_core::{util, Paths};

use crate::manifest::{self, Manifest, ProcessSpec};

/// How many restarts the supervisor will perform before giving up.
pub const MAX_RESTARTS: i64 = 5;

/// How many log lines are returned by default.
pub const DEFAULT_LOG_LINES: usize = 200;

/// A project process manager.
pub struct ProjectManager {
    store: Arc<Store>,
    paths: Arc<Paths>,
    supervisor: Supervisor,
    owner_pid: u32,
    running: Arc<AtomicBool>,
    /// Serialises start/stop so two clicks cannot race one process.
    gate: Mutex<()>,
}

impl ProjectManager {
    /// Build a manager.
    pub fn new(store: Arc<Store>, paths: Arc<Paths>) -> Arc<Self> {
        Arc::new(Self {
            store,
            paths,
            supervisor: Supervisor::new(),
            owner_pid: std::process::id(),
            running: Arc::new(AtomicBool::new(true)),
            gate: Mutex::new(()),
        })
    }

    /// Start the background monitor, returning a handle that stops it.
    pub fn start_monitor(self: &Arc<Self>) -> ProjectMonitor {
        let manager = self.clone();
        let running = self.running.clone();
        let handle = std::thread::spawn(move || {
            while running.load(Ordering::SeqCst) {
                manager.tick();
                for _ in 0..20 {
                    if !running.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        ProjectMonitor {
            running: self.running.clone(),
            handle: Some(handle),
        }
    }

    /// The manifest for a project, validated.
    pub fn manifest(&self, project: &str) -> Result<Manifest> {
        manifest::load(Path::new(project))
    }

    /// A plan suitable for a human to review before approving.
    pub fn plan(&self, project: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        let approval = self.store.get_project_approval(&manifest.root)?;
        let approved = approval
            .as_ref()
            .map(|(hash, _, _)| hash == &manifest.hash)
            .unwrap_or(false);
        let processes: Vec<Value> = manifest
            .processes
            .values()
            .map(|spec| {
                let mut env = BTreeMap::new();
                for (k, v) in &spec.declared_env {
                    env.insert(
                        k.clone(),
                        if manifest::is_secret_key(k) {
                            "[redacted]".to_string()
                        } else {
                            v.clone()
                        },
                    );
                }
                json!({
                    "name": spec.name,
                    "command": spec.command,
                    "working_dir": spec.working_dir,
                    "env": env,
                    "health": spec.health,
                    "restart": spec.restart,
                    "depends_on": spec.depends_on,
                    "startup_timeout_secs": spec.startup_timeout_secs,
                })
            })
            .collect();
        Ok(json!({
            "project": manifest.project,
            "root": manifest.root,
            "hash": manifest.hash,
            "approved": approved,
            "approved_by": approval.as_ref().map(|(_, by, _)| by.clone()),
            "approved_at": approval.as_ref().map(|(_, _, at)| at.clone()),
            "processes": processes,
            "start_order": manifest::start_order(&manifest).unwrap_or_default(),
        }))
    }

    /// Record a human's approval of the current manifest.
    pub fn approve(&self, project: &str, actor: &str) -> Result<Manifest> {
        let manifest = self.manifest(project)?;
        self.store
            .set_project_approval(&manifest.root, &manifest.hash, actor)?;
        self.sync(&manifest)?;
        Ok(manifest)
    }

    /// Ensure a row exists for every declared process.
    pub fn sync(&self, manifest: &Manifest) -> Result<()> {
        for spec in manifest.processes.values() {
            if self
                .store
                .get_project_process(&manifest.root, &spec.name)?
                .is_none()
            {
                self.store.upsert_project_process(&ProjectProcess {
                    id: util::new_id(),
                    project: manifest.root.clone(),
                    name: spec.name.clone(),
                    command: serde_json::to_string(&spec.command)
                        .map_err(|e| DevdError::internal(e.to_string()))?,
                    working_dir: spec.working_dir.clone(),
                    env: serde_json::to_string(&spec.declared_env)
                        .map_err(|e| DevdError::internal(e.to_string()))?,
                    health: spec
                        .health
                        .as_ref()
                        .map(|h| serde_json::to_string(h).unwrap_or_default()),
                    restart: spec.restart.clone(),
                    depends_on: serde_json::to_string(&spec.depends_on)
                        .map_err(|e| DevdError::internal(e.to_string()))?,
                    status: "stopped".into(),
                    health_status: if spec.health.is_some() {
                        "unknown"
                    } else {
                        "none"
                    }
                    .into(),
                    health_error: None,
                    pid: None,
                    owner_pid: 0,
                    desired_state: "stopped".into(),
                    restart_count: 0,
                    last_exit_code: None,
                    last_error: None,
                    started_at: None,
                    updated_at: util::now(),
                })?;
            }
        }
        Ok(())
    }

    /// Current status of every declared process.
    pub fn status(&self, project: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        self.sync(&manifest)?;
        self.reconcile(&manifest);
        let rows = self.store.list_project_processes(&manifest.root)?;
        let approval = self.store.get_project_approval(&manifest.root)?;
        let approved = approval
            .as_ref()
            .map(|(hash, _, _)| hash == &manifest.hash)
            .unwrap_or(false);
        let items: Vec<Value> = manifest
            .processes
            .values()
            .map(|spec| {
                let row = rows.iter().find(|r| r.name == spec.name);
                let log_path = self.log_path(&manifest.root, &spec.name);
                json!({
                    "name": spec.name,
                    "command": spec.command,
                    "working_dir": spec.working_dir,
                    "depends_on": spec.depends_on,
                    "restart": spec.restart,
                    "health_declared": spec.health.is_some(),
                    "status": row.map(|r| r.status.clone()).unwrap_or_else(|| "stopped".into()),
                    "health": row.map(|r| r.health_status.clone()).unwrap_or_else(|| "unknown".into()),
                    "health_error": row.and_then(|r| r.health_error.clone()),
                    "pid": row.and_then(|r| r.pid),
                    "owner_pid": row.map(|r| r.owner_pid).unwrap_or(0),
                    "supervised": row.map(|r| r.owner_pid == self.owner_pid).unwrap_or(false),
                    "restart_count": row.map(|r| r.restart_count).unwrap_or(0),
                    "last_exit_code": row.and_then(|r| r.last_exit_code),
                    "last_error": row.and_then(|r| r.last_error.clone()),
                    "started_at": row.and_then(|r| r.started_at.clone()),
                    "log_path": log_path.to_string_lossy().to_string(),
                })
            })
            .collect();
        Ok(json!({
            "project": manifest.project,
            "root": manifest.root,
            "hash": manifest.hash,
            "approved": approved,
            "processes": items,
        }))
    }

    /// Start one process. Requires an approval for the current manifest.
    pub fn start(&self, project: &str, name: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        self.require_approval(&manifest)?;
        self.sync(&manifest)?;
        let spec = manifest
            .processes
            .get(name)
            .ok_or_else(|| DevdError::not_found(format!("no process `{name}` in devd.yml")))?
            .clone();
        let _gate = self
            .gate
            .lock()
            .map_err(|_| DevdError::internal("the process gate is poisoned"))?;
        self.spawn_spec(&manifest, &spec, false)?;
        self.status(project)
    }

    /// Start every declared process in dependency order, waiting for readiness.
    pub fn start_all(&self, project: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        self.require_approval(&manifest)?;
        self.sync(&manifest)?;
        let _gate = self
            .gate
            .lock()
            .map_err(|_| DevdError::internal("the process gate is poisoned"))?;
        let order = manifest::start_order(&manifest)?;
        let mut started: Vec<String> = Vec::new();
        for name in order {
            let spec = manifest
                .processes
                .get(&name)
                .expect("start order names come from the manifest");
            if let Err(e) = self.spawn_spec(&manifest, spec, false) {
                return Err(DevdError::internal(format!(
                    "`{name}` did not start: {e}{}",
                    self.diagnostics(&manifest, &name)
                )));
            }
            if let Err(e) = self.wait_ready(&manifest, spec) {
                return Err(DevdError::internal(format!(
                    "`{name}` did not become ready: {e}{}",
                    self.diagnostics(&manifest, &name)
                )));
            }
            started.push(name);
        }
        let mut out = self.status(project)?;
        out["started"] = json!(started);
        Ok(out)
    }

    /// Stop one process and its child tree.
    pub fn stop(&self, project: &str, name: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        let _gate = self
            .gate
            .lock()
            .map_err(|_| DevdError::internal("the process gate is poisoned"))?;
        self.stop_inner(&manifest, name)?;
        self.status(project)
    }

    /// Restart one process.
    pub fn restart(&self, project: &str, name: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        self.require_approval(&manifest)?;
        self.sync(&manifest)?;
        let _gate = self
            .gate
            .lock()
            .map_err(|_| DevdError::internal("the process gate is poisoned"))?;
        self.stop_inner(&manifest, name)?;
        let spec = manifest
            .processes
            .get(name)
            .ok_or_else(|| DevdError::not_found(format!("no process `{name}` in devd.yml")))?
            .clone();
        self.spawn_spec(&manifest, &spec, false)?;
        let _ = self.wait_ready(&manifest, &spec);
        self.status(project)
    }

    /// Stop every process this devd process supervises for a project.
    pub fn stop_all(&self, project: &str) -> Result<Value> {
        let manifest = self.manifest(project)?;
        let rows = self.store.list_project_processes(&manifest.root)?;
        for row in rows.iter().filter(|r| r.pid.is_some()) {
            let _ = self.stop_inner(&manifest, &row.name);
        }
        self.status(project)
    }

    /// Stop every supervised project process, for process shutdown.
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
        for row in self.supervisor.list() {
            process::kill_tree(row.pid);
        }
        let _ = self.supervisor.stop_all();
    }

    /// Tail of a process log, optionally filtered by a substring.
    pub fn logs(
        &self,
        project: &str,
        name: &str,
        lines: usize,
        query: Option<&str>,
    ) -> Result<Value> {
        let manifest = self.manifest(project)?;
        let spec = manifest
            .processes
            .get(name)
            .ok_or_else(|| DevdError::not_found(format!("no process `{name}` in devd.yml")))?;
        let path = self.log_path(&manifest.root, name);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut selected: Vec<String> = text
            .lines()
            .filter(|line| query.map(|q| line.contains(q)).unwrap_or(true))
            .map(|line| redact(spec, line))
            .collect();
        let limit = lines.clamp(1, 2000);
        if selected.len() > limit {
            selected.drain(0..selected.len() - limit);
        }
        Ok(json!({
            "name": name,
            "path": path.to_string_lossy().to_string(),
            "lines": selected,
            "total_lines": text.lines().count(),
        }))
    }

    // -------------------------------------------------------------- internals

    fn require_approval(&self, manifest: &Manifest) -> Result<()> {
        let approval = self.store.get_project_approval(&manifest.root)?;
        match approval {
            Some((hash, _, _)) if hash == manifest.hash => Ok(()),
            Some(_) => Err(DevdError::denied(
                "devd.yml changed since it was approved; review the commands and approve them again",
            )),
            None => Err(DevdError::denied(
                "this project's commands have not been approved; review devd.yml and approve it first",
            )),
        }
    }

    fn spawn_spec(
        &self,
        manifest: &Manifest,
        spec: &ProcessSpec,
        from_monitor: bool,
    ) -> Result<()> {
        let key = key(&manifest.root, &spec.name);
        if self.supervisor.is_running(&key) {
            return Ok(());
        }
        let program = find_program(&spec.command[0]).ok_or_else(|| {
            DevdError::not_found(format!(
                "`{}` was not found on PATH{}",
                spec.command[0],
                self.diagnostics(manifest, &spec.name)
            ))
        })?;
        let (program, args) = shim(&program, &spec.command[1..]);
        let log = self.log_path(&manifest.root, &spec.name);
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let existing = self.store.get_project_process(&manifest.root, &spec.name)?;
        if let Some(row) = &existing {
            if row.owner_pid != 0
                && row.owner_pid != self.owner_pid
                && process::pid_alive(row.owner_pid)
            {
                return Err(DevdError::conflict(format!(
                    "`{}` is supervised by another devd process (pid {}); manage it there",
                    spec.name, row.owner_pid
                )));
            }
        }
        self.store
            .set_process_owner(&manifest.root, &spec.name, self.owner_pid)?;
        let restarts = existing.as_ref().map(|r| r.restart_count).unwrap_or(0);
        let started = util::now();
        let spawn = SpawnSpec {
            name: key.clone(),
            kind: "project".into(),
            program,
            args,
            cwd: Some(PathBuf::from(&spec.working_dir)),
            envs: spec
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            clear_env: true,
            log_path: log.clone(),
            restarts,
        };
        match self.supervisor.start(spawn, util::new_id()) {
            Ok(pid) => {
                self.store.update_project_process(&manifest.root, &spec.name, ProcessUpdate { status: "running", health_status: if spec.health.is_some() {
                        "unknown"
                    } else {
                        "none"
                    }, pid: Some(pid), started_at: Some(&started), desired: Some("started"), ..Default::default() })?;
                if from_monitor {
                    let _ = self.store.bump_process_restarts(&manifest.root, &spec.name);
                }
                Ok(())
            }
            Err(e) => {
                self.store.update_project_process(&manifest.root, &spec.name, ProcessUpdate { status: "failed", health_status: "unknown", error: Some(&e.to_string()), desired: Some("started"), ..Default::default() })?;
                Err(e)
            }
        }
    }

    fn stop_inner(&self, manifest: &Manifest, name: &str) -> Result<()> {
        if let Some(row) = self.store.get_project_process(&manifest.root, name)? {
            if row.owner_pid != 0
                && row.owner_pid != self.owner_pid
                && process::pid_alive(row.owner_pid)
            {
                return Err(DevdError::conflict(format!(
                    "`{name}` is supervised by another devd process (pid {}); stop it there",
                    row.owner_pid
                )));
            }
        }
        let key = key(&manifest.root, name);
        if let Some((code, _)) = self.supervisor.stop(&key)? {
            self.store.update_project_process(&manifest.root, name, ProcessUpdate { status: "stopped", health_status: "none", exit_code: Some(code), desired: Some("stopped"), ..Default::default() })?;
        } else {
            self.store.update_project_process(&manifest.root, name, ProcessUpdate { status: "stopped", health_status: "none", desired: Some("stopped"), ..Default::default() })?;
        }
        Ok(())
    }

    fn wait_ready(&self, manifest: &Manifest, spec: &ProcessSpec) -> Result<()> {
        let started = Instant::now();
        let timeout = Duration::from_secs(spec.startup_timeout_secs);
        let interval = spec
            .health
            .as_ref()
            .map(|h| Duration::from_millis(h.interval_ms))
            .unwrap_or(Duration::from_millis(250));
        loop {
            if !self.supervisor.is_running(&key(&manifest.root, &spec.name)) {
                return Err(DevdError::invalid(format!(
                    "the process exited during startup{}",
                    self.diagnostics(manifest, &spec.name)
                )));
            }
            match &spec.health {
                Some(health) => {
                    if let Some((ok, error)) = check_health(health) {
                        if ok {
                            self.store.set_process_health(
                                &manifest.root,
                                &spec.name,
                                "healthy",
                                None,
                            )?;
                            return Ok(());
                        }
                        self.store.set_process_health(
                            &manifest.root,
                            &spec.name,
                            "unhealthy",
                            Some(&error),
                        )?;
                    }
                }
                None => {
                    // No declared readiness signal: a process that is still
                    // alive after a short settle window counts as started.
                    if started.elapsed() > Duration::from_millis(500) {
                        return Ok(());
                    }
                }
            }
            if started.elapsed() > timeout {
                let error = format!(
                    "health check did not pass within {}s",
                    spec.startup_timeout_secs
                );
                self.store.set_process_health(
                    &manifest.root,
                    &spec.name,
                    "unhealthy",
                    Some(&error),
                )?;
                return Err(DevdError::Timeout(error));
            }
            std::thread::sleep(interval);
        }
    }

    fn tick(&self) {
        // Health of everything currently running.
        let names = self.supervisor.list();
        let mut by_project: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in &names {
            if let Some((project, name)) = split_key(&row.name) {
                by_project
                    .entry(project.to_string())
                    .or_default()
                    .push(name.to_string());
            }
        }
        for (project, names) in &by_project {
            let manifest = match manifest::load(Path::new(project)) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for name in names {
                let Some(spec) = manifest.processes.get(name) else {
                    continue;
                };
                if let Some(health) = &spec.health {
                    if let Some((ok, error)) = check_health(health) {
                        let status = if ok { "healthy" } else { "unhealthy" };
                        let _ = self.store.set_process_health(
                            project,
                            name,
                            status,
                            if ok { None } else { Some(&error) },
                        );
                    }
                }
            }
        }

        // Reap exits and decide whether to restart.
        for (reaped, _run, code) in self.supervisor.reaped() {
            let Some((project, name)) = split_key(&reaped) else {
                continue;
            };
            let manifest = match manifest::load(Path::new(project)) {
                Ok(m) => m,
                Err(e) => {
                    let _ = self.store.update_project_process(project, name, ProcessUpdate { status: "failed", health_status: "unknown", exit_code: Some(code), error: Some(&format!("{e}")), ..Default::default() });
                    continue;
                }
            };
            let Some(spec) = manifest.processes.get(name) else {
                let _ = self.store.update_project_process(project, name, ProcessUpdate { status: "stopped", health_status: "none", exit_code: Some(code), error: Some("the process is no longer declared in devd.yml"), desired: Some("stopped"), ..Default::default() });
                continue;
            };
            let row = self.store.get_project_process(project, name).ok().flatten();
            let desired = row
                .as_ref()
                .map(|r| r.desired_state.as_str())
                .unwrap_or("stopped");
            let restarts = row.as_ref().map(|r| r.restart_count).unwrap_or(0);
            let should_restart = desired == "started"
                && restarts < MAX_RESTARTS
                && (spec.restart == "always" || (spec.restart == "on-failure" && code != 0));
            let approved = self
                .store
                .get_project_approval(project)
                .ok()
                .flatten()
                .map(|(hash, _, _)| hash == manifest.hash)
                .unwrap_or(false);
            if should_restart && !approved {
                let _ = self.store.update_project_process(project, name, ProcessUpdate { status: "failed", health_status: "unknown", exit_code: Some(code), error: Some("devd.yml changed since it was approved; review and approve it before it can restart"), desired: Some("stopped"), ..Default::default() });
            } else if should_restart {
                let _ = self.store.update_project_process(project, name, ProcessUpdate { status: "starting", health_status: "unknown", exit_code: Some(code), desired: Some("started"), ..Default::default() });
                if let Err(e) = self.spawn_spec(&manifest, spec, true) {
                    let _ = self.store.update_project_process(project, name, ProcessUpdate { status: "failed", health_status: "unknown", exit_code: Some(code), error: Some(&format!("restart failed: {e}")), desired: Some("stopped"), ..Default::default() });
                }
            } else {
                let state = if code == 0 { "stopped" } else { "failed" };
                let mut error = format!("exited with code {code}");
                if desired == "started" && restarts >= MAX_RESTARTS {
                    error.push_str(&format!("; restart limit of {MAX_RESTARTS} reached"));
                }
                if let Some(line) = self.last_log_line(&manifest.root, name) {
                    error.push_str(&format!("; last output: {line}"));
                }
                let _ = self.store.update_project_process(project, name, ProcessUpdate { status: state, health_status: "none", exit_code: Some(code), error: Some(&error), desired: if desired == "started" {
                        Some("stopped")
                    } else {
                        None
                    }, ..Default::default() });
            }
        }

        // A row that claims to be running but is not in this supervisor is
        // either from a previous devd process or already gone. Never report it
        // as running.
        for project in by_project.keys() {
            let Ok(rows) = self.store.list_project_processes(project) else {
                continue;
            };
            for row in rows {
                if row.status == "running" || row.status == "starting" {
                    if row.owner_pid != 0
                        && row.owner_pid != self.owner_pid
                        && process::pid_alive(row.owner_pid)
                    {
                        continue;
                    }
                    let key = key(&row.project, &row.name);
                    let known = self.supervisor.list().iter().any(|p| p.name == key);
                    if !known {
                        let note = match row.pid {
                            Some(pid) if process::pid_alive(pid) => format!(
                                "process {pid} is still alive but is not supervised by this devd process"
                            ),
                            _ => "the process is no longer running".to_string(),
                        };
                        let _ = self.store.update_project_process(&row.project, &row.name, ProcessUpdate { status: "failed", health_status: "unknown", exit_code: row.last_exit_code, error: Some(&note), desired: Some("stopped"), ..Default::default() });
                    }
                }
            }
        }
    }

    /// Reconcile rows before a status read, so the UI never waits for a tick.
    fn reconcile(&self, manifest: &Manifest) {
        let Ok(rows) = self.store.list_project_processes(&manifest.root) else {
            return;
        };
        let live = self.supervisor.list();
        for row in rows {
            if row.status == "running" || row.status == "starting" {
                if row.owner_pid != 0
                    && row.owner_pid != self.owner_pid
                    && process::pid_alive(row.owner_pid)
                {
                    continue;
                }
                let key = key(&row.project, &row.name);
                if !live.iter().any(|p| p.name == key) {
                    let note = match row.pid {
                        Some(pid) if process::pid_alive(pid) => format!(
                            "process {pid} is still alive but is not supervised by this devd process"
                        ),
                        _ => "the process is no longer running".to_string(),
                    };
                    let _ = self.store.update_project_process(&row.project, &row.name, ProcessUpdate { status: "failed", health_status: "unknown", exit_code: row.last_exit_code, error: Some(&note), desired: Some("stopped"), ..Default::default() });
                }
            }
        }
    }

    fn diagnostics(&self, manifest: &Manifest, name: &str) -> String {
        match self.last_log_line(&manifest.root, name) {
            Some(line) => format!("; last output: {line}"),
            None => String::new(),
        }
    }

    fn last_log_line(&self, project: &str, name: &str) -> Option<String> {
        let path = self.log_path(project, name);
        let text = std::fs::read_to_string(path).ok()?;
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| util::truncate(line, 300))
    }

    fn log_path(&self, project: &str, name: &str) -> PathBuf {
        self.paths
            .projects()
            .join(slug(project))
            .join("logs")
            .join(format!("{name}.log"))
    }
}

/// Stops the monitor when dropped.
pub struct ProjectMonitor {
    running: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ProjectMonitor {
    /// Ask the monitor to stop and wait for it.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProjectMonitor {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

fn key(project: &str, name: &str) -> String {
    format!("{project}\u{1f}{name}")
}

fn split_key(k: &str) -> Option<(&str, &str)> {
    k.split_once('\u{1f}')
}

fn slug(project: &str) -> String {
    let digest = util::sha256_hex(project.as_bytes());
    let name = Path::new(project)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into())
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("{}-{}", &digest[..12], util::truncate(&name, 40))
}

/// Find an executable, accepting an explicit path or a PATH lookup.
fn find_program(program: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(program);
    if direct.is_file() {
        return Some(direct);
    }
    util::which(program)
}

/// Wrap a Windows batch or PowerShell shim so it can actually be executed.
fn shim(program: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    let ext = program
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "cmd" || ext == "bat" {
        let shell = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"));
        let mut out = vec!["/c".to_string(), program.to_string_lossy().to_string()];
        out.extend(args.iter().cloned());
        (shell, out)
    } else if ext == "ps1" {
        let shell = util::which("pwsh").unwrap_or_else(|| PathBuf::from("powershell.exe"));
        let mut out = vec![
            "-NoProfile".to_string(),
            "-File".to_string(),
            program.to_string_lossy().to_string(),
        ];
        out.extend(args.iter().cloned());
        (shell, out)
    } else {
        (program.to_path_buf(), args.to_vec())
    }
}

/// Check a declared health endpoint. Returns `None` when no check applies.
fn check_health(health: &manifest::Health) -> Option<(bool, String)> {
    if let Some(url) = &health.url {
        let response = ureq::get(url).timeout(Duration::from_secs(3)).call();
        return Some(match response {
            Ok(res) => {
                let code = res.status();
                if code < 400 {
                    (true, String::new())
                } else {
                    (false, format!("health endpoint returned HTTP {code}"))
                }
            }
            Err(e) => (false, format!("health request failed: {e}")),
        });
    }
    if let Some(port) = health.port {
        use std::net::{SocketAddr, TcpStream};
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        return Some(
            match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
                Ok(_) => (true, String::new()),
                Err(e) => (
                    false,
                    format!("port {port} is not accepting connections: {e}"),
                ),
            },
        );
    }
    None
}

fn redact(spec: &ProcessSpec, line: &str) -> String {
    let mut out = line.to_string();
    for (key, value) in &spec.declared_env {
        if manifest::is_secret_key(key) && value.len() >= 4 {
            out = out.replace(value, "[redacted]");
        }
    }
    out
}

/// The JSON shape of a start/stop result, for endpoints.
#[derive(Debug, Serialize)]
pub struct ProcessAction {
    /// Process affected.
    pub name: String,
    /// Resulting state.
    pub status: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::state::Store;

    fn fixture() -> (tempfile::TempDir, Arc<ProjectManager>, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("app");
        std::fs::create_dir_all(&project).unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.state_db()).unwrap());
        store.migrate().unwrap();
        let manager = ProjectManager::new(store, Arc::new(paths));
        (dir, manager, project)
    }

    /// A command that stays alive for a while, on either platform.
    fn slow_command(seconds: u32) -> String {
        if cfg!(windows) {
            format!(r#"["cmd.exe", "/c", "ping -n {seconds} 127.0.0.1 > NUL"]"#)
        } else {
            format!(r#"["sleep", "{seconds}"]"#)
        }
    }

    /// A command that prints a line and exits, on either platform.
    fn echo_command(text: &str) -> String {
        if cfg!(windows) {
            format!(r#"["cmd.exe", "/c", "echo {text}"]"#)
        } else {
            format!(r#"["echo", "{text}"]"#)
        }
    }

    fn write_manifest(project: &Path) {
        let text = format!(
            "version: 1\nproject:\n  name: test-app\nprocesses:\n  slow:\n    command: {}\n    restart: never\n",
            slow_command(30)
        );
        std::fs::write(project.join("devd.yml"), text).unwrap();
    }

    #[test]
    fn starting_requires_an_approval() {
        let (_dir, manager, project) = fixture();
        write_manifest(&project);
        let root = project.to_string_lossy().to_string();
        let err = manager.start(&root, "slow").unwrap_err();
        assert!(err.to_string().contains("approved"), "{err}");
        manager.approve(&root, "human:test").unwrap();
        let status = manager.start(&root, "slow").unwrap();
        assert_eq!(status["processes"][0]["status"], "running");
        manager.stop(&root, "slow").unwrap();
    }

    #[test]
    fn changing_the_manifest_invalidates_the_approval() {
        let (_dir, manager, project) = fixture();
        write_manifest(&project);
        let root = project.to_string_lossy().to_string();
        manager.approve(&root, "human:test").unwrap();
        let text = std::fs::read_to_string(project.join("devd.yml"))
            .unwrap()
            .replace("30", "31");
        std::fs::write(project.join("devd.yml"), text).unwrap();
        let err = manager.start(&root, "slow").unwrap_err();
        assert!(err.to_string().contains("changed"), "{err}");
    }

    #[test]
    fn logs_are_captured_and_secret_values_are_redacted() {
        let (_dir, manager, project) = fixture();
        let text = format!(
            "version: 1\nproject:\n  name: test-app\nprocesses:\n  echoer:\n    command: {}\n    env:\n      API_TOKEN: super-secret-value\n    restart: never\n",
            echo_command("hello-from-project")
        );
        std::fs::write(project.join("devd.yml"), text).unwrap();
        let root = project.to_string_lossy().to_string();
        manager.approve(&root, "human:test").unwrap();
        manager.start(&root, "echoer").unwrap();
        std::thread::sleep(Duration::from_millis(800));
        let logs = manager.logs(&root, "echoer", 50, None).unwrap();
        let joined = logs["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("hello-from-project"), "{joined}");
        assert!(!joined.contains("super-secret-value"));
    }

    #[test]
    fn process_status_survives_a_view_change() {
        let (_dir, manager, project) = fixture();
        write_manifest(&project);
        let root = project.to_string_lossy().to_string();
        manager.approve(&root, "human:test").unwrap();
        manager.start(&root, "slow").unwrap();
        // A fresh status call is what a view change does.
        let status = manager.status(&root).unwrap();
        assert_eq!(status["processes"][0]["status"], "running");
        assert!(status["processes"][0]["pid"].as_u64().unwrap() > 0);
        manager.stop_all(&root).unwrap();
        let stopped = manager.status(&root).unwrap();
        assert_eq!(stopped["processes"][0]["status"], "stopped");
    }

    #[test]
    fn a_missing_dependency_is_a_useful_validation_error() {
        let (_dir, _manager, project) = fixture();
        let text = r#"
version: 1
project:
  name: test-app
processes:
  web:
    command: ["echo", "hi"]
    depends_on: ["nope"]
"#;
        std::fs::write(project.join("devd.yml"), text).unwrap();
        let err = manifest::load(&project).unwrap_err();
        assert!(err.to_string().contains("not declared"), "{err}");
    }
}
