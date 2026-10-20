//! Persistent agent sessions.
//!
//! The backend owns the PTY. A terminal component unmounting, a view change or a
//! page reload disposes a *view* of a session, never the session itself. Each
//! session has a stable id, a metadata row in SQLite and a bounded transcript on
//! disk, so it can be listed, reattached to, replayed and inspected later.
//!
//! # What persistence does not mean
//!
//! A row cannot outlive the operating-system resources it describes. When the
//! process that owns a PTY exits, its child process tree is gone (or has been
//! orphaned in a way this platform does not let us re-adopt). A new devd process
//! marks those rows `stale` and keeps the transcript. It never reports a stale
//! row as running, because that would be a lie the user would act on.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use serde_json::{json, Value};

use devd_core::error::{DevdError, Result};
use devd_core::process;
use devd_core::state::Store;
use devd_core::store_work::AgentSession;
use devd_core::{util, Paths};

/// How much scrollback a live terminal keeps before the oldest bytes are
/// dropped. A reattaching client is sent the retained window and told to reset,
/// so memory is bounded while recovery remains useful.
pub const MAX_BUFFER: usize = 2 * 1024 * 1024;

/// How much transcript one session may keep on disk.
pub const MAX_TRANSCRIPT: u64 = 8 * 1024 * 1024;

/// How much of a transcript is replayed on attach after a restart.
pub const REPLAY_BYTES: u64 = 256 * 1024;

/// Largest number of live sessions one process will own.
pub const MAX_LIVE_SESSIONS: usize = 12;

/// Session metadata plus derived activity, as returned to a client.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    #[serde(flatten)]
    /// The durable row.
    pub session: AgentSession,
    /// Whether this process can write to the PTY right now.
    pub attachable: bool,
    /// Why it is not attachable, when it is not.
    pub unattached_reason: Option<String>,
    /// Whether a transcript can still be replayed.
    pub replayable: bool,
    /// Best-known lifecycle state, reconciled with the live table.
    pub live_state: String,
}

/// Options for starting a terminal.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Harness kind: `claude`, `opencode`, `shell` or `custom`.
    pub harness: String,
    /// Working directory.
    pub cwd: String,
    /// Project scope for listing and shared state.
    pub project: Option<String>,
    /// Display label. Defaults to the harness name and directory.
    pub label: Option<String>,
    /// Terminal columns.
    pub cols: u16,
    /// Terminal rows.
    pub rows: u16,
    /// Task this session was started for.
    pub task_id: Option<String>,
    /// Session that delegated the task.
    pub parent_session_id: Option<String>,
    /// Extra harness arguments, for delegated tasks.
    pub args: Vec<String>,
    /// Full argv for the `custom` harness.
    pub command: Option<Vec<String>>,
}

struct Terminal {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Taken and dropped when the child exits, which releases the pseudoconsole
    /// and lets the reader reach end-of-file even on ConPTY.
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    pid: u32,
    output: Mutex<Output>,
    stopped: AtomicBool,
}

struct Output {
    buffer: Vec<u8>,
    base: u64,
    exited: bool,
    code: Option<i32>,
    reason: Option<String>,
    touched: Instant,
}

/// The live terminals owned by this process.
pub struct SessionManager {
    store: Arc<Store>,
    paths: Arc<Paths>,
    owner_pid: u32,
    owner_started_at: String,
    live: Mutex<HashMap<String, Arc<Terminal>>>,
}

impl SessionManager {
    /// Build a manager and reconcile rows left by previous processes.
    pub fn new(store: Arc<Store>, paths: Arc<Paths>) -> Result<Arc<Self>> {
        paths.ensure()?;
        let owner_pid = std::process::id();
        let stale = store.mark_sessions_stale(owner_pid)?;
        if stale > 0 {
            tracing::info!(stale, "marked sessions from a previous process as stale");
        }
        Ok(Arc::new(Self {
            store,
            paths,
            owner_pid,
            owner_started_at: util::now(),
            live: Mutex::new(HashMap::new()),
        }))
    }

    /// The store this manager writes to.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// The filesystem layout.
    pub fn paths(&self) -> &Arc<Paths> {
        &self.paths
    }

