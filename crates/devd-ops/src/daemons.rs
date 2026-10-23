//! Queue workers and other background programs for a site.
//!
//! A worker is an ordinary devd service: it is supervised, restarted when it exits, started with the
//! rest of the stack and its output goes to a log. For a Laravel site it is `php artisan queue:work`
//! with the site's own PHP; for anything else it is the command you give, run in the site's folder.

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::state::Service;
use devd_core::{util, DevdError, Result};

/// The kind under which worker definitions are stored.
pub const KIND: &str = "daemon";

/// What a worker is made of.
#[derive(Debug, Clone)]
pub struct Spec {
    /// The site.
    pub site: String,
    /// A short name, unique within the site. Defaults to `queue` for a queue worker.
    pub name: String,
    /// A command line, for a program that is not `queue:work`.
    pub command: Option<String>,
    /// Queue connection (`database`, `redis`, ...). Laravel's default when empty.
    pub connection: Option<String>,
    /// Queue name(s), comma separated.
    pub queue: Option<String>,
    /// `--tries`.
    pub tries: Option<u32>,
    /// `--sleep`.
    pub sleep: Option<u32>,
    /// `--max-time`: the worker exits and is restarted after this many seconds, so it picks up new code.
    pub max_time: Option<u32>,
    /// How many processes, 1 to 8.
    pub processes: u8,
}

/// A worker as the capabilities show it.
#[derive(Debug, Clone, Serialize)]
pub struct Daemon {
    /// Id: `<site>-<name>`.
    pub id: String,
    /// The site.
    pub site: String,
    /// The name.
    pub name: String,
    /// What it runs, as a line a person can read.
    pub command: String,
    /// How many processes.
    pub processes: u8,
    /// How many of them are running.
    pub running: usize,
    /// The service names behind it.
    pub services: Vec<String>,
}

fn valid_part(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn service_name(site: &str, name: &str, n: u8) -> String {
    if n <= 1 {
        format!("worker-{site}-{name}")
    } else {
        format!("worker-{site}-{name}-{n}")
    }
}

fn arg_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.,:".contains(c))
}

/// The argument array for one process of a worker.
pub fn argv(ctx: &Ctx, spec: &Spec) -> Result<Vec<String>> {
    let (site, root) = crate::site_root(ctx, &spec.site)?;
    match &spec.command {
        None => {
            let artisan = root.join("artisan");
            if !artisan.is_file() {
                return Err(DevdError::invalid(format!(
                    "{} has no artisan file, so it is not a Laravel project; pass a command to run instead",
                    root.display()
                )));
            }
            let mut v = vec![crate::php_for_site(ctx, &site)?.display().to_string(), artisan.display().to_string(), "queue:work".to_string()];
            if let Some(c) = &spec.connection {
                if !arg_ok(c) {
                    return Err(DevdError::invalid(format!("`{c}` is not a queue connection name")));
                }
                v.push(c.clone());
            }
            if let Some(q) = &spec.queue {
                if !arg_ok(q) {
                    return Err(DevdError::invalid(format!("`{q}` is not a queue name")));
                }
                v.push(format!("--queue={q}"));
            }
            v.push(format!("--tries={}", spec.tries.unwrap_or(3)));
            v.push(format!("--sleep={}", spec.sleep.unwrap_or(3)));
            v.push(format!("--max-time={}", spec.max_time.unwrap_or(3600)));
            v.push("--no-interaction".to_string());
            Ok(v)
        }
        Some(line) => {
            let line = line.trim();
            if line.is_empty() || line.len() > 2000 || line.contains(['\0', '\n', '\r']) {
                return Err(DevdError::invalid("the command must be one line of at most 2000 characters"));
            }
            let _ = (&site, &root, line);
            // The program is a small launcher script: passing a quoted command line to `cmd /C` loses its quotes.
            Ok(vec![launcher_path(ctx, &spec.site, &spec.name).display().to_string()])
        }
    }
}

/// Where the launcher script of a worker lives.
fn launcher_path(ctx: &Ctx, site: &str, name: &str) -> std::path::PathBuf {
    ctx.paths.etc().join("workers").join(format!("{site}-{name}.{}", if cfg!(windows) { "cmd" } else { "sh" }))
}

/// The launcher script of a program worker: the site's PHP and devd's tools first on PATH, then the
/// site's folder, then the command (`exec` where the shell has it, so stopping the worker stops the program).
fn launcher_text(ctx: &Ctx, spec: &Spec) -> Result<String> {
    let (site, root) = crate::site_root(ctx, &spec.site)?;
    let path = crate::command_path(ctx, &site);
    let line = spec.command.as_deref().unwrap_or("").trim();
    Ok(if cfg!(windows) {
        format!("@echo off\r\nset \"PATH={path}\"\r\ncd /d \"{}\"\r\n{line}\r\n", root.display())
    } else {
        format!("#!/bin/sh\nPATH='{}'\nexport PATH\ncd '{}' || exit 1\nexec {line}\n", path.replace('\'', ""), root.display())
    })
}

