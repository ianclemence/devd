//! devd as a **client** of other programs' Model Context Protocol servers.
//!
//! The server side of MCP (this crate's [`crate::McpServer`]) lets an outside agent drive devd. This
//! module is the other direction, and it is the feature the product asked for: the assistant can use
//! the tools of an MCP server configured here, the way Claude Code uses the servers configured for it.
//!
//! # What is trusted, and what is not
//!
//! A server is a program (or an endpoint) a person added. Its *tools* are not devd capabilities: they
//! have no tier, no schema devd wrote, and no audit record devd can vouch for. So they are exposed to
//! the model under a separate prefix (`mcp__<server>__<tool>`) and they are governed by the server's
//! [`McpTrust`]:
//!
//! * `ask` (default) - an agent's call waits for a person's approval, every time, through the same
//!   approval queue as every other consequential operation.
//! * `allow` - a person decided this server's tools may run without asking.
//! * `deny` - agents do not see or call its tools.
//!
//! A tool result is **data**. It is inserted into the model's context, so it can contain text an
//! attacker controls (a file the server read, a page it fetched). Nothing here turns that text into an
//! instruction or into a capability call.
//!
//! # Two transports
//!
//! `stdio` starts the server and speaks newline-delimited JSON-RPC over its pipes. `http` speaks the
//! Streamable HTTP transport: one POST per message, JSON or an SSE response, with the session id the
//! server returned. The older two-endpoint HTTP+SSE transport is not implemented; a server that needs
//! it says so when it is added, rather than being half-supported.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::{Ctx, DevdError, McpServer, McpTool, McpTrust, Paths, Result};

use crate::PROTOCOL_VERSION;

/// The longest result text handed to the model from one external tool call.
const MAX_RESULT_TEXT: usize = 200_000;

/// How much of a server's standard error we keep to explain a failure.
const STDERR_TAIL: usize = 8_192;

/// One tool call's answer.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// The text the model reads.
    pub content: String,
    /// Whether the server marked the call as an error.
    pub is_error: bool,
}

/// A tool from an external server, with the name it is exposed under.
#[derive(Debug, Clone)]
pub struct ExternalTool {
    /// The server it belongs to.
    pub server: McpServer,
    /// The tool as the server published it.
    pub tool: McpTool,
    /// The name the model calls: `mcp__<server>__<tool>`, sanitised for tool-name rules.
    pub exposed: String,
}

/// The tools of every enabled server an agent may use, and the servers that could not be reached.
///
/// Discovery is a connection: a failure is recorded on the server row (`last_error`) and reported in
/// the second list, never silently treated as "no tools".
pub fn external_tools(ctx: &Ctx) -> (Vec<ExternalTool>, Vec<String>) {
    let mut out = Vec::new();
    let mut failures = Vec::new();
    let servers = match ctx.store.mcp_servers() {
        Ok(s) => s,
        Err(e) => return (out, vec![format!("the MCP server list could not be read: {e}")]),
    };
    let mut used = std::collections::HashSet::new();
    for server in servers.into_iter().filter(|s| s.enabled && s.trust != McpTrust::Deny) {
        match pool().catalogue(&ctx.paths, &server) {
            Ok(tools) => {
                let _ = ctx.store.mcp_set_catalogue(&server.id, &tools, None);
                for tool in tools {
                    let exposed = exposed_name(&server.name, &tool.name);
                    if !used.insert(exposed.clone()) {
                        failures.push(format!("{} has two tools exposed as `{exposed}`; only the first is offered", server.name));
                        continue;
                    }
                    out.push(ExternalTool { server: server.clone(), tool, exposed });
                }
            }
            Err(e) => {
                let message = e.to_string();
                let _ = ctx.store.mcp_set_catalogue(&server.id, &[], Some(&message));
                failures.push(format!("{}: {message}", server.name));
            }
        }
    }
    (out, failures)
}

/// The tool name the model sees. Tool names are `[A-Za-z0-9_-]`, at most 64 characters.
pub fn exposed_name(server: &str, tool: &str) -> String {
    fn clean(s: &str) -> String {
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
    }
    let server = clean(server);
    let tool = clean(tool);
    let mut name = format!("mcp__{server}__{tool}");
    if name.len() > 64 {
        name.truncate(64);
    }
    name
}

// ---------------------------------------------------------------------------------- configuration

/// Whether a name is usable as a server name and a tool prefix.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
}

/// The owner-only file holding one server's environment.
pub fn env_path(paths: &Paths, id: &str) -> PathBuf {
    paths.mcp().join(format!("{id}.env.json"))
}

