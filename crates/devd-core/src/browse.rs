//! Directory browsing for the folder picker.
//!
//! Every interface that lets a person choose a folder - the desktop shell, the
//! web application, the CLI - needs the same three things: a list of the
//! directories inside a location, a way to walk up to a parent, and a way to
//! jump to the places people actually keep their work. This module provides
//! them once, in the core, so the picker cannot drift between surfaces.
//!
//! # Platform notes
//!
//! * **Windows** exposes several roots (one per drive letter), and the parent of
//!   a drive root is that root itself. UNC paths are treated as ordinary
//!   directories. Names are compared case-insensitively, as the filesystem is.
//! * **Linux** has a single root. The usual "home, desktop, documents,
//!   downloads" places are discovered from the XDG base directory
//!   specification and fall back to `$HOME` when the environment is minimal.
//!
//! Nothing here writes, moves or deletes anything. It only reads directory
//! entries, and it reports a directory it cannot read instead of failing the
//! whole listing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::error::{DevdError, Result};

/// The most entries one listing will return.
///
/// A picker that renders ten thousand rows is unusable, and a truncated list
/// that says so is honest. The flag travels with the result.
const MAX_ENTRIES: usize = 2000;

/// What an entry in a listing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A directory.
    Directory,
    /// A file.
    File,
    /// Something else, such as a socket or a device node.
    Other,
}

impl Kind {
    /// Whether this entry can be opened in a folder picker.
    pub fn is_directory(self) -> bool {
        matches!(self, Self::Directory)
    }
}

/// One entry in a directory listing.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// The file name, without any path.
    pub name: String,
    /// The full path as it should be handed back to a capability.
    pub path: String,
    /// What the entry is.
    pub kind: Kind,
    /// Whether the entry is a symbolic link.
    ///
    /// A link may point outside the directory that was listed, so a picker that
    /// lets a person follow one has to say so.
    pub symlink: bool,
    /// Whether the entry starts with a dot.
    pub hidden: bool,
    /// Size in bytes, for files.
    pub size: Option<u64>,
    /// Last modification time, as an ISO-8601 string.
    pub modified: Option<String>,
}

impl Entry {
    /// Whether a person may open this entry as a folder.
    pub fn selectable(&self) -> bool {
        self.kind.is_directory()
    }
}

/// One step of the path from a root to the directory being listed.
#[derive(Debug, Clone, Serialize)]
pub struct Crumb {
    /// The label to show.
    pub label: String,
    /// The path the label points at.
    pub path: String,
}

/// A place worth starting from.
#[derive(Debug, Clone, Serialize)]
pub struct Shortcut {
    /// The label to show.
    pub label: String,
    /// The path it points at.
    pub path: String,
    /// Whether the shortcut points at something that exists.
    pub present: bool,
}

/// The result of listing one directory.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    /// The directory that was listed.
    pub path: String,
    /// The parent directory, absent at a root.
    pub parent: Option<String>,
    /// The path from the root down to `path`.
    pub crumbs: Vec<Crumb>,
    /// Directories first, then files, each group sorted by name.
    pub entries: Vec<Entry>,
    /// The roots of the filesystem: drives on Windows, `/` elsewhere.
    pub roots: Vec<Shortcut>,
    /// The places people keep their work.
    pub shortcuts: Vec<Shortcut>,
    /// Whether the listing was cut short by the entry limit.
    pub truncated: bool,
    /// A note about something that could not be read, if anything.
    pub warning: Option<String>,
}

/// A location the picker can open.
#[derive(Debug, Clone, Serialize)]
pub struct Place {
    /// The label to show.
    pub label: String,
    /// The path it points at.
    pub path: String,
    /// Whether it exists and is a directory.
    pub present: bool,
}

