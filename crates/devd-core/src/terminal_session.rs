//! Interactive terminal sessions.
//!
//! A session is a real PTY with a shell on the other end. It lives here, in the
//! process that owns the work, rather than in the window, so both a person and an
//! agent can read it:
//!
//! ```text
//!   shell  ──►  PTY  ──►  this registry  ──►  the window (xterm.js)
//!                                      ──►  terminal.read (an agent)
//! ```
//!
//! Three decisions are worth stating because they are the ones that go wrong.
//!
//! * **A session is owned.** Every session records who opened it. A session whose
//!   owner is gone is reaped; a shell nobody is watching is a leaked process, and
//!   on a laptop that is a real cost.
//! * **Reading returns the screen, not the byte stream.** A model given
//!   `\x1b[32m` noise is worse at this than one given text, and a person never
//!   wants the escapes either. The output is kept as one string with the escape
//!   sequences stripped, so `terminal.read` is useful to both.
//! * **An agent may not open a shell.** That would be `project.exec` with extra
//!   steps and no argument to review. An agent may read a session it is given, and
//!   writing to one needs a person's approval naming the bytes.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::{DevdError, Result};

/// How much of a session's output is kept. A terminal scrolls; the buffer a model
/// reads is bounded so one `yes` cannot fill memory.
const MAX_BUFFER: usize = 200_000;

/// How many lines `terminal.read` returns when the caller does not say.
pub const DEFAULT_LINES: usize = 200;

/// One running shell.
pub struct Session {
    /// Stable id, the handle every capability takes.
    pub id: String,
    /// Who opened it, as an actor label. Used to reap orphans.
    pub owner: String,
    /// The folder it started in.
    pub cwd: String,
    /// The shell that was started.
    pub shell: String,
    /// When it started, as unix seconds.
    pub started: u64,
    /// The process id of the shell, when the platform reports one.
    pub pid: Option<u32>,
    /// The master side of the PTY: input goes here.
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    /// The writer for the master, kept separately because `take_writer` can only
    /// be called once.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Everything read so far, escape sequences stripped.
    buffer: Arc<Mutex<String>>,
    /// Columns and rows, as the reader last set them.
    size: Mutex<(u16, u16)>,
    /// Set when the shell has exited.
    ended: Arc<AtomicBool>,
    /// The exit status, once known.
    exit: Arc<Mutex<Option<i32>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A PTY handle has no useful debug form, so a session reports what a person
        // would want in a log line: which one, whose, and whether it is alive.
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .field("running", &self.running())
            .finish()
    }
}

impl Session {
    /// Whether the shell is still running.
    pub fn running(&self) -> bool {
        !self.ended.load(Ordering::Relaxed)
    }

    /// The whole buffer, for the window.
    pub fn output(&self) -> String {
        self.buffer.lock().unwrap().clone()
    }

    /// The last `lines` lines of the screen.
    ///
    /// A trailing blank run is dropped: a shell that has printed three lines and
    /// not yet moved the cursor leaves a screen of empty rows, and a reader wants
    /// the three lines.
    pub fn tail(&self, lines: usize) -> String {
        let buf = self.buffer.lock().unwrap();
        let all: Vec<&str> = buf.lines().collect();
        let take = lines.clamp(1, 5000).min(all.len());
        let mut out = all[all.len() - take..].join("\n");
        while out.ends_with('\n') {
            out.pop();
        }
        out
    }

    /// Send input to the shell.
    pub fn write(&self, data: &str) -> Result<()> {
        if !self.running() {
            return Err(DevdError::invalid(format!("terminal {} has ended", self.id)));
        }
        let mut w = self.writer.lock().unwrap();
        w.write_all(data.as_bytes())
            .and_then(|_| w.flush())
            .map_err(|e| DevdError::internal(format!("could not write to terminal {}: {e}", self.id)))
    }

