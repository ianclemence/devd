//! An interactive browser an agent and a person share.
//!
//! `web.open` answers "what does this page say?". This answers "what happens if
//! I press that?", which needs a real tab: a page the agent can navigate, read
//! as an accessibility tree, click and type in, and screenshot for the person.
//!
//! The shape follows the browser model that works well in mature agent tools:
//!
//! * **One tab id, everywhere.** A tab is a Chromium target id; every command
//!   names the tab it acts on. There is no ambient "current page" that can drift.
//! * **Refs come from a snapshot.** `browser.snapshot` assigns `e1`, `e2`, ...
//!   to the actionable nodes in the accessibility tree and remembers the mapping.
//!   A navigation clears it, so a stale ref can never click a different element.
//! * **Typed states, not empty strings.** A command says `ok`, `empty`,
//!   `unavailable` or `failed` and why. An agent can tell "no console output"
//!   from "the tab is gone".
//! * **The picture is the evidence.** A screenshot is written to a file and
//!   shown in devd's Browser pane; the model gets the file name and the page's
//!   text, never a claim that something "looks right".
//!
//! The browser is Chromium driven over the DevTools Protocol. devd launches a
//! private headless instance (Edge or Chrome) with its own profile, so the
//! person's normal browser, cookies and windows are never touched. Pages may be
//! public or a local dev server: a browser that could not reach `localhost`
//! would be useless for the work this exists to do.

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};
use tungstenite::{connect, Message};

use devd_core::{DevdError, Result};

/// Most nodes one snapshot returns. A huge page is a wall of text, not a map.
const MAX_NODES: usize = 400;
/// How long a navigation or a settle may take before the command says so.
const WAIT: Duration = Duration::from_secs(20);
/// Console and network buffers are bounded so a noisy page cannot grow without
/// end. The same bound is applied inside the page.
const MAX_LOG: usize = 300;

/// The page-side recorder. It runs before any page script and keeps the last
/// few hundred console lines and network requests on `window`, so `browser.console`
/// can read them without keeping a socket listener alive between commands.
const RECORDER: &str = r#"
(function () {
  if (window.__devdRecorder) return;
  window.__devdRecorder = true;
  var logs = [], net = [];
  function clip(v) { try { return String(v).slice(0, 2000); } catch (e) { return ''; } }
  function push(level, args) {
    try {
      var text = Array.prototype.map.call(args, function (a) {
        if (typeof a === 'string') return a;
        try { return JSON.stringify(a); } catch (e) { return String(a); }
      }).join(' ');
      logs.push({ level: level, text: clip(text) });
      if (logs.length > 300) logs.splice(0, logs.length - 300);
    } catch (e) {}
  }
  ['log', 'info', 'warn', 'error', 'debug'].forEach(function (level) {
    var original = console[level];
    console[level] = function () { push(level, arguments); return original.apply(console, arguments); };
  });
  window.addEventListener('error', function (e) {
    push('error', [(e.message || 'script error') + ' @ ' + (e.filename || '') + ':' + (e.lineno || 0)]);
  });
  window.addEventListener('unhandledrejection', function (e) {
    push('error', ['Unhandled rejection: ' + (e.reason && e.reason.stack || e.reason)]);
  });
  function record(method, url, status, ms, error) {
    net.push({ method: method, url: clip(url), status: status, ms: ms, error: error || null });
    if (net.length > 300) net.splice(0, net.length - 300);
  }
  var fetch0 = window.fetch;
  if (fetch0) window.fetch = function (input, init) {
    var url = typeof input === 'string' ? input : (input && input.url) || '';
    var method = (init && init.method) || (input && input.method) || 'GET';
    var start = Date.now();
    return fetch0.apply(this, arguments).then(function (res) {
      record(method, url, res.status, Date.now() - start, null);
      return res;
    }, function (err) {
      record(method, url, 0, Date.now() - start, clip(err));
      throw err;
    });
  };
  var open0 = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__devd = { method: method, url: String(url) };
    return open0.apply(this, arguments);
  };
  var send0 = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function () {
    var xhr = this, start = Date.now();
    xhr.addEventListener('loadend', function () {
      var d = xhr.__devd || {};
      record(d.method || 'GET', d.url || '', xhr.status, Date.now() - start, xhr.status ? null : 'network error');
    });
    return send0.apply(this, arguments);
  };
  window.__devdConsole = logs;
  window.__devdNetwork = net;
})();
"#;

fn manager() -> &'static Mutex<Manager> {
    static MANAGER: OnceLock<Mutex<Manager>> = OnceLock::new();
    MANAGER.get_or_init(|| Mutex::new(Manager::default()))
}

fn lock() -> std::sync::MutexGuard<'static, Manager> {
    // A panic while holding the browser lock is a bug, but refusing every later
    // call would turn one bad page into a dead feature. Recover the state and
    // let the next command report what it sees.
    manager().lock().unwrap_or_else(|e| e.into_inner())
}

/// The whole browser: one private Chromium process and the tabs it owns.
#[derive(Default)]
struct Manager {
    child: Option<Child>,
    cdp: Option<Cdp>,
    root: Option<PathBuf>,
    tabs: BTreeMap<String, Tab>,
    focused: Option<String>,
    seq: u64,
    /// Why the browser could not start, kept so a retry does not re-launch a
    /// doomed process on every call.
    last_error: Option<String>,
}