/// The places a person keeps their work, in the order they should be offered.
///
/// A place that does not exist is still worth offering - it is where the
/// operating system would put it - but the same place is never offered twice:
/// Windows keeps some folders in OneDrive and some in the profile, and only
/// one of each exists in practice.
pub fn places() -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    let home = home_dir();

    if let Some(h) = &home {
        out.push(place("Home", h));
    }
    for (label, dir) in xdg_dirs() {
        let Some(d) = dir else { continue };
        // A home directory that also is the desktop is not worth listing twice.
        if home.as_deref() == Some(d.as_path()) {
            continue;
        }
        if let Some(existing) = out.iter_mut().find(|p| p.label == label) {
            // Keep the one that is really there; keep the first if both are.
            if !existing.present && d.is_dir() {
                existing.path = d.to_string_lossy().to_string();
                existing.present = true;
            }
            continue;
        }
        out.push(place(label, &d));
    }
    out
}

/// The roots of the filesystem.
pub fn roots() -> Vec<Shortcut> {
    let mut out = Vec::new();
    for path in platform_roots() {
        out.push(Shortcut {
            label: root_label(&path),
            path: path.to_string_lossy().to_string(),
            present: path.is_dir(),
        });
    }
    out
}

/// The user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// List the contents of a directory.
///
/// `show_hidden` includes dot files and Windows hidden files. The result never
/// fails because one child could not be read: that child is skipped and the
/// reason is reported in `warning`.
pub fn browse(path: &Path, show_hidden: bool) -> Result<Listing> {
    let canonical = resolve(path);

    let meta = fs::metadata(&canonical).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            DevdError::denied(format!(
                "{} cannot be read: permission denied",
                canonical.display()
            ))
        } else if e.kind() == std::io::ErrorKind::NotFound {
            DevdError::not_found(format!("{} does not exist", canonical.display()))
        } else {
            DevdError::Io(e)
        }
    })?;
    if !meta.is_dir() {
        return Err(DevdError::invalid(format!(
            "{} is a file, not a folder",
            canonical.display()
        )));
    }

    let mut directories: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();
    let mut warning: Option<String> = None;
    let mut truncated = false;

    let read = fs::read_dir(&canonical);
    let iter = match read {
        Ok(it) => it,
        Err(e) => {
            return Err(DevdError::denied(format!(
                "{} cannot be read: {e}",
                canonical.display()
            )))
        }
    };

    for item in iter {
        if directories.len() + files.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let item = match item {
            Ok(i) => i,
            Err(e) => {
                if warning.is_none() {
                    warning = Some(format!("some entries could not be read: {e}"));
                }
                continue;
            }
        };
        let name = item.file_name().to_string_lossy().to_string();
        let child = item.path();
        let symlink = fs::symlink_metadata(&child)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        // A symlink is reported as what it points at, because a person picks
        // the target, but it is flagged so the caller can say it is a link.
        let meta = fs::metadata(&child).ok();
        let kind = match &meta {
            Some(m) if m.is_dir() => Kind::Directory,
            Some(m) if m.is_file() => Kind::File,
            _ => Kind::Other,
        };
        let hidden = is_hidden(&child, &name);
        if hidden && !show_hidden {
            continue;
        }
        let entry = Entry {
            name: name.clone(),
            path: child.to_string_lossy().to_string(),
            kind,
            symlink,
            hidden,
            size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
            modified: meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.to_rfc3339()
                }),
        };
        match kind {
            Kind::Directory => directories.push(entry),
            _ => files.push(entry),
        }
    }

    directories.sort_by(|a, b| sort_key(&a.name).cmp(&sort_key(&b.name)));
    files.sort_by(|a, b| sort_key(&a.name).cmp(&sort_key(&b.name)));
    let mut entries = directories;
    entries.append(&mut files);

    Ok(Listing {
        path: canonical.to_string_lossy().to_string(),
        parent: parent_of(&canonical).map(|p| p.to_string_lossy().to_string()),
        crumbs: crumbs(&canonical),
        entries,
        roots: roots(),
        shortcuts: places()
            .into_iter()
            .map(|p| Shortcut {
                label: p.label,
                path: p.path,
                present: p.present,
            })
            .collect(),
        truncated,
        warning,
    })
}

