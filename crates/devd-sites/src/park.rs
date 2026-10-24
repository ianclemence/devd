//! Parked folders: finding the projects in them, and making sites of the ones that are web sites.
//!
//! Four things are kept apart, because they have different consequences and
//! used to happen as one:
//!
//! * **Discovery** ([`discover`]) reads the parked folders and reports what is
//!   in each directory: what it is, whether a site already serves it, whether it
//!   could be one, and why not. It changes nothing.
//! * **Registration** ([`sync`]) makes a site of each directory that discovery
//!   found servable and that has none. It is the only step that writes a record,
//!   issues a certificate and re-renders the web server, and it does so for
//!   exactly the directories its plan names.
//! * **Configuration** (`site.set_serving`, `site.set_php`) is a person's choice
//!   about one site, never a side effect of the two above.
//! * **Activation** (the hosts file, starting services) is `hosts.sync` and the
//!   stack commands, each with its own approval.
//!
//! A directory is a *project*; only a project with something a web server can
//! serve is a candidate *site*. A Go service or a PHP library is a project and
//! stays one. `sync` never removes a site whose folder is gone (that is
//! `site.remove`, which asks), and says what it skipped and why.

use std::path::{Path, PathBuf};

use serde::Serialize;

use devd_core::capability::Ctx;
use devd_core::serving::{self, Content, Detection};
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

/// What discovery found for one directory.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    /// The project directory.
    pub folder: String,
    /// The site name it would get, when its folder name can be a hostname.
    pub name: Option<String>,
    /// The hostname it would get.
    pub host: Option<String>,
    /// The site that already serves this directory, when one does.
    pub site: Option<String>,
    /// What the directory contains.
    pub detection: Detection,
    /// A sentence for a person.
    pub summary: String,
    /// Whether `sync` would make a site of it.
    pub eligible: bool,
    /// Why it is not eligible, or what to know first. `None` when it simply is.
    pub reason: Option<String>,
}

/// What discovery found in the parked folders. Nothing was changed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Discovery {
    /// Every project directory, in folder order.
    pub candidates: Vec<Candidate>,
    /// Folders that could not be read.
    pub unreadable: Vec<Skipped>,
    /// Sites in a parked folder whose directory no longer exists.
    pub gone: Vec<String>,
}

impl Discovery {
    /// The directories `sync` would make sites of.
    pub fn eligible(&self) -> impl Iterator<Item = &Candidate> {
        self.candidates.iter().filter(|c| c.eligible)
    }
}

/// The folders whose projects are looked at: the projects root, then the parked ones.
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

fn same_path(a: &str, b: &Path) -> bool {
    Path::new(a) == b || a.eq_ignore_ascii_case(&b.to_string_lossy())
}