/// Write the launcher script of a program worker.
fn write_launcher(ctx: &Ctx, spec: &Spec) -> Result<()> {
    if spec.command.is_none() {
        return Ok(());
    }
    let path = launcher_path(ctx, &spec.site, &spec.name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    devd_core::util::atomic_write(&path, &launcher_text(ctx, spec)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn readable(spec: &Spec, argv: &[String]) -> String {
    match &spec.command {
        Some(c) => c.clone(),
        None => format!("php artisan {}", argv.iter().skip(2).cloned().collect::<Vec<_>>().join(" ")),
    }
}

/// Create (or replace) a worker. It is not started.
pub fn add(ctx: &Ctx, spec: &Spec) -> Result<Daemon> {
    if !valid_part(&spec.name) {
        return Err(DevdError::invalid("a worker name is letters, digits, `-` and `_`, up to 40 characters"));
    }
    if spec.processes == 0 || spec.processes > 8 {
        return Err(DevdError::invalid("processes must be between 1 and 8"));
    }
    let args = argv(ctx, spec)?;
    write_launcher(ctx, spec)?;
    let id = format!("{}-{}", spec.site, spec.name);
    // Replace an existing worker of this id: stop and drop its services first.
    if ctx.store.ops_get(KIND, &id)?.is_some() {
        remove(ctx, &id)?;
    }
    for n in 1..=spec.processes {
        let name = service_name(&spec.site, &spec.name, n);
        ctx.store.upsert_service(&Service {
            id: util::new_id(),
            name,
            kind: "worker".to_string(),
            runtime_id: None,
            argv: serde_json::to_string(&args).unwrap_or_default(),
            port: None,
            health: None,
            desired_state: "stopped".to_string(),
            restart_policy: "always".to_string(),
            enabled: true,
            created_at: util::now(),
        })?;
    }
    ctx.store.ops_put(
        KIND,
        &id,
        &spec.site,
        &json!({
            "name": spec.name, "processes": spec.processes, "command": readable(spec, &args),
            "queue": spec.command.is_none(),
        }),
    )?;
    show(ctx, &id)
}

/// The service names of a worker.
fn services_of(item: &devd_core::store_ops::OpsItem) -> Vec<String> {
    let name = item.data["name"].as_str().unwrap_or("");
    let n = item.data["processes"].as_u64().unwrap_or(1).clamp(1, 8) as u8;
    (1..=n).map(|i| service_name(&item.site, name, i)).collect()
}

fn describe(ctx: &Ctx, item: &devd_core::store_ops::OpsItem) -> Daemon {
    let services = services_of(item);
    let running = services.iter().filter(|s| devd_services::status(ctx, s).map(|st| st.running).unwrap_or(false)).count();
    Daemon {
        id: item.id.clone(),
        site: item.site.clone(),
        name: item.data["name"].as_str().unwrap_or("").to_string(),
        command: item.data["command"].as_str().unwrap_or("").to_string(),
        processes: item.data["processes"].as_u64().unwrap_or(1) as u8,
        running,
        services,
    }
}

fn get(ctx: &Ctx, id: &str) -> Result<devd_core::store_ops::OpsItem> {
    ctx.store.ops_get(KIND, id)?.ok_or_else(|| DevdError::not_found(format!("no worker `{id}`; `devd call queue.list` shows them")))
}

/// One worker.
pub fn show(ctx: &Ctx, id: &str) -> Result<Daemon> {
    Ok(describe(ctx, &get(ctx, id)?))
}

/// The workers, optionally one site's.
pub fn list(ctx: &Ctx, site: Option<&str>) -> Result<Vec<Daemon>> {
    Ok(ctx.store.ops_list(KIND, site)?.iter().map(|i| describe(ctx, i)).collect())
}

/// Start, stop or restart every process of a worker.
pub fn control(ctx: &Ctx, id: &str, action: &str) -> Result<Daemon> {
    let item = get(ctx, id)?;
    for s in services_of(&item) {
        match action {
            "start" => {
                devd_services::start(ctx, &s)?;
            }
            "stop" => {
                devd_services::stop(ctx, &s)?;
            }
            "restart" => {
                devd_services::restart(ctx, &s)?;
            }
            other => return Err(DevdError::invalid(format!("`{other}` is not start, stop or restart"))),
        }
    }
    show(ctx, id)
}

/// Stop a worker and forget it.
pub fn remove(ctx: &Ctx, id: &str) -> Result<()> {
    let item = get(ctx, id)?;
    for s in services_of(&item) {
        if devd_services::status(ctx, &s).map(|st| st.running).unwrap_or(false) {
            let _ = devd_services::stop(ctx, &s);
        }
        ctx.store.delete_service(&s)?;
    }
    ctx.store.ops_delete(KIND, id)?;
    // The launcher script of a program worker is ours; the site's own files are never touched.
    let script = launcher_path(ctx, &item.site, item.data["name"].as_str().unwrap_or(""));
    if script.is_file() {
        let _ = std::fs::remove_file(script);
    }
    Ok(())
}

/// The tail of the first process's log.
pub fn logs(ctx: &Ctx, id: &str, lines: usize) -> Result<Vec<String>> {
    let item = get(ctx, id)?;
    let first = services_of(&item).into_iter().next().ok_or_else(|| DevdError::internal("a worker with no process"))?;
    devd_services::logs(ctx, &first, lines.clamp(1, 1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_names_are_per_process() {
        assert_eq!(service_name("shop", "queue", 1), "worker-shop-queue");
        assert_eq!(service_name("shop", "queue", 3), "worker-shop-queue-3");
    }

    #[test]
    fn names_and_arguments_are_checked() {
        assert!(valid_part("queue-2") && !valid_part("a b") && !valid_part("") && !valid_part("a;b"));
        assert!(arg_ok("redis") && arg_ok("emails,default") && !arg_ok("x y") && !arg_ok("a&b"));
    }
}