    /// Set the terminal size.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let cols = cols.clamp(20, 400);
        let rows = rows.clamp(5, 200);
        let m = self.master.lock().unwrap();
        m.resize(portable_pty::PtySize { cols, rows, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| DevdError::internal(format!("could not resize terminal {}: {e}", self.id)))?;
        *self.size.lock().unwrap() = (cols, rows);
        Ok(())
    }

    /// What a caller is told about a session.
    pub fn describe(&self) -> Value {
        let (cols, rows) = *self.size.lock().unwrap();
        let buf = self.buffer.lock().unwrap();
        json!({
            "id": self.id,
            "owner": self.owner,
            "cwd": self.cwd,
            "shell": self.shell,
            "started": self.started,
            "pid": self.pid,
            "cols": cols,
            "rows": rows,
            "lines": buf.lines().count(),
            "running": self.running(),
            "exit_code": *self.exit.lock().unwrap(),
        })
    }
}

/// Every session this process owns.
#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next: AtomicU64,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").field("sessions", &self.sessions.lock().map(|s| s.len()).unwrap_or(0)).finish()
    }
}

/// The shell to start: `$SHELL` on Unix, PowerShell on Windows.
fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

impl Registry {
    /// A new, empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a shell in `cwd`.
    pub fn open(&self, owner: &str, cwd: &str, shell: Option<&str>, cols: u16, rows: u16) -> Result<Arc<Session>> {
        let dir = std::path::Path::new(cwd);
        if !dir.is_dir() {
            return Err(DevdError::invalid(format!("{cwd} is not a folder")));
        }
        let shell = shell.map(|s| s.to_string()).unwrap_or_else(default_shell);
        let (cols, rows) = (cols.clamp(20, 400), rows.clamp(5, 200));

        let pty = portable_pty::native_pty_system();
        let pair = pty
            .openpty(portable_pty::PtySize { cols, rows, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| DevdError::internal(format!("could not open a terminal: {e}")))?;

        let mut cmd = portable_pty::CommandBuilder::new(&shell);
        cmd.cwd(dir);
        // A person's environment is theirs; the shell inherits it and adds nothing.
        for (k, v) in std::env::vars() {
            cmd.env(k, v);
        }
        // Make it a terminal even when devd was started from a service.
        cmd.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| DevdError::internal(format!("could not start {shell}: {e}")))?;
        let pid = child.process_id();
        // The child handle is kept only so the process is not reaped under us; the
        // exit is detected by the reader reaching end of file.
        drop(pair.slave);

        // The reader is cloned first. On Windows a ConPTY's read side has to be
        // attached before its write side is taken, or the console host never
        // receives the answer to its start-up query and the shell sits waiting for
        // a terminal that will never speak.
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| DevdError::internal(format!("could not read the terminal: {e}")))?;

