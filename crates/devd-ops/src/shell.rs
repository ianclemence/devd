//! `php` and `composer` from a terminal: which program a launcher runs, and Composer itself.
//!
//! The launchers in `<home>/bin` (written by [`devd_core::shims`]) run `devd shim <command>`.
//! Here that is decided, at the moment of the call, from the folder the person is standing in:
//!
//! 1. a site whose project folder contains it runs with the PHP that site is served with, so
//!    `php artisan migrate` in a project behaves as the project does in the browser;
//! 2. else a runtime pin on the folder or one above it (`devd runtime pin`);
//! 3. else devd's default PHP.
//!
//! Composer is the vendor's `composer.phar` under `<home>/tools`, run by that same PHP.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::download::{self, Artifact};
use devd_core::shims;
use devd_core::state::Site;
use devd_core::{DevdError, Result};

use crate::{tools_dir, php_for_site};

/// What a launcher is going to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The program.
    pub program: PathBuf,
    /// Arguments that come before the person's own (Composer's phar, for one).
    pub prefix: Vec<String>,
    /// Folders put in front of `PATH` for the program, so `composer` finds `php`.
    pub path_dirs: Vec<PathBuf>,
    /// Why this one, in a sentence.
    pub reason: String,
}

/// The folder a site's project lives in.
pub fn project_dir(site: &Site) -> PathBuf {
    site.project_dir()
}