struct Tab {
    session: String,
    url: String,
    title: String,
    /// The page's own icon URL, as a real browser tab shows.
    favicon: Option<String>,
    refs: BTreeMap<String, i64>,
}

impl Manager {
    fn ensure(&mut self, root: &Path) -> Result<()> {
        if self.cdp.is_some() {
            return Ok(());
        }
        if let Some(why) = &self.last_error {
            return Err(DevdError::invalid(why.clone()));
        }
        let exe = match find_browser() {
            Ok(exe) => exe,
            Err(e) => {
                let why = e.to_string();
                self.last_error = Some(why.clone());
                return Err(e);
            }
        };
        let profile = root.join("browser").join("profile");
        std::fs::create_dir_all(&profile)?;
        let (mut child, websocket) = launch(&exe, &profile)?;
        // Prove the port answers before the WebSocket handshake, so a failure
        // names the address rather than a generic "connection closed".
        if let Some((host, port)) = ws_host_port(&websocket) {
            let addr = format!("{host}:{port}");
            let parsed = addr
                .parse()
                .map_err(|_| DevdError::internal(format!("the browser announced an unusable DevTools address: {addr}")))?;
            if let Err(e) = std::net::TcpStream::connect_timeout(&parsed, Duration::from_secs(5)) {
                let _ = kill_tree(&mut child);
                self.last_error = Some(format!("the browser did not answer on its DevTools port {addr}: {e}"));
                return Err(DevdError::internal(self.last_error.clone().unwrap_or_default()));
            }
        }
        let (mut socket, _) = connect(websocket.as_str()).map_err(|e| {
            let _ = kill_tree(&mut child);
            DevdError::internal(format!("could not open the browser's DevTools session at {websocket}: {e}"))
        })?;
        // A browser that stops answering must not hang the capability (and the
        // window) for ever. The protocol is local and fast; thirty seconds is
        // already a broken page.
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_mut() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        }
        self.child = Some(child);
        self.cdp = Some(Cdp { socket, next: 0 });
        self.root = Some(root.to_path_buf());
        // The launch opens one about:blank page; attach it so the first command
        // has a tab to act on.
        self.refresh()?;
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        let Some(cdp) = self.cdp.as_mut() else { return Ok(()) };
        let targets = cdp.call(None, "Target.getTargets", json!({}))?;
        let mut live = HashSet::new();
        let mut pages = Vec::new();
        for t in targets["targetInfos"].as_array().cloned().unwrap_or_default() {
            if t["type"].as_str() == Some("page") {
                if let Some(id) = t["targetId"].as_str() {
                    live.insert(id.to_string());
                    pages.push((id.to_string(), t["url"].as_str().unwrap_or("").to_string(), t["title"].as_str().unwrap_or("").to_string()));
                }
            }
        }
        // A page the site opened (a popup, a new window) becomes a tab too, so
        // the model never loses a page because it was not the one it created.
        for (id, url, title) in pages {
            if self.tabs.contains_key(&id) {
                continue;
            }
            let v = cdp.call(None, "Target.attachToTarget", json!({ "targetId": id, "flatten": true }))?;
            if let Some(session) = v["sessionId"].as_str() {
                let session = session.to_string();
                enable(&mut *cdp, &session)?;
                self.tabs.insert(id.clone(), Tab { session, url, title, favicon: None, refs: BTreeMap::new() });
            }
        }
        self.tabs.retain(|id, _| live.contains(id));
        if self.focused.as_ref().map(|f| !self.tabs.contains_key(f)).unwrap_or(true) {
            self.focused = self.tabs.keys().next().cloned();
        }
        Ok(())
    }

    fn focused_tab(&self, requested: Option<&str>) -> Result<String> {
        if let Some(id) = requested.filter(|s| !s.is_empty()) {
            if self.tabs.contains_key(id) {
                return Ok(id.to_string());
            }
            return Err(DevdError::not_found(format!(
                "there is no tab `{id}`; browser.tabs.list shows the ids"
            )));
        }
        self.focused
            .clone()
            .or_else(|| self.tabs.keys().next().cloned())
            .ok_or_else(|| DevdError::invalid("no tab is open; call browser.tabs.open or browser.open"))
    }

    fn open_tab(&mut self, url: Option<&str>) -> Result<String> {
        let cdp = self.cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
        let target = cdp.call(None, "Target.createTarget", json!({ "url": "about:blank" }))?;
        let id = target["targetId"]
            .as_str()
            .ok_or_else(|| DevdError::internal("the browser did not return a tab id"))?
            .to_string();
        let attached = cdp.call(None, "Target.attachToTarget", json!({ "targetId": id, "flatten": true }))?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| DevdError::internal("the browser did not attach to the tab"))?
            .to_string();
        enable(cdp, &session)?;
        self.tabs.insert(id.clone(), Tab {
            session,
            url: "about:blank".into(),
            title: String::new(),
            favicon: None,
            refs: BTreeMap::new(),
        });
        self.focused = Some(id.clone());
        if let Some(url) = url.filter(|u| !u.is_empty()) {
            self.navigate(&id, url)?;
        }
        Ok(id)
    }

    fn navigate(&mut self, id: &str, url: &str) -> Result<Value> {
        check_url(url)?;
        let cdp = self.cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
        let tab = self.tabs.get_mut(id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?;
        cdp.call(Some(&tab.session), "Page.navigate", json!({ "url": url }))?;
        tab.refs.clear();
        wait_ready(cdp, &tab.session)?;
        let (now, title) = location(cdp, &tab.session)?;
        tab.url = now.clone();
        tab.title = title.clone();
        Ok(json!({ "state": "ok", "tab": id, "url": now, "title": title }))
    }

    fn current(&self, id: &str) -> Result<(String, String)> {
        let tab = self.tabs.get(id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?;
        Ok((tab.url.clone(), tab.title.clone()))
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            #[cfg(not(windows))]
            {
                let _ = child.kill();
            }
        }
    }
}

