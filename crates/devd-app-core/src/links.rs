//! Finding the things in prose that can be clicked: web addresses and files.
//!
//! An agent writes "I changed `src/main.rs:42` and updated C:\work\app\README.md,
//! see https://example.com/docs". A person wants to click each of those. This reads
//! text and says where they are. It does not touch the disk: whether a path is a
//! real file is for the caller to check, so a word like "and/or" or "v1.2" costs
//! one failed lookup and never becomes a dead link.
//!
//! What counts as a path is deliberately narrow in what it *accepts as worth
//! looking up*: it has a separator or a file extension that begins with a letter,
//! and uses only characters a path is made of. A `/skill` word or a sentence's
//! full stop does not qualify.

use std::ops::Range;

/// A file or folder named in text, with the place in it, when one was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRef {
    /// The path as written: absolute, `~/`, or relative to wherever the text is about.
    pub path: String,
    /// `:42` or `#L42`.
    pub line: Option<u32>,
    /// `:42:7`.
    pub col: Option<u32>,
}

/// What a click leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A web address.
    Url(String),
    /// A file or folder.
    Path(PathRef),
    /// A place in devd itself: `devd://runs/<id>`, `devd://projects/<dir>`, `devd://assistant/<id>`.
    /// Holds the route as the window names it (`/runs/<id>`).
    Devd(String),
}

/// One clickable thing in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Where it is, in bytes, without the punctuation around it.
    pub range: Range<usize>,
    /// What it leads to.
    pub target: Target,
}

const OPENERS: &[char] = &['(', '[', '{', '<', '"', '\'', '`', '*', '_'];
const CLOSERS: &[char] = &[')', ']', '}', '>', '"', '\'', '`', '*', '_', '.', ',', ';', ':', '!', '?'];

/// Every web address and path-like word in `text`, in order.
pub fn find(text: &str) -> Vec<Found> {
    let mut out = Vec::new();
    let mut word_start: Option<usize> = None;
    let bytes_end = text.len();
    let consider = |start: usize, end: usize, out: &mut Vec<Found>| {
        if let Some((range, target)) = word(text, start, end) {
            out.push(Found { range, target });
        }
    };
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = word_start.take() {
                consider(s, i, &mut out);
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(s) = word_start {
        consider(s, bytes_end, &mut out);
    }
    out
}

/// One whitespace-separated word, with the quotes and full stops around it taken off.
fn word(text: &str, start: usize, end: usize) -> Option<(Range<usize>, Target)> {
    let mut s = start;
    let mut e = end;
    while s < e && text[s..e].starts_with(OPENERS) {
        s += text[s..].chars().next().map_or(1, char::len_utf8);
    }
    // A closing bracket belongs to the address when its opening one does too.
    while e > s {
        let last = text[s..e].chars().next_back()?;
        if !CLOSERS.contains(&last) {
            break;
        }
        if matches!(last, ')' | ']' | '}') && balanced(&text[s..e]) {
            break;
        }
        e -= last.len_utf8();
    }
    if s >= e {
        return None;
    }
    let token = &text[s..e];
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (token.len() > "https://".len()).then(|| (s..e, Target::Url(token.to_string())));
    }
    if lower.starts_with("devd://") {
        // Only an address that names a place the window has. Anything else is plain text, never a guess.
        return crate::nav::Route::parse(token).map(|route| (s..e, Target::Devd(route.path())));
    }
    path_ref(token).map(|p| (s..e, Target::Path(p)))
}

fn balanced(token: &str) -> bool {
    let count = |c: char| token.chars().filter(|x| *x == c).count();
    count('(') >= count(')') && count('[') >= count(']') && count('{') >= count('}')
}

/// The path a single token names, when it looks like one worth looking up.
///
/// Also what a piece of inline code is checked with: `` `src/main.rs` `` is a path,
/// `` `cargo test` `` is not.
pub fn path_ref(token: &str) -> Option<PathRef> {
    if token.is_empty() || token.contains("://") || token.contains('@') || token.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '|' | '*' | '?' | '"' | '=' | ',' | ';')) {
        return None;
    }
    let (core, line, col) = position(token);
    if core.is_empty() || !looks_like_path(core) {
        return None;
    }
    Some(PathRef { path: core.to_string(), line, col })
}

