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

use devd_app_core::links::{PathRef, Target, find, path_ref, would_run};
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
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// The file a written path is, when there is one.
fn locate(p: &PathRef, root: Option<&Path>) -> Option<PathBuf> {
    let key = (root.map(Path::to_path_buf), p.path.clone());
    let cached = SEEN.with(|seen| {
        seen.borrow()
            .get(&key)
            .filter(|(at, _)| at.elapsed() < FRESH)
            .map(|(_, found)| found.clone())
    });
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
    if let Some(rest) = written
        .strip_prefix("~/")
        .or_else(|| written.strip_prefix("~\\"))
    {
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
            Target::Path(p) => {
                locate(&p, root).map(|path| (found.range, Link::File { path, line: p.line }))
            }
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

/// Where a written link (`[label](target)`) leads. An agent writes web addresses, devd addresses, and paths:
/// `src/main.rs`, `src/main.rs:42`, `./README.md`, `src/main.rs#L42`, `file:///D:/www/app/main.go`. A path
/// that does not exist is no link at all, so nothing on screen promises a click that goes nowhere.
pub fn target(written: &str, root: Option<&Path>) -> Option<Link> {
    let written = written.trim();
    let lower = written.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        return Some(Link::Url(written.to_string()));
    }
    if lower.starts_with("devd://") {
        return resolve_token(written, root);
    }
    let mut path = written.to_string();
    if lower.starts_with("file://") {
        path = path[7..].trim_start_matches('/').to_string();
        // `file:///home/x` is an absolute path on a machine without drive letters.
        if !path.as_bytes().get(1).is_some_and(|b| *b == b':') {
            path = format!("/{path}");
        }
        path = path.replace("%20", " ");
    }
    // `main.rs#L42` is the line, as a code host writes it.
    if let Some((file, anchor)) = path.split_once("#L")
        && let Ok(line) = anchor.split('-').next().unwrap_or_default().parse::<u32>()
    {
        path = format!("{file}:{line}");
    }
    let path = path.strip_prefix("./").map(str::to_string).unwrap_or(path);
    resolve_token(&path, root)
}

/// The project a file belongs to, among the projects devd knows, and the file's path within it. The deepest
/// project wins, so a file in a project inside another opens in the inner one.
pub fn within(projects: &[String], file: &Path) -> Option<(String, String)> {
    let norm = |p: &str| {
        let p = p.replace('\\', "/").trim_end_matches('/').to_string();
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    let file_text = file.to_string_lossy().replace('\\', "/");
    let wanted = norm(&file_text);
    projects
        .iter()
        .filter(|dir| {
            let d = norm(dir);
            wanted.len() > d.len() && wanted.starts_with(&d) && wanted.as_bytes()[d.len()] == b'/'
        })
        .max_by_key(|dir| dir.len())
        .map(|dir| {
            let cut = dir.trim_end_matches(['/', '\\']).len() + 1;
            (dir.clone(), file_text[cut..].to_string())
        })
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
        Link::File { path, line } => {
            if !path.exists() {
                return;
            }
            // A file in a project opens in devd, at the line, beside everything else about the project. A
            // folder, a program, or a file outside every project is handed to the system as before.
            if path.is_file() && !would_run(&path.to_string_lossy()) {
                let projects: Vec<String> = cx
                    .global::<Devd>()
                    .sync
                    .peek(&crate::state::q::projects())
                    .map(|snap| snap.list("").iter().filter_map(|p| p["path"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                if let Some((dir, rel)) = within(&projects, path) {
                    cx.global_mut::<Devd>().pending_file = Some((dir, rel, *line));
                    cx.refresh_windows();
                    return;
                }
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
        assert!(
            matches!(&links[0].1, Link::File { path, line: Some(3) } if path.ends_with("main.rs"))
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_devd_address_is_shown_as_a_chip_and_the_rest_of_the_text_is_untouched() {
        let text = "Started devd://runs/9f3a21c4d5e6 for devd://projects/D%3A%5Cwww%5Cluna, see https://x.dev now.";
        let (shown, links) = present(text, None);
        assert_eq!(
            shown,
            "Started Run 9f3a21c4 for Project luna, see https://x.dev now."
        );
        assert_eq!(links.len(), 3);
        assert_eq!(&shown[links[0].0.clone()], "Run 9f3a21c4");
        assert!(matches!(&links[0].1, Link::Route(r) if r == "/runs/9f3a21c4d5e6"));
        assert_eq!(&shown[links[2].0.clone()], "https://x.dev");
        // Text with nothing to link comes back as it was.
        assert_eq!(present("nothing here", None).0, "nothing here");
    }

    #[test]
    fn inline_code_can_be_a_devd_address() {
        assert!(
            matches!(resolve_token("devd://inbox", None), Some(Link::Route(r)) if r == "/inbox")
        );
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

    #[test]
    fn a_written_link_goes_where_its_target_points() {
        let root = scratch("target");
        let file = |link: Option<Link>| match link {
            Some(Link::File { path, line }) => Some((path.ends_with("main.rs"), line)),
            _ => None,
        };
        // The forms an agent writes a file in.
        assert_eq!(file(target("src/main.rs", Some(&root))), Some((true, None)));
        assert_eq!(file(target("src/main.rs:42", Some(&root))), Some((true, Some(42))));
        assert_eq!(file(target("./src/main.rs", Some(&root))), Some((true, None)));
        assert_eq!(file(target("src/main.rs#L7-L9", Some(&root))), Some((true, Some(7))));
        let absolute = format!("file:///{}", root.join("src").join("main.rs").display().to_string().replace('\\', "/"));
        assert_eq!(file(target(&absolute, None)), Some((true, None)), "{absolute}");
        // Web and devd addresses stay what they are.
        assert!(matches!(target("https://devd.test/x", None), Some(Link::Url(_))));
        assert!(matches!(target("mailto:a@b.test", None), Some(Link::Url(_))));
        assert!(matches!(target("devd://inbox", None), Some(Link::Route(r)) if r == "/inbox"));
        // A file that is not there is no link, rather than a dead one.
        assert!(target("src/missing.rs", Some(&root)).is_none());
        assert!(target("notes", Some(&root)).is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_file_opens_in_the_deepest_project_that_holds_it() {
        let projects = vec!["D:/www/shop".to_string(), "D:/www/shop/admin".to_string(), "D:/www/blog".to_string()];
        assert_eq!(
            within(&projects, Path::new(r"D:\www\shop\admin\src\main.rs")),
            Some(("D:/www/shop/admin".to_string(), "src/main.rs".to_string()))
        );
        assert_eq!(
            within(&projects, Path::new("D:/www/shop/README.md")),
            Some(("D:/www/shop".to_string(), "README.md".to_string()))
        );
        // A neighbour whose name starts the same is not inside.
        assert_eq!(within(&projects, Path::new("D:/www/shopfront/x.rs")), None);
        assert_eq!(within(&projects, Path::new("C:/elsewhere/x.rs")), None);
    }
}
