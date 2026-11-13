//! The window's connection to devd.
//!
//! The window is a client of the same loopback API the browser interface and
//! the tray use, so it cannot do anything they cannot, and an approval it shows
//! is the same server-held record a browser would show. Two ways to get there:
//!
//! * **devd is already running** (the resident daemon, `devd ui`, another
//!   window). This process attaches to it and owns no background work.
//! * **nothing is running.** This process becomes the instance: it resumes the
//!   agents, starts the scheduled work and the supervisor, and serves the API on
//!   a loopback port of its own for itself and for anything else that attaches.
//!
//! Either way the rest of the application sees one [`Connection`]: an address
//! and a session token.

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use devd_app_core::sync::{Backend, Failure, Reply};
use devd_bootstrap::Bootstrap;
use devd_core::capability::{Actor, Ctx, Surface};
use devd_server::{Attach, Bound, Options};
use serde_json::{Value, json};

/// A running devd this window talks to.
pub struct Connection {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    /// The session's bearer token. Held in memory only.
    token: String,
    /// The loopback port, which is also the `Host` the server expects.
    pub port: u16,
    /// Whether this process is the instance, or a client of another.
    pub owner: bool,
    /// devd's data directory, for display.
    pub home: String,
    /// devd's version.
    pub version: String,
    /// Stops the server this process started, when it started one.
    shutdown: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

/// Find a running devd or become one.
pub fn connect() -> Result<Arc<Connection>, String> {
    let boot = Bootstrap::discover().map_err(|e| format!("devd could not start: {e}"))?;
    let home = boot.paths.root().display().to_string();
    let version = env!("CARGO_PKG_VERSION").to_string();
    // The team's work belongs to a process that stays up, not to this window: start devd in the background (or use the one
    // already running) and attach to it, so closing the window does not stop a run half way. Only when that cannot be done
    // does the window carry the work itself, as it always has.
    ensure_resident(&boot.paths, &version);
    match devd_server::attach_or_claim(&boot.paths, "ui")
        .map_err(|e| format!("devd could not start: {e}"))?
    {
        Attach::Client { port, .. } => {
            let token = session_token(port)?;
            Ok(Arc::new(Connection {
                base: format!("http://127.0.0.1:{port}"),
                token,
                port,
                owner: false,
                home,
                version,
                shutdown: Mutex::new(None),
            }))
        }
        Attach::Own(claim) => {
            let (bound, shutdown) = serve(boot, claim)?;
            Ok(Arc::new(Connection {
                base: format!("http://127.0.0.1:{}", bound.port),
                token: bound.token,
                port: bound.port,
                owner: true,
                home,
                version,
                shutdown: Mutex::new(Some(shutdown)),
            }))
        }
    }
}

/// Make sure devd is running in the background, the same version as this window.
///
/// `DEVD_RESIDENT=0` turns this off, for a window that must own everything itself (a test with a throwaway data folder).
fn ensure_resident(paths: &devd_core::Paths, version: &str) {
    if std::env::var("DEVD_RESIDENT").as_deref() == Ok("0") {
        return;
    }
    if let Some(held) = devd_core::daemon::current(paths) {
        // A background devd from before an update would serve this window an older devd: replace it. A window that owns
        // the instance is left alone; it is the other window, and this one will attach to it.
        if held.role == "daemon" && held.version != version {
            devd_core::process::kill_tree(held.pid);
            let deadline = Instant::now() + Duration::from_secs(10);
            while held.is_live() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = std::fs::remove_file(paths.root().join("instance.json"));
        } else {
            return;
        }
    }
    let Some(cli) = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()
                .map(|dir| dir.join(if cfg!(windows) { "devd.exe" } else { "devd" }))
        })
        .filter(|p| p.is_file())
    else {
        return;
    };
    let Ok(out) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.root().join("daemon.log"))
    else {
        return;
    };
    let Ok(err) = out.try_clone() else { return };
    let mut command = std::process::Command::new(cli);
    command
        .args(["daemon", "run"])
        .env("DEVD_HOME", paths.root())
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console, its own process group, and it outlives this window.
        command.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if command.spawn().is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if devd_core::daemon::current(paths).is_some_and(|held| held.port != 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Stop devd running in the background, when it is: the work of every run stops with it until devd is opened again.
pub fn stop_resident(home: &str) -> bool {
    let paths = devd_core::Paths::rooted(std::path::Path::new(home));
    match devd_core::daemon::current(&paths) {
        Some(held) if held.role == "daemon" => {
            devd_core::process::kill_tree(held.pid);
            let _ = std::fs::remove_file(paths.root().join("instance.json"));
            true
        }
        _ => false,
    }
}

/// Become the instance: resume the agents, start background work and serve the API.
fn serve(
    boot: Bootstrap,
    claim: devd_core::daemon::Claim,
) -> Result<(Bound, tokio::sync::watch::Sender<bool>), String> {
    boot.agents.resume(&boot.paths, &boot.store, &boot.registry);
    if devd_bootstrap::start_ops(&boot.paths, &boot.store, &boot.settings, &boot.events) {
        devd_bootstrap::start_reconcile(&boot.paths, &boot.store, &boot.settings, &boot.events);
        devd_bootstrap::start_supervisor(
            &boot.paths,
            &boot.store,
            &boot.settings,
            &boot.events,
            &boot.registry,
        );
    }
    let ctx = Ctx {
        store: boot.store.clone(),
        paths: Arc::new(boot.paths.clone()),
        settings: boot.settings.clone(),
        events: boot.events.clone(),
        actor: Actor::human(Surface::Gui),
        run: None,
        dry_run: false,
    };
    let (ready_tx, ready_rx) = mpsc::channel::<Result<Bound, String>>();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let registry = boot.registry.clone();
    let workspace = boot.workspace.clone();
    let claim = Mutex::new(claim);
    std::thread::Builder::new()
        .name("devd-server".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("cannot start the async runtime: {e}")));
                    return;
                }
            };
            let tx = ready_tx.clone();
            let options = Options {
                port: 0,
                extra_checks: Vec::new(),
                workspace: Some(workspace),
                shutdown: Some(shutdown_rx),
                picker: None,
            };
            let result = runtime.block_on(devd_server::serve_with(
                registry,
                ctx,
                options,
                move |bound| {
                    // The instance record carries the port, so `devd ui` and the
                    // tray find this window instead of starting a second server.
                    if let Ok(mut claim) = claim.lock() {
                        let _ = claim.set_port(bound.port);
                    }
                    let _ = tx.send(Ok(bound.clone()));
                },
            ));
            if let Err(e) = result {
                let _ = ready_tx.send(Err(format!("the devd backend stopped: {e}")));
            }
            // `boot` owns the agents and the event bus; they live as long as the server.
            drop(boot);
        })
        .map_err(|e| format!("cannot start the backend thread: {e}"))?;
    let bound = ready_rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "the devd backend did not become ready within 30 seconds".to_string())??;
    Ok((bound, shutdown_tx))
}

