//! Quick Share through ngrok.
//!
//! A project becomes reachable from the internet through an ngrok tunnel. devd
//! owns the binary (a managed copy under `tools/ngrok`, or an existing `ngrok`
//! on PATH), the authtoken (a file only the current user can read, passed to the
//! child as `NGROK_AUTHTOKEN`, never in argv), the region, and the process. The
//! public address is read from the agent's own JSON log, never constructed.
//!
//! Each share is a small state file under `share/`, keyed by project, so several
//! projects can be shared at once and a later process can list and stop them. A
//! share whose tunnel has died is closed on the next look.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use devd_core::capability::Ctx;
use devd_core::download::{self, Artifact};
use devd_core::process::{self, SpawnSpec};
use devd_core::{util, DevdError, Result};

/// The ngrok agent build for this machine.
const DOWNLOAD_WINDOWS: &str = "https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-windows-amd64.zip";
const DOWNLOAD_LINUX: &str = "https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-linux-amd64.zip";
const DOWNLOAD_MACOS: &str = "https://bin.equinox.io/c/bNyj1mQVY4c/ngrok-v3-stable-darwin-amd64.zip";
const TIMEOUT: Duration = Duration::from_secs(45);

/// Where a tunnel may connect through.
pub const REGIONS: &[(&str, &str)] = &[
    ("auto", "Automatic"),
    ("us", "United States"),
    ("eu", "Europe"),
    ("ap", "Asia/Pacific"),
    ("au", "Australia"),
];

/// One open ngrok share.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    /// Project directory.
    pub project: String,
    /// The public address.
    pub url: String,
    /// The tunnel process.
    pub pid: u32,
    /// The local port it points at.
    pub port: u16,
    /// The region it was opened in.
    pub region: String,
    /// Whether basic authentication is required.
    pub protected: bool,
    /// When it opened, RFC 3339.
    pub started_at: String,
}

/// Whether a region code is one this agent supports.
pub fn region_ok(region: &str) -> bool {
    REGIONS.iter().any(|(code, _)| *code == region)
}

/// The region menu, as data.
pub fn regions() -> serde_json::Value {
    serde_json::json!(REGIONS
        .iter()
        .map(|(code, label)| serde_json::json!({ "code": code, "label": label }))
        .collect::<Vec<_>>())
}

/// Where the managed agent lives.
pub fn dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("tools").join("ngrok")
}

/// The managed agent.
pub fn binary(ctx: &Ctx) -> PathBuf {
    dir(ctx).join(if cfg!(windows) { "ngrok.exe" } else { "ngrok" })
}

/// The agent to run: the managed copy, or one already on PATH.
pub fn installed(ctx: &Ctx) -> Option<PathBuf> {
    let managed = binary(ctx);
    if managed.is_file() {
        return Some(managed);
    }
    let name = if cfg!(windows) { "ngrok.exe" } else { "ngrok" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn token_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("secrets").join("ngrok.key")
}

/// Whether an authtoken is stored.
pub fn has_token(ctx: &Ctx) -> bool {
    token(ctx).is_some()
}

/// The stored authtoken.
pub fn token(ctx: &Ctx) -> Option<String> {
    std::fs::read_to_string(token_path(ctx))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Store the authtoken with owner-only permissions.
pub fn set_token(ctx: &Ctx, token: &str) -> Result<()> {
    let t = token.trim();
    if t.len() < 20 || t.len() > 512 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(DevdError::invalid(
            "that does not look like an ngrok authtoken (20-512 characters, no spaces)",
        ));
    }
    let path = token_path(ctx);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        restrict(parent, true);
    }
    std::fs::write(&path, t)?;
    restrict(&path, false);
    Ok(())
}

/// Forget the stored authtoken.
pub fn clear_token(ctx: &Ctx) {
    let _ = std::fs::remove_file(token_path(ctx));
}

#[cfg(unix)]
fn restrict(path: &Path, dir: bool) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(if dir { 0o700 } else { 0o600 }),
    );
}
#[cfg(not(unix))]
fn restrict(_path: &Path, _dir: bool) {}