fn normal(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.strip_prefix("//?/").unwrap_or(&s).trim_end_matches('/').to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

/// Whether `dir` is `folder` or inside it.
pub fn within(dir: &Path, folder: &Path) -> bool {
    let (d, f) = (normal(dir), normal(folder));
    !f.is_empty() && (d == f || d.starts_with(&format!("{f}/")))
}

/// The site whose project folder holds `cwd`. The deepest match wins, for sites inside sites.
pub fn site_for_dir(ctx: &Ctx, cwd: &Path) -> Result<Option<Site>> {
    let mut best: Option<(usize, Site)> = None;
    for site in ctx.store.list_sites()? {
        if site.upstream.is_some() {
            continue;
        }
        let folder = project_dir(&site);
        if within(cwd, &folder) {
            let depth = normal(&folder).len();
            if best.as_ref().is_none_or(|(d, _)| depth > *d) {
                best = Some((depth, site));
            }
        }
    }
    Ok(best.map(|(_, s)| s))
}

fn version_of(exe: &Path) -> String {
    exe.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The PHP for a folder, and why.
pub fn php_for_dir(ctx: &Ctx, cwd: &Path) -> Result<(PathBuf, String)> {
    if let Some(site) = site_for_dir(ctx, cwd)? {
        // Only a site that serves PHP has a PHP of its own. A Go module or a static site has none, and a
        // pin left on it by an older devd is reported as unused everywhere else, so it must not steer the
        // terminal either: the folder falls through to its own pin, or the default.
        if site.effective_serving().serving.uses_php() {
            let exe = php_for_site(ctx, &site)?;
            return Ok((exe.clone(), format!("`{}` is served with PHP {}", site.name, version_of(&exe))));
        }
    }
    for ancestor in cwd.ancestors() {
        let key = ancestor.display().to_string();
        if ctx.store.runtime_pins_of(&key)?.iter().any(|(kind, _)| kind == "php") {
            let exe = devd_runtime::executable_for(ctx, Some(&key), "php")?;
            return Ok((exe.clone(), format!("{} is pinned to PHP {}", ancestor.display(), version_of(&exe))));
        }
    }
    let exe = devd_runtime::executable_for(ctx, None, "php")?;
    Ok((exe.clone(), format!("no site or pin here, so devd's default PHP {}", version_of(&exe))))
}

/// What `command` should run, from `cwd`.
pub fn resolve(ctx: &Ctx, command: &str, cwd: &Path) -> Result<Resolved> {
    let (php, reason) = php_for_dir(ctx, cwd)?;
    let mut path_dirs: Vec<PathBuf> = php.parent().map(|d| vec![d.to_path_buf()]).unwrap_or_default();
    path_dirs.push(tools_dir(ctx));
    match command {
        "php" => Ok(Resolved { program: php, prefix: Vec::new(), path_dirs, reason }),
        "composer" => {
            let phar = shims::composer_phar(&ctx.paths);
            if !phar.is_file() {
                return Err(DevdError::not_found("Composer is not installed in devd; run `devd call composer.install`"));
            }
            Ok(Resolved { program: php, prefix: vec![phar.display().to_string()], path_dirs, reason })
        }
        other => Err(DevdError::unsupported(format!("devd has no launcher for `{other}`; it has {}", shims::COMMANDS.join(" and ")))),
    }
}

/// The `PATH` the program runs with: its own folders first.
pub fn path_for(dirs: &[PathBuf]) -> String {
    let mut parts: Vec<PathBuf> = dirs.to_vec();
    if let Some(existing) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(parts).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Run `command` from `cwd` with the terminal's own input and output, and return its exit code.
pub fn run(ctx: &Ctx, command: &str, args: &[String], cwd: &Path) -> Result<i32> {
    let r = resolve(ctx, command, cwd)?;
    let status = std::process::Command::new(&r.program)
        .args(&r.prefix)
        .args(args)
        .current_dir(cwd)
        .env("PATH", path_for(&r.path_dirs))
        .status()
        .map_err(|e| DevdError::Command { program: r.program.display().to_string(), code: -1, stderr: e.to_string() })?;
    Ok(status.code().unwrap_or(1))
}

// ------------------------------------------------------------------- Composer

#[derive(Deserialize)]
struct Versions {
    stable: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    path: String,
    version: String,
}

/// Run a small network read, and once more if it fails: a connection that was fine a moment
/// ago sometimes drops, and the person should not have to notice.
fn twice<T>(mut read: impl FnMut() -> Result<T>) -> Result<T> {
    read().or_else(|_| {
        std::thread::sleep(std::time::Duration::from_secs(1));
        read()
    })
}

/// The newest stable Composer and the SHA-256 getcomposer.org publishes for it.
pub fn latest_composer() -> Result<(String, String, String)> {
    let timeout = std::time::Duration::from_secs(30);
    let versions: Versions = twice(|| download::fetch_json("https://getcomposer.org/versions", timeout))?;
    let release = versions.stable.into_iter().next().ok_or_else(|| DevdError::not_found("getcomposer.org lists no stable release"))?;
    let url = format!("https://getcomposer.org{}", release.path);
    let sums = twice(|| download::fetch_text(&format!("{url}.sha256sum"), timeout))?;
    let digest = sums
        .split_whitespace()
        .find(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| DevdError::invalid("getcomposer.org published no SHA-256 for this release"))?
        .to_lowercase();
    Ok((release.version, url, digest))
}

/// Download Composer, check it against the vendor's published digest, put it in the tools
/// folder and make `composer` runnable.
pub fn install_composer(ctx: &Ctx, force: bool) -> Result<Value> {
    let phar = shims::composer_phar(&ctx.paths);
    if phar.is_file() && !force {
        return Err(DevdError::conflict(format!("Composer is already installed at {}; pass force to replace it", phar.display())));
    }
    let (version, url, digest) = latest_composer()?;
    let artifact = Artifact::new(&url).sha256(digest.clone()).filename(format!("composer-{version}.phar"));
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), None)?;
    std::fs::create_dir_all(tools_dir(ctx))?;
    std::fs::copy(&fetched.path, &phar)?;

    // Prove it starts with the PHP devd would give it, rather than assuming.
    let verified = devd_runtime::executable_for(ctx, None, "php")
        .ok()
        .and_then(|php| devd_core::util::run(&php, &[phar.display().to_string(), "--version".into(), "--no-ansi".into()], None, &[], Some(std::time::Duration::from_secs(60))).ok())
        .filter(|o| o.ok())
        .map(|o| o.stdout.trim().to_string());

    if let Ok(exe) = devd_core::util::cli_exe() {
        shims::ensure(&ctx.paths, &exe)?;
    }
    let state = shims::path_state(&ctx.paths).ok();
    Ok(json!({
        "version": version,
        "path": phar.display().to_string(),
        "sha256": digest,
        "bytes": fetched.bytes,
        "from_cache": fetched.from_cache,
        "verified": verified,
        "on_path": state.as_ref().map(|s| s.on_path),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(root: &str) -> Site {
        Site {
            id: "1".into(),
            name: "s".into(),
            host: "s.test".into(),
            root: root.into(),
            php: None,
            tls_mode: "internal".into(),
            db_name: None,
            upstream: None,
            serving: None,
            web_server: "nginx".into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn a_web_directory_belongs_to_the_project_above_it() {
        let sep = std::path::MAIN_SEPARATOR;
        let public = format!("{sep}work{sep}shop{sep}public");
        assert_eq!(project_dir(&site(&public)), PathBuf::from(format!("{sep}work{sep}shop")));
        let plain = format!("{sep}work{sep}blog");
        assert_eq!(project_dir(&site(&plain)), PathBuf::from(&plain));
    }

    #[test]
    fn a_folder_is_inside_another_only_by_whole_names() {
        assert!(within(Path::new("/work/shop/app/Http"), Path::new("/work/shop")));
        assert!(within(Path::new("/work/shop"), Path::new("/work/shop/")));
        assert!(!within(Path::new("/work/shopping"), Path::new("/work/shop")), "a name that merely starts the same is a different folder");
        if cfg!(windows) {
            assert!(within(Path::new("D:\\Work\\Shop\\x"), Path::new("d:/work/shop")));
            assert!(within(Path::new("\\\\?\\D:\\work\\shop\\x"), Path::new("D:\\work\\shop")), "a canonical path with its \\\\?\\ prefix still matches");
        }
    }

    #[test]
    fn the_programs_own_folders_come_before_the_existing_path() {
        let joined = path_for(&[PathBuf::from("/devd/php"), PathBuf::from("/devd/tools")]);
        let parts: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(parts[0], PathBuf::from("/devd/php"));
        assert_eq!(parts[1], PathBuf::from("/devd/tools"));
        assert!(parts.len() >= 2);
    }
}