/// The path with a trailing `:42`, `:42:7` or `#L42` taken off.
fn position(token: &str) -> (&str, Option<u32>, Option<u32>) {
    if let Some(at) = token.rfind("#L") {
        let tail = token[at + 2..].split('-').next().unwrap_or("");
        if let Ok(line) = tail.parse::<u32>() {
            return (&token[..at], Some(line), None);
        }
    }
    let digits = |s: &str| !s.is_empty() && s.len() <= 7 && s.bytes().all(|b| b.is_ascii_digit());
    // `path:42:7`, then `path:42`. A drive letter's colon is not a position.
    if let Some(c2) = token.rfind(':') {
        let (head, last) = (&token[..c2], &token[c2 + 1..]);
        if digits(last) {
            if let Some(c1) = head.rfind(':') {
                let (path, mid) = (&head[..c1], &head[c1 + 1..]);
                if digits(mid) && path.len() > 1 {
                    return (path, mid.parse().ok(), last.parse().ok());
                }
            }
            if head.len() > 1 {
                return (head, last.parse().ok(), None);
            }
        }
    }
    (token, None, None)
}

fn looks_like_path(p: &str) -> bool {
    let b = p.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    if drive {
        return true;
    }
    if p.starts_with("~/") || p.starts_with("~\\") {
        return p.len() > 2;
    }
    let has_sep = p.contains('/') || p.contains('\\');
    if let Some(rest) = p.strip_prefix('/') {
        // "/recall" is a skill, "//" a comment: an absolute path has somewhere to go.
        return !p.starts_with("//") && rest.contains('/') || has_extension(p);
    }
    if has_sep {
        // "and/or": needs something file-like or a real folder; the caller checks the disk.
        let last = p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
        return !last.is_empty() && p.chars().all(is_path_char) && !p.starts_with('-');
    }
    // A bare name needs an extension of two letters at least: "e.g." and "i.e." are not files.
    has_extension(p) && p.rsplit('.').next().map_or(0, str::len) >= 2 && p.chars().all(is_path_char)
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '\\' | '~' | '+' | '@' | '%' | '[' | ']' | '(' | ')' | '$' | '#')
}

