//! `/learn <source>`: turn a repository, a page or a local document into a skill.
//!
//! The point is not to copy the source into a prompt. It is to make one short skill a person can
//! read, with the material it was learned from kept beside it as reference pages:
//!
//! ```text
//! <devd home>/skills/<name>.md
//! <devd home>/skills/<name>/references/...
//! ```
//!
//! The body is deterministic: the headings tell the reader what the material covers, and the
//! reference pages hold the text itself. Nothing is executed, and a built-in skill is never
//! replaced. A source may be a public https page, a local file or folder, or `owner/name` for a
//! repository whose README is fetched. The page is data, not instructions.

use std::path::{Path, PathBuf};

use devd_core::{DevdError, Paths, Result};

use crate::skills;

/// The most files one learn may keep, and the most text.
const MAX_FILES: usize = 30;
const MAX_BYTES: usize = 200_000;
const MAX_TOTAL_BYTES: usize = 2_000_000;

/// What was learned.
#[derive(Debug, Clone)]
pub struct Learned {
    /// The installed skill.
    pub skill: skills::Skill,
    /// Its reference pages, by path.
    pub files: Vec<String>,
    /// Where the material came from.
    pub sources: Vec<String>,
}

fn keep(text: &str) -> String {
    text.chars().take(MAX_BYTES).collect()
}

fn slug(text: &str) -> String {
    let out: String = text
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    out.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The first heading, or the first non-empty line, as a title.
fn title_of(text: &str, fallback: &str) -> String {
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let clean = line.trim_start_matches('#').trim();
        if !clean.is_empty() && clean.chars().count() <= 100 {
            return clean.to_string();
        }
    }
    fallback.to_string()
}

/// The headings a skill lists so a reader knows what the material covers.
fn headings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix('#') else { continue };
        let name = rest.trim_start_matches('#').trim();
        if name.is_empty() || name.chars().count() > 90 {
            continue;
        }
        if !out.iter().any(|h| h == name) {
            out.push(name.to_string());
        }
        if out.len() == 12 {
            break;
        }
    }
    out
}

/// A name from the source, when the person did not give one.
fn name_of(source: &str) -> String {
    let clean = source
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github:");
    let last = clean
        .split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .next_back()
        .unwrap_or("learned");
    let last = last.split(['?', '#']).next().unwrap_or(last);
    let last = last.trim_end_matches(".md").trim_end_matches(".html");
    let name = skills::normalise_name(last);
    if name.is_empty() {
        "learned".to_string()
    } else {
        name
    }
}

fn is_text(name: &str) -> bool {
    ["md", "mdx", "txt", "rst", "html", "htm", "adoc"]
        .iter()
        .any(|e| name.to_ascii_lowercase().ends_with(&format!(".{e}")))
}

fn read_local(path: &Path) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    let mut spent = 0usize;
    if path.is_file() {
        let text = std::fs::read_to_string(path)
            .map_err(|e| DevdError::invalid(format!("{} cannot be read as text: {e}", path.display())))?;
        out.push((
            path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "source".into()),
            keep(&text),
            path.display().to_string(),
        ));
        return Ok(out);
    }
    if !path.is_dir() {
        return Err(DevdError::not_found(format!("there is no file or folder at {}", path.display())));
    }
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(path)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| is_text(&p.to_string_lossy()))
        .filter(|p| !crate::fs::is_secret_file(p))
        .collect();
    files.sort();
    for file in files.into_iter().take(MAX_FILES) {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        spent += text.len();
        if spent > MAX_TOTAL_BYTES {
            break;
        }
        let rel = file
            .strip_prefix(path)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        out.push((rel, keep(&text), file.display().to_string()));
    }
    if out.is_empty() {
        return Err(DevdError::not_found(format!(
            "there is no Markdown, text or HTML to learn from in {}",
            path.display()
        )));
    }
    Ok(out)
}

fn read_source(source: &str) -> Result<Vec<(String, String, String)>> {
    if source.starts_with("https://") {
        let page = crate::docs::fetch(source, 0, MAX_BYTES)?;
        let text = page["text"].as_str().unwrap_or("").to_string();
        if text.trim().is_empty() {
            return Err(DevdError::not_found(format!("{source} returned no readable text")));
        }
        return Ok(vec![("source.md".to_string(), keep(&text), source.to_string())]);
    }
    let local = Path::new(source);
    if local.exists() {
        return read_local(local);
    }
    // `owner/name` names a repository; its README is the material.
    let parts: Vec<&str> = source.split('/').collect();
    if parts.len() == 2
        && parts.iter().all(|p| !p.is_empty() && !p.contains('\\') && !p.contains('.'))
    {
        let url = format!("https://raw.githubusercontent.com/{}/{}/HEAD/README.md", parts[0], parts[1]);
        if let Ok(page) = crate::docs::fetch(&url, 0, MAX_BYTES) {
            let text = page["text"].as_str().unwrap_or("").to_string();
            if !text.trim().is_empty() {
                return Ok(vec![("readme.md".to_string(), keep(&text), url)]);
            }
        }
        return Err(DevdError::not_found(format!(
            "could not read {source}'s README; give its https address or a local folder instead"
        )));
    }
    Err(DevdError::not_found(format!(
        "there is no https page, file, folder or owner/name repository at `{source}`"
    )))
}

