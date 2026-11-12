//! Making the files and addresses in text clickable.
//!
//! `devd_app_core::links` finds what looks like a file or an address. This decides
//! which of those are real, draws nothing itself, and opens them.
//!
//! A path becomes a link only if it exists. A reply that says "and/or" or "v1.2" or
//! names a file it has not made yet gets no dead link; one that names `src/main.rs`
//! gets one the moment the file is there. Relative paths are read against the
//! conversation's project, which is where an agent's relative paths come from.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use devd_app_core::links::{find, path_ref, would_run, PathRef, Target};
use gpui::App;

use crate::state::Devd;

/// Where a click goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A web address, opened in the browser.
    Url(String),
    /// A place in devd: a route the window can go to.
    Route(String),
    /// A file or folder that exists.
    File {
        /// Where it is.
        path: PathBuf,
        /// The line named, if any.
        line: Option<u32>,
    },
}

/// How long a lookup is believed. A file an agent has just written is found within this.
const FRESH: Duration = Duration::from_secs(2);

/// What was last found for a path in a project, and when: the file, or None when there was none.
type Seen = HashMap<(Option<PathBuf>, String), (Instant, Option<PathBuf>)>;

thread_local! {
    static SEEN: RefCell<Seen> = RefCell::new(HashMap::new());
}

/// The folder relative paths are read against: the open conversation's project.
pub fn root(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Devd>().and_then(|d| d.link_root.clone())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// The file a written path is, when there is one.
fn locate(p: &PathRef, root: Option<&Path>) -> Option<PathBuf> {
    let key = (root.map(Path::to_path_buf), p.path.clone());
    let cached = SEEN.with(|seen| seen.borrow().get(&key).filter(|(at, _)| at.elapsed() < FRESH).map(|(_, found)| found.clone()));
    if let Some(found) = cached {
        return found;
    }
    let found = candidate(p, root).filter(|path| path.exists());
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if seen.len() > 2000 {
            seen.clear();
        }
        seen.insert(key, (Instant::now(), found.clone()));
    });
    found
}

fn candidate(p: &PathRef, root: Option<&Path>) -> Option<PathBuf> {
    let written = p.path.as_str();
    if let Some(rest) = written.strip_prefix("~/").or_else(|| written.strip_prefix("~\\")) {
        return home().map(|h| h.join(rest));
    }
    let path = Path::new(written);
    if path.is_absolute() || written.as_bytes().get(1) == Some(&b':') {
        return Some(path.to_path_buf());
    }
    root.map(|r| r.join(written))
}

/// The link a written token leads to, when it is a path that exists or a place in devd.
pub fn resolve_token(token: &str, root: Option<&Path>) -> Option<Link> {
    if token.trim().to_ascii_lowercase().starts_with("devd://") {
        return devd_app_core::nav::Route::parse(token.trim()).map(|r| Link::Route(r.path()));
    }
    let p = path_ref(token.trim())?;
    locate(&p, root).map(|path| Link::File { path, line: p.line })
}

/// Every address and existing file in `text`, with where each is.
pub fn resolve(text: &str, root: Option<&Path>) -> Vec<(Range<usize>, Link)> {
    find(text)
        .into_iter()
        .filter_map(|found| match found.target {
            Target::Url(url) => Some((found.range, Link::Url(url))),
            Target::Devd(route) => Some((found.range, Link::Route(route))),
            Target::Path(p) => locate(&p, root).map(|path| (found.range, Link::File { path, line: p.line })),
        })
        .collect()
}

/// The text as it is shown, and the links in it. A `devd://` address is shown as a chip
/// ("Run 9f3a21c4", "Project luna"); everything else is the text as written.
pub fn present(text: &str, root: Option<&Path>) -> (String, Vec<(Range<usize>, Link)>) {
    let mut shown = String::with_capacity(text.len());
    let mut links = Vec::new();
    let mut at = 0;
    for (range, link) in resolve(text, root) {
        if range.start < at {
            continue;
        }
        shown.push_str(&text[at..range.start]);
        let start = shown.len();
        match &link {
            Link::Route(route) => shown.push_str(&devd_app_core::links::chip_label(route)),
            _ => shown.push_str(&text[range.clone()]),
        }
        links.push((start..shown.len(), link));
        at = range.end;
    }
    shown.push_str(&text[at..]);
    (shown, links)
}

/// Do what a click on a link does. A program is shown in its folder, never started.
pub fn open(link: &Link, cx: &mut App) {
    match link {
        Link::Route(route) => {
            // The window goes there the next time it draws: the same path an agent's `app.go` takes.
            cx.global_mut::<Devd>().pending_go = Some(route.clone());
            cx.refresh_windows();
        }
        Link::Url(url) => cx.open_url(url),
        Link::File { path, .. } => {
            if !path.exists() {
                return;
            }
            if path.is_dir() {
                cx.open_with_system(path);
            } else if would_run(&path.to_string_lossy()) {
                cx.reveal_path(path);
            } else {
                cx.open_with_system(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devd-links-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}").unwrap();
        dir
    }

    #[test]
    fn only_files_that_exist_become_links() {
        let root = scratch("exist");
        let text = "see src/main.rs:3, src/gone.rs and and/or";
        let links = resolve(text, Some(&root));
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(&text[links[0].0.clone()], "src/main.rs:3");
        assert!(matches!(&links[0].1, Link::File { path, line: Some(3) } if path.ends_with("main.rs")));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_devd_address_is_shown_as_a_chip_and_the_rest_of_the_text_is_untouched() {
        let text = "Started devd://runs/9f3a21c4d5e6 for devd://projects/D%3A%5Cwww%5Cluna, see https://x.dev now.";
        let (shown, links) = present(text, None);
        assert_eq!(shown, "Started Run 9f3a21c4 for Project luna, see https://x.dev now.");
        assert_eq!(links.len(), 3);
        assert_eq!(&shown[links[0].0.clone()], "Run 9f3a21c4");
        assert!(matches!(&links[0].1, Link::Route(r) if r == "/runs/9f3a21c4d5e6"));
        assert_eq!(&shown[links[2].0.clone()], "https://x.dev");
        // Text with nothing to link comes back as it was.
        assert_eq!(present("nothing here", None).0, "nothing here");
    }

    #[test]
    fn inline_code_can_be_a_devd_address() {
        assert!(matches!(resolve_token("devd://inbox", None), Some(Link::Route(r)) if r == "/inbox"));
        assert!(resolve_token("devd://nowhere", None).is_none());
    }

    #[test]
    fn a_relative_path_without_a_project_is_not_guessed_at() {
        assert!(resolve("src/main.rs", None).is_empty());
    }

    #[test]
    fn an_absolute_path_and_an_address_need_no_project() {
        let root = scratch("abs");
        let file = root.join("src").join("main.rs");
        let text = format!("{} and https://devd.test/x", file.display());
        let links = resolve(&text, None);
        assert_eq!(links.len(), 2, "{links:?}");
        assert!(matches!(links[1].1, Link::Url(_)));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn inline_code_is_a_link_only_when_it_is_a_file() {
        let root = scratch("code");
        assert!(resolve_token("src/main.rs", Some(&root)).is_some());
        assert!(resolve_token("cargo test", Some(&root)).is_none());
        std::fs::remove_dir_all(root).ok();
    }
}
