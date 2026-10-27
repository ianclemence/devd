//! Public share links: a site on this computer, reachable from the internet for a while.
//!
//! The tunnel is a Cloudflare quick tunnel: `cloudflared` makes an outbound connection and
//! Cloudflare answers on a random `https://<words>.trycloudflare.com` address. It needs no
//! account and no inbound port. devd downloads `cloudflared` from Cloudflare's GitHub release,
//! checks the SHA-256 GitHub records for the file, and runs it only while a link is open.
//!
//! Every link has an end time. The state of a link is a small file under `share/`, so a command
//! run later (or the window) can list and stop what an earlier command started; whoever looks
//! next also closes anything past its time, and the process that owns the agents closes them on
//! a timer.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use devd_core::capability::Ctx;
use devd_core::download::{self, Artifact};
use devd_core::process::{self, SpawnSpec};
use devd_core::state::Site;
use devd_core::{DevdError, Result};

const RELEASES: &str = "https://api.github.com/repos/cloudflare/cloudflared/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(30);

/// Longest a link may stay open.
pub const MAX_MINUTES: u64 = 24 * 60;
/// How long a link stays open when nobody says.
pub const DEFAULT_MINUTES: u64 = 120;

/// One open link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    /// The site it shows.
    pub site: String,
    /// The public address.
    pub url: String,
    /// The tunnel process.
    pub pid: u32,
    /// When it opened, RFC 3339.
    pub started_at: String,
    /// When it closes, RFC 3339.
    pub expires_at: String,
}

/// The `cloudflared` build for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// Release tag, e.g. `2026.10.3`.
    pub version: String,
    /// Where it is downloaded from.
    pub url: String,
    /// The SHA-256 GitHub records for the file.
    pub sha256: String,
    /// Size in bytes.
    pub bytes: Option<u64>,
    /// Whether the download is a `.tgz` that must be unpacked.
    pub archive: bool,
}

fn asset_name() -> Option<&'static str> {
    Some(if cfg!(all(windows, target_arch = "x86_64")) {
        "cloudflared-windows-amd64.exe"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "cloudflared-linux-amd64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "cloudflared-linux-arm64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "cloudflared-darwin-arm64.tgz"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "cloudflared-darwin-amd64.tgz"
    } else {
        return None;
    })
}

/// The newest release that has a build for this machine and a recorded digest.
pub fn latest_build() -> Result<Build> {
    let name = asset_name().ok_or_else(|| {
        DevdError::unsupported("Cloudflare publishes no cloudflared build for this platform")
    })?;
    build_from(&download::fetch_text(RELEASES, TIMEOUT)?, name)
}

pub(crate) fn build_from(json_text: &str, name: &str) -> Result<Build> {
    let release: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|e| DevdError::internal(format!("the cloudflared release is not JSON: {e}")))?;
    let version = release["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v')
        .to_string();
    for file in release["assets"].as_array().into_iter().flatten() {
        if file["name"].as_str() != Some(name) {
            continue;
        }
        let digest = file["digest"]
            .as_str()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(|d| d.to_ascii_lowercase());
        if let (Some(url), Some(sha256)) = (file["browser_download_url"].as_str(), digest) {
            if version.is_empty() {
                break;
            }
            return Ok(Build {
                version,
                url: url.to_string(),
                sha256,
                bytes: file["size"].as_u64(),
                archive: name.ends_with(".tgz"),
            });
        }
    }
    Err(DevdError::not_found("the newest cloudflared release has no build for this platform with a recorded checksum, so devd will not install it"))
}

fn tools_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("tools").join("cloudflared")
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

/// The installed `cloudflared`, newest version first.
pub fn installed(ctx: &Ctx) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(tools_dir(ctx))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(exe_name()).is_file())
        .collect();
    dirs.sort_by_key(|d| {
        devd_runtime::catalogue::version_key(
            &d.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    });
    dirs.pop().map(|d| d.join(exe_name()))
}

/// Download, verify and unpack `cloudflared`.
pub fn install(ctx: &Ctx, build: &Build) -> Result<PathBuf> {
    let dest = tools_dir(ctx).join(&build.version);
    let exe = dest.join(exe_name());
    if exe.is_file() {
        return Ok(exe);
    }
    let mut artifact = Artifact::new(build.url.clone()).sha256(build.sha256.clone());
    if let Some(b) = build.bytes {
        artifact = artifact.bytes(b);
    }
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), None)?;
    if build.archive {
        devd_core::archive::extract(&fetched.path, &dest, true)?;
    } else {
        std::fs::create_dir_all(&dest)?;
        std::fs::copy(&fetched.path, &exe)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }
    if !exe.is_file() {
        return Err(DevdError::internal(
            "cloudflared was unpacked but its program is not where expected",
        ));
    }
    Ok(exe)
}