/// Learn one source into a skill with reference pages.
pub fn learn(
    paths: &Paths,
    source: &str,
    name: Option<&str>,
    description: Option<&str>,
    kind: Option<&str>,
    roles: &[String],
) -> Result<Learned> {
    let source = source.trim();
    if source.is_empty() {
        return Err(DevdError::invalid("give a source: a public https page, a file or folder, or owner/name"));
    }
    let name = skills::normalise_name(name.unwrap_or(&name_of(source)));
    if name.is_empty() {
        return Err(DevdError::invalid("that source does not make a usable skill name; give `name`"));
    }
    if skills::builtin_description(&name).is_some() {
        return Err(DevdError::conflict(format!(
            "`{name}` is a built-in skill and cannot be replaced; choose another name"
        )));
    }
    if skills::get(paths, &name).is_ok() {
        return Err(DevdError::conflict(format!(
            "a skill named `{name}` already exists; remove it first or choose another name"
        )));
    }
    let kind = kind.unwrap_or("discover");
    if !skills::known_kind(kind) {
        return Err(DevdError::invalid(format!(
            "`{kind}` is not a stage devd knows; use one of {}",
            skills::KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
        )));
    }
    let material = read_source(source)?;
    let title = title_of(&material[0].1, &name);
    let description = description
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("What {title} teaches, with the material kept as references"));
    let topics = headings(&material[0].1);
    let mut body = format!(
        "# {title}\n\nLearned from {source}. Read the reference pages before using this skill; they hold the material, this page holds what to do with it.\n\n## When to use\n\n{description}\n"
    );
    if !topics.is_empty() {
        body.push_str("\n## What it covers\n\n");
        for topic in &topics {
            body.push_str(&format!("- {topic}\n"));
        }
    }
    body.push_str("\n## Procedure\n\n1. Read the reference page that covers the question; `skills.get` lists them as `files`, and `file` reads one.\n2. Follow the source's own words for anything exact: a command, an option, an example. Do not paraphrase a detail you can quote.\n3. Say which reference page an answer came from.\n\n## Verification\n\nThe answer names the reference it came from and does not claim something the source does not say.\n");
    skills::save(paths, &name, &description, kind, roles, &body)?;
    let mut pages: Vec<(String, String)> = Vec::new();
    for (i, (rel, text, origin)) in material.iter().enumerate() {
        let stem = slug(rel);
        let stem = if stem.is_empty() { format!("source-{i}") } else { stem };
        let mut file = format!("references/{stem}.md");
        if pages.iter().any(|(p, _)| p == &file) {
            file = format!("references/{stem}-{i}.md");
        }
        let page = format!("# {}\n\nSource: {origin}\n\n{text}\n", title_of(text, rel));
        pages.push((file, page));
    }
    let files = match skills::save_resources(paths, &name, &pages) {
        Ok(n) if n > 0 => skills::get(paths, &name)?.files,
        Ok(_) => Vec::new(),
        Err(e) => {
            // The skill itself is useful; the pages are not allowed to leave a half-made skill.
            let _ = skills::delete(paths, &name);
            return Err(e);
        }
    };
    let skill = skills::get(paths, &name)?;
    let sources = material.iter().map(|(_, _, origin)| origin.clone()).collect();
    Ok(Learned { skill, files, sources })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        (d, p)
    }

    #[test]
    fn a_local_document_becomes_a_skill_with_its_headings_and_a_reference_page() {
        let (d, p) = temp();
        let doc = d.path().join("deploy.md");
        std::fs::write(
            &doc,
            "# Deploying the shop\n\n## Before you start\n\nRun the checks.\n\n## Rollback\n\nRestore the last release.\n",
        )
        .unwrap();
        let learned = learn(&p, &doc.display().to_string(), None, None, None, &[]).unwrap();
        assert_eq!(learned.skill.name, "deploy");
        assert!(learned.skill.body.contains("Before you start"));
        assert_eq!(learned.files, vec!["references/deploy-md.md".to_string()]);
        let page = skills::read_resource(&p, "deploy", &learned.files[0]).unwrap();
        assert!(page.contains("Rollback"));
        assert!(page.contains("Source:"));
    }

    #[test]
    fn a_folder_learns_its_text_files_and_refuses_to_replace_a_skill() {
        let (d, p) = temp();
        let dir = d.path().join("handbook");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.md"), "# One\n\nThe first rule.\n").unwrap();
        std::fs::write(dir.join("two.md"), "# Two\n\nThe second rule.\n").unwrap();
        std::fs::write(dir.join("skip.bin"), "not text").unwrap();
        let learned = learn(&p, &dir.display().to_string(), Some("handbook"), Some("How the handbook works"), Some("house"), &[]).unwrap();
        assert_eq!(learned.files.len(), 2);
        let again = learn(&p, &dir.display().to_string(), Some("handbook"), None, None, &[]).unwrap_err();
        assert_eq!(again.code(), "conflict");
        assert!(learn(&p, &dir.display().to_string(), Some("work"), None, None, &[]).is_err(), "a built-in name is refused");
    }

    #[test]
    fn a_name_is_made_from_the_source_and_bad_stages_are_refused() {
        assert_eq!(name_of("https://example.com/guides/deploying/"), "deploying");
        assert_eq!(name_of("owner/name"), "name");
        assert_eq!(name_of(r"D:\docs\The Handbook.md"), "the-handbook");
        let (d, p) = temp();
        let doc = d.path().join("x.md");
        std::fs::write(&doc, "# X\n\nA body long enough to save.\n").unwrap();
        assert!(learn(&p, &doc.display().to_string(), None, None, Some("nope"), &[]).is_err());
        assert!(learn(&p, "", None, None, None, &[]).is_err());
    }
}