/// Download the managed agent and check that it runs.
pub fn install(ctx: &Ctx) -> Result<PathBuf> {
    let url = if cfg!(windows) {
        DOWNLOAD_WINDOWS
    } else if cfg!(target_os = "macos") {
        DOWNLOAD_MACOS
    } else {
        DOWNLOAD_LINUX
    };
    let artifact = Artifact {
        url: url.to_string(),
        filename: "ngrok.zip".to_string(),
        // ngrok publishes the stable build on its own CDN without a separate
        // digest file; the archive is unpacked and the program is then run
        // (`ngrok version`) before it is used.
        sha256: None,
        bytes: None,
    };
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), None)?;
    std::fs::create_dir_all(dir(ctx))?;
    devd_core::archive::extract(&fetched.path, &dir(ctx), true)?;
    let exe = binary(ctx);
    if !exe.is_file() {
        return Err(DevdError::internal(
            "the ngrok archive did not contain ngrok",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }
    let out = std::process::Command::new(&exe)
        .arg("version")
        .output()
        .map_err(|e| DevdError::internal(format!("ngrok could not be started: {e}")))?;
    if !out.status.success() {
        return Err(DevdError::internal(
            "the downloaded ngrok did not run; remove tools/ngrok and try again",
        ));
    }
    Ok(exe)
}

fn state_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("share")
}

fn safe_key(project: &str) -> String {
    let name = Path::new(project)
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    // A space or punctuation becomes a dash rather than vanishing, so
    // "My Project!" and "MyProject" do not share a state file.
    let dashed: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean: String = dashed
        .trim_matches('-')
        .chars()
        .take(48)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    if clean.is_empty() {
        "project".to_string()
    } else {
        clean
    }
}

fn state_file(ctx: &Ctx, project: &str) -> PathBuf {
    state_dir(ctx).join(format!("ngrok-{}.json", safe_key(project)))
}

fn is_ngrok(pid: u32) -> bool {
    process::process_image(pid)
        .is_some_and(|image| image.to_ascii_lowercase().contains("ngrok"))
}

fn close(ctx: &Ctx, link: &Link) {
    if is_ngrok(link.pid) {
        process::kill_tree(link.pid);
    }
    let _ = std::fs::remove_file(state_file(ctx, &link.project));
}

/// Every open share; a dead tunnel is closed first.
pub fn list(ctx: &Ctx) -> Result<Vec<Link>> {
    let mut open = Vec::new();
    for entry in std::fs::read_dir(state_dir(ctx)).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("ngrok-") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(link) = serde_json::from_str::<Link>(&text) else {
            continue;
        };
        if is_ngrok(link.pid) {
            open.push(link);
        } else {
            close(ctx, &link);
        }
    }
    open.sort_by(|a, b| a.project.cmp(&b.project));
    Ok(open)
}

/// The first ngrok address in the agent's JSON log.
pub fn find_url(log: &str) -> Option<String> {
    for line in log.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(url) = v["url"].as_str().filter(|u| u.starts_with("https://")) {
                if url.contains(".ngrok") {
                    return Some(url.trim_end_matches('/').to_string());
                }
            }
        }
    }
    // A plain-text line as a fallback (an older agent, or a wrapped log).
    for word in log.split(|c: char| c.is_whitespace() || c == '"' || c == '|') {
        if let Some(rest) = word.strip_prefix("https://") {
            if rest.contains(".ngrok") {
                return Some(format!("https://{}", rest.trim_end_matches('/')));
            }
        }
    }
    None
}

