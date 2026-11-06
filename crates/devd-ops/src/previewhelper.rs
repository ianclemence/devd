//! `site.preview_helper`: what lets the Preview tab see inside the page it shows.
//!
//! The preview is the site, framed from another address, so the window cannot read the
//! page's errors or know which element a person means. A small script in the page can,
//! and it tells the window by message. This capability puts that script where the site
//! serves it (`devd-inspector.js` in the document root, so its address is stable) and says
//! whether the project already links to it.
//!
//! Linking it is one line in the project's layout. That is the project's file, so devd
//! reports the exact line and where it is missing; it does not edit the layout itself.
//! The script is inert outside the preview frame and on any address that is not local.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

/// The helper script, as shipped.
pub const SCRIPT: &str = include_str!("../assets/devd-inspector.js");
/// Its file name in the document root.
pub const FILE: &str = "devd-inspector.js";
/// The line a layout needs.
pub const TAG: &str = "<script src=\"/devd-inspector.js\" defer></script>";

const MARKER: &str = "/* devd-inspector:";
const VERSION: u32 = 1;

/// What the file in the document root is.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum State {
    Missing,
    Current,
    Older,
    /// A file with that name that devd did not write.
    Custom,
}

fn state_of(text: Option<&str>) -> State {
    match text {
        None => State::Missing,
        Some(t) => match t.lines().next().and_then(|l| l.strip_prefix(MARKER)).and_then(|r| r.trim().trim_end_matches("*/").trim().parse::<u32>().ok()) {
            Some(v) if v == VERSION => State::Current,
            Some(v) if v < VERSION => State::Older,
            _ => State::Custom,
        },
    }
}

fn label(s: State) -> &'static str {
    match s {
        State::Missing => "missing",
        State::Current => "current",
        State::Older => "older",
        State::Custom => "custom",
    }
}