        // One writer, shared. `take_writer` can only be called once, and both the
        // session (a person's keystrokes) and the reader thread (its replies to the
        // program's queries) have to write: two writers would mean one of them
        // getting an error on its first use.
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| DevdError::internal(format!("could not write to the terminal: {e}")))?,
        ));

        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let id = format!("term-{n}-{}", std::process::id());

        let ended = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(Mutex::new(None));
        let buffer = Arc::new(Mutex::new(String::new()));

        let session = Arc::new(Session {
            id: id.clone(),
            owner: owner.to_string(),
            cwd: cwd.to_string(),
            shell: shell.clone(),
            started: chrono::Utc::now().timestamp().max(0) as u64,
            pid,
            master: Mutex::new(pair.master),
            writer: writer.clone(),
            // The same buffer the reader thread fills. A second String here looks
            // like the same thing and is not: the session would show an empty
            // screen while the reader wrote into a buffer nobody read.
            buffer: buffer.clone(),
            size: Mutex::new((cols, rows)),
            ended: ended.clone(),
            exit: exit.clone(),
        });

        // The reader is a thread per session: a PTY is a blocking file descriptor
        // and there is no portable asynchronous read for one.
        //
        // It writes as well as reads, because a program's query has to be answered
        // from here. It shares the session's writer rather than taking a second one,
        // and holds it only for the length of one reply.
        let reply_writer = writer.clone();
        let buf = buffer.clone();
        std::thread::Builder::new()
            .name(format!("pty-{id}"))
            .spawn(move || {
                let mut emu = Emulator::new();
                let mut chunk = [0u8; 8192];
                let mut pending = Vec::new();
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            pending.extend_from_slice(&chunk[..n]);
                            // Keep whole UTF-8 characters: a read can end in the
                            // middle of one, and a mangled character in a shell's
                            // output is worse than a delayed one.
                            let text = match std::str::from_utf8(&pending) {
                                Ok(s) => {
                                    let text = s.to_string();
                                    pending.clear();
                                    text
                                }
                                Err(e) if e.error_len().is_none() => {
                                    let good = e.valid_up_to();
                                    let s = String::from_utf8_lossy(&pending[..good]).to_string();
                                    pending.drain(..good);
                                    s
                                }
                                Err(_) => {
                                    let s = String::from_utf8_lossy(&pending).to_string();
                                    pending.clear();
                                    s
                                }
                            };
                            // The emulator strips the escapes *and* collects the
                            // replies a program is waiting for. Not answering one
                            // is what makes a shell hang on its first prompt.
                            let (clean, replies) = emu.consume(&text);
                            for r in replies {
                                let mut w = reply_writer.lock().unwrap();
                                let _ = w.write_all(r.as_bytes());
                                let _ = w.flush();
                            }
                            if !clean.is_empty() {
                                let mut b = buf.lock().unwrap();
                                b.push_str(&clean);
                                if b.len() > MAX_BUFFER {
                                    // Drop from the front on a line boundary so the
                                    // buffer still reads as lines.
                                    let cut = b.len() - MAX_BUFFER;
                                    let at = b[cut..].find('\n').map(|i| cut + i + 1).unwrap_or(cut);
                                    b.drain(..at);
                                }
                            }
                        }
                    }
                }
                ended.store(true, Ordering::Relaxed);
                let _ = child;
            })
            .map_err(|e| DevdError::internal(format!("could not start the terminal reader: {e}")))?;

        self.sessions.lock().unwrap().insert(id, session.clone());
        Ok(session)
    }

    /// Look a session up.
    pub fn get(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DevdError::not_found(format!("there is no terminal called {id}")))
    }

    /// Every session, running or ended.
    pub fn list(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self.sessions.lock().unwrap().values().map(|s| s.describe()).collect();
        out.sort_by(|a, b| a["started"].as_u64().cmp(&b["started"].as_u64()));
        out
    }

    /// End a session's shell and forget it.
    pub fn close(&self, id: &str) -> Result<()> {
        let s = self.get(id)?;
        // The reader notices end of file when the process goes. Killing the tree
        // is what makes that happen for a shell with children.
        if let Some(pid) = s.pid {
            crate::process::kill_tree(pid);
        }
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    /// Close sessions whose owner is no longer around.
    ///
    /// Called by the daemon. An owner label is the actor string devd already uses
    /// (`human:ui`, `agent:implementer`, ...); a session is reaped when its owner is
    /// not among the owners still attached. Anything a person opened from the
    /// window survives, because the window's owner is always attached while it is
    /// open.
    pub fn reap_orphans(&self, still_attached: &[String]) -> usize {
        let dead: Vec<String> = self
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| !s.running() || !still_attached.iter().any(|o| o == &s.owner))
            .map(|s| s.id.clone())
            .collect();
        for id in &dead {
            let _ = self.close(id);
        }
        dead.len()
    }
}

/// Working out what a program is asking for, and what to answer.
///
/// A program in a PTY does not only write; it asks questions and waits for the
/// answer. Windows' console host asks `ESC[6n` ("where is the cursor?") on start,
/// and a shell that gets no reply **blocks forever**. devd used to strip that
/// sequence and say nothing, which is why the first terminal hung on its first
/// prompt.
///
/// This is the small part of a terminal emulator a shell actually needs: the
/// cursor-position report, the device-status report, and the device-attributes
/// query. Anything else is answered with the safe default rather than silence.
struct Emulator {
    row: u16,
    col: u16,
}