/// One DevTools connection. Commands for a tab carry its `sessionId`; commands
/// about the browser itself (`Target.*`) do not.
struct Cdp {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
    next: u64,
}

impl Cdp {
    fn call(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value> {
        let id = self.next;
        self.next += 1;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(message.to_string()))
            .map_err(|e| DevdError::internal(format!("could not talk to the browser: {e}")))?;
        loop {
            let incoming = self
                .socket
                .read()
                .map_err(|e| DevdError::internal(format!("the browser closed the connection: {e}")))?;
            match incoming {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|e| DevdError::internal(format!("the browser sent an unreadable message: {e}")))?;
                    if value["id"].as_u64() == Some(id) {
                        if !value["error"].is_null() {
                            return Err(DevdError::invalid(format!(
                                "{method} was refused: {}",
                                value["error"]["message"].as_str().unwrap_or("the browser did not say why")
                            )));
                        }
                        return Ok(value["result"].clone());
                    }
                    // Events are read past; the recorder keeps what a later
                    // command needs, so no listener has to stay alive.
                }
                Message::Ping(payload) => {
                    let _ = self.socket.send(Message::Pong(payload));
                }
                Message::Close(_) => {
                    return Err(DevdError::internal("the browser closed its DevTools connection"));
                }
                _ => {}
            }
        }
    }
}

fn enable(cdp: &mut Cdp, session: &str) -> Result<()> {
    for method in ["Page.enable", "Runtime.enable", "DOM.enable", "Accessibility.enable", "Log.enable"] {
        cdp.call(Some(session), method, json!({}))?;
    }
    // Install the recorder now and again before every document, so a console
    // line written before devd's first command is still captured.
    cdp.call(
        Some(session),
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": RECORDER }),
    )?;
    let _ = cdp.call(Some(session), "Runtime.evaluate", json!({ "expression": RECORDER }));
    Ok(())
}

fn location(cdp: &mut Cdp, session: &str) -> Result<(String, String)> {
    let value = cdp.call(
        Some(session),
        "Runtime.evaluate",
        json!({ "expression": "JSON.stringify({ url: location.href, title: document.title })", "returnByValue": true }),
    )?;
    let text = value["result"]["value"].as_str().unwrap_or("{}");
    let parsed: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    Ok((
        parsed["url"].as_str().unwrap_or("").to_string(),
        parsed["title"].as_str().unwrap_or("").to_string(),
    ))
}

/// The page's own title and icon, the way a real browser tab reads them.
///
/// The icon is the URL the page declared, or `/favicon.ico`; the browser tab
/// fetches it itself. Reading the DOM here is safe because this runs in the
/// page, not in a cross-origin iframe.
fn page_meta(cdp: &mut Cdp, session: &str) -> Result<(String, String, Option<String>)> {
    let expression = r#"(() => {
        let href = '';
        try {
            const icon = document.querySelector('link[rel~="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]');
            href = icon && icon.href ? icon.href : new URL('/favicon.ico', location.href).href;
        } catch (e) { href = ''; }
        return JSON.stringify({ url: location.href, title: document.title, favicon: href || null });
    })()"#;
    let value = cdp.call(
        Some(session),
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true }),
    )?;
    let text = value["result"]["value"].as_str().unwrap_or("{}");
    let parsed: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    Ok((
        parsed["url"].as_str().unwrap_or("").to_string(),
        parsed["title"].as_str().unwrap_or("").to_string(),
        parsed["favicon"].as_str().map(str::to_string),
    ))
}

fn ready(cdp: &mut Cdp, session: &str) -> Result<bool> {
    let value = cdp.call(
        Some(session),
        "Runtime.evaluate",
        json!({ "expression": "document.readyState", "returnByValue": true }),
    )?;
    Ok(value["result"]["value"].as_str() == Some("complete"))
}

fn wait_ready(cdp: &mut Cdp, session: &str) -> Result<()> {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if ready(cdp, session).unwrap_or(false) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    // A page that never reaches `complete` is still a page; report what is there
    // rather than failing the whole command. The caller sees the state it has.
    Ok(())
}

// ------------------------------------------------------------------ snapshot

fn actionable(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "listbox"
            | "option"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "tab"
            | "switch"
            | "slider"
            | "spinbutton"
            | "treeitem"
    )
}

fn content(role: &str) -> bool {
    matches!(
        role,
        "heading" | "StaticText" | "LabelText" | "image" | "paragraph" | "cell" | "columnheader" | "rowheader" | "listitem"
    )
}