    /// Start a real PTY and record it.
    pub fn start(&self, opts: StartOptions) -> Result<SessionView> {
        let dir = PathBuf::from(&opts.cwd);
        let dir = dir
            .canonicalize()
            .map_err(|e| DevdError::invalid(format!("the working directory is not usable: {e}")))?;
        let dir = util::strip_verbatim(&dir);
        if !dir.is_dir() {
            return Err(DevdError::invalid(
                "the working directory is not a directory",
            ));
        }

        {
            let live = self
                .live
                .lock()
                .map_err(|_| DevdError::internal("the session table is poisoned"))?;
            let active = live
                .values()
                .filter(|t| t.output.lock().map(|o| !o.exited).unwrap_or(false))
                .count();
            if active >= MAX_LIVE_SESSIONS {
                return Err(DevdError::conflict(format!(
                    "this devd process is already running {MAX_LIVE_SESSIONS} sessions; stop one first"
                )));
            }
        }
        {
            // Finished terminals are retained for immediate replay, but their
            // transcript is the durable copy. Prune them if the table grows, so
            // a long-lived process does not accumulate an unbounded screen
            // buffer in memory.
            let mut live = self
                .live
                .lock()
                .map_err(|_| DevdError::internal("the session table is poisoned"))?;
            if live.len() > 48 {
                live.retain(|_, t| t.output.lock().map(|o| !o.exited).unwrap_or(false));
            }
        }

        let (program, mut args) = resolve(&opts)?;
        // Claude Code gets devd as an MCP server automatically, so a session
        // started from devd can use project.detect, project.run, the inbox and
        // approvals without the person wiring anything up.
        if opts.harness == "claude" && opts.command.is_none() {
            if let Some(config) = devd_mcp_config(&self.paths) {
                args.push("--mcp-config".to_string());
                args.push(config);
            }
        }
        args.extend(opts.args.clone());
        let cols = opts.cols.max(20);
        let rows = opts.rows.max(8);

        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| DevdError::internal(format!("could not open a terminal: {e}")))?;

