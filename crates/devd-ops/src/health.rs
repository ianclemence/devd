//! Is the site answering, and is the machine healthy?
//!
//! A health check asks a site for its front page and records whether the web server answered with anything
//! below 500, and how long it took. It tells the alert targets when a site goes down and when it is back.
//! A local site is asked over plain HTTP on devd's own port with the site's name, without following the
//! redirect to HTTPS: an answer of any kind means the web server and the site's block are working.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, DevdError, Result};

/// The kind under which health checks are stored.
pub const KIND: &str = "health";

/// How often each check runs while devd is open.
const EVERY: Duration = Duration::from_secs(60);

/// What asking once showed.
#[derive(Debug, Clone)]
pub struct Probe {
    /// The HTTP status, when anything answered.
    pub status: Option<u16>,
    /// Milliseconds it took.
    pub ms: u128,
    /// Why nothing answered.
    pub error: Option<String>,
}

impl Probe {
    /// Whether the site is up: it answered with something that is not a server error.
    pub fn up(&self) -> bool {
        self.status.is_some_and(|s| s < 500)
    }
}

/// Ask a URL, with a `Host` header when it is a local site asked by address.
pub fn probe(url: &str, host: Option<&str>) -> Probe {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(15)).build();
    let mut req = agent.get(url).set("User-Agent", "devd-health");
    if let Some(h) = host {
        req = req.set("Host", h);
    }
    let started = Instant::now();
    match req.call() {
        Ok(r) => Probe { status: Some(r.status()), ms: started.elapsed().as_millis(), error: None },
        Err(ureq::Error::Status(code, _)) => Probe { status: Some(code), ms: started.elapsed().as_millis(), error: None },
        Err(e) => Probe { status: None, ms: started.elapsed().as_millis(), error: Some(e.to_string()) },
    }
}

/// A URL a person may ask devd to watch: http or https, a host, no credentials.
pub fn valid_url(url: &str) -> Result<()> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).ok_or_else(|| DevdError::invalid("a health URL starts with http:// or https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') || url.len() > 500 || url.contains(char::is_whitespace) {
        return Err(DevdError::invalid("that is not a usable URL (a host, no credentials, no spaces)"));
    }
    Ok(())
}

/// Start watching a site. With no URL, devd asks its own web server for the site.
pub fn add(ctx: &Ctx, site: &str, url: Option<&str>) -> Result<Value> {
    crate::site_named(ctx, site)?;
    if let Some(u) = url {
        valid_url(u)?;
    }
    ctx.store.ops_put(KIND, site, site, &json!({ "url": url, "enabled": true, "state": "unknown" }))?;
    check(ctx, site)
}

/// Stop watching a site.
pub fn remove(ctx: &Ctx, site: &str) -> Result<()> {
    if !ctx.store.ops_delete(KIND, site)? {
        return Err(DevdError::not_found(format!("{site} has no health check")));
    }
    Ok(())
}

/// Every check and how the site was last found.
pub fn list(ctx: &Ctx) -> Result<Vec<Value>> {
    Ok(ctx
        .store
        .ops_list(KIND, None)?
        .into_iter()
        .map(|i| {
            let mut v = i.data.clone();
            v["site"] = json!(i.site);
            v
        })
        .collect())
}

/// Ask now, record the result and tell the alert targets when the state changed.
pub fn check(ctx: &Ctx, site: &str) -> Result<Value> {
    let s = crate::site_named(ctx, site)?;
    let item = ctx.store.ops_get(KIND, site)?;
    let custom = item.as_ref().and_then(|i| i.data["url"].as_str().map(str::to_string));
    let p = match &custom {
        Some(u) => probe(u, None),
        None => {
            let port = devd_core::config::Settings::load(&ctx.paths.settings_file()).map(|x| x.http_port).unwrap_or(ctx.settings.http_port);
            probe(&format!("http://127.0.0.1:{port}/"), Some(&s.host))
        }
    };
    let was = item.as_ref().and_then(|i| i.data["state"].as_str().map(str::to_string)).unwrap_or_else(|| "unknown".to_string());
    let now_state = if p.up() { "up" } else { "down" };
    let mut d = item.map(|i| i.data).unwrap_or_else(|| json!({ "url": custom, "enabled": true }));
    d["state"] = json!(now_state);
    d["last_status"] = json!(p.status);
    d["last_ms"] = json!(p.ms);
    d["last_error"] = json!(p.error);
    d["last_at"] = json!(util::now());
    if was != now_state {
        d["since"] = json!(util::now());
    }
    ctx.store.ops_put(KIND, site, site, &d)?;
    if was != now_state && was != "unknown" {
        if now_state == "down" {
            crate::notify::send(ctx, "health.down", site, &format!("{site} is down: {}", p.error.clone().unwrap_or_else(|| format!("it answered {}", p.status.unwrap_or(0)))), json!({ "status": p.status }));
        } else {
            crate::notify::send(ctx, "health.up", site, &format!("{site} is back up ({} ms)", p.ms), json!({ "status": p.status }));
        }
    }
    let mut out = d;
    out["site"] = json!(site);
    Ok(out)
}