fn clip(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

fn snapshot(cdp: &mut Cdp, session: &str) -> Result<Value> {
    let tree = cdp.call(Some(session), "Accessibility.getFullAXTree", json!({}))?;
    let nodes = tree["nodes"].as_array().cloned().unwrap_or_default();
    let mut refs = BTreeMap::new();
    let mut out = Vec::new();
    let mut n = 0usize;
    for node in nodes {
        if node["ignored"].as_bool().unwrap_or(false) {
            continue;
        }
        let role = node["role"]["value"].as_str().unwrap_or("");
        let name = node["name"]["value"].as_str().unwrap_or("");
        let value = node["value"]["value"].as_str().unwrap_or("");
        if name.trim().is_empty() && value.trim().is_empty() {
            continue;
        }
        let mut item = json!({ "role": role, "name": clip(name, 180) });
        if !value.is_empty() {
            item["value"] = json!(clip(value, 180));
        }
        for p in node["properties"].as_array().cloned().unwrap_or_default() {
            let key = p["name"].as_str().unwrap_or("");
            if matches!(key, "checked" | "disabled" | "expanded" | "selected" | "required" | "level") {
                item[key] = p["value"]["value"].clone();
            }
        }
        if actionable(role) {
            if let Some(backend) = node["backendDOMNodeId"].as_i64() {
                n += 1;
                let reference = format!("e{n}");
                refs.insert(reference.clone(), backend);
                item["ref"] = json!(reference);
            }
        }
        if item.get("ref").is_some() || content(role) {
            out.push(item);
        }
        if out.len() >= MAX_NODES {
            break;
        }
    }
    Ok(json!({
        "state": if out.is_empty() { "empty" } else { "ok" },
        "refs": refs.len(),
        "nodes": out,
        "_refs": refs,
    }))
}

fn object_for(cdp: &mut Cdp, session: &str, backend: i64) -> Result<String> {
    let resolved = cdp.call(Some(session), "DOM.resolveNode", json!({ "backendNodeId": backend }))?;
    resolved["object"]["objectId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| DevdError::invalid("that element is no longer on the page; take a new snapshot"))
}

// ---------------------------------------------------------------- utilities

/// Split `ws://host:port/path` into the host and port a TCP check needs.
fn ws_host_port(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("ws://")?;
    let authority = rest.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

/// Stop the private browser and every process it started. Windows keeps the
/// renderer and GPU children alive after the parent is killed, so a plain
/// `kill` leaks a tree.
fn kill_tree(child: &mut Child) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
    Ok(())
}

// --------------------------------------------------------------- browser find

fn find_browser() -> Result<PathBuf> {    if let Ok(explicit) = std::env::var("DEVD_BROWSER") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(path);
        }
        return Err(DevdError::not_found(format!(
            "DEVD_BROWSER points at {}, which is not a file",
            path.display()
        )));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        for var in ["PROGRAMFILES(X86)", "PROGRAMFILES", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                candidates.push(base.join("Microsoft/Edge/Application/msedge.exe"));
                candidates.push(base.join("Google/Chrome/Application/chrome.exe"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"));
        candidates.push(PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
        candidates.push(PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for name in ["microsoft-edge", "google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] {
            if let Ok(output) = Command::new("which").arg(name).output() {
                if output.status.success() {
                    candidates.push(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
                }
            }
        }
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            DevdError::not_found(
                "no Chromium-based browser was found. Install Microsoft Edge or Google Chrome, or set DEVD_BROWSER to its executable.",
            )
        })
}

fn launch(exe: &Path, profile: &Path) -> Result<(Child, String)> {
    let mut command = Command::new(exe);
    command
        .args([
            "--headless=new",
            "--remote-debugging-port=0",
            "--remote-allow-origins=*",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-sync",
            "--hide-scrollbars",
            "--mute-audio",
            "--window-size=1440,900",
            // One CSS pixel is one screenshot pixel, so a click in the pane maps
            // to the exact point in the page.
            "--force-device-scale-factor=1",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window for the browser devd owns.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| DevdError::internal(format!("could not start {}: {e}", exe.display())))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| DevdError::internal("the browser started without a stderr pipe"))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            if let Some(at) = line.find("DevTools listening on ") {
                let url = line[at + "DevTools listening on ".len()..].trim().to_string();
                let _ = tx.send(url);
                return;
            }
        }
    });
    match rx.recv_timeout(Duration::from_secs(25)) {
        Ok(url) => Ok((child, url)),
        Err(_) => {
            let _ = child.kill();
            Err(DevdError::internal(
                "the browser did not open its DevTools port. Check that the browser is not blocked by policy.",
            ))
        }
    }
}

fn check_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed == "about:blank" {
        return Ok(());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(());
    }
    Err(DevdError::invalid(format!(
        "`{url}` is not a web address. Use http://, https:// or about:blank; the browser does not open file: or script: addresses."
    )))
}

fn clip_data(value: Value) -> Value {
    // Tool results are stored and shown; a page that returns a megabyte of JSON
    // should not become a megabyte in the run history.
    if serde_json::to_string(&value).map(|s| s.len()).unwrap_or(0) > 32_000 {
        json!({ "state": "ok", "truncated": true, "preview": clip(&value.to_string(), 32_000) })
    } else {
        value
    }
}

fn current_of(tabs: &BTreeMap<String, Tab>, id: &str) -> Result<(String, String)> {
    let tab = tabs.get(id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?;
    Ok((tab.url.clone(), tab.title.clone()))
}

fn update_seen(tabs: &mut BTreeMap<String, Tab>, cdp: &mut Cdp, id: &str) -> Result<()> {
    let session = tabs
        .get(id)
        .ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?
        .session
        .clone();
    let (url, title) = location(cdp, &session)?;
    if let Some(tab) = tabs.get_mut(id) {
        tab.url = url;
        tab.title = title;
    }
    Ok(())
}

// ---------------------------------------------------------------- operations

/// Start the browser (if needed) and load a page in the focused tab.
pub fn open(root: &Path, url: Option<&str>) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(None)?;
    match url.filter(|u| !u.is_empty()) {
        Some(url) => m.navigate(&id, url),
        None => {
            let (url, title) = m.current(&id)?;
            Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
        }
    }
}

/// Open a page in a fresh tab.
pub fn tabs_open(root: &Path, url: Option<&str>) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.open_tab(url)?;
    let (url, title) = m.current(&id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title, "focused": m.focused }))
}