/// The parent directory of a path, or `None` at a root.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }
    if parent == path {
        return None;
    }
    Some(parent.to_path_buf())
}

/// Turn a path into an absolute, symlink-free path.
///
/// `canonicalize` needs the path to exist, so a path that does not is returned
/// as given, made absolute against the current directory. The Windows
/// verbatim prefix (`\\?\`) is removed: it is an implementation detail of the
/// filesystem, and a person reading a path should not have to.
pub fn resolve(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return crate::util::strip_verbatim(&p);
    }
    if path.is_absolute() {
        return crate::util::strip_verbatim(path);
    }
    match std::env::current_dir() {
        Ok(cwd) => crate::util::strip_verbatim(&cwd.join(path)),
        Err(_) => path.to_path_buf(),
    }
}

/// The name of a path, with a sensible fallback for a root.
pub fn display_name(path: &Path) -> String {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    root_label(path)
}

/// The crumbs from the root down to a path.
pub fn crumbs(path: &Path) -> Vec<Crumb> {
    let mut out = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => parts.clear(),
            Component::Prefix(p) => {
                // The drive itself is a place you can go, and it must be an
                // absolute path so that it can be listed.
                let mut root = p.as_os_str().to_os_string();
                root.push(std::path::MAIN_SEPARATOR.to_string());
                out.push(Crumb {
                    label: p.as_os_str().to_string_lossy().to_string(),
                    path: root.to_string_lossy().to_string(),
                });
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir | Component::ParentDir => {}
        }
    }
    let mut acc = if path.is_absolute() {
        PathBuf::from(path.root_or_prefix())
    } else {
        PathBuf::new()
    };
    for part in parts {
        acc.push(&part);
        out.push(Crumb {
            label: part.clone(),
            path: acc.to_string_lossy().to_string(),
        });
    }
    out
}

trait RootOrPrefix {
    fn root_or_prefix(&self) -> String;
}

impl RootOrPrefix for Path {
    fn root_or_prefix(&self) -> String {
        let mut prefix = String::new();
        for component in self.components() {
            if let Component::Prefix(p) = component {
                prefix = p.as_os_str().to_string_lossy().to_string();
            } else if let Component::RootDir = component {
                prefix.push(std::path::MAIN_SEPARATOR);
            }
        }
        if prefix.is_empty() {
            std::path::MAIN_SEPARATOR.to_string()
        } else {
            prefix
        }
    }
}

/// The label for a root directory.
fn root_label(path: &Path) -> String {
    if let Some(prefix) = path.file_name().and_then(|n| n.to_str()) {
        return prefix.to_string();
    }
    let text = path.to_string_lossy().to_string();
    if text.is_empty() {
        "Root".into()
    } else {
        text
    }
}

/// The roots of the filesystem for this platform.
fn platform_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for letter in b'A'..=b'Z' {
            let candidate = PathBuf::from(format!("{}:\\", letter as char));
            if candidate.is_dir() {
                out.push(candidate);
            }
        }
        if out.is_empty() {
            out.push(PathBuf::from("C:\\"));
        }
        out
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