        let mut command = CommandBuilder::new(program);
        command.args(&args);
        command.cwd(&dir);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        // Tell the agent where it is and how to learn the project, rather than
        // leaving it to guess the stack from file names.
        command.env("DEVD_PROJECT_DIR", dir.to_string_lossy().to_string());
        command.env(
            "DEVD_AGENT_HINT",
            "Call the devd MCP tool project.detect with this directory for the stack, package manager, runtimes, tasks and services; project.run and project.exec to run commands; toolchain.install to request missing tools.",
        );

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| DevdError::internal(format!("could not start `{}`: {e}", opts.harness)))?;
        drop(pair.slave);
        let pid = child.process_id();

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| DevdError::internal(format!("could not read the terminal: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| DevdError::internal(format!("could not write to the terminal: {e}")))?;

        let id = util::new_id();
        let transcript = self.paths.sessions().join(format!("{id}.log"));
        std::fs::create_dir_all(self.paths.sessions())?;

        let label = opts.label.clone().unwrap_or_else(|| {
            let name = match opts.harness.as_str() {
                "claude" => "Claude Code".to_string(),
                "opencode" => "OpenCode".to_string(),
                "shell" => "Shell".to_string(),
                other => other.to_string(),
            };
            format!(
                "{name} — {}",
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir.display().to_string())
            )
        });
        let label = util::truncate(&label, 120);

        let now = util::now();
        let session = AgentSession {
            id: id.clone(),
            label,
            harness: opts.harness.clone(),
            cwd: dir.to_string_lossy().to_string(),
            project: opts.project.clone(),
            task_id: opts.task_id.clone(),
            parent_session_id: opts.parent_session_id.clone(),
            state: "running".to_string(),
            exit_code: None,
            exit_reason: None,
            pid,
            cols,
            rows,
            owner_pid: self.owner_pid,
            owner_started_at: self.owner_started_at.clone(),
            created_at: now.clone(),
            last_activity_at: now,
            ended_at: None,
            transcript_path: Some(transcript.to_string_lossy().to_string()),
        };

        if let Err(e) = self.store.insert_session(&session) {
            let _ = child.kill();
            return Err(e);
        }

        let terminal = Arc::new(Terminal {
            writer: Mutex::new(writer),
            master: Mutex::new(Some(pair.master)),
            child: Mutex::new(child),
            pid: pid.unwrap_or(0),
            output: Mutex::new(Output {
                buffer: Vec::new(),
                base: 0,
                exited: false,
                code: None,
                reason: None,
                touched: Instant::now(),
            }),
            stopped: AtomicBool::new(false),
        });

        let pumping = terminal.clone();
        let store = self.store.clone();
        let session_id = id.clone();
        let transcript_for_pump = transcript.clone();
        std::thread::spawn(move || {
            pump(
                reader.as_mut(),
                &pumping,
                &store,
                &session_id,
                &transcript_for_pump,
            );
        });

        let watching = terminal.clone();
        let store_for_watch = self.store.clone();
        let session_for_watch = id.clone();
        let task_for_watch = opts.task_id.clone();
        let transcript_for_watch = transcript.clone();
        std::thread::spawn(move || {
            watch(
                &watching,
                &store_for_watch,
                &session_for_watch,
                task_for_watch.as_deref(),
                &transcript_for_watch,
            );
        });

        self.live
            .lock()
            .map_err(|_| DevdError::internal("the session table is poisoned"))?
            .insert(id, terminal);

        self.view(&session, true)
    }

    /// Reconcile a durable row with a live terminal.
    fn view(&self, session: &AgentSession, live: bool) -> Result<SessionView> {
        let terminal = if live {
            self.terminal(&session.id).ok()
        } else {
            None
        };
        let (state, attachable, reason, code, exited) = match &terminal {
            Some(t) => {
                let out = t
                    .output
                    .lock()
                    .map_err(|_| DevdError::internal("the session state is poisoned"))?;
                let state = if out.exited {
                    match out.code {
                        Some(0) => "completed".to_string(),
                        Some(_) => "failed".to_string(),
                        None => "failed".to_string(),
                    }
                } else {
                    "running".to_string()
                };
                (
                    state,
                    !out.exited,
                    if out.exited { out.reason.clone() } else { None },
                    out.code,
                    out.exited,
                )
            }
            None => {
                let mut state = session.state.clone();
                let mut reason = session.exit_reason.clone();
                if session.is_live() {
                    if session.owner_pid == self.owner_pid {
                        state = "stale".to_string();
                        reason =
                            Some("the terminal is no longer held by this devd process".to_string());
                    } else {
                        state = "unavailable".to_string();
                        reason = Some(format!(
                            "this session is owned by another devd process (pid {}) and cannot be attached here",
                            session.owner_pid
                        ));
                    }
                }
                (state, false, reason, session.exit_code, true)
            }
        };
        Ok(SessionView {
            session: session.clone(),
            attachable,
            unattached_reason: reason,
            replayable: exited,
            live_state: state,
        }
        .with_code(code))
    }

    /// Every recorded session, newest first, with live state folded in.
    pub fn list(&self, project: Option<&str>) -> Result<Vec<SessionView>> {
        let rows = self.store.list_sessions(project, 200)?;
        let mut out = Vec::new();
        for row in rows {
            let live = self
                .live
                .lock()
                .map(|m| m.contains_key(&row.id))
                .unwrap_or(false);
            out.push(self.view(&row, live)?);
        }
        Ok(out)
    }

    /// One session, with live state folded in.
    pub fn get(&self, id: &str) -> Result<SessionView> {
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
        let live = self
            .live
            .lock()
            .map(|m| m.contains_key(id))
            .unwrap_or(false);
        self.view(&row, live)
    }

    /// Start a session or attach to an existing one.
    ///
    /// Returns the session and a replay of recent output from the beginning. The
    /// client uses the returned cursor to continue polling.
    pub fn attach(&self, id: &str) -> Result<Value> {
        let view = self.get(id)?;
        let out = self.output(id, 0)?;
        Ok(json!({
            "session": view,
            "output": out["data"],
            "cursor": out["cursor"],
            "reset": out["reset"],
            "exited": out["exited"],
            "code": out["code"],
        }))
    }

    /// Bytes since `cursor`, base64 encoded.
    pub fn output(&self, id: &str, cursor: u64) -> Result<Value> {
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
        if let Some(terminal) = self.terminal(id).ok() {
            let state = terminal
                .output
                .lock()
                .map_err(|_| DevdError::internal("the session state is poisoned"))?;
            let total = state.base + state.buffer.len() as u64;
            let reset = cursor < state.base || cursor > total;
            let start = if reset {
                0
            } else {
                (cursor - state.base) as usize
            };
            let data = &state.buffer[start.min(state.buffer.len())..];
            return Ok(json!({
                "data": base64::engine::general_purpose::STANDARD.encode(data),
                "cursor": total,
                "reset": reset,
                "exited": state.exited,
                "code": state.code,
                "state": if state.exited { "finished" } else { "running" },
                "reason": state.reason,
            }));
        }

        // No live PTY: replay the durable transcript. This is what makes a
        // completed session inspectable and what a stale row can honestly offer.
        let data = read_tail(
            Path::new(row.transcript_path.as_deref().unwrap_or_default()),
            REPLAY_BYTES,
        )?;
        Ok(json!({
            "data": base64::engine::general_purpose::STANDARD.encode(&data),
            "cursor": data.len() as u64,
            "reset": true,
            "exited": true,
            "code": row.exit_code,
            "state": row.state,
            "reason": row.exit_reason,
        }))
    }

    /// Send keystrokes or pasted text.
    pub fn write(&self, id: &str, data: &str) -> Result<()> {
        let terminal = self.owned_terminal(id)?;
        let mut writer = terminal
            .writer
            .lock()
            .map_err(|_| DevdError::internal("the terminal writer is poisoned"))?;
        writer
            .write_all(data.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| DevdError::invalid(format!("could not write to the terminal: {e}")))?;
        if let Ok(mut out) = terminal.output.lock() {
            out.touched = Instant::now();
        }
        self.store.touch_session(id)?;
        Ok(())
    }

    /// Resize the PTY.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let terminal = self.owned_terminal(id)?;
        let master = terminal
            .master
            .lock()
            .map_err(|_| DevdError::internal("the terminal is poisoned"))?;
        let Some(master) = master.as_ref() else {
            return Err(DevdError::conflict("the session has already ended"));
        };
        master
            .resize(PtySize {
                rows: rows.max(8),
                cols: cols.max(20),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| DevdError::invalid(format!("could not resize the terminal: {e}")))?;
        Ok(())
    }

    /// Rename a session. Works for finished sessions too.
    pub fn rename(&self, id: &str, label: &str) -> Result<SessionView> {
        if !self.store.rename_session(id, label)? {
            return Err(DevdError::not_found(format!("no session `{id}`")));
        }
        self.get(id)
    }

    /// Stop a session and its child tree.
    ///
    /// The row is retained with the transcript: stopping a terminal should not
    /// erase what it printed, especially when the interesting part is why it
    /// failed.
    pub fn stop(&self, id: &str) -> Result<SessionView> {
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
        if row.owner_pid != self.owner_pid && row.is_live() {
            return Err(DevdError::conflict(format!(
                "this session is owned by another devd process (pid {}); stop it there",
                row.owner_pid
            )));
        }
        let terminal = self.terminal(id).ok();
        if let Some(terminal) = &terminal {
            terminal.stopped.store(true, Ordering::SeqCst);
            process::kill_tree(terminal.pid);
            if let Ok(mut out) = terminal.output.lock() {
                out.exited = true;
                out.reason = Some("stopped by the user".to_string());
            }
            if let Ok(mut master) = terminal.master.lock() {
                let _ = master.take();
            }
        }
        self.store.update_session_state(
            id,
            "cancelled",
            row.exit_code,
            Some("stopped by the user"),
            Some(&util::now()),
        )?;
        self.live
            .lock()
            .map_err(|_| DevdError::internal("the session table is poisoned"))?
            .remove(id);
        if let Some(task_id) = row.task_id {
            let _ = self.store.finish_agent_task(
                &task_id,
                "cancelled",
                None,
                None,
                Some("cancelled by the user"),
            );
        }
        self.get(id)
    }

    /// Forget a finished session's metadata. Refuses a live or attachable one.
    pub fn dismiss(&self, id: &str) -> Result<()> {
        let view = self.get(id)?;
        if view.attachable {
            return Err(DevdError::conflict("stop the session before dismissing it"));
        }
        let mut map = self
            .live
            .lock()
            .map_err(|_| DevdError::internal("the session table is poisoned"))?;
        map.remove(id);
        drop(map);
        self.store.delete_session(id)?;
        Ok(())
    }

    /// Kill every owned terminal, for a clean process exit.
    pub fn shutdown(&self) {
        let ids: Vec<String> = self
            .live
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            let _ = self.stop(&id);
        }
    }

    /// Whether a session id is owned by this process.
    pub fn is_attachable(&self, id: &str) -> bool {
        self.terminal(id)
            .ok()
            .map(|t| t.output.lock().map(|o| !o.exited).unwrap_or(false))
            .unwrap_or(false)
    }

    /// The tail of a session's transcript, for a task result.
    pub fn tail_text(&self, id: &str, bytes: u64) -> Result<String> {
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
        let path = PathBuf::from(row.transcript_path.unwrap_or_default());
        let data = read_tail(&path, bytes)?;
        Ok(String::from_utf8_lossy(&data).to_string())
    }

    fn terminal(&self, id: &str) -> Result<Arc<Terminal>> {
        self.live
            .lock()
            .map_err(|_| DevdError::internal("the session table is poisoned"))?
            .get(id)
            .cloned()
            .ok_or_else(|| DevdError::not_found(format!("no live terminal `{id}`")))
    }

    fn owned_terminal(&self, id: &str) -> Result<Arc<Terminal>> {
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| DevdError::not_found(format!("no session `{id}`")))?;
        if row.owner_pid != self.owner_pid {
            return Err(DevdError::conflict(format!(
                "this session is owned by another devd process (pid {})",
                row.owner_pid
            )));
        }
        self.terminal(id)
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl SessionView {
    fn with_code(mut self, code: Option<i32>) -> Self {
        if self.session.exit_code.is_none() {
            self.session.exit_code = code;
        }
        self
    }
}