pub fn tabs_list(root: &Path) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    m.refresh()?;
    let Manager { cdp, tabs, focused, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    // A real browser tab keeps the page's current title and icon. Read them
    // from each page, not from a cross-origin frame.
    for tab in tabs.values_mut() {
        if let Ok((url, title, favicon)) = page_meta(cdp, &tab.session) {
            tab.url = url;
            tab.title = title;
            tab.favicon = favicon;
        }
    }
    let rows: Vec<Value> = tabs
        .iter()
        .map(|(id, t)| json!({
            "id": id,
            "url": t.url,
            "title": t.title,
            "favicon": t.favicon,
            "focused": focused.as_deref() == Some(id)
        }))
        .collect();
    Ok(json!({
        "state": if rows.is_empty() { "empty" } else { "ok" },
        "focused": focused.clone(),
        "tabs": rows,
    }))
}

pub fn tabs_focus(tab: &str) -> Result<Value> {
    let mut m = lock();
    if !m.tabs.contains_key(tab) {
        return Err(DevdError::not_found(format!("there is no tab `{tab}`")));
    }
    m.focused = Some(tab.to_string());
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let mut favicon = None;
    if let Some(t) = tabs.get(tab) {
        if let Ok((url, title, icon)) = page_meta(cdp, &t.session) {
            if let Some(t) = tabs.get_mut(tab) {
                t.url = url;
                t.title = title;
                t.favicon = icon.clone();
            }
            favicon = icon;
        }
    }
    let (url, title) = current_of(tabs, tab)?;
    Ok(json!({ "state": "ok", "tab": tab, "url": url, "title": title, "favicon": favicon }))
}

pub fn tabs_close(tab: &str) -> Result<Value> {
    let mut m = lock();
    let Manager { cdp, tabs, focused, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    if !tabs.contains_key(tab) {
        return Err(DevdError::not_found(format!("there is no tab `{tab}`")));
    }
    cdp.call(None, "Target.closeTarget", json!({ "targetId": tab }))?;
    tabs.remove(tab);
    if focused.as_deref() == Some(tab) {
        *focused = tabs.keys().next().cloned();
    }
    Ok(json!({ "state": "ok", "closed": tab, "focused": focused.clone() }))
}

pub fn navigate(tab: Option<&str>, url: &str) -> Result<Value> {
    let mut m = lock();
    let id = m.focused_tab(tab)?;
    m.navigate(&id, url)
}

pub fn back(tab: Option<&str>) -> Result<Value> {
    history(tab, -1)
}

pub fn forward(tab: Option<&str>) -> Result<Value> {
    history(tab, 1)
}

fn history(tab: Option<&str>, delta: i64) -> Result<Value> {
    let mut m = lock();
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let state = cdp.call(Some(&session), "Page.getNavigationHistory", json!({}))?;
    let index = state["currentIndex"].as_i64().unwrap_or(0);
    let entries = state["entries"].as_array().cloned().unwrap_or_default();
    let target = index + delta;
    if target < 0 || target as usize >= entries.len() {
        return Ok(json!({ "state": "empty", "tab": id, "reason": if delta < 0 { "there is no earlier page" } else { "there is no later page" } }));
    }
    let entry = entries[target as usize]["id"].as_i64().unwrap_or(0);
    cdp.call(Some(&session), "Page.navigateToHistoryEntry", json!({ "entryId": entry }))?;
    wait_ready(cdp, &session)?;
    if let Some(t) = tabs.get_mut(&id) {
        t.refs.clear();
    }
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
}

pub fn reload(tab: Option<&str>) -> Result<Value> {
    let mut m = lock();
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    cdp.call(Some(&session), "Page.reload", json!({ "ignoreCache": true }))?;
    if let Some(t) = tabs.get_mut(&id) {
        t.refs.clear();
    }
    wait_ready(cdp, &session)?;
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
}

pub fn snapshot_tab(root: &Path, tab: Option<&str>) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let mut result = snapshot(cdp, &session)?;
    let refs = result["_refs"].as_object().cloned().unwrap_or_default();
    if let Some(t) = tabs.get_mut(&id) {
        t.refs = refs
            .iter()
            .filter_map(|(k, v)| v.as_i64().map(|v| (k.clone(), v)))
            .collect();
    }
    if let Some(obj) = result.as_object_mut() {
        obj.remove("_refs");
    }
    result["tab"] = json!(id);
    let (url, title) = current_of(tabs, &id)?;
    result["url"] = json!(url);
    result["title"] = json!(title);
    Ok(result)
}

