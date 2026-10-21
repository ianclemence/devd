//! Installing skills from a GitHub repository.
//!
//! The skills published on skills.sh live in GitHub repositories, in the
//! Agent Skills layout: a folder per skill, each with a `SKILL.md` that has
//! front matter and a body. devd reads the repository tree, fetches the
//! `SKILL.md` files it recognises, and saves them in devd's own folder.
//!
//! Nothing is executed and nothing is trusted: a skill is a Markdown file, it
//! is read as text, and the person sees where it came from before it is
//! installed.

use std::collections::BTreeMap;

use serde::Serialize;

use devd_core::paths::Paths;
use devd_core::{DevdError, Result};

use super::skills::Skill;

/// Where a repository's files are downloaded from.
///
/// The archive on codeload is a CDN with no rate limit. GitHub's API is not:
/// it allows sixty requests an hour from an address, which is nothing for a
/// shared one, and devd would start failing for reasons that have nothing to
/// do with the person asking.
const ARCHIVE: &str = "https://codeload.github.com";

/// A skill found in a repository.
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    /// devd's name for it.
    pub name: String,
    /// The description the author wrote.
    pub description: String,
    /// The kind, when the author said.
    pub kind: String,
    /// The roles the author aimed it at.
    pub roles: Vec<String>,
    /// Where it came from, for the record.
    pub source: String,
    /// The instructions.
    pub body: String,
}

/// Split `owner/repo` or `owner/repo/path` into its parts.
fn split_source(source: &str) -> Result<(String, String, Option<String>)> {
    let source = source.trim();
    let source = source
        .strip_prefix("https://github.com/")
        .or_else(|| source.strip_prefix("http://github.com/"))
        .or_else(|| source.strip_prefix("github:"))
        .unwrap_or(source)
        .trim_matches('/');
    let mut parts = source.split('/');
    let owner = parts.next().unwrap_or("").trim();
    let repo = parts.next().unwrap_or("").trim();
    if owner.is_empty() || repo.is_empty() {
        return Err(DevdError::invalid(
            "give a repository as owner/name, for example anthropics/skills",
        ));
    }
    let rest: Vec<&str> = parts.filter(|p| !p.is_empty()).collect();
    let path = if rest.is_empty() {
        None
    } else {
        Some(rest.join("/"))
    };
    Ok((owner.to_string(), repo.to_string(), path))
}

/// Every skill in a repository, with its instructions.
pub fn fetch(paths: &Paths, source: &str, revision: Option<&str>) -> Result<Vec<Found>> {
    let (owner, repo, path) = split_source(source)?;
    let rev = revision.unwrap_or("HEAD");
    let url = format!("{ARCHIVE}/{owner}/{repo}/zip/{rev}");
    let file = devd_core::download::fetch(
        &devd_core::download::Artifact::new(url).filename(format!("{owner}-{repo}.zip")),
        &paths.cache(),
        None,
    )?;

    // The archive is unpacked into a scratch directory and read there. It is
    // a copy of somebody's repository, not something to keep.
    let scratch = paths.cache().join(format!("skills-src-{owner}-{repo}"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    devd_core::archive::extract(&file.path, &scratch, true)?;

    let prefix = path.unwrap_or_default();
    let mut files: BTreeMap<String, std::path::PathBuf> = BTreeMap::new();
    let mut stack = vec![scratch.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.file_name().map(|n| n == "SKILL.md").unwrap_or(false) {
                let folder = p
                    .parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                files.insert(folder, p);
            }
        }
    }

    // The skills are read before the scratch directory goes: it is a copy of
    // somebody's repository, not something to keep.
    let mut out = Vec::new();
    for (folder, file) in files {
        if !prefix.is_empty() && folder != prefix && !folder.starts_with(&format!("{prefix}/")) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&file) else { continue };
        let Some((name, description, kind, roles, body)) = split_skill(&raw) else { continue };
        out.push(Found {
            name,
            description,
            kind,
            roles,
            source: format!("github:{owner}/{repo}/{folder}"),
            body,
        });
    }
    let _ = std::fs::remove_dir_all(&scratch);

    if out.is_empty() {
        return Err(DevdError::not_found(format!(
            "{owner}/{repo} has skill files, but none of them has the front matter a skill needs"
        )));
    }
    Ok(out)
}

/// A skill's name, description, kind, roles and body.
fn split_skill(raw: &str) -> Option<(String, String, String, Vec<String>, String)> {
    let (name, description, kind, roles) = crate::skills::front_matter(raw)?;
    let rest = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    let body = rest[end + 4..].trim_start_matches('-').trim_start_matches('\n').trim().to_string();
    Some((name, description, kind, roles, body))
}

/// Fetch skills from a repository and save them.
///
/// A skill that cannot be saved - no name, no description, no instructions -
/// is reported and skipped: one bad file in a repository must not stop the
/// rest from being installed.
pub fn install(paths: &Paths, source: &str, revision: Option<&str>, only: Option<&str>) -> Result<(Vec<Skill>, Vec<String>)> {
    let found = fetch(paths, source, revision)?;
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for f in found {
        if let Some(only) = only {
            if f.name != only && crate::skills::normalise_name(&f.name) != crate::skills::normalise_name(only) {
                continue;
            }
        }
        let name = crate::skills::normalise_name(&f.name);
        if name.is_empty() {
            skipped.push(format!("{}: the name is not usable", f.source));
            continue;
        }
        let description = f.description.lines().next().unwrap_or("").trim().to_string();
        if description.is_empty() {
            skipped.push(format!("{}: no description", f.source));
            continue;
        }
        match crate::skills::save(paths, &name, &description, &f.kind, &f.roles, &f.body) {
            Ok(()) => out.push(crate::skills::get(paths, &name)?),
            Err(e) => skipped.push(format!("{}: {e}", f.source)),
        }
    }
    if out.is_empty() {
        let detail = if skipped.is_empty() {
            String::new()
        } else {
            format!(" ({})", skipped.join("; "))
        };
        return Err(DevdError::not_found(format!("no skill in that repository could be installed{detail}")));
    }
    Ok((out, skipped))
}
