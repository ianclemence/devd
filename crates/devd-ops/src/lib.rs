//! Site operations: what a server panel such as Ploi offers per site, for a machine you develop on.
//!
//! * [`env`] - the `.env` file: read masked, set, unset, and diff against another environment.
//! * [`cron`] - cron jobs, with the Laravel scheduler as a one-line job, run by devd while it is open.
//! * [`daemons`] - queue workers and other background programs, supervised like any service.
//! * [`logs`] - the Laravel log as entries, and the web server and PHP logs as tails.
//! * [`deploy`] - a deploy script per site, the run, its history, rollback, and the push-to-deploy hook.
//! * [`notify`] - where to tell people when a deploy or a cron job finishes or fails.
//! * [`ploi`] - the same operations on a real Ploi server, through its API.
//!
//! Redirects live in `devd-services`, because the web server configuration renders them.
//!
//! Everything is a capability, so a person in the window and an agent over MCP do the same
//! things through the same approval and audit path. Anything that runs a command is `exec`
//! tier; anything that reaches a remote server is `remote`.

pub mod caps;
pub mod caps_more;
pub mod caps_remote;
pub mod cron;
pub mod daemons;
pub mod dbadmin;
pub mod deploy;
pub mod env;
pub mod health;
pub mod laravel;
pub mod limits;
pub mod logs;
pub mod notify;
pub mod ploi;
pub mod remote;
pub mod runner;
pub mod servers;
pub mod ssh;

use std::path::{Path, PathBuf};

use devd_core::capability::Ctx;
use devd_core::state::Site;
use devd_core::{DevdError, Result};

/// A site by name.
pub fn site_named(ctx: &Ctx, name: &str) -> Result<Site> {
    ctx.store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`; `devd site list` shows them")))
}

/// The project folder of a site, which must exist.
pub fn site_root(ctx: &Ctx, name: &str) -> Result<(Site, PathBuf)> {
    let site = site_named(ctx, name)?;
    let root = PathBuf::from(&site.root);
    if !root.is_dir() {
        return Err(DevdError::not_found(format!("the project folder of `{name}` ({}) does not exist", root.display())));
    }
    Ok((site, root))
}

/// The PHP a site runs: its pinned series, else the primary, else whichever is installed.
pub fn php_for_site(ctx: &Ctx, site: &Site) -> Result<PathBuf> {
    let wanted = site
        .php
        .clone()
        .or_else(|| devd_core::config::Settings::load(&ctx.paths.settings_file()).ok().and_then(|s| s.primary_php))
        .map(|v| devd_services::php_series(&v));
    let mut best: Option<String> = None;
    for r in ctx.store.list_runtimes()? {
        if r.kind != "php" {
            continue;
        }
        let series = devd_services::php_series(&r.version);
        if wanted.as_ref().is_some_and(|w| *w != series) {
            continue;
        }
        if best.as_ref().is_none_or(|v| r.version.as_str() > v.as_str()) {
            best = Some(r.version.clone());
        }
    }
    let version = best.ok_or_else(|| DevdError::not_found("no PHP runtime is installed for this site; `devd runtime install php@8.4` adds one"))?;
    devd_runtime::executable(ctx, "php", Some(&version))
}

/// Where operation logs go.
pub fn log_dir(ctx: &Ctx) -> PathBuf {
    let d = ctx.paths.logs();
    let _ = std::fs::create_dir_all(&d);
    d
}

/// The PATH a command in a site runs with: the site's PHP and devd's tools folder first.
pub fn command_path(ctx: &Ctx, site: &Site) -> String {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Ok(php) = php_for_site(ctx, site) {
        if let Some(dir) = php.parent() {
            parts.push(dir.to_path_buf());
        }
    }
    parts.push(tools_dir(ctx));
    if let Some(existing) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(parts).map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

/// devd's own small tools (a `composer` shim) live here.
pub fn tools_dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("tools")
}

/// Make `composer` runnable by name in site commands, when `composer.phar` is in the tools folder.
pub fn ensure_composer_shim(ctx: &Ctx) -> Result<()> {
    let dir = tools_dir(ctx);
    if !dir.join("composer.phar").is_file() {
        return Ok(());
    }
    if cfg!(windows) {
        let shim = dir.join("composer.cmd");
        if !shim.is_file() {
            std::fs::write(&shim, "@echo off\r\nphp \"%~dp0composer.phar\" %*\r\n")?;
        }
    } else {
        let shim = dir.join("composer");
        if !shim.is_file() {
            std::fs::write(&shim, "#!/bin/sh\nexec php \"$(dirname \"$0\")/composer.phar\" \"$@\"\n")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

/// The shell invocation for one command line: `cmd /C` on Windows, `sh -c` elsewhere.
pub fn shell_command(line: &str) -> std::process::Command {
    if cfg!(windows) {
        let mut c = std::process::Command::new(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string()));
        c.arg("/C").arg(line);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(line);
        c
    }
}

/// What running one command line in a site produced.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Ran {
    /// Exit code, or -1 when the process could not be started or was killed.
    pub exit: i32,
    /// Seconds it took.
    pub seconds: f64,
}

/// Run one command line in a site's folder with the site's PHP on PATH, appending its output to a log file.
pub fn run_line(ctx: &Ctx, site: &Site, root: &Path, line: &str, log: &Path) -> Ran {
    run_line_with(ctx, site, root, line, log, &[])
}

/// [`run_line`] with extra environment variables (a site's deploy variables).
pub fn run_line_with(ctx: &Ctx, site: &Site, root: &Path, line: &str, log: &Path, envs: &[(String, String)]) -> Ran {
    use std::io::Write;
    let started = std::time::Instant::now();
    let _ = ensure_composer_shim(ctx);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(log).ok();
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "$ {line}");
    }
    let mut cmd = shell_command(line);
    cmd.current_dir(root).env("PATH", command_path(ctx, site)).env("DEVD_SITE", &site.name);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::null());
    match (file.as_ref().and_then(|f| f.try_clone().ok()), file.as_ref().and_then(|f| f.try_clone().ok())) {
        (Some(out), Some(err)) => {
            cmd.stdout(out).stderr(err);
        }
        _ => {
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
    let exit = match cmd.status() {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => {
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "could not start: {e}");
            }
            -1
        }
    };
    if let Some(f) = file.as_mut() {
        let _ = writeln!(f, "[exit {exit}]");
    }
    Ran { exit, seconds: started.elapsed().as_secs_f64() }
}

/// The last `max` bytes of a text file, from a line boundary.
pub fn tail_bytes(path: &Path, max: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max as u64);
    let _ = f.seek(SeekFrom::Start(start));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or(text)
    } else {
        text
    }
}

/// A context for tests: an in-memory store and a scratch layout.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::Arc;

    use devd_core::capability::{Actor, Ctx, Surface};
    use devd_core::config::Settings;
    use devd_core::events::EventBus;
    use devd_core::paths::Paths;
    use devd_core::state::Store;

    pub fn ctx(dir: &std::path::Path) -> Ctx {
        let paths = Paths::rooted(dir);
        paths.ensure().unwrap();
        Ctx { store: Arc::new(Store::open_in_memory().unwrap()), paths: Arc::new(paths), settings: Arc::new(Settings::default()), events: EventBus::new(), actor: Actor::human(Surface::Cli), dry_run: false }
    }
}