pub fn click(tab: Option<&str>, reference: &str) -> Result<Value> {
    let mut m = lock();
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let backend = tabs
        .get(&id)
        .and_then(|t| t.refs.get(reference).copied())
        .ok_or_else(|| DevdError::invalid(format!("`{reference}` is not a ref from this tab's latest snapshot; call browser.snapshot first")))?;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let object = object_for(cdp, &session, backend)?;
    cdp.call(
        Some(&session),
        "Runtime.callFunctionOn",
        json!({
            "objectId": object,
            "functionDeclaration": "function(){ this.scrollIntoView({block:'center', inline:'center'}); this.click(); return true; }",
            "returnByValue": true,
            "awaitPromise": true
        }),
    )?;
    std::thread::sleep(Duration::from_millis(350));
    wait_ready(cdp, &session)?;
    if let Some(t) = tabs.get_mut(&id) {
        t.refs.clear();
    }
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
}

pub fn type_text(tab: Option<&str>, reference: &str, text: &str, submit: bool) -> Result<Value> {
    let mut m = lock();
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let backend = tabs
        .get(&id)
        .and_then(|t| t.refs.get(reference).copied())
        .ok_or_else(|| DevdError::invalid(format!("`{reference}` is not a ref from this tab's latest snapshot; call browser.snapshot first")))?;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let object = object_for(cdp, &session, backend)?;
    cdp.call(
        Some(&session),
        "Runtime.callFunctionOn",
        json!({
            "objectId": object,
            "functionDeclaration": "function(value, submit){\
                this.focus();\
                if (this.isContentEditable) { this.textContent = value; }\
                else {\
                    const proto = this instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;\
                    const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;\
                    if (setter && (this instanceof HTMLInputElement || this instanceof HTMLTextAreaElement)) { setter.call(this, value); } else { this.value = value; }\
                }\
                this.dispatchEvent(new Event('input', {bubbles:true}));\
                this.dispatchEvent(new Event('change', {bubbles:true}));\
                if (submit) {\
                    if (this.form && this.form.requestSubmit) { this.form.requestSubmit(); }\
                    else if (this.form) { this.form.submit(); }\
                    else {\
                        this.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', code:'Enter', keyCode:13, which:13, bubbles:true}));\
                        this.dispatchEvent(new KeyboardEvent('keyup', {key:'Enter', code:'Enter', keyCode:13, which:13, bubbles:true}));\
                    }\
                }\
                return true;\
            }",
            "arguments": [{ "value": text }, { "value": submit }],
            "returnByValue": true,
            "awaitPromise": true
        }),
    )?;
    std::thread::sleep(Duration::from_millis(250));
    if submit {
        wait_ready(cdp, &session)?;
    }
    // Typing does not navigate, so the refs stay meaningful for the next field.
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title, "submitted": submit }))
}

/// Click at a point in the page. Coordinates are CSS pixels in the screenshot
/// the person is looking at.
pub fn mouse(root: &Path, tab: Option<&str>, x: f64, y: f64, button: &str) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let button = if button == "right" { "right" } else { "left" };
    for (kind, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
        cdp.call(
            Some(&session),
            "Input.dispatchMouseEvent",
            json!({
                "type": kind,
                "x": x,
                "y": y,
                "button": button,
                "buttons": buttons,
                "clickCount": 1,
                "modifiers": 0
            }),
        )?;
        std::thread::sleep(Duration::from_millis(40));
    }
    std::thread::sleep(Duration::from_millis(350));
    wait_ready(cdp, &session)?;
    if let Some(t) = tabs.get_mut(&id) {
        t.refs.clear();
    }
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title, "x": x, "y": y }))
}

/// Type text into whatever the page has focused, or press one named key.
pub fn key(root: &Path, tab: Option<&str>, text: Option<&str>, name: Option<&str>) -> Result<Value> {
    let (text, name) = match (text, name) {
        (Some(t), _) if !t.is_empty() => (Some(t), None),
        (_, Some(n)) if !n.is_empty() => (None, Some(n)),
        _ => return Err(DevdError::invalid("give text to type or a key to press")),
    };
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    if let Some(text) = text {
        cdp.call(Some(&session), "Input.insertText", json!({ "text": text }))?;
    }
    if let Some(name) = name {
        let (code, vk) = key_code(name).ok_or_else(|| DevdError::invalid(format!("`{name}` is not a key devd sends; use text for ordinary characters")))?;
        let mut down = json!({
            "type": "keyDown",
            "key": code,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk
        });
        // Enter carries the carriage return Chrome uses to submit a form.
        if code == "Enter" {
            down["text"] = json!("\r");
            down["unmodifiedText"] = json!("\r");
        }
        cdp.call(Some(&session), "Input.dispatchKeyEvent", down)?;
        cdp.call(Some(&session), "Input.dispatchKeyEvent", json!({
            "type": "keyUp",
            "key": code,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk
        }))?;
    }
    std::thread::sleep(Duration::from_millis(250));
    wait_ready(cdp, &session)?;
    if let Some(t) = tabs.get_mut(&id) {
        t.refs.clear();
    }
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
}