fn pump(
    reader: &mut dyn Read,
    terminal: &Terminal,
    store: &Store,
    session_id: &str,
    transcript: &Path,
) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(transcript)
        .ok();
    let mut written = file
        .as_ref()
        .and_then(|f| f.metadata().ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => {
                break;
            }
            Ok(n) => {
                let bytes = &chunk[..n];
                if let Ok(mut state) = terminal.output.lock() {
                    state.buffer.extend_from_slice(bytes);
                    if state.buffer.len() > MAX_BUFFER {
                        let drop = state.buffer.len() - MAX_BUFFER;
                        state.buffer.drain(..drop);
                        state.base += drop as u64;
                    }
                    if state.touched.elapsed() > Duration::from_secs(2) {
                        state.touched = Instant::now();
                        let _ = store.touch_session(session_id);
                    }
                }
                if let Some(f) = file.as_mut() {
                    if f.write_all(bytes).is_ok() {
                        written += n as u64;
                        if written > MAX_TRANSCRIPT {
                            if let Ok(mut fresh) = rewrite_tail(transcript) {
                                let _ = fresh.flush();
                                file = Some(fresh);
                                written =
                                    std::fs::metadata(transcript).map(|m| m.len()).unwrap_or(0);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Wait for the child and record its exit.
///
/// This is deliberately a separate thread from the reader. On Windows a
/// pseudoconsole does not necessarily reach end-of-file just because the child
/// exited; waiting on the child is the authoritative signal, and dropping the
/// master afterwards releases the reader. Treating the reader as the authority
/// is how a terminal ends up "running" forever after a command finished.
fn watch(
    terminal: &Terminal,
    store: &Store,
    session_id: &str,
    task_id: Option<&str>,
    transcript: &Path,
) {
    let code: Option<i32> = match terminal.child.lock() {
        Ok(mut child) => match child.wait() {
            Ok(status) => status.exit_code().try_into().ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };

    // Let the reader drain bytes that arrived with the exit, then close the
    // pseudoconsole so its read cannot block for ever.
    std::thread::sleep(Duration::from_millis(120));
    if let Ok(mut master) = terminal.master.lock() {
        let _ = master.take();
    }
    std::thread::sleep(Duration::from_millis(60));

    let stopped = terminal.stopped.load(Ordering::SeqCst);
    let state = if stopped {
        "cancelled"
    } else if code == Some(0) {
        "completed"
    } else {
        "failed"
    };
    let reason = if stopped {
        "stopped by the user".to_string()
    } else {
        match code {
            Some(0) => "exited normally".to_string(),
            Some(c) => format!("exited with code {c}"),
            None => "the process ended without reporting an exit status".to_string(),
        }
    };
    if let Ok(mut state_out) = terminal.output.lock() {
        state_out.exited = true;
        state_out.code = code;
        state_out.reason = Some(reason.clone());
    }
    let _ = store.update_session_state(session_id, state, code, Some(&reason), Some(&util::now()));

    if let Some(task_id) = task_id {
        let result = read_tail(transcript, 16 * 1024)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        let task_state = match state {
            "cancelled" => "cancelled",
            "completed" => "succeeded",
            _ => "failed",
        };
        let error = if task_state == "failed" {
            Some(reason.as_str())
        } else {
            None
        };
        let _ = store.finish_agent_task(task_id, task_state, code, Some(&result), error);
    }
}

fn rewrite_tail(path: &Path) -> std::io::Result<std::fs::File> {
    let mut source = std::fs::File::open(path)?;
    let len = source.metadata()?.len();
    let keep = MAX_TRANSCRIPT / 2;
    if len > keep {
        source.seek(SeekFrom::Start(len - keep))?;
    }
    let mut tail = Vec::new();
    source.read_to_end(&mut tail)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&tail)?;
    }
    std::fs::rename(&tmp, path)?;
    std::fs::OpenOptions::new().append(true).open(path)
}

fn read_tail(path: &Path, max: u64) -> Result<Vec<u8>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|e| DevdError::internal(format!("cannot read the transcript: {e}")))?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > max {
        file.seek(SeekFrom::Start(len - max))
            .map_err(|e| DevdError::internal(format!("cannot seek the transcript: {e}")))?;
    }
    let mut out = Vec::new();
    file.read_to_end(&mut out)
        .map_err(|e| DevdError::internal(format!("cannot read the transcript: {e}")))?;
    Ok(out)
}

/// The inline MCP configuration that points an agent at this devd installation.
fn devd_mcp_config(paths: &Paths) -> Option<String> {
    let name = if cfg!(windows) { "devd.exe" } else { "devd" };
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .or_else(|| util::which("devd"))?;
    Some(
        json!({ "mcpServers": { "devd": {
            "command": exe.to_string_lossy(),
            "args": ["mcp", "--client", "claude-code"],
            "env": { "DEVD_HOME": paths.root().to_string_lossy() },
        } } })
        .to_string(),
    )
}

/// Resolve a harness to a program and fixed arguments.
fn resolve(opts: &StartOptions) -> Result<(PathBuf, Vec<String>)> {
    if let Some(command) = &opts.command {
        if command.is_empty() {
            return Err(DevdError::invalid(
                "a custom command needs at least a program",
            ));
        }
        let program = PathBuf::from(&command[0]);
        return Ok((program, command[1..].to_vec()));
    }
    match opts.harness.as_str() {
        "shell" => {
            let shell = std::env::var_os("COMSPEC")
                .map(PathBuf::from)
                .or_else(|| util::which("pwsh"))
                .or_else(|| util::which("sh"))
                .or_else(|| util::which("bash"))
                .ok_or_else(|| DevdError::not_found("no system shell was found on this machine"))?;
            Ok((shell, Vec::new()))
        }
        "claude" => util::which("claude")
            .map(|p| (p, Vec::new()))
            .ok_or_else(|| {
                DevdError::not_found("Claude Code (`claude`) is not installed or not on PATH")
            }),
        "opencode" => util::which("opencode")
            .map(|p| {
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                    (p, Vec::new())
                } else {
                    let cmd = util::which("cmd").unwrap_or_else(|| PathBuf::from("cmd.exe"));
                    (cmd, vec!["/c".to_string(), p.display().to_string()])
                }
            })
            .ok_or_else(|| {
                DevdError::not_found("OpenCode (`opencode`) is not installed or not on PATH")
            }),
        other => Err(DevdError::invalid(format!(
            "unknown harness `{other}`; expected claude, opencode, shell or custom"
        ))),
    }
}

/// The harnesses found on this machine.
pub fn harnesses() -> Value {
    let list = [
        (
            "claude",
            "Claude Code",
            "The Claude Code CLI installed on this machine, with its own account and permissions.",
        ),
        (
            "opencode",
            "OpenCode",
            "The OpenCode CLI installed on this machine.",
        ),
        (
            "shell",
            "Shell",
            "The system shell, in the project directory.",
        ),
    ];
    let rows: Vec<Value> = list
        .iter()
        .map(|(kind, label, detail)| {
            let available = resolve(&StartOptions {
                harness: (*kind).to_string(),
                ..Default::default()
            })
            .is_ok();
            json!({
                "kind": kind,
                "label": label,
                "detail": detail,
                "available": available,
            })
        })
        .collect();
    json!(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::config::Settings;
    use devd_core::state::Store;

    fn manager() -> (tempfile::TempDir, Arc<SessionManager>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.state_db()).unwrap());
        store.migrate().unwrap();
        let _ = Settings::default();
        (dir, SessionManager::new(store, Arc::new(paths)).unwrap())
    }

    fn shell_command() -> Vec<String> {
        #[cfg(windows)]
        {
            vec![
                "cmd.exe".to_string(),
                "/c".to_string(),
                "echo session-output & ping -n 5 127.0.0.1 > NUL".to_string(),
            ]
        }
        #[cfg(unix)]
        {
            vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo session-output; sleep 2".to_string(),
            ]
        }
    }

    #[test]
    fn a_session_is_recorded_and_survives_a_view_reload() {
        let (dir, manager) = manager();
        let view = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                command: Some(shell_command()),
                ..Default::default()
            })
            .unwrap();
        assert!(view.attachable);
        assert_eq!(view.live_state, "running");

        // A "reload" is another list call: the same id, still attachable.
        let listed = manager.list(None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session.id, view.session.id);

        let attached = manager.attach(&view.session.id).unwrap();
        assert!(attached["session"]["attachable"].as_bool().unwrap());
        manager.stop(&view.session.id).unwrap();
        let stopped = manager.get(&view.session.id).unwrap();
        assert_eq!(stopped.live_state, "cancelled");
        assert!(stopped.replayable);
    }

    #[test]
    fn output_is_buffered_and_replayable_after_the_process_exits() {
        let (dir, manager) = manager();
        let view = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                command: Some(shell_command()),
                ..Default::default()
            })
            .unwrap();
        let mut seen = Vec::new();
        for _ in 0..100 {
            let out = manager.output(&view.session.id, 0).unwrap();
            let raw = base64::engine::general_purpose::STANDARD
                .decode(out["data"].as_str().unwrap())
                .unwrap();
            seen = raw;
            if String::from_utf8_lossy(&seen).contains("session-output") {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(String::from_utf8_lossy(&seen).contains("session-output"));
    }

    #[test]
    fn two_sessions_are_independent_ptys() {
        let (dir, manager) = manager();
        let cwd = dir.path().to_string_lossy().to_string();
        let a = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd: cwd.clone(),
                command: Some(shell_command()),
                label: Some("a".into()),
                ..Default::default()
            })
            .unwrap();
        let b = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd,
                command: Some(shell_command()),
                label: Some("b".into()),
                ..Default::default()
            })
            .unwrap();
        assert_ne!(a.session.id, b.session.id);
        assert!(a.attachable && b.attachable);
        manager.stop(&a.session.id).unwrap();
        assert_eq!(manager.get(&a.session.id).unwrap().live_state, "cancelled");
        assert_eq!(manager.get(&b.session.id).unwrap().live_state, "running");
        manager.stop(&b.session.id).unwrap();
    }

    #[test]
    fn sessions_left_by_a_previous_process_are_stale_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join("home"));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open(&paths.state_db()).unwrap());
        store.migrate().unwrap();
        let row = AgentSession {
            id: "old".into(),
            label: "old".into(),
            harness: "shell".into(),
            cwd: dir.path().to_string_lossy().to_string(),
            project: None,
            task_id: None,
            parent_session_id: None,
            state: "running".into(),
            exit_code: None,
            exit_reason: None,
            pid: Some(1234),
            cols: 100,
            rows: 30,
            owner_pid: 999_999,
            owner_started_at: util::now(),
            created_at: util::now(),
            last_activity_at: util::now(),
            ended_at: None,
            transcript_path: None,
        };
        store.insert_session(&row).unwrap();
        let manager = SessionManager::new(store, Arc::new(paths)).unwrap();
        let view = manager.get("old").unwrap();
        assert_eq!(view.live_state, "stale");
        assert!(!view.attachable);
        assert!(view
            .unattached_reason
            .unwrap()
            .contains("no longer running"));
    }

    #[test]
    fn a_short_custom_command_reports_its_exit() {
        let (dir, manager) = manager();
        let view = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                command: Some(shell_command()),
                ..Default::default()
            })
            .unwrap();
        for _ in 0..100 {
            let state = manager.get(&view.session.id).unwrap().live_state;
            if state != "running" {
                assert_eq!(state, "completed");
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the short command never reported an exit");
    }

    #[test]
    fn resize_is_accepted_for_a_live_session() {
        let (dir, manager) = manager();
        let view = manager
            .start(StartOptions {
                harness: "custom".into(),
                cwd: dir.path().to_string_lossy().to_string(),
                command: Some(shell_command()),
                ..Default::default()
            })
            .unwrap();
        manager.resize(&view.session.id, 120, 40).unwrap();
        manager.stop(&view.session.id).unwrap();
    }
}