/// Store a server's environment values in its owner-only file. Nothing here reaches the database.
pub fn save_env(paths: &Paths, id: &str, env: &BTreeMap<String, String>) -> Result<()> {
    std::fs::create_dir_all(paths.mcp())?;
    restrict(&paths.mcp(), true);
    let path = env_path(paths, id);
    if env.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    devd_core::util::atomic_write(&path, &serde_json::to_string_pretty(env)?)?;
    restrict(&path, false);
    Ok(())
}

/// Read a server's environment. A missing file is an empty environment, not a failure.
pub fn read_env(paths: &Paths, id: &str) -> Result<BTreeMap<String, String>> {
    let path = env_path(paths, id);
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&path)?;
    // A corrupt environment is reported, not silently replaced with an empty one: starting a server
    // without the token it was configured with looks like the server's fault and hides the real one.
    serde_json::from_str(&text).map_err(|e| DevdError::invalid(format!("the environment file for this server is not valid JSON: {e}")))
}

/// The names of a server's environment values, never their values.
pub fn env_keys(paths: &Paths, id: &str) -> Vec<String> {
    read_env(paths, id).map(|e| e.keys().cloned().collect()).unwrap_or_default()
}

/// Remove a server's environment file.
pub fn delete_env(paths: &Paths, id: &str) {
    let _ = std::fs::remove_file(env_path(paths, id));
}

#[cfg(unix)]
fn restrict(p: &std::path::Path, dir: bool) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(if dir { 0o700 } else { 0o600 }));
}

#[cfg(not(unix))]
fn restrict(_p: &std::path::Path, _dir: bool) {
    // Under the user's own profile, whose ACLs already exclude other users, as devd's other
    // owner-only files do.
}

// -------------------------------------------------------------------------------------- connection

struct Connection {
    server: McpServer,
    transport: Transport,
    next_id: u64,
    timeout: Duration,
}

enum Transport {
    Stdio {
        child: Child,
        stdin: ChildStdin,
        lines: Receiver<std::io::Result<String>>,
        stderr: Arc<Mutex<String>>,
    },
    Http {
        agent: ureq::Agent,
        url: String,
        session: Option<String>,
    },
}