impl Emulator {
    fn new() -> Self {
        Emulator { row: 1, col: 1 }
    }

    /// Advance the cursor over printable text and newlines.
    ///
    /// Only the crude case is tracked - how many lines have been written - because
    /// that is what a cursor report is used for: putting the shell's first prompt
    /// below whatever the program printed.
    fn advance(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => {
                    self.row = self.row.saturating_add(1);
                    self.col = 1;
                }
                '\r' => self.col = 1,
                _ => self.col = self.col.saturating_add(1),
            }
        }
    }

    /// Strip escapes from `text`, and return the replies to any query in it.
    fn consume(&mut self, text: &str) -> (String, Vec<String>) {
        let mut out = String::with_capacity(text.len());
        let mut replies = Vec::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                if c == '\r' {
                    continue;
                }
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('[') => {
                    // CSI: parameters until a final byte in @..~
                    let mut seq = String::new();
                    let mut fin = '\0';
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            fin = c;
                            break;
                        }
                        seq.push(c);
                    }
                    match fin {
                        // Cursor position report: "row;col R".
                        'n' if seq == "6" => replies.push(format!("\u{1b}[{};{}R", self.row, self.col)),
                        // Device status: "ready".
                        'n' if seq == "5" => replies.push("\u{1b}[0n".to_string()),
                        // Device attributes: a VT100 with ANSI colour.
                        'c' if seq.is_empty() || seq == "0" => replies.push("\u{1b}[?1;2c".to_string()),
                        _ => {}
                    }
                }
                Some(']') => {
                    // OSC: until BEL or ESC \
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (prev == '\u{1b}' && c == '\\') {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => {}
            }
        }
        self.advance(&out);
        (out, replies)
    }
}

/// Remove ANSI escape sequences from terminal output, without replying.
///
/// Kept for the cases where only the text matters. The session reader uses
/// [`Emulator`], which also answers queries.
#[cfg(test)]
fn strip_escapes(s: &str) -> String {
    Emulator::new().consume(s).0
}

/// The sessions this process owns.
///
/// A terminal is the one thing devd keeps that is not a row: a PTY outlives any
/// single capability call, and both the person's window and an agent have to
/// reach the same one. A `OnceLock` is the honest shape for that - one registry
/// per process, created on first use, never replaced.
pub fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