/// Read the session token of a running devd the way a browser does: from the
/// page it serves on loopback. Nothing writes the token to disk, so there is
/// nowhere else to learn it, and only a local process can ask.
fn session_token(port: u16) -> Result<String, String> {
    let page = ureq::get(&format!("http://127.0.0.1:{port}/"))
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| format!("devd is running on port {port} but did not answer: {e}"))?
        .into_string()
        .map_err(|e| format!("devd's page could not be read: {e}"))?;
    token_in(&page)
        .ok_or_else(|| format!("the devd on port {port} served a page with no session token"))
}

/// The token in the served page's `<meta name="devd-token" content="…">`.
fn token_in(page: &str) -> Option<String> {
    let at = page.find("name=\"devd-token\"")?;
    let rest = &page[at..];
    let start = rest.find("content=\"")? + "content=\"".len();
    let end = rest[start..].find('"')?;
    let token = &rest[start..start + end];
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())).then(|| token.to_string())
}

impl Connection {
    fn post(&self, path: &str, body: &Value) -> Result<Value, Failure> {
        let response = ureq::post(&format!("{}{path}", self.base))
            .timeout(Duration::from_secs(60 * 30))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string());
        match response {
            Ok(response) => response.into_json::<Value>().map_err(|e| {
                Failure::new(
                    "invalid_reply",
                    format!("devd answered with something unreadable: {e}"),
                )
            }),
            Err(ureq::Error::Status(status, response)) => {
                let body: Value = response.into_json().unwrap_or(Value::Null);
                let code = body["error"]["code"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("http_{status}"));
                let message = body["error"]["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("devd answered HTTP {status}"));
                Err(Failure::new(code, message))
            }
            Err(e) => Err(Failure::new(
                "unreachable",
                format!("devd is not answering: {e}"),
            )),
        }
    }