/// A name ending in `.ext` where the extension starts with a letter: `main.rs`, `README.md`, not `v1.2`.
fn has_extension(p: &str) -> bool {
    let name = p.rsplit(['/', '\\']).next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && !ext.is_empty() && ext.len() <= 10 && ext.starts_with(|c: char| c.is_ascii_alphabetic()) && ext.chars().all(|c| c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// What a `devd://` route is, in a few words, for a chip: "Run 9f3a21", "Project luna".
///
/// The id is shortened, not hidden: it is what two things with the same name are told apart by.
pub fn chip_label(route_path: &str) -> String {
    let Some(route) = crate::nav::Route::parse(route_path) else { return route_path.to_string() };
    use crate::nav::Route;
    let short = |id: &str| id.chars().take(8).collect::<String>();
    match &route {
        Route::Runs(Some(id)) => format!("Run {}", short(id)),
        Route::Assistant(Some(id)) => format!("Conversation {}", short(id)),
        Route::Project(dir, tab) => format!("Project {} {}", dir.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(dir), tab.label().to_lowercase()),
        Route::Projects(Some(dir)) => format!("Project {}", dir.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(dir)),
        Route::Sites(Some(name)) => format!("Site {name}"),
        Route::Runs(None) => "Runs".to_string(),
        Route::Assistant(None) => "Assistant".to_string(),
        Route::Projects(None) => "Projects".to_string(),
        Route::Sites(None) => "Sites".to_string(),
        Route::Home => "Home".to_string(),
        Route::Inbox => "Inbox".to_string(),
        Route::Team => "Team".to_string(),
        Route::Server => "Server".to_string(),
        Route::Intelligence => "Intelligence".to_string(),
        Route::Activity => "Activity".to_string(),
        Route::Settings => "Settings".to_string(),
        Route::Page(_) => route.title().to_string(),
    }
}

/// Whether opening this file with the system could run it. Such a file is shown in its
/// folder instead: a reply can name a file, and a click must not start a program.
pub fn would_run(path: &str) -> bool {
    const RUNS: &[&str] = &[
        "exe", "bat", "cmd", "com", "scr", "msi", "msp", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "lnk", "url", "reg", "cpl", "jar", "sh", "bash", "zsh", "command", "app", "dmg", "pkg", "appimage", "desktop", "run", "bin", "py", "pl", "rb",
    ];
    let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    match name.rsplit_once('.') {
        Some((_, ext)) => RUNS.contains(&ext),
        // No extension on a Unix file is often an executable.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(text: &str) -> Vec<(String, Option<u32>)> {
        find(text)
            .into_iter()
            .filter_map(|f| match f.target {
                Target::Path(p) => Some((p.path, p.line)),
                Target::Url(_) | Target::Devd(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_reply_names_files_in_every_usual_way() {
        let text = "Edited `src/main.rs:42` and C:\\work\\app\\README.md, then /etc/hosts and ./run/x.toml (see docs/guide.md#L7).";
        assert_eq!(
            paths(text),
            vec![
                ("src/main.rs".to_string(), Some(42)),
                ("C:\\work\\app\\README.md".to_string(), None),
                ("/etc/hosts".to_string(), None),
                ("./run/x.toml".to_string(), None),
                ("docs/guide.md".to_string(), Some(7)),
            ]
        );
    }

    #[test]
    fn a_line_and_column_are_read_and_a_drive_colon_is_not_a_position() {
        let p = path_ref("crates/a/b.rs:12:5").unwrap();
        assert_eq!((p.path.as_str(), p.line, p.col), ("crates/a/b.rs", Some(12), Some(5)));
        let d = path_ref("D:\\x\\y.txt").unwrap();
        assert_eq!((d.path.as_str(), d.line), ("D:\\x\\y.txt", None));
    }

    #[test]
    fn urls_lose_the_punctuation_around_them_but_keep_their_own_brackets() {
        let found = find("See https://example.com/a_(b), or (https://devd.test/x). Done.");
        let urls: Vec<String> = found.into_iter().filter_map(|f| if let Target::Url(u) = f.target { Some(u) } else { None }).collect();
        assert_eq!(urls, ["https://example.com/a_(b)", "https://devd.test/x"]);
    }

    #[test]
    fn the_range_covers_the_address_and_nothing_else() {
        let text = "now (src/lib.rs).";
        let f = &find(text)[0];
        assert_eq!(&text[f.range.clone()], "src/lib.rs");
    }

    #[test]
    fn skills_sentences_versions_and_commands_are_not_paths() {
        for text in ["/recall please", "run cargo test now", "version 1.2 and 3.14", "e.g. this", "me@example.com", "a == b", "// a comment", "Done."] {
            assert!(paths(text).is_empty(), "{text}: {:?}", paths(text));
        }
    }

    #[test]
    fn a_bare_file_name_needs_a_real_extension() {
        assert_eq!(paths("open Cargo.toml and main.rs")[0].0, "Cargo.toml");
        assert_eq!(paths("open Cargo.toml and main.rs").len(), 2);
        assert!(paths("see v1.2").is_empty());
    }

    #[test]
    fn a_devd_address_is_a_link_only_when_it_names_a_place_the_window_has() {
        let text = "See devd://runs/abc123, then (devd://projects/D%3A%5Cwww%5Cluna) and devd://nowhere/x or devd://inbox.";
        let places: Vec<String> = find(text).into_iter().filter_map(|f| if let Target::Devd(p) = f.target { Some(p) } else { None }).collect();
        assert_eq!(places, ["/runs/abc123", "/projects/D%3A%5Cwww%5Cluna", "/inbox"]);
    }

    #[test]
    fn a_chip_says_what_it_leads_to() {
        assert_eq!(chip_label("/runs/9f3a21c4d5e6"), "Run 9f3a21c4");
        assert_eq!(chip_label("/projects/D%3A%5Cwww%5Cluna"), "Project luna");
        assert_eq!(chip_label("/sites/luna.test"), "Site luna.test");
        assert_eq!(chip_label("/inbox"), "Inbox");
        assert_eq!(chip_label("/assistant"), "Assistant");
    }

    #[test]
    fn a_click_never_starts_a_program() {
        for p in ["a/setup.exe", "run.BAT", "x.ps1", "deploy.sh", "tool", "link.lnk", "page.js"] {
            assert!(would_run(p), "{p}");
        }
        for p in ["notes.md", "src/main.rs", "photo.png", "data.json", "doc.pdf"] {
            assert!(!would_run(p), "{p}");
        }
    }

    #[test]
    fn unicode_around_a_path_does_not_split_a_letter() {
        let text = "café — src/é.rs, voilà";
        let f = &find(text)[0];
        assert_eq!(&text[f.range.clone()], "src/é.rs");
    }
}