fn state_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("share")
}

fn state_file(ctx: &Ctx, site: &str) -> PathBuf {
    state_dir(ctx).join(format!("{site}.json"))
}

fn is_tunnel(pid: u32) -> bool {
    process::process_image(pid)
        .is_some_and(|image| image.to_ascii_lowercase().contains("cloudflared"))
}

fn close(ctx: &Ctx, share: &Share) {
    if is_tunnel(share.pid) {
        process::kill_tree(share.pid);
    }
    let _ = std::fs::remove_file(state_file(ctx, &share.site));
}

/// Every open link. A link whose tunnel has died, or whose time is up, is closed first.
pub fn list(ctx: &Ctx) -> Result<Vec<Share>> {
    let now = Utc::now();
    let mut open = Vec::new();
    for entry in std::fs::read_dir(state_dir(ctx))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(share) = serde_json::from_str::<Share>(&text) else {
            continue;
        };
        let expired = DateTime::parse_from_rfc3339(&share.expires_at)
            .map(|t| t.with_timezone(&Utc) <= now)
            .unwrap_or(true);
        if expired || !is_tunnel(share.pid) {
            close(ctx, &share);
        } else {
            open.push(share);
        }
    }
    open.sort_by(|a, b| a.site.cmp(&b.site));
    Ok(open)
}

/// Close what is past its time. Returns how many links were open before.
pub fn sweep(ctx: &Ctx) {
    let _ = list(ctx);
}

/// Close the link for a site. Returns whether one was open.
pub fn stop(ctx: &Ctx, site: &str) -> Result<bool> {
    let _ = list(ctx)?;
    let file = state_file(ctx, site);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return Ok(false);
    };
    let share: Share = serde_json::from_str(&text)
        .map_err(|e| DevdError::internal(format!("unreadable share record: {e}")))?;
    close(ctx, &share);
    Ok(true)
}

/// Where the tunnel connects on this computer, and the arguments that make the web server
/// answer as the site.
pub(crate) fn tunnel_args(ctx: &Ctx, site: &Site) -> Vec<String> {
    let tls = site.tls_mode != "off";
    let mut args = vec![
        "tunnel".to_string(),
        "--no-autoupdate".into(),
        "--url".into(),
    ];
    if tls {
        args.push(format!("https://127.0.0.1:{}", ctx.settings.https_port));
        args.push("--no-tls-verify".into());
        args.push("--origin-server-name".into());
        args.push(site.host.clone());
    } else {
        args.push(format!("http://127.0.0.1:{}", ctx.settings.http_port));
    }
    args.push("--http-host-header".into());
    args.push(site.host.clone());
    args
}

/// The first quick-tunnel address in the tunnel's log.
pub(crate) fn find_url(log: &str) -> Option<String> {
    for word in log.split(|c: char| c.is_whitespace() || c == '|') {
        let Some(rest) = word.strip_prefix("https://") else {
            continue;
        };
        let host = rest.split('/').next().unwrap_or("");
        if let Some(label) = host.strip_suffix(".trycloudflare.com") {
            if !label.is_empty()
                && label != "api"
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return Some(format!("https://{host}"));
            }
        }
    }
    None
}

/// What a visitor could download from the site's folder that should not be public.
pub fn exposures(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for name in [
        ".env",
        ".env.local",
        ".env.production",
        ".git",
        ".htpasswd",
        "wp-config.php.bak",
        "database.sqlite",
        "dump.sql",
        "backup.sql",
    ] {
        if root.join(name).exists() {
            out.push(name.to_string());
        }
    }
    out
}