/// Read the folders and report every project directory in them. Changes nothing, and gives the
/// same answer however often it is run.
pub fn discover(ctx: &Ctx, folders: &[PathBuf]) -> Result<Discovery> {
    let mut found = Discovery::default();
    let sites = ctx.store.list_sites()?;
    // Names and hostnames handed out within this pass count too, so two folders that
    // slug to the same name are reported as a conflict, not both offered.
    let mut taken_names: Vec<String> = sites.iter().map(|s| s.name.clone()).collect();
    let mut taken_hosts: Vec<String> = sites.iter().map(|s| s.host.to_ascii_lowercase()).collect();

    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            found.unreadable.push(Skipped { folder: folder.display().to_string(), reason: "the folder cannot be read".to_string() });
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_project_dir(p)).collect();
        dirs.sort();
        for dir in dirs {
            // The same directory reached through two parked folders is one project.
            if found.candidates.iter().any(|c| same_path(&c.folder, &dir)) {
                continue;
            }
            let detection = serving::detect(&dir);
            let summary = detection.summary();
            let leaf = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let name = slug(&leaf);
            let host = name.as_ref().map(|n| format!("{n}.{}", ctx.settings.tld));
            let existing = sites.iter().find(|s| same_path(&s.root, &dir)).map(|s| s.name.clone());

            let (eligible, reason) = if existing.is_some() {
                (false, Some("already has a site".to_string()))
            } else if name.is_none() {
                (false, Some("its name has nothing a hostname can use".to_string()))
            } else if detection.content.is_none() {
                (
                    false,
                    Some(format!(
                        "not a web project: {summary}. A program that listens on a port can be served with `site.create --upstream`"
                    )),
                )
            } else if name.as_ref().is_some_and(|n| taken_names.contains(n)) {
                (false, Some(format!("the site name `{}` is already taken by another directory", name.clone().unwrap_or_default())))
            } else if host.as_ref().is_some_and(|h| taken_hosts.contains(&h.to_ascii_lowercase())) {
                (false, Some(format!("the hostname {} is already served by another site", host.clone().unwrap_or_default())))
            } else {
                (true, detection.notes.first().cloned())
            };
            if eligible {
                taken_names.extend(name.clone());
                taken_hosts.extend(host.clone().map(|h| h.to_ascii_lowercase()));
            }
            found.candidates.push(Candidate {
                folder: dir.display().to_string(),
                name,
                host,
                site: existing,
                detection,
                summary,
                eligible,
                reason,
            });
        }
        for site in &sites {
            if Path::new(&site.root).starts_with(folder) && site.upstream.is_none() && !Path::new(&site.root).is_dir() && !found.gone.contains(&site.name) {
                found.gone.push(site.name.clone());
            }
        }
    }
    Ok(found)
}

/// Make a site of each directory in `folders` that discovery found servable and that has none.
///
/// `only` limits it to those site names; a name that is not an eligible candidate is reported
/// as skipped, with the reason, instead of being created anyway.
pub fn sync(ctx: &Ctx, folders: &[PathBuf], only: Option<&[String]>) -> Result<SyncReport> {
    let found = discover(ctx, folders)?;
    let mut report = SyncReport { gone: found.gone.clone(), ..SyncReport::default() };
    for bad in &found.unreadable {
        report.skipped.push(bad.clone());
    }
    let wanted = |c: &Candidate| only.is_none_or(|names| c.name.as_ref().is_some_and(|n| names.contains(n)));

    if let Some(names) = only {
        for n in names {
            if !found.candidates.iter().any(|c| c.name.as_deref() == Some(n.as_str()) || c.site.as_deref() == Some(n.as_str())) {
                report.skipped.push(Skipped { folder: n.clone(), reason: "no project directory with that name was found in the parked folders".into() });
            }
        }
    }

    for candidate in found.candidates.iter().filter(|c| wanted(c)) {
        if let Some(site) = &candidate.site {
            report.existing.push(site.clone());
            continue;
        }
        if !candidate.eligible {
            report.skipped.push(Skipped { folder: candidate.folder.clone(), reason: candidate.reason.clone().unwrap_or_else(|| candidate.summary.clone()) });
            continue;
        }
        let name = candidate.name.clone().unwrap_or_default();
        let request = CreateRequest {
            name: name.clone(),
            host: None,
            root: Some(candidate.folder.clone()),
            tls: true,
            db: false,
            upstream: None,
            // What the directory serves is left to be read, so it stays true as the project changes.
            serving: None,
        };
        match create_with(ctx, &request, false, false) {
            Ok(created) => {
                report.warnings.extend(created.warnings.into_iter().filter(|w| !w.contains("does not resolve")));
                report.created.push((name, created.site.host));
            }
            Err(e) => report.skipped.push(Skipped { folder: candidate.folder.clone(), reason: e.to_string() }),
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

/// A content choice a person typed, as the thing a directory serves.
pub fn parse_serving(text: &str) -> Result<Content> {
    Content::parse(text).ok_or_else(|| {
        DevdError::invalid(format!(
            "`{text}` is not something a directory serves; choose `php` or `static` (a program on a port is a proxy site: `site.set_upstream`)"
        ))
    })
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