fn key_code(name: &str) -> Option<(&'static str, u32)> {
    Some(match name {
        "Enter" => ("Enter", 13),
        "Tab" => ("Tab", 9),
        "Backspace" => ("Backspace", 8),
        "Escape" => ("Escape", 27),
        "ArrowLeft" => ("ArrowLeft", 37),
        "ArrowUp" => ("ArrowUp", 38),
        "ArrowRight" => ("ArrowRight", 39),
        "ArrowDown" => ("ArrowDown", 40),
        "Delete" => ("Delete", 46),
        "Home" => ("Home", 36),
        "End" => ("End", 35),
        "PageUp" => ("PageUp", 33),
        "PageDown" => ("PageDown", 34),
        _ => return None,
    })
}

/// Scroll at a point in the page. `delta_y` follows the wheel: positive is down.
pub fn scroll(root: &Path, tab: Option<&str>, x: f64, y: f64, delta_y: f64) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    cdp.call(
        Some(&session),
        "Input.dispatchMouseEvent",
        json!({
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": 0,
            "deltaY": delta_y,
            "button": "none",
            "buttons": 0,
            "modifiers": 0
        }),
    )?;
    std::thread::sleep(Duration::from_millis(180));
    update_seen(tabs, cdp, &id)?;
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({ "state": "ok", "tab": id, "url": url, "title": title }))
}

pub fn screenshot(root: &Path, tab: Option<&str>) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, seq, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let shot = cdp.call(
        Some(&session),
        "Page.captureScreenshot",
        json!({ "format": "png", "captureBeyondViewport": false, "fromSurface": true }),
    )?;
    // The pane needs the page's own pixel size to map a click back to a point.
    let metrics = cdp.call(Some(&session), "Page.getLayoutMetrics", json!({}))?;
    let width = metrics["cssLayoutViewport"]["clientWidth"].as_f64().unwrap_or(0.0);
    let height = metrics["cssLayoutViewport"]["clientHeight"].as_f64().unwrap_or(0.0);
    let encoded = shot["data"]
        .as_str()
        .ok_or_else(|| DevdError::internal("the browser returned no screenshot data"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| DevdError::internal(format!("the screenshot was not valid base64: {e}")))?;
    let uploads = root.join("uploads");
    std::fs::create_dir_all(&uploads)?;
    *seq += 1;
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect();
    let name = format!("browser-{safe}-{}-{}.png", *seq, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    std::fs::write(uploads.join(&name), &bytes)?;
    let mut favicon = None;
    if let Ok((url, title, icon)) = page_meta(cdp, &session) {
        favicon = icon.clone();
        if let Some(t) = tabs.get_mut(&id) {
            t.url = url;
            t.title = title;
            t.favicon = icon;
        }
    }
    let (url, title) = current_of(tabs, &id)?;
    Ok(json!({
        "state": "ok",
        "tab": id,
        "url": url,
        "title": title,
        "favicon": favicon,
        "file": name,
        "bytes": bytes.len(),
        "width": width,
        "height": height,
    }))
}

pub fn console(root: &Path, tab: Option<&str>) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let value = cdp.call(
        Some(&session),
        "Runtime.evaluate",
        json!({
            "expression": "JSON.stringify({ logs: window.__devdConsole || [], net: window.__devdNetwork || [] })",
            "returnByValue": true
        }),
    )?;
    let text = value["result"]["value"].as_str().unwrap_or("{}");
    let parsed: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({}));
    let logs: Vec<Value> = parsed["logs"].as_array().cloned().unwrap_or_default().into_iter().take(MAX_LOG).collect();
    let net: Vec<Value> = parsed["net"].as_array().cloned().unwrap_or_default().into_iter().take(MAX_LOG).collect();
    Ok(json!({
        "state": if logs.is_empty() && net.is_empty() { "empty" } else { "ok" },
        "tab": id,
        "entries": logs,
        "network": net,
    }))
}