impl Connection {
    fn open(server: &McpServer, env: &BTreeMap<String, String>) -> Result<Self> {
        let timeout = Duration::from_secs(server.timeout_secs.clamp(5, 600));
        let transport = match server.transport.as_str() {
            "http" => {
                let url = server.url.clone().ok_or_else(|| DevdError::invalid(format!("`{}` has no url", server.name)))?;
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(DevdError::invalid(format!("`{url}` is not an http(s) address")));
                }
                let agent = ureq::AgentBuilder::new()
                    .timeout_connect(timeout)
                    .timeout_read(timeout)
                    .timeout_write(timeout)
                    .redirects(0)
                    .build();
                Transport::Http { agent, url, session: None }
            }
            _ => {
                let program = server.command.clone().ok_or_else(|| DevdError::invalid(format!("`{}` has no command", server.name)))?;
                let mut cmd = Command::new(&program);
                cmd.args(&server.args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    // The server gets a small, named environment, not devd's: an MCP server is another
                    // program, and it has no business reading the API keys in this process's environment.
                    .env_clear()
                    .envs(base_env())
                    .envs(env);
                if let Some(dir) = &server.cwd {
                    cmd.current_dir(dir);
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    // No console window for a server the window started.
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    cmd.creation_flags(CREATE_NO_WINDOW);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    // Its own group, so stopping it stops the workers it forked.
                    cmd.process_group(0);
                }
                let mut child = cmd.spawn().map_err(|e| DevdError::mcp(format!("could not start `{program}` for `{}`: {e}", server.name)))?;
                let stdin = child.stdin.take().ok_or_else(|| DevdError::internal("no stdin"))?;
                let stdout = child.stdout.take().ok_or_else(|| DevdError::internal("no stdout"))?;
                let stderr_pipe = child.stderr.take().ok_or_else(|| DevdError::internal("no stderr"))?;
                let (tx, lines) = mpsc::channel();
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        match line {
                            Ok(l) => {
                                if tx.send(Ok(l)).is_err() {
                                    return;
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e));
                                return;
                            }
                        }
                    }
                    let _ = tx.send(Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the server closed its output")));
                });
                let stderr = Arc::new(Mutex::new(String::new()));
                let sink = stderr.clone();
                std::thread::spawn(move || {
                    let mut kept = String::new();
                    for line in BufReader::new(stderr_pipe).lines().map_while(std::result::Result::ok) {
                        kept.push_str(&line);
                        kept.push('\n');
                        if kept.len() > STDERR_TAIL {
                            let cut = kept.len() - STDERR_TAIL;
                            kept.drain(..cut);
                        }
                    }
                    if let Ok(mut s) = sink.lock() {
                        *s = kept;
                    }
                });
                Transport::Stdio { child, stdin, lines, stderr }
            }
        };
        let mut conn = Self { server: server.clone(), transport, next_id: 0, timeout };
        conn.handshake()?;
        Ok(conn)
    }

    fn handshake(&mut self) -> Result<()> {
        let info = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "devd", "version": env!("CARGO_PKG_VERSION") },
        });
        self.request("initialize", info)?;
        self.notify("notifications/initialized", json!({}))?;
        Ok(())
    }

    fn alive(&mut self) -> bool {
        match &mut self.transport {
            Transport::Stdio { child, .. } => matches!(child.try_wait(), Ok(None)),
            Transport::Http { .. } => true,
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &mut self.transport {
            Transport::Stdio { stdin, .. } => {
                writeln!(stdin, "{body}")?;
                stdin.flush()?;
                Ok(())
            }
            Transport::Http { agent, url, session } => {
                let text = post_http(agent, url, session.as_deref(), &body, self.timeout)?;
                // A notification's response body is ignored; some servers answer 202 with none.
                let _ = text;
                Ok(())
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let timeout = self.timeout;
        let name = self.server.name.clone();
        match &mut self.transport {
            Transport::Stdio { child, stdin, lines, stderr } => {
                writeln!(stdin, "{body}")?;
                stdin.flush()?;
                let deadline = Instant::now() + timeout;
                loop {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        devd_core::process::kill_tree(child.id());
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(DevdError::Timeout(format!(
                            "`{name}` did not answer `{method}` within {} seconds{}",
                            timeout.as_secs(),
                            tail_note(&stderr.lock().map(|s| s.clone()).unwrap_or_default())
                        )));
                    }
                    let line = match lines.recv_timeout(left) {
                        Ok(Ok(line)) => line,
                        Ok(Err(e)) => {
                            let tail = stderr.lock().map(|s| s.trim().to_string()).unwrap_or_default();
                            return Err(DevdError::mcp(format!("`{name}` stopped answering: {e}{}", if tail.is_empty() { String::new() } else { format!(" — {tail}") })));
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            devd_core::process::kill_tree(child.id());
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(DevdError::Timeout(format!(
                                "`{name}` did not answer `{method}` within {} seconds{}",
                                timeout.as_secs(),
                                tail_note(&stderr.lock().map(|s| s.clone()).unwrap_or_default())
                            )));
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            let tail = stderr.lock().map(|s| s.trim().to_string()).unwrap_or_default();
                            return Err(DevdError::mcp(format!("`{name}` closed its output{}", if tail.is_empty() { String::new() } else { format!(" — {tail}") })));
                        }
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let value: Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    // A response to this request.
                    if value.get("id").and_then(Value::as_u64) == Some(id) {
                        return response_result(&name, method, value);
                    }
                    // A request from the server (sampling, roots, ...). devd implements none of them.
                    if value.get("method").is_some() && value.get("id").is_some() {
                        let reply = json!({
                            "jsonrpc": "2.0",
                            "id": value["id"],
                            "error": { "code": -32601, "message": "devd does not implement this client method" }
                        });
                        let _ = writeln!(stdin, "{reply}");
                        let _ = stdin.flush();
                    }
                    // Anything else is a notification; ignore it.
                }
            }
            Transport::Http { agent, url, session } => {
                let text = post_http(agent, url, session.as_deref(), &body, timeout)?;
                if let Some(sid) = text.1 {
                    *session = Some(sid);
                }
                let value: Value = if text.0.trim().is_empty() {
                    return Err(DevdError::mcp(format!("`{name}` answered `{method}` with nothing")));
                } else if text.0.trim_start().starts_with("event:") || text.0.contains("\ndata:") {
                    sse_response(&text.0, id)?
                } else {
                    serde_json::from_str(&text.0).map_err(|e| DevdError::mcp(format!("`{name}` answered `{method}` with data that is not JSON: {e}")))?
                };
                response_result(&name, method, value)
            }
        }
    }

    fn tools(&mut self) -> Result<Vec<McpTool>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.request("tools/list", params)?;
            if let Some(items) = result.get("tools").and_then(Value::as_array) {
                for item in items {
                    let Some(name) = item.get("name").and_then(Value::as_str) else { continue };
                    if name.is_empty() {
                        continue;
                    }
                    out.push(McpTool {
                        name: name.to_string(),
                        description: item.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                        schema: item.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
                    });
                }
            }
            cursor = result.get("nextCursor").and_then(Value::as_str).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn call(&mut self, tool: &str, args: &Value) -> Result<ToolOutput> {
        let result = self.request("tools/call", json!({ "name": tool, "arguments": args }))?;
        let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let mut text = String::new();
        if let Some(blocks) = result.get("content").and_then(Value::as_array) {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                            text.push('\n');
                        }
                    }
                    Some("resource") => {
                        let r = &block["resource"];
                        if let Some(t) = r.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        } else {
                            text.push_str(&format!("[resource: {}]", r.get("uri").and_then(Value::as_str).unwrap_or("?")));
                        }
                        text.push('\n');
                    }
                    Some(kind @ ("image" | "audio")) => {
                        let mime = block.get("mimeType").and_then(Value::as_str).unwrap_or("?");
                        let bytes = block.get("data").and_then(Value::as_str).map(|d| d.len() * 3 / 4).unwrap_or(0);
                        text.push_str(&format!("[{kind}: {mime}, {bytes} bytes]\n"));
                    }
                    _ => {
                        text.push_str(&block.to_string());
                        text.push('\n');
                    }
                }
            }
        }
        if text.trim().is_empty() {
            if let Some(structured) = result.get("structuredContent") {
                text = serde_json::to_string_pretty(structured).unwrap_or_default();
            }
        }
        Ok(ToolOutput { content: devd_core::util::truncate(text.trim_end(), MAX_RESULT_TEXT).to_string(), is_error })
    }
}

