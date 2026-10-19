//! Live terminals for coding agents.
//!
//! `devd` does not reimplement an agent. It gives the harness the user already
//! has — Claude Code, OpenCode, or a plain shell — a real PTY inside the
//! workspace, so that harness's own models, credentials and permission prompts
//! apply. Nothing is proxied: the bytes on screen are the tool's own.
//!
//! Output is polled over the same bearer-token API as everything else. A
//! WebSocket would be lower latency, but it would also be a second transport
//! with its own authentication path; polling at 80ms is indistinguishable for a
//! terminal and keeps one gate on the whole surface.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};

use devd_core::util;

/// How much scrollback a terminal keeps before the oldest bytes are dropped.
const MAX_BUFFER: usize = 2 * 1024 * 1024;

/// A harness the application can start.
struct Harness {
    kind: &'static str,
    label: &'static str,
    detail: &'static str,
}

const HARNESSES: &[Harness] = &[
    Harness {
        kind: "claude",
        label: "Claude Code",
        detail: "The Claude Code CLI installed on this machine, with its own account and permissions.",
    },
    Harness {
        kind: "opencode",
        label: "OpenCode",
        detail: "The OpenCode CLI installed on this machine.",
    },
    Harness {
        kind: "shell",
        label: "Shell",
        detail: "The system shell, in the project directory.",
    },
];

/// Every terminal this server is holding.
#[derive(Default)]
pub struct Terminals {
    map: Mutex<HashMap<String, Arc<Terminal>>>,
}

struct Terminal {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    output: Mutex<Output>,
}

struct Output {
    buffer: Vec<u8>,
    /// Absolute offset of `buffer[0]`, so a client cursor survives trimming.
    base: u64,
    exited: bool,
    code: Option<i32>,
}

/// The harnesses found on this machine.
pub fn harnesses() -> Value {
    let list: Vec<Value> = HARNESSES
        .iter()
        .map(|h| {
            json!({
                "kind": h.kind,
                "label": h.label,
                "detail": h.detail,
                "available": resolved(h.kind).is_some(),
            })
        })
        .collect();
    json!(list)
}

/// Resolve a harness to a program and arguments.
///
/// npm installs `.ps1` and `.cmd` shims rather than an executable; those have to
/// go through `cmd.exe`, while a real `.exe` is started directly.
fn resolved(kind: &str) -> Option<(PathBuf, Vec<String>)> {
    match kind {
        "shell" => {
            let shell = std::env::var_os("COMSPEC")
                .map(PathBuf::from)
                .or_else(|| util::which("cmd"))
                .or_else(|| util::which("pwsh"))
                .or_else(|| util::which("sh"))?;
            Some((shell, Vec::new()))
        }
        "claude" => util::which("claude").map(|p| (p, Vec::new())),
        "opencode" => util::which("opencode").map(|p| {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                (p, Vec::new())
            } else {
                let cmd = util::which("cmd").unwrap_or_else(|| PathBuf::from("cmd.exe"));
                (cmd, vec!["/c".to_string(), p.display().to_string()])
            }
        }),
        _ => None,
    }
}

impl Terminals {
    /// Start a harness in `cwd` and return the terminal id.
    pub fn start(&self, kind: &str, cwd: Option<&str>, cols: u16, rows: u16) -> Result<String, String> {
        let (program, args) =
            resolved(kind).ok_or_else(|| format!("`{kind}` is not installed on this machine"))?;

        let dir = cwd
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "the working directory does not exist".to_string())?;

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: rows.max(8),
                cols: cols.max(20),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("could not open a terminal: {e}"))?;

        let mut command = CommandBuilder::new(program);
        command.args(&args);
        command.cwd(&dir);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("could not start `{kind}`: {e}"))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("could not read the terminal: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("could not write to the terminal: {e}"))?;

        let id = util::new_id();
        let terminal = Arc::new(Terminal {
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            output: Mutex::new(Output {
                buffer: Vec::new(),
                base: 0,
                exited: false,
                code: None,
            }),
        });

        let pumping = terminal.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => push(&pumping, &chunk[..n]),
                }
            }
            let code = pumping
                .child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok().flatten())
                .and_then(|status| status.exit_code().try_into().ok());
            if let Ok(mut state) = pumping.output.lock() {
                state.exited = true;
                state.code = code;
            }
        });

        self.map
            .lock()
            .map_err(|_| "the terminal table is poisoned".to_string())?
            .insert(id.clone(), terminal);
        Ok(id)
    }

    /// Bytes since `cursor`, as base64 so a multi-byte character is never split.
    pub fn output(&self, id: &str, cursor: u64) -> Result<Value, String> {
        let terminal = self.get(id)?;
        let state = terminal
            .output
            .lock()
            .map_err(|_| "the terminal state is poisoned".to_string())?;
        let total = state.base + state.buffer.len() as u64;
        let reset = cursor < state.base;
        let start = if reset { 0 } else { (cursor - state.base) as usize };
        let data = &state.buffer[start.min(state.buffer.len())..];
        Ok(json!({
            "data": base64::engine::general_purpose::STANDARD.encode(data),
            "cursor": total,
            "reset": reset,
            "exited": state.exited,
            "code": state.code,
        }))
    }

    /// Send keystrokes or pasted text.
    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let terminal = self.get(id)?;
        let mut writer = terminal
            .writer
            .lock()
            .map_err(|_| "the terminal writer is poisoned".to_string())?;
        writer
            .write_all(data.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| format!("could not write to the terminal: {e}"))
    }

    /// Tell the PTY its new size, so curses programs redraw correctly.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let terminal = self.get(id)?;
        let master = terminal
            .master
            .lock()
            .map_err(|_| "the terminal is poisoned".to_string())?;
        master
            .resize(PtySize {
                rows: rows.max(8),
                cols: cols.max(20),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("could not resize the terminal: {e}"))
    }

    /// Kill the harness and forget the terminal.
    pub fn stop(&self, id: &str) -> Result<(), String> {
        let terminal = {
            let mut map = self
                .map
                .lock()
                .map_err(|_| "the terminal table is poisoned".to_string())?;
            map.remove(id)
        };
        if let Some(terminal) = terminal {
            if let Ok(mut child) = terminal.child.lock() {
                let _ = child.kill();
            }
        }
        Ok(())
    }

    /// Whether the terminal is still in the table.
    pub fn contains(&self, id: &str) -> bool {
        self.map
            .lock()
            .map(|map| map.contains_key(id))
            .unwrap_or(false)
    }

    fn get(&self, id: &str) -> Result<Arc<Terminal>, String> {
        self.map
            .lock()
            .map_err(|_| "the terminal table is poisoned".to_string())?
            .get(id)
            .cloned()
            .ok_or_else(|| "that terminal is no longer running".to_string())
    }
}

impl Drop for Terminals {
    fn drop(&mut self) {
        if let Ok(map) = self.map.lock() {
            for terminal in map.values() {
                if let Ok(mut child) = terminal.child.lock() {
                    let _ = child.kill();
                }
            }
        }
    }
}

fn push(terminal: &Terminal, bytes: &[u8]) {
    let Ok(mut state) = terminal.output.lock() else {
        return;
    };
    state.buffer.extend_from_slice(bytes);
    if state.buffer.len() > MAX_BUFFER {
        let drop = state.buffer.len() - MAX_BUFFER;
        state.buffer.drain(..drop);
        state.base += drop as u64;
    }
}

/// A working directory that exists, for the UI's project picker.
pub fn directory_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}