pub fn evaluate(root: &Path, tab: Option<&str>, expression: &str) -> Result<Value> {
    let mut m = lock();
    m.ensure(root)?;
    let id = m.focused_tab(tab)?;
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(&id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    let value = cdp.call(
        Some(&session),
        "Runtime.evaluate",
        json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
    )?;
    let result = value["result"].clone();
    let exception = value["exceptionDetails"]["text"].as_str().map(str::to_string);
    Ok(json!({
        "state": if exception.is_some() { "failed" } else { "ok" },
        "tab": id,
        "result": clip_data(result),
        "error": exception,
    }))
}

/// Whether a Chromium-based browser is installed for devd to drive.
pub fn available() -> bool {
    find_browser().is_ok()
}

/// A tab opened to photograph a page at chosen moments, closed again when the
/// work is done. It holds no lock between calls, so the person's Browser pane
/// keeps working while a long render runs.
pub struct Capture {
    tab: String,
}

fn with_session<T>(id: &str, f: impl FnOnce(&mut Cdp, &str) -> Result<T>) -> Result<T> {
    let mut m = lock();
    let Manager { cdp, tabs, .. } = &mut *m;
    let cdp = cdp.as_mut().ok_or_else(|| DevdError::invalid("the browser is not running"))?;
    let session = tabs.get(id).ok_or_else(|| DevdError::not_found(format!("there is no tab `{id}`")))?.session.clone();
    f(cdp, &session)
}

impl Capture {
    /// What the composition says about itself, once its runtime has loaded.
    pub fn info(&self) -> Result<Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let said = with_session(&self.tab, |cdp, session| {
                let v = cdp.call(
                    Some(session),
                    "Runtime.evaluate",
                    json!({ "expression": "(typeof window.__motion === 'object') ? JSON.stringify(window.__motion.info()) : ''", "returnByValue": true }),
                )?;
                Ok(v["result"]["value"].as_str().unwrap_or("").to_string())
            })?;
            if let Ok(info) = serde_json::from_str::<Value>(&said) {
                return Ok(info);
            }
            if Instant::now() > deadline {
                return Err(DevdError::invalid("the page never defined window.__motion. A composition loads devd-motion.js and calls Motion.config and Motion.draw; see motion.new."));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Make the page exactly this size, as a film frame.
    pub fn resize(&self, width: u32, height: u32) -> Result<()> {
        with_session(&self.tab, |cdp, session| {
            cdp.call(
                Some(session),
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
            )?;
            Ok(())
        })
    }

    /// The page at second `t`, as PNG bytes, once it has been painted.
    pub fn frame(&self, t: f64) -> Result<Vec<u8>> {
        with_session(&self.tab, |cdp, session| {
            let seek = cdp.call(
                Some(session),
                "Runtime.evaluate",
                json!({ "expression": format!("window.__motion.seek({t})"), "returnByValue": true, "awaitPromise": true }),
            )?;
            if let Some(why) = seek["exceptionDetails"]["exception"]["description"].as_str().or(seek["exceptionDetails"]["text"].as_str()) {
                return Err(DevdError::invalid(format!("the composition failed at {t}s: {why}")));
            }
            let shot = cdp.call(Some(session), "Page.captureScreenshot", json!({ "format": "png", "captureBeyondViewport": false, "fromSurface": true }))?;
            let encoded = shot["data"].as_str().ok_or_else(|| DevdError::internal("the browser returned no frame"))?;
            base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|e| DevdError::internal(format!("a frame was not valid base64: {e}")))
        })
    }
}

/// Open `url` in a tab of its own, run `work` with it, and close the tab.
pub fn with_capture<T>(root: &Path, url: &str, work: impl FnOnce(&Capture) -> Result<T>) -> Result<T> {
    let id = {
        let mut m = lock();
        m.ensure(root)?;
        m.open_tab(Some(url))?
    };
    let capture = Capture { tab: id.clone() };
    let result = work(&capture);
    let _ = tabs_close(&id);
    result
}

pub fn close() -> Result<Value> {
    let mut m = lock();
    let Manager { cdp, tabs, focused, child, .. } = &mut *m;
    let count = tabs.len();
    if let Some(connection) = cdp.as_mut() {
        for id in tabs.keys() {
            let _ = connection.call(None, "Target.closeTarget", json!({ "targetId": id }));
        }
    }
    tabs.clear();
    *focused = None;
    *cdp = None;
    if let Some(mut process) = child.take() {
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &process.id().to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = process.kill();
        }
        let _ = process.wait();
    }
    Ok(json!({ "state": "ok", "closed": count }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_are_navigated_to() {
        assert!(check_url("https://example.com").is_ok());
        assert!(check_url("http://127.0.0.1:8000").is_ok());
        assert!(check_url("about:blank").is_ok());
        assert!(check_url("file:///C:/Windows/win.ini").is_err());
        assert!(check_url("javascript:alert(1)").is_err());
        assert!(check_url("data:text/html,hi").is_err());
    }

    #[test]
    fn actionability_matches_what_a_model_can_press() {
        for role in ["button", "link", "textbox", "checkbox", "combobox", "tab", "switch"] {
            assert!(actionable(role), "{role}");
        }
        assert!(!actionable("StaticText"));
        assert!(!actionable("heading"));
    }

    #[test]
    fn a_huge_value_is_clipped_rather_than_stored_whole() {
        let value = json!({ "text": "x".repeat(40_000) });
        let clipped = clip_data(value);
        assert_eq!(clipped["truncated"], true);
        assert!(clipped["preview"].as_str().unwrap().len() < 40_000);
    }

    #[test]
    fn no_browser_is_reported_as_a_missing_capability_not_a_panic() {
        // This runs with an explicit path that does not exist, so it never
        // launches anything.
        std::env::set_var("DEVD_BROWSER", "C:/definitely/not/a/browser.exe");
        let err = find_browser().unwrap_err();
        assert!(err.to_string().contains("not a file"));
        std::env::remove_var("DEVD_BROWSER");
    }

    #[test]
    fn named_keys_are_the_ones_a_form_uses() {
        assert_eq!(key_code("Enter"), Some(("Enter", 13)));
        assert_eq!(key_code("Tab"), Some(("Tab", 9)));
        assert_eq!(key_code("Backspace"), Some(("Backspace", 8)));
        assert_eq!(key_code("ArrowDown"), Some(("ArrowDown", 40)));
        assert!(key_code("F13").is_none());
    }
}