fn tail_note(tail: &str) -> String {
    let t = tail.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!(" — {t}")
    }
}

fn response_result(name: &str, method: &str, value: Value) -> Result<Value> {
    if let Some(err) = value.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = err.get("message").and_then(Value::as_str).unwrap_or("no message");
        return Err(DevdError::mcp(format!("`{name}` refused `{method}`: {message} (code {code})")));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Transport::Stdio { child, .. } = &mut self.transport {
            devd_core::process::kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The environment a stdio MCP server is given: what a process needs to run, and nothing that looks
/// like a credential. A server that needs a token is given one explicitly with `mcp.add`; it is stored
/// in its own owner-only file.
fn base_env() -> BTreeMap<String, String> {
    const KEEP: &[&str] = &[
        "PATH", "PATHEXT", "SYSTEMROOT", "WINDIR", "COMSPEC", "TEMP", "TMP", "HOME", "USERPROFILE",
        "APPDATA", "LOCALAPPDATA", "PROGRAMDATA", "PROGRAMFILES", "PROGRAMFILES(X86)",
        "PROGRAMW6432", "HOMEDRIVE", "HOMEPATH", "USERNAME", "USERDOMAIN", "OS",
        "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE", "LANG", "LC_ALL", "TZ", "SHELL", "TERM",
        "TMPDIR", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "no_proxy",
    ];
    std::env::vars_os()
        .filter_map(|(k, v)| {
            let key = k.to_string_lossy().to_string();
            let upper = key.to_ascii_uppercase();
            if KEEP.contains(&upper.as_str()) {
                v.into_string().ok().map(|v| (key, v))
            } else {
                None
            }
        })
        .collect()
}

/// POST one JSON-RPC message over Streamable HTTP. Returns the body and a session id, if the server
/// issued one.
fn post_http(
    agent: &ureq::Agent,
    url: &str,
    session: Option<&str>,
    body: &Value,
    timeout: Duration,
) -> Result<(String, Option<String>)> {
    let mut req = agent
        .post(url)
        .timeout(timeout)
        .set("content-type", "application/json")
        .set("accept", "application/json, text/event-stream")
        .set("mcp-protocol-version", PROTOCOL_VERSION);
    if let Some(s) = session {
        req = req.set("mcp-session-id", s);
    }
    match req.send_string(&body.to_string()) {
        Ok(resp) => {
            let sid = resp.header("mcp-session-id").map(str::to_string);
            let text = resp.into_string().unwrap_or_default();
            Ok((text, sid))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(DevdError::mcp(format!("the server answered HTTP {code}: {}", text.trim())))
        }
        Err(ureq::Error::Transport(t)) => Err(DevdError::mcp(format!("could not reach the server: {t}"))),
    }
}

/// Find the message with `want` in a text/event-stream body.
fn sse_response(text: &str, want: u64) -> Result<Value> {
    let mut data = String::new();
    for line in text.lines().chain(std::iter::once("")) {
        if line.trim().is_empty() {
            if !data.trim().is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(&data) {
                    if v.get("id").and_then(Value::as_u64) == Some(want) {
                        return Ok(v);
                    }
                }
                data.clear();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
            data.push('\n');
        }
    }
    Err(DevdError::mcp("the server's event stream ended without an answer"))
}

// -------------------------------------------------------------------------------------------- pool

/// Connections, kept open between calls so an `npx` server is not restarted for every tool.
#[derive(Default)]
pub struct Pool {
    held: Mutex<HashMap<String, Held>>,
}

struct Held {
    fingerprint: String,
    conn: Connection,
}

impl Pool {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Held>>> {
        self.held.lock().map_err(|_| DevdError::internal("the MCP connection pool was poisoned"))
    }

    /// The tools a server publishes, reusing a live connection.
    pub fn catalogue(&self, paths: &Paths, server: &McpServer) -> Result<Vec<McpTool>> {
        if !server.enabled {
            return Err(DevdError::conflict(format!("`{}` is disabled", server.name)));
        }
        let env = read_env(paths, &server.id)?;
        let fingerprint = fingerprint(server);
        let mut held = self.lock()?;
        if let Some(entry) = held.get_mut(&server.id) {
            if entry.fingerprint == fingerprint && entry.conn.alive() {
                if let Ok(t) = entry.conn.tools() {
                    return Ok(t);
                }
            }
        }
        held.remove(&server.id);
        let mut conn = Connection::open(server, &env)?;
        let tools = conn.tools()?;
        held.insert(server.id.clone(), Held { fingerprint, conn });
        Ok(tools)
    }

    /// Call a tool. A failure is returned as it is: a tool may already have had its side effect, so a
    /// call is never retried here.
    pub fn call(&self, paths: &Paths, server: &McpServer, tool: &str, args: &Value) -> Result<ToolOutput> {
        if !server.enabled {
            return Err(DevdError::conflict(format!("`{}` is disabled", server.name)));
        }
        let env = read_env(paths, &server.id)?;
        let fingerprint = fingerprint(server);
        let mut held = self.lock()?;
        let reusable = held.get_mut(&server.id).map(|e| e.fingerprint == fingerprint && e.conn.alive()).unwrap_or(false);
        if !reusable {
            held.remove(&server.id);
        }
        if !held.contains_key(&server.id) {
            let conn = Connection::open(server, &env)?;
            held.insert(server.id.clone(), Held { fingerprint, conn });
        }
        let result = {
            let entry = held.get_mut(&server.id).expect("inserted");
            entry.conn.call(tool, args)
        };
        if result.is_err() {
            // Drop the connection: the next call starts a clean one.
            held.remove(&server.id);
        }
        result
    }

    /// A fresh connection, used by the Test action so it proves the configuration rather than a cached
    /// process.
    pub fn test(&self, paths: &Paths, server: &McpServer) -> Result<Vec<McpTool>> {
        let env = read_env(paths, &server.id)?;
        let mut conn = Connection::open(server, &env)?;
        conn.tools()
    }

    /// Forget a server's connection, after its configuration changed or it was removed.
    pub fn forget(&self, id: &str) {
        if let Ok(mut held) = self.held.lock() {
            held.remove(id);
        }
    }
}

/// The process-wide pool. It holds live connections, not business state: the rows in `mcp_servers` are
/// the truth, and a dropped connection is reopened from them.
pub fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::default)
}