/// Open a link. `cloudflared` must already be installed ([`install`]).
pub fn start(ctx: &Ctx, site: &Site, minutes: u64) -> Result<Share> {
    let exe = installed(ctx).ok_or_else(|| DevdError::not_found("cloudflared is not installed"))?;
    if minutes == 0 || minutes > MAX_MINUTES {
        return Err(DevdError::invalid(format!(
            "a link stays open for 1 to {MAX_MINUTES} minutes"
        )));
    }
    if list(ctx)?.iter().any(|s| s.site == site.name) {
        return Err(DevdError::conflict(format!(
            "{} already has an open link; stop it first",
            site.name
        )));
    }
    let log = ctx.paths.logs().join(format!("share-{}.log", site.name));
    let _ = std::fs::remove_file(&log);
    let name = format!("share-{}", site.name);
    let pid = process::shared().start(
        SpawnSpec {
            name,
            kind: "share".into(),
            program: exe,
            args: tunnel_args(ctx, site),
            cwd: None,
            envs: vec![],
            clear_env: true,
            log_path: log.clone(),
            restarts: 0,
            // The tunnel stays open until the person closes it with
            // `devd site unshare`, not until the command that opened it exits.
            persist: true,
        },
        devd_core::util::new_id(),
    )?;

    let began = Instant::now();
    let url = loop {
        if let Some(url) = std::fs::read_to_string(&log)
            .ok()
            .and_then(|t| find_url(&t))
        {
            break url;
        }
        if !process::pid_alive(pid) {
            let tail = std::fs::read_to_string(&log)
                .unwrap_or_default()
                .lines()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .join(" / ");
            return Err(DevdError::internal(format!(
                "the tunnel stopped before it had an address: {tail}"
            )));
        }
        if began.elapsed() > Duration::from_secs(45) {
            process::kill_tree(pid);
            return Err(DevdError::internal(
                "the tunnel gave no address within 45 seconds; check the internet connection",
            ));
        }
        std::thread::sleep(Duration::from_millis(300));
    };

    let now = Utc::now();
    let share = Share {
        site: site.name.clone(),
        url,
        pid,
        started_at: now.to_rfc3339(),
        expires_at: (now + chrono::Duration::minutes(minutes as i64)).to_rfc3339(),
    };
    std::fs::create_dir_all(state_dir(ctx))?;
    std::fs::write(
        state_file(ctx, &site.name),
        serde_json::to_string_pretty(&share).unwrap_or_default(),
    )?;
    ctx.events.emit_with(
        "share.started",
        format!("{} is public at {}", site.name, share.url),
        serde_json::json!({ "site": site.name, "url": share.url, "expires_at": share.expires_at }),
    );
    Ok(share)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_needs_a_name_match_and_a_recorded_digest() {
        let json = r#"{"tag_name":"2026.10.3","assets":[
            {"name":"cloudflared-windows-amd64.exe","size":55329336,"browser_download_url":"https://github.com/cloudflare/cloudflared/releases/download/2026.10.3/cloudflared-windows-amd64.exe","digest":"sha256:76B2805253D0D6FFACB092A7ACFBEE99E7AF6917835C0C5410E23F3EF5052DA2"},
            {"name":"cloudflared-linux-amd64","size":1,"browser_download_url":"https://x/y","digest":null}]}"#;
        let b = build_from(json, "cloudflared-windows-amd64.exe").unwrap();
        assert_eq!(b.version, "2026.10.3");
        assert_eq!(
            b.sha256,
            "76b2805253d0d6ffacb092a7acfbee99e7af6917835c0c5410e23f3ef5052da2"
        );
        assert!(!b.archive);
        assert!(
            build_from(json, "cloudflared-linux-amd64").is_err(),
            "no digest, no install"
        );
        assert!(build_from(json, "cloudflared-darwin-arm64.tgz").is_err());
    }

    #[test]
    fn the_address_is_read_from_the_tunnel_log() {
        let log = "2026-10-22T10:00:00Z INF Requesting new quick Tunnel on trycloudflare.com...\n\
                   2026-10-22T10:00:02Z INF +--------------------------------------------------------------+\n\
                   2026-10-22T10:00:02Z INF |  https://calm-river-wolf-test.trycloudflare.com                |\n";
        assert_eq!(
            find_url(log).as_deref(),
            Some("https://calm-river-wolf-test.trycloudflare.com")
        );
        assert_eq!(
            find_url("INF see https://api.trycloudflare.com/tunnel for details"),
            None
        );
        assert_eq!(find_url("nothing yet"), None);
    }

    #[test]
    fn secrets_in_the_folder_are_named() {
        let dir = std::env::temp_dir().join(format!("devd-share-exp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".env"), "A=1").unwrap();
        std::fs::write(dir.join("index.php"), "").unwrap();
        assert_eq!(
            exposures(&dir),
            vec![".env".to_string(), ".git".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