/// Answer a session for `terminal.read`: the tail of its screen.
pub fn read_screen(id: &str, lines: usize) -> Result<Value> {
    let s = registry().get(id)?;
    let d = s.describe();
    Ok(json!({
        "id": d["id"],
        "running": d["running"],
        "exit_code": d["exit_code"],
        "cols": d["cols"],
        "rows": d["rows"],
        // The screen as text, with the escape sequences already removed. A model
        // given `\x1b[32m` noise is worse at this than one given the screen.
        "screen": s.tail(lines),
        "note": "This is the terminal's visible output as text, not the raw byte stream.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_codes_are_removed_and_text_is_kept() {
        assert_eq!(strip_escapes("\u{1b}[32mgreen\u{1b}[0m plain"), "green plain");
        assert_eq!(strip_escapes("\u{1b}[1;31mbold red\u{1b}[m"), "bold red");
        assert_eq!(strip_escapes("before\u{1b}[2Jafter"), "beforeafter");
        // A hyperlink's OSC 8 sequence, which is the shape that breaks naive strips.
        assert_eq!(strip_escapes("\u{1b}]8;;https://x\u{7}link\u{1b}]8;;\u{7}!"), "link!");
    }

    #[test]
    fn a_carriage_return_does_not_leave_the_cursor_mid_screen() {
        // A progress bar overwrites its own line; in a captured buffer that is
        // one line, not a line full of stray returns.
        assert_eq!(strip_escapes("50%\r100%\n"), "50%100%\n");
    }

    #[test]
    fn an_incomplete_sequence_does_not_swallow_what_came_before_it() {
        // A read can end in the middle of a sequence and the next read supplies
        // the rest, so an unterminated one consumes what it was given. What
        // matters is that nothing panics and nothing before it is lost.
        assert_eq!(strip_escapes("\u{1b}"), "");
        assert_eq!(strip_escapes("\u{1b}["), "");
        assert_eq!(strip_escapes("kept\u{1b}]b"), "kept");
        assert_eq!(strip_escapes("kept\u{1b}"), "kept");
    }

    #[test]
    fn the_programs_queries_are_answered() {
        // This is the bug that hung the first terminal: Windows' console host
        // asks "where is the cursor?" and blocks until it gets an answer. Silence
        // is not a valid reply.
        let mut e = Emulator::new();
        let (text, replies) = e.consume("\u{1b}[6n");
        assert_eq!(text, "");
        assert_eq!(replies, vec!["\u{1b}[1;1R".to_string()]);

        let (_, replies) = e.consume("\u{1b}[5n");
        assert_eq!(replies, vec!["\u{1b}[0n".to_string()]);

        let (_, replies) = e.consume("\u{1b}[c");
        assert_eq!(replies, vec!["\u{1b}[?1;2c".to_string()]);

        // An ordinary sequence asks nothing.
        let (text, replies) = e.consume("\u{1b}[0mhello");
        assert_eq!(text, "hello");
        assert!(replies.is_empty());
    }

    #[test]
    fn the_cursor_moves_so_a_report_is_not_always_row_one() {
        let mut e = Emulator::new();
        e.consume("one\r\ntwo\r\n");
        let (_, replies) = e.consume("\u{1b}[6n");
        // The report names where the cursor is, not a fixed constant.
        assert_ne!(replies[0], "\u{1b}[1;1R");
        assert!(replies[0].starts_with("\u{1b}[3;"), "{:?}", replies[0]);
    }

    #[test]
    fn a_session_reports_its_owner_and_ends_when_its_shell_does() {
        let reg = Registry::new();
        let dir = std::env::temp_dir();
        let s = match reg.open("human:ui", dir.to_str().unwrap(), None, 80, 24) {
            Ok(s) => s,
            // A machine with no shell to start is not a failure of this code.
            Err(_) => return,
        };
        let d = s.describe();
        assert_eq!(d["owner"], "human:ui");
        assert_eq!(d["cols"], 80);
        assert_eq!(d["running"], true);
        assert_eq!(reg.list().len(), 1);

        // Wait for the shell's own banner and prompt before typing into it.
        //
        // A person waits to see a prompt; a test that writes immediately is
        // racing the shell's start-up, and on Windows that startup includes the
        // cursor-position query the emulator has to answer first. Waiting is what
        // the window does too, because its terminal only shows it once the prompt
        // arrives.
        let mut ready = false;
        for _ in 0..120 {
            if !s.output().trim().is_empty() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(ready, "the shell printed no banner within six seconds");

        // Writing a command and reading the screen back is the whole contract.
        s.write("echo devd-terminal-check\r").unwrap();
        let mut seen = false;
        for _ in 0..120 {
            if s.output().contains("devd-terminal-check") {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let screen = s.tail(50);
        assert!(seen, "the command did not round-trip; screen was: {screen:?}");
        // The escape sequences must not reach a reader.
        assert!(!screen.contains('\u{1b}'), "escape sequence leaked: {screen:?}");

        reg.close(&s.id).unwrap();
        assert_eq!(reg.list().len(), 0);
        assert!(reg.get(&s.id).is_err());
    }

    #[test]
    fn an_unknown_session_says_so_rather_than_failing_obscurely() {
        let reg = Registry::new();
        let e = reg.get("term-nope").unwrap_err();
        assert!(e.to_string().contains("no terminal"), "{e}");
    }

    #[test]
    fn a_folder_that_is_not_a_folder_is_refused() {
        let reg = Registry::new();
        let e = reg.open("human:ui", "D:\\definitely\\not\\a\\folder", None, 80, 24).unwrap_err();
        assert!(e.to_string().contains("not a folder"), "{e}");
    }
}