/// Which of the project's template files already link to the helper.
///
/// Looks at the kinds of file a layout lives in, a few levels down, and gives up quietly
/// at a limit: this is a hint about where the line is, not a search of the whole project.
pub fn linked_from(project: &Path) -> Vec<PathBuf> {
    const KINDS: [&str; 8] = ["html", "htm", "php", "twig", "njk", "ejs", "vue", "svelte"];
    const SKIP: [&str; 6] = ["node_modules", "vendor", ".git", "storage", "target", "dist"];
    let mut found = Vec::new();
    let mut stack = vec![(project.to_path_buf(), 0u8)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 4000 {
                return found;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if depth < 5 && !SKIP.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push((path, depth + 1));
                }
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| KINDS.contains(&e.to_ascii_lowercase().as_str())) {
                let small = entry.metadata().map(|m| m.len() < 400_000).unwrap_or(false);
                if small && std::fs::read_to_string(&path).is_ok_and(|t| t.contains(FILE)) {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// Look, and write the script when asked.
pub fn apply(project: &Path, doc_root: &Path, write: bool) -> Result<Value> {
    let path = doc_root.join(FILE);
    let existing = std::fs::read_to_string(&path).ok();
    let before = state_of(existing.as_deref());
    let mut wrote = false;
    if write {
        match before {
            State::Custom => {
                return Err(DevdError::conflict(format!("{} exists and was not written by devd, so it is left alone", path.display())))
            }
            State::Current => {}
            State::Missing | State::Older => {
                std::fs::create_dir_all(doc_root)?;
                devd_core::util::atomic_write(&path, SCRIPT)?;
                wrote = true;
            }
        }
    }
    let after = state_of(std::fs::read_to_string(&path).ok().as_deref());
    let linked: Vec<String> = linked_from(project).iter().map(|p| p.strip_prefix(project).unwrap_or(p).display().to_string()).collect();
    Ok(json!({
        "file": path.display().to_string(),
        "state": label(after),
        "was": label(before),
        "wrote": wrote,
        "tag": TAG,
        "linked_from": linked,
        "linked": !linked.is_empty(),
        "next": if after == State::Missing || after == State::Older { "write the script (write=true)" } else if linked.is_empty() { "add the tag to the layout, inside <head> or before </body>" } else { "done: the Preview tab can now see errors and pick elements" },
    }))
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Args {
    /// The site to look at.
    pub site: String,
}

fn site_dirs(ctx: &devd_core::capability::Ctx, name: &str) -> Result<(PathBuf, PathBuf)> {
    ctx.store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| (s.project_dir(), PathBuf::from(s.document_root())))
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`")))
}

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<Args>("site.preview_helper", "Whether the Preview tab can see a site's errors and pick its elements")
        .description("Reports, and changes nothing: whether devd-inspector.js is in the site's document root, the one-line tag a layout needs to load it, and which templates (if any) already link to it. With the helper linked, the Preview tab shows the page's console errors and failed requests, and a person can point at an element and send it, with the page's errors, to an agent. The script is inert outside devd's preview frame and on any address that is not local. Write it with site.preview_helper_install.")
        .group("site")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("site.preview_helper", format!("look at the preview helper for {}", a.site))),
            |ctx, a| {
                let (project, root) = site_dirs(ctx, &a.site)?;
                Ok(Outcome::with_data("the preview helper was looked at", apply(&project, &root, false)?))
            },
        )?;

    reg.define::<Args>("site.preview_helper_install", "Put devd-inspector.js in a site so the Preview tab can see inside it")
        .description("Writes devd-inspector.js into the site's document root. That is the only file it writes: the one line that loads it belongs in the project's layout, which devd does not edit (site.preview_helper reports the line and whether any template already has it). A file with that name that devd did not write is never replaced, and one devd wrote is only replaced by a newer version of itself.")
        .group("site")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .idempotent()
        .handlers(
            |ctx, a| {
                let (_, root) = site_dirs(ctx, &a.site)?;
                Ok(Plan::new("site.preview_helper_install", format!("write the preview helper for {}", a.site)).step(format!("write {}", root.join(FILE).display())))
            },
            |ctx, a| {
                let (project, root) = site_dirs(ctx, &a.site)?;
                let v = apply(&project, &root, true)?;
                let msg = if v["wrote"].as_bool().unwrap_or(false) { "the preview helper is written" } else { "the preview helper was already current" };
                Ok(Outcome::with_data(msg, v))
            },
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("devd-helper-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_script_says_what_it_is_and_stays_inert_outside_a_frame() {
        assert!(SCRIPT.starts_with("/* devd-inspector:1 */"));
        assert!(SCRIPT.contains("window.parent === window"), "does nothing unless framed");
        assert!(SCRIPT.contains(".test|"), "does nothing off a local address");
    }

    #[test]
    fn it_writes_once_and_never_over_a_file_a_person_wrote() {
        let root = dir("write");
        let first = apply(&root, &root, true).unwrap();
        assert_eq!(first["wrote"], true);
        assert_eq!(first["state"], "current");
        assert_eq!(apply(&root, &root, true).unwrap()["wrote"], false);
        std::fs::write(root.join(FILE), "// mine\n").unwrap();
        assert!(apply(&root, &root, true).is_err());
        assert_eq!(std::fs::read_to_string(root.join(FILE)).unwrap(), "// mine\n");
    }

    #[test]
    fn looking_changes_nothing() {
        let root = dir("look");
        let v = apply(&root, &root, false).unwrap();
        assert_eq!(v["state"], "missing");
        assert!(!root.join(FILE).exists());
    }

    #[test]
    fn a_layout_that_links_to_it_is_found_and_vendor_is_not_searched() {
        let root = dir("linked");
        std::fs::create_dir_all(root.join("resources/views")).unwrap();
        std::fs::create_dir_all(root.join("vendor/x")).unwrap();
        std::fs::write(root.join("resources/views/app.blade.php"), format!("<head>{TAG}</head>")).unwrap();
        std::fs::write(root.join("vendor/x/page.php"), TAG).unwrap();
        let found = linked_from(&root);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("app.blade.php"));
        let none = dir("unlinked");
        std::fs::write(none.join("index.html"), "<html></html>").unwrap();
        assert!(linked_from(&none).is_empty());
    }
}