    /// Apply the invocation a token stands for. The server replays its own
    /// record of the arguments, so what was shown is what runs.
    pub fn approve(&self, token: &str) -> Result<Value, Failure> {
        self.post("/api/approve", &json!({ "token": token }))
    }

    /// Decline it. The refusal is recorded.
    pub fn deny(&self, token: &str) -> Result<Value, Failure> {
        self.post("/api/deny", &json!({ "token": token }))
    }

    /// Stop the server this process started. A client leaves the instance running.
    pub fn close(&self) {
        if let Some(tx) = self.shutdown.lock().ok().and_then(|mut held| held.take()) {
            let _ = tx.send(true);
        }
    }

    /// Follow devd's event stream, calling `on_event` with each event's kind and
    /// payload. Returns only when the connection ends; the caller reconnects.
    pub fn follow(
        &self,
        last_id: &mut Option<String>,
        mut on_event: impl FnMut(&str, &Value),
    ) -> Result<(), String> {
        let mut request =
            ureq::get(&format!("{}/api/events", self.base)).set("Accept", "text/event-stream");
        if let Some(id) = last_id.as_deref() {
            // What the server replays from, so nothing is missed across a reconnect.
            request = request.set("Last-Event-ID", id);
        }
        let response = request.call().map_err(|e| e.to_string())?;
        // Connected. A quiet devd sends nothing for a long time, and that is
        // not the same as being unreachable.
        on_event("stream.open", &Value::Null);
        let reader = BufReader::new(response.into_reader());
        let (mut event, mut data) = (String::new(), String::new());
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.is_empty() {
                // A blank line ends one event.
                if !data.is_empty() {
                    let payload: Value = serde_json::from_str(&data).unwrap_or(Value::Null);
                    let kind = payload["kind"].as_str().unwrap_or(&event).to_string();
                    on_event(&kind, &payload);
                }
                event.clear();
                data.clear();
            } else if let Some(value) = line.strip_prefix("event:") {
                event = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push_str(value.trim_start());
            } else if let Some(value) = line.strip_prefix("id:") {
                *last_id = Some(value.trim().to_string());
            }
        }
        Ok(())
    }
}

impl Connection {
    /// Read one of the server's own listings that is not a capability.
    fn get(&self, path: &str) -> Result<Value, Failure> {
        let response = ureq::get(&format!("{}{path}", self.base))
            .timeout(Duration::from_secs(60))
            .set("Authorization", &format!("Bearer {}", self.token))
            .call();
        match response {
            Ok(response) => response.into_json::<Value>().map_err(|e| {
                Failure::new(
                    "invalid_reply",
                    format!("devd answered with something unreadable: {e}"),
                )
            }),
            Err(ureq::Error::Status(status, _)) => Err(Failure::new(
                format!("http_{status}"),
                format!("devd answered HTTP {status}"),
            )),
            Err(e) => Err(Failure::new(
                "unreachable",
                format!("devd is not answering: {e}"),
            )),
        }
    }
}

impl Backend for Connection {
    fn invoke(&self, capability: &str, args: &Value) -> Result<Reply, Failure> {
        // Everything devd can do, as a list. It is the server's own description, not a capability, so it is
        // asked for here and reaches the screens like any other query.
        if capability == "api.capabilities" {
            // The server pages this list; the page shows all of it, so ask for
            // the largest page it serves rather than its default hundred.
            return self.get("/api/capabilities?limit=500").map(Reply::Done);
        }
        let reply = self.post("/api/invoke", &json!({ "id": capability, "args": args }))?;
        if reply["approval_required"] == json!(true) {
            let token = reply["token"].as_str().unwrap_or_default().to_string();
            return Ok(Reply::NeedsApproval { token, plan: reply });
        }
        Ok(Reply::Done(reply["outcome"]["data"].clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_read_from_the_served_page() {
        let token = "a".repeat(64);
        let page = format!(
            "<head><meta charset=\"utf-8\"><meta name=\"devd-token\" content=\"{token}\"></head>"
        );
        assert_eq!(token_in(&page), Some(token));
    }

    #[test]
    fn a_page_without_a_real_token_is_refused() {
        assert_eq!(
            token_in("<meta name=\"devd-token\" content=\"__DEVD_TOKEN__\">"),
            None
        );
        assert_eq!(token_in("<html></html>"), None);
        assert_eq!(
            token_in(&format!(
                "<meta name=\"devd-token\" content=\"{}\">",
                "z".repeat(64)
            )),
            None
        );
    }
}