/// Open a tunnel to a local port and return the verified public address.
pub fn start(
    ctx: &Ctx,
    project: &str,
    port: u16,
    host_header: Option<&str>,
    region: &str,
    basic_auth: Option<&str>,
) -> Result<Link> {
    if !region_ok(region) {
        return Err(DevdError::invalid(format!(
            "`{region}` is not an ngrok region ({})",
            REGIONS.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ")
        )));
    }
    let exe = installed(ctx).ok_or_else(|| {
        DevdError::not_found("ngrok is not installed yet; set it up from the Share panel")
    })?;
    let token = token(ctx).ok_or_else(|| {
        DevdError::not_found("ngrok needs an authtoken; add it in the Share settings")
    })?;
    if list(ctx)?.iter().any(|l| l.project == project) {
        return Err(DevdError::conflict(format!(
            "{} is already shared; stop that link first",
            project
        )));
    }
    if !port_answers(port) {
        return Err(DevdError::conflict(format!(
            "nothing is answering on 127.0.0.1:{port}, so a tunnel would point at a dead service"
        )));
    }
    let _ = ctx.store.set_setting("share.ngrok.region", region);
    let log = ctx
        .paths
        .logs()
        .join(format!("share-ngrok-{}.log", safe_key(project)));
    let _ = std::fs::remove_file(&log);

    let mut args = vec![
        "http".to_string(),
        format!("127.0.0.1:{port}"),
        "--log=stdout".to_string(),
        "--log-format=json".to_string(),
    ];
    if region != "auto" {
        args.push(format!("--region={region}"));
    }
    if let Some(host) = host_header.filter(|h| !h.trim().is_empty()) {
        args.push(format!("--host-header={host}"));
    }
    if let Some(auth) = basic_auth.filter(|a| !a.trim().is_empty()) {
        args.push("--basic-auth".to_string());
        args.push(auth.to_string());
    }
    let pid = process::shared().start(
        SpawnSpec {
            name: format!("ngrok-{}", safe_key(project)),
            kind: "share".into(),
            program: exe,
            args,
            cwd: None,
            envs: vec![("NGROK_AUTHTOKEN".to_string(), token)],
            clear_env: true,
            log_path: log.clone(),
            restarts: 0,
            persist: true,
        },
        util::new_id(),
    )?;

    let began = Instant::now();
    let url = loop {
        if let Some(url) = std::fs::read_to_string(&log).ok().and_then(|t| find_url(&t)) {
            break url;
        }
        if !process::pid_alive(pid) {
            let tail = std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(DevdError::internal(format!(
                "ngrok stopped before it opened a tunnel: {tail}"
            )));
        }
        if began.elapsed() > TIMEOUT {
            process::kill_tree(pid);
            return Err(DevdError::internal(
                "ngrok did not report a public address within 45 seconds; check the token and the region",
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
    };

    let link = Link {
        project: project.to_string(),
        url: url.clone(),
        pid,
        port,
        region: region.to_string(),
        protected: basic_auth.is_some(),
        started_at: util::now(),
    };
    std::fs::create_dir_all(state_dir(ctx))?;
    std::fs::write(
        state_file(ctx, project),
        serde_json::to_string_pretty(&link).unwrap_or_default(),
    )?;
    ctx.events.emit_with(
        "share.started",
        format!("{} is public at {}", safe_key(project), url),
        serde_json::json!({ "project": project, "url": url, "port": port }),
    );
    Ok(link)
}

/// Close the share for a project. Returns whether one was open.
pub fn stop(ctx: &Ctx, project: &str) -> Result<bool> {
    let file = state_file(ctx, project);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Ok(false);
    };
    let link: Link = serde_json::from_str(&text)
        .map_err(|e| DevdError::internal(format!("the share record is unreadable: {e}")))?;
    close(ctx, &link);
    ctx.events.emit_with(
        "share.stopped",
        format!("{} is no longer public", safe_key(project)),
        serde_json::json!({ "project": project, "url": link.url }),
    );
    Ok(true)
}

/// The region last chosen, or `auto`.
pub fn saved_region(ctx: &Ctx) -> String {
    ctx.store
        .get_setting("share.ngrok.region")
        .ok()
        .flatten()
        .filter(|r| region_ok(r))
        .unwrap_or_else(|| "auto".to_string())
}

/// A QR code for a public address, as SVG. Drawn locally: the address is never
/// sent to a QR service.
pub fn qr_svg(url: &str) -> Result<String> {
    let code = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| DevdError::invalid(format!("that address cannot be a QR code: {e}")))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .build())
}

fn port_answers(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Close shares whose tunnel has gone. Called by the supervisor tick.
pub fn sweep(ctx: &Ctx) {
    let _ = list(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_address_is_read_from_the_agents_json_log() {
        let log = "{\"t\":\"2026-10-22T10:00:00Z\",\"event\":\"started\",\"url\":\"https://calm-river-wolf.ngrok-free.app\"}\n";
        assert_eq!(
            find_url(log).as_deref(),
            Some("https://calm-river-wolf.ngrok-free.app")
        );
        assert_eq!(find_url("starting...\n"), None);
    }

    #[test]
    fn a_region_must_be_one_the_agent_supports() {
        for (code, _) in REGIONS {
            assert!(region_ok(code));
        }
        assert!(!region_ok("mars"));
    }

    #[test]
    fn a_project_key_is_a_safe_file_name() {
        assert_eq!(safe_key("C:\\work\\My Project!"), "my-project");
        assert_eq!(safe_key("/home/x/shop"), "shop");
        assert!(!safe_key("!!").is_empty());
    }
}