/// The well-known user directories, honouring the XDG specification.
fn xdg_dirs() -> Vec<(&'static str, Option<PathBuf>)> {
    #[cfg(target_os = "windows")]
    {
        let home = home_dir();
        let mut out = Vec::new();
        for (label, var) in [
            ("Desktop", "OneDrive\\Desktop"),
            ("Documents", "OneDrive\\Documents"),
            ("Downloads", "OneDrive\\Downloads"),
        ] {
            let p = home.as_ref().map(|h| h.join(var));
            out.push((label, p));
        }
        // The user profile usually has these directly too, which is what a
        // Windows machine without OneDrive has.
        for (label, name) in [("Desktop", "Desktop"), ("Documents", "Documents"), ("Downloads", "Downloads")] {
            let p = home.as_ref().map(|h| h.join(name));
            out.push((label, p));
        }
        out
    }
    #[cfg(not(windows))]
    {
        let home = home_dir();
        let xdg = |var: &str, fallback: &str| -> Option<PathBuf> {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return Some(PathBuf::from(v));
                }
            }
            home.as_ref().map(|h| h.join(fallback))
        };
        vec![
            ("Desktop", xdg("XDG_DESKTOP_DIR", "Desktop")),
            ("Documents", xdg("XDG_DOCUMENTS_DIR", "Documents")),
            ("Downloads", xdg("XDG_DOWNLOAD_DIR", "Downloads")),
        ]
    }
}

fn place(label: &str, path: &Path) -> Place {
    Place {
        label: label.to_string(),
        path: path.to_string_lossy().to_string(),
        present: path.is_dir(),
    }
}

/// Whether an entry is hidden from a person looking at a folder.
fn is_hidden(path: &Path, name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        // The hidden attribute is not the same as the name, and Windows
        // Explorer honours it.
        use std::os::windows::fs::MetadataExt;
        if let Ok(m) = fs::metadata(path) {
            return m.file_attributes() & 0x2 != 0;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    false
}

/// A sort key that keeps digits in names in numeric order.
///
/// A run of digits becomes a zero-padded number, so `item2` sorts before
/// `item10` the way a person expects rather than the way a dictionary would.
fn sort_key(name: &str) -> Vec<(u8, String)> {
    let mut parts: Vec<(u8, String)> = Vec::new();
    let mut digits = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            parts.push((0, pad_number(&digits)));
            digits.clear();
        }
        parts.push((1, ch.to_string()));
    }
    if !digits.is_empty() {
        parts.push((0, pad_number(&digits)));
    }
    parts
}

/// Zero-pad a number so that comparing the strings compares the numbers.
fn pad_number(digits: &str) -> String {
    const WIDTH: usize = 20;
    if digits.len() >= WIDTH {
        return digits.to_string();
    }
    let mut out = String::with_capacity(WIDTH);
    for _ in 0..(WIDTH - digits.len()) {
        out.push('0');
    }
    out.push_str(digits);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    #[test]
    fn lists_directories_before_files() {
        let dir = temp();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("z")).unwrap();
        fs::create_dir(dir.path().join("y")).unwrap();
        let listing = browse(dir.path(), false).expect("a listing");
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["y", "z", "a.txt", "b.txt"]);
    }

    #[test]
    fn hidden_entries_are_omitted_unless_asked_for() {
        let dir = temp();
        fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        let hidden = browse(dir.path(), false).expect("a listing");
        assert!(hidden.entries.is_empty());
        let shown = browse(dir.path(), true).expect("a listing");
        assert_eq!(shown.entries.len(), 1);
        assert!(shown.entries[0].hidden);
    }

    #[test]
    fn a_missing_directory_is_not_found() {
        let dir = temp();
        let missing = dir.path().join("nope");
        let err = browse(&missing, false).expect_err("an error");
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let dir = temp();
        let file = dir.path().join("f.txt");
        fs::write(&file, "x").unwrap();
        let err = browse(&file, false).expect_err("an error");
        assert_eq!(err.code(), "invalid_input");
    }

    #[test]
    fn the_parent_of_a_root_is_the_root() {
        let root = PathBuf::from("/");
        assert_eq!(parent_of(&root), None);
    }

    #[test]
    fn crumbs_walk_from_the_root() {
        let crumbs = crumbs(Path::new("/home/person/code/app"));
        let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["home", "person", "code", "app"]);
    }

    #[test]
    fn names_with_digits_sort_numerically() {
        let mut names = vec!["item10", "item2", "item1"];
        names.sort_by_key(|n| sort_key(n));
        assert_eq!(names, vec!["item1", "item2", "item10"]);
    }
}
