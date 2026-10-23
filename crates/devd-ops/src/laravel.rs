//! The Laravel tab: run `artisan`, switch maintenance mode, link `storage`, clear caches.
//!
//! Every command is `php artisan ...` run with the site's own PHP in the site's folder, as an argument
//! list and never through a shell, so a command cannot smuggle a second one in behind a `;`.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, DevdError, Result};

/// What an artisan command printed.
#[derive(Debug, Clone, Serialize)]
pub struct ArtisanRun {
    /// The arguments it ran with.
    pub command: String,
    /// Exit code.
    pub exit: i32,
    /// What it printed (standard output, then standard error), cut to the last 60 000 characters.
    pub output: String,
    /// How long it took.
    pub seconds: f64,
}

/// Split a command line into arguments the way a person writes one: spaces separate, quotes group, nothing else is special.
pub fn split_args(line: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err(DevdError::invalid("a quote was opened and never closed"));
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn check_args(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(DevdError::invalid("name an artisan command, e.g. `migrate:status`"));
    }
    if args.len() > 40 || args.iter().any(|a| a.len() > 500 || a.contains(['\0', '\n', '\r'])) {
        return Err(DevdError::invalid("an artisan command is at most 40 arguments of at most 500 characters, on one line"));
    }
    Ok(())
}

/// Run `php artisan <args>` for a site.
pub fn artisan(ctx: &Ctx, site: &str, args: &[String]) -> Result<ArtisanRun> {
    check_args(args)?;
    let (s, root) = crate::site_root(ctx, site)?;
    let artisan = root.join("artisan");
    if !artisan.is_file() {
        return Err(DevdError::invalid(format!("{site} has no artisan file, so it is not a Laravel project")));
    }
    let php = crate::php_for_site(ctx, &s)?;
    let mut argv = vec![artisan.display().to_string()];
    argv.extend(args.iter().cloned());
    argv.push("--no-interaction".to_string());
    argv.push("--no-ansi".to_string());
    let started = std::time::Instant::now();
    let out = util::run(&php, &argv, Some(&root), &[("PATH".to_string(), crate::command_path(ctx, &s))], Some(Duration::from_secs(600)))?;
    let mut text = out.stdout;
    if !out.stderr.trim().is_empty() {
        text.push_str(&out.stderr);
    }
    if text.chars().count() > 60_000 {
        text = text.chars().rev().take(60_000).collect::<Vec<_>>().into_iter().rev().collect();
    }
    Ok(ArtisanRun { command: args.join(" "), exit: out.code, output: text, seconds: started.elapsed().as_secs_f64() })
}

fn ok(run: ArtisanRun) -> Result<ArtisanRun> {
    if run.exit == 0 {
        Ok(run)
    } else {
        Err(DevdError::unsupported(format!("`artisan {}` exited with {}: {}", run.command, run.exit, run.output.trim().lines().last().unwrap_or(""))))
    }
}

/// Where the maintenance marker is.
fn down_file(root: &Path) -> std::path::PathBuf {
    root.join("storage").join("framework").join("down")
}

/// What the site's Laravel state is, without running PHP: maintenance mode, the storage link, the environment.
pub fn status(ctx: &Ctx, site: &str) -> Result<Value> {
    let (s, root) = crate::site_root(ctx, site)?;
    let laravel = root.join("artisan").is_file();
    let vars = crate::env::variables(&std::fs::read_to_string(crate::env::path_of(&root)).unwrap_or_default());
    let link = root.join("public").join("storage");
    let down = down_file(&root);
    let secret = std::fs::read_to_string(&down).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["secret"].as_str().map(str::to_string));
    Ok(json!({
        "site": site,
        "is_laravel": laravel,
        "maintenance": down.is_file(),
        "maintenance_bypass_path": secret.map(|x| format!("/{x}")),
        "storage_linked": link.exists(),
        "storage_target_exists": root.join("storage").join("app").join("public").is_dir(),
        "app_env": vars.get("APP_ENV"),
        "app_debug": vars.get("APP_DEBUG"),
        "config_cached": root.join("bootstrap").join("cache").join("config.php").is_file(),
        "routes_cached": std::fs::read_dir(root.join("bootstrap").join("cache")).map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("routes-"))).unwrap_or(false),
        "url": format!("https://{}", s.host),
    }))
}