fn fingerprint(server: &McpServer) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        server.updated_at,
        server.command.as_deref().unwrap_or(""),
        server.args.join("\u{1f}"),
        server.url.as_deref().unwrap_or(""),
        server.cwd.as_deref().unwrap_or(""),
        server.timeout_secs
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_sanitised_and_bounded() {
        assert_eq!(exposed_name("files", "read_file"), "mcp__files__read_file");
        assert_eq!(exposed_name("my-server", "tools/list"), "mcp__my-server__tools_list");
        let long = exposed_name("a".repeat(32).as_str(), &"b".repeat(100));
        assert!(long.len() <= 64, "{long}");
    }

    #[test]
    fn a_server_name_is_a_slug() {
        for ok in ["files", "my-server", "a1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "Files", "my_server", "-x", "a b"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn an_environment_file_is_owner_only_and_never_returns_nothing_it_should_not() {
        let dir = std::env::temp_dir().join(format!("devd-mcp-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let mut env = BTreeMap::new();
        env.insert("GITHUB_TOKEN".to_string(), "secret-value".to_string());
        save_env(&paths, "abc", &env).unwrap();
        assert_eq!(read_env(&paths, "abc").unwrap()["GITHUB_TOKEN"], "secret-value");
        assert_eq!(env_keys(&paths, "abc"), vec!["GITHUB_TOKEN".to_string()]);
        delete_env(&paths, "abc");
        assert!(read_env(&paths, "abc").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
