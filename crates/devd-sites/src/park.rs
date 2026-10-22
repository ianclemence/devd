//! Parked folders: every project directory in them is a site.
//!
//! Auto virtual hosts: put a folder in `www`, and
//! `https://<folder>.test` works. devd does the same on request. A folder is
//! parked (the projects root always is, and `site.park` adds more), and
//! `site.sync` makes a site of every project directory in it that does not have
//! one. It never writes into a project, never removes a site whose folder is gone
//! (that is `site.remove`, which asks), and says what it skipped and why.

use std::path::{Path, PathBuf};

use serde::Serialize;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

use crate::{create_with, CreateRequest};

/// One folder that was not turned into a site, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    /// The directory.
    pub folder: String,
    /// What is wrong with it.
    pub reason: String,
}

/// What a sync did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    /// Folders that now have a site, as (site, hostname).
    pub created: Vec<(String, String)>,
    /// Folders that already had one.
    pub existing: Vec<String>,
    /// Folders that could not become a site.
    pub skipped: Vec<Skipped>,
    /// Sites in a parked folder whose directory no longer exists.
    pub gone: Vec<String>,
    /// Whether the web server was re-rendered and reloaded.
    pub reloaded: bool,
    /// Anything the caller should know.
    pub warnings: Vec<String>,
}

/// The folders whose projects become sites: the projects root, then the parked ones.
pub fn folders(ctx: &Ctx) -> Vec<PathBuf> {
    let mut out = vec![ctx.settings.effective_www_root(&ctx.paths.www())];
    for extra in &ctx.settings.parked {
        if !out.contains(extra) {
            out.push(extra.clone());
        }
    }
    out
}

/// A folder name as a site name: lower case, with anything a hostname cannot
/// hold turned into a hyphen. `None` when nothing usable is left.
pub fn slug(folder_name: &str) -> Option<String> {
    let mut out = String::new();
    for c in folder_name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty() && out.len() <= 63).then_some(out)
}

/// Whether a directory is a project rather than tooling that lives beside projects.
fn is_project_dir(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    path.is_dir() && !name.starts_with('.') && !matches!(name.as_str(), "node_modules" | "vendor" | "lost+found")
}

/// Make a site of every project directory in `folders` that has none.
pub fn sync(ctx: &Ctx, folders: &[PathBuf]) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let sites = ctx.store.list_sites()?;
    let same = |a: &str, b: &Path| Path::new(a) == b || a.eq_ignore_ascii_case(&b.to_string_lossy());

    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            report.skipped.push(Skipped {
                folder: folder.display().to_string(),
                reason: "the folder cannot be read".to_string(),
            });
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_project_dir(p)).collect();
        dirs.sort();
        for dir in dirs {
            let label = dir.display().to_string();
            if let Some(site) = ctx.store.list_sites()?.iter().find(|s| same(&s.root, &dir)) {
                report.existing.push(site.name.clone());
                continue;
            }
            let leaf = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Some(name) = slug(&leaf) else {
                report.skipped.push(Skipped { folder: label, reason: "its name has nothing a hostname can use".into() });
                continue;
            };
            let request = CreateRequest {
                name: name.clone(),
                host: None,
                root: Some(label.clone()),
                tls: true,
                db: false,
                upstream: None,
            };
            match create_with(ctx, &request, false, false) {
                Ok(created) => {
                    report.warnings.extend(created.warnings.into_iter().filter(|w| !w.contains("does not resolve")));
                    report.created.push((name, created.site.host));
                }
                Err(e) if e.code() == "conflict" => {
                    report.skipped.push(Skipped { folder: label, reason: e.to_string() });
                }
                Err(e) => report.skipped.push(Skipped { folder: label, reason: e.to_string() }),
            }
        }
        for site in &sites {
            if Path::new(&site.root).starts_with(folder) && site.upstream.is_none() && !Path::new(&site.root).is_dir() {
                report.gone.push(site.name.clone());
            }
        }
    }

    if !report.created.is_empty() {
        let mut warnings = Vec::new();
        report.reloaded = crate::reconcile(ctx, &mut warnings)?;
        report.warnings.extend(warnings);
        report.warnings.push(format!(
            "{} new name(s) do not resolve until the hosts file has them: `devd hosts sync`",
            report.created.len()
        ));
    }
    Ok(report)
}

/// Add a folder to the parked list in the settings file.
pub fn park(ctx: &Ctx, folder: &Path) -> Result<PathBuf> {
    if !folder.is_absolute() {
        return Err(DevdError::invalid("give the absolute path of the folder"));
    }
    let folder = folder.to_path_buf();
    if !folder.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a folder", folder.display())));
    }
    let file = ctx.paths.settings_file();
    let mut settings = devd_core::config::Settings::load(&file)?;
    if !settings.parked.contains(&folder) {
        settings.parked.push(folder.clone());
        settings.save(&file)?;
    }
    Ok(folder)
}

/// Remove a folder from the parked list. The sites it produced stay.
pub fn unpark(ctx: &Ctx, folder: &Path) -> Result<bool> {
    let file = ctx.paths.settings_file();
    let mut settings = devd_core::config::Settings::load(&file)?;
    let before = settings.parked.len();
    settings.parked.retain(|p| p != folder);
    let removed = settings.parked.len() != before;
    if removed {
        settings.save(&file)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_name_becomes_a_hostname_label() {
        assert_eq!(slug("My Blog").as_deref(), Some("my-blog"));
        assert_eq!(slug("shop_v2").as_deref(), Some("shop-v2"));
        assert_eq!(slug("--odd--").as_deref(), Some("odd"));
        assert_eq!(slug("ünï").as_deref(), Some("n"));
        assert_eq!(slug("日本語"), None);
        assert_eq!(slug(&"a".repeat(64)), None);
    }

    #[test]
    fn tooling_folders_are_not_projects() {
        let dir = std::env::temp_dir().join(format!("devd-park-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("blog")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        assert!(is_project_dir(&dir.join("blog")));
        assert!(!is_project_dir(&dir.join(".git")));
        assert!(!is_project_dir(&dir.join("node_modules")));
        assert!(!is_project_dir(&dir.join("notes.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