/// Run the checks that are due. Returns how many ran.
pub fn tick(ctx: &Ctx) -> usize {
    let Ok(items) = ctx.store.ops_list(KIND, None) else { return 0 };
    let mut ran = 0;
    for i in items {
        if !i.data["enabled"].as_bool().unwrap_or(true) {
            continue;
        }
        let last = i.data["last_at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
        let due = last.is_none_or(|t| chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc)) >= chrono::Duration::from_std(EVERY).unwrap_or_default());
        if due {
            let (ctx, site) = (ctx.clone(), i.site.clone());
            std::thread::spawn(move || {
                let _ = check(&ctx, &site);
            });
            ran += 1;
        }
    }
    ran
}

/// CPU, memory and disks of this machine, and devd's own services.
pub fn monitor(ctx: &Ctx) -> Result<Value> {
    let machine = if cfg!(windows) { windows_stats() } else { unix_stats() };
    let services = ctx.store.list_services().unwrap_or_default();
    let running = services.iter().filter(|s| devd_services::status(ctx, &s.name).map(|st| st.running).unwrap_or(false)).count();
    let mut v = machine.unwrap_or_else(|e| json!({ "error": e.to_string() }));
    v["devd"] = json!({ "services": services.len(), "running": running });
    Ok(v)
}

fn windows_stats() -> Result<Value> {
    let script = "$os=Get-CimInstance Win32_OperatingSystem; \
        $cpu=(Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average; \
        $d=@(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | ForEach-Object { @{ mount=$_.DeviceID; total_gb=[math]::Round($_.Size/1GB,1); free_gb=[math]::Round($_.FreeSpace/1GB,1) } }); \
        @{ cpu_percent=$cpu; memory=@{ total_mb=[int]($os.TotalVisibleMemorySize/1024); free_mb=[int]($os.FreePhysicalMemory/1024) }; disks=$d; uptime_hours=[math]::Round(((Get-Date)-$os.LastBootUpTime).TotalHours,1) } | ConvertTo-Json -Depth 4 -Compress";
    let out = util::run(std::path::Path::new("powershell.exe"), &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script.into()], None, &[], Some(Duration::from_secs(40)))?;
    let mut v: Value = serde_json::from_str(out.stdout.trim()).map_err(|e| DevdError::internal(format!("the machine's numbers could not be read: {e}")))?;
    if let (Some(t), Some(f)) = (v["memory"]["total_mb"].as_u64(), v["memory"]["free_mb"].as_u64()) {
        v["memory"]["used_mb"] = json!(t.saturating_sub(f));
    }
    Ok(v)
}

fn unix_stats() -> Result<Value> {
    let mem = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| mem.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    let load = std::fs::read_to_string("/proc/loadavg").ok().and_then(|l| l.split_whitespace().next().and_then(|n| n.parse::<f64>().ok()));
    let (total, free) = (kb("MemTotal:") / 1024, kb("MemAvailable:") / 1024);
    Ok(json!({ "load_average": load, "memory": { "total_mb": total, "free_mb": free, "used_mb": total.saturating_sub(free) }, "disks": [] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_error_or_silence_is_down() {
        let p = |status: Option<u16>| Probe { status, ms: 5, error: None };
        assert!(p(Some(200)).up() && p(Some(301)).up() && p(Some(404)).up() && p(Some(403)).up());
        assert!(!p(Some(500)).up() && !p(Some(502)).up() && !p(None).up());
    }

    #[test]
    fn a_health_url_is_checked() {
        assert!(valid_url("https://example.com/health").is_ok() && valid_url("http://127.0.0.1:8000").is_ok());
        for bad in ["ftp://x.com", "https://", "https://u:p@x.com", "https://a b.com", "example.com"] {
            assert!(valid_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_closed_port_is_down_and_says_why() {
        let p = probe("http://127.0.0.1:9/", None);
        assert!(!p.up() && p.error.is_some());
    }
}