/// Put the site in maintenance mode.
pub fn down(ctx: &Ctx, site: &str, secret: Option<&str>, retry: Option<u32>) -> Result<ArtisanRun> {
    let mut args = vec!["down".to_string()];
    if let Some(sec) = secret {
        if sec.is_empty() || sec.len() > 64 || !sec.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(DevdError::invalid("the bypass secret is letters, digits, `-` and `_`, up to 64 characters"));
        }
        args.push(format!("--secret={sec}"));
    }
    if let Some(r) = retry {
        args.push(format!("--retry={}", r.min(86_400)));
    }
    ok(artisan(ctx, site, &args)?)
}

/// Bring the site back.
pub fn up(ctx: &Ctx, site: &str) -> Result<ArtisanRun> {
    ok(artisan(ctx, site, &["up".to_string()])?)
}

/// `php artisan storage:link`.
pub fn storage_link(ctx: &Ctx, site: &str) -> Result<ArtisanRun> {
    ok(artisan(ctx, site, &["storage:link".to_string()])?)
}

/// What can be cleared, and the artisan command that does it.
pub const CLEARABLE: &[(&str, &str, &str)] = &[
    ("views", "view:clear", "compiled Blade views"),
    ("config", "config:clear", "the cached configuration"),
    ("routes", "route:clear", "the cached routes"),
    ("events", "event:clear", "the cached events"),
    ("cache", "cache:clear", "the application cache (whatever the cache driver holds)"),
    ("all", "optimize:clear", "views, configuration, routes and events together (not the application cache)"),
];

/// Clear one kind of cache, or several.
pub fn clear(ctx: &Ctx, site: &str, what: &[String]) -> Result<Vec<ArtisanRun>> {
    let mut runs = Vec::new();
    let wanted: Vec<String> = if what.is_empty() { vec!["all".to_string()] } else { what.to_vec() };
    for w in &wanted {
        let Some((_, cmd, _)) = CLEARABLE.iter().find(|(name, _, _)| name == w) else {
            return Err(DevdError::invalid(format!("`{w}` is not something to clear; use {}", CLEARABLE.iter().map(|c| c.0).collect::<Vec<_>>().join(", "))));
        };
        runs.push(ok(artisan(ctx, site, &[cmd.to_string()])?)?);
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_line_is_split_into_arguments_and_nothing_is_a_shell() {
        assert_eq!(split_args("migrate --force").unwrap(), vec!["migrate", "--force"]);
        assert_eq!(split_args("tinker --execute=\"echo 1;\"").unwrap(), vec!["tinker", "--execute=echo 1;"]);
        assert_eq!(split_args("  make:model   'Blog Post'  ").unwrap(), vec!["make:model", "Blog Post"]);
        // A semicolon or a pipe is just text in an argument.
        assert_eq!(split_args("about ; rm -rf x").unwrap(), vec!["about", ";", "rm", "-rf", "x"]);
        assert_eq!(split_args("a \"\" b").unwrap(), vec!["a", "", "b"]);
        assert!(split_args("a \"unclosed").is_err());
        assert!(split_args("   ").unwrap().is_empty());
    }

    #[test]
    fn arguments_are_checked() {
        assert!(check_args(&[]).is_err());
        assert!(check_args(&["a\nb".to_string()]).is_err());
        assert!(check_args(&vec!["x".to_string(); 41]).is_err());
        assert!(check_args(&["migrate".to_string()]).is_ok());
    }

    #[test]
    fn only_known_caches_can_be_cleared() {
        assert!(CLEARABLE.iter().any(|c| c.0 == "all" && c.1 == "optimize:clear"));
        assert!(down_file(Path::new("D:/www/shop")).ends_with("storage/framework/down") || down_file(Path::new("D:/www/shop")).to_string_lossy().ends_with("down"));
    }
}
