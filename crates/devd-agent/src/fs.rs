//! File tools for agents: list, read, search, write, edit, move, delete.
//!
//! These are what makes the built-in agent (and any MCP agent) able to change a
//! project, so the guard rails matter more than the features:
//!
//! * Every path must be absolute and pass the same directory authorisation as
//!   project creation (agents stay inside the projects root and imported
//!   projects; nobody touches system directories).
//! * devd's own home (database, keys) and `.git` internals are off limits; Git has
//!   its own capabilities with their own safety checks.
//! * Files that conventionally hold secrets are neither read nor written, so a
//!   secret cannot enter a model's context or be overwritten by one.
//! * Edits are exact-match and must be unambiguous, so a stale view of a file
//!   cannot silently corrupt it.

use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};
use walkdir::WalkDir;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};
use devd_stacks::authz;

const MAX_READ_BYTES: u64 = 2_000_000;
const MAX_WRITE_BYTES: usize = 2_000_000;
pub(crate) const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "vendor", ".venv", "venv", "__pycache__", ".next", ".nuxt", "dist", "build", ".gradle", ".idea"];

/// Whether a file name conventionally holds secrets.
pub fn is_secret_file(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name == ".env" || (name.starts_with(".env.") && !matches!(name.as_str(), ".env.example" | ".env.sample" | ".env.dist" | ".env.template")) {
        return true;
    }
    let exact = [".npmrc", ".pypirc", ".netrc", "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", "credentials", "credentials.json", "secrets.json", "secrets.yml", "secrets.yaml", "master.key"];
    if exact.contains(&name.as_str()) {
        return true;
    }
    [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"].iter().any(|e| name.ends_with(e))
}

/// Authorise a path for a tool and return it normalised.
pub fn authorize(ctx: &Ctx, path: &str, write: bool) -> Result<PathBuf> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(DevdError::invalid(format!("`{path}` is not an absolute path; tools take absolute paths (use project.detect to find the project directory)")));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(DevdError::invalid("paths must not contain `..`"));
    }
    let norm = authz::authorize_dir(ctx, p)?;
    let home = authz::normalise(ctx.paths.root());
    // A portable install can put the projects root (and therefore every site)
    // inside the devd home. That folder is the person's work, not devd's data,
    // so it is carved out of the home fence. Uploads are the one other folder
    // inside the home that an agent must be able to read: an attached file is
    // useless if the tool cannot open it.
    let roots = authz::roots(ctx);
    let in_projects = roots.iter().any(|r| norm.starts_with(r))
        || ctx
            .store
            .list_stack_projects()
            .map(|ps| ps.iter().any(|p| norm.starts_with(authz::normalise(Path::new(&p.path)))))
            .unwrap_or(false);
    let uploads = authz::normalise(&ctx.paths.root().join("uploads"));
    if norm.starts_with(&home) && !in_projects && !norm.starts_with(&uploads) {
        return Err(DevdError::denied("devd's own data directory (database, keys, logs) is not accessible to file tools"));
    }
    if norm.components().any(|c| c.as_os_str() == ".git") {
        return Err(DevdError::denied("`.git` is managed through the git capabilities, not file tools"));
    }
    if is_secret_file(&norm) {
        return Err(DevdError::denied(format!(
            "{} is a secrets file. {} Refer to its variables by name; a person edits secrets themselves.",
            norm.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            if write { "File tools never write secrets files." } else { "File tools never read secrets files." }
        )));
    }
    Ok(norm)
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// List a directory (optionally a few levels deep).
pub fn list(ctx: &Ctx, path: &str, depth: usize, limit: usize) -> Result<Value> {
    let root = authorize(ctx, path, false)?;
    if !root.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a directory", root.display())));
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    for e in WalkDir::new(&root)
        .min_depth(1)
        .max_depth(depth.clamp(1, 4))
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref())))
        .filter_map(|e| e.ok())
    {
        if entries.len() >= limit {
            truncated = true;
            break;
        }
        let rel = e.path().strip_prefix(&root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
        let secret = is_secret_file(e.path());
        entries.push(json!({
            "path": rel,
            "type": if e.file_type().is_dir() { "dir" } else { "file" },
            "bytes": if e.file_type().is_file() { e.metadata().ok().map(|m| m.len()) } else { None },
            "secret": secret,
        }));
    }
    Ok(json!({ "root": root.display().to_string(), "entries": entries, "truncated": truncated, "skipped_directories": SKIP_DIRS }))
}

/// Read a text file with line numbers.
pub fn read(ctx: &Ctx, path: &str, offset: usize, limit: usize) -> Result<Value> {
    let p = authorize(ctx, path, false)?;
    let meta = std::fs::metadata(&p).map_err(|_| DevdError::not_found(format!("{} does not exist", p.display())))?;
    if !meta.is_file() {
        return Err(DevdError::invalid(format!("{} is not a file; use fs.list", p.display())));
    }
    if meta.len() > MAX_READ_BYTES {
        return Err(DevdError::invalid(format!("{} is {} bytes; read a part of it with offset and limit after searching with fs.search", p.display(), meta.len())));
    }
    let bytes = std::fs::read(&p)?;
    if looks_binary(&bytes) {
        return Err(DevdError::invalid(format!("{} looks like a binary file", p.display())));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = (start + limit.clamp(1, 2000)).min(lines.len());
    let body: Vec<String> = lines[start..end].iter().enumerate().map(|(i, l)| format!("{:>6}\t{}", start + i + 1, l)).collect();
    Ok(json!({
        "path": p.display().to_string(),
        "total_lines": lines.len(),
        "from_line": start + 1,
        "to_line": end,
        "truncated": end < lines.len(),
        "content": body.join("\n"),
    }))
}

/// Search file contents with a regular expression.
pub fn search(ctx: &Ctx, path: &str, pattern: &str, glob: Option<&str>, limit: usize, context: usize) -> Result<Value> {
    let root = authorize(ctx, path, false)?;
    let re = Regex::new(pattern).map_err(|e| DevdError::invalid(format!("invalid regular expression: {e}")))?;
    let glob_re = glob.map(|g| {
        let esc = regex::escape(g).replace("\\*\\*", ".*").replace("\\*", "[^/]*").replace("\\?", ".");
        Regex::new(&format!("(^|/){esc}$")).map_err(|e| DevdError::invalid(format!("invalid glob: {e}")))
    });
    let glob_re = match glob_re {
        Some(r) => Some(r?),
        None => None,
    };
    let files: Vec<PathBuf> = if root.is_file() {
        vec![root.clone()]
    } else {
        WalkDir::new(&root)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref())))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect()
    };
    let mut hits = Vec::new();
    let mut scanned = 0;
    'outer: for f in files {
        if is_secret_file(&f) {
            continue;
        }
        let rel = f.strip_prefix(&root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        if let Some(g) = &glob_re {
            if !g.is_match(&rel) {
                continue;
            }
        }
        let Ok(meta) = std::fs::metadata(&f) else { continue };
        if meta.len() > 1_000_000 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else { continue };
        if looks_binary(&bytes) {
            continue;
        }
        scanned += 1;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if re.is_match(l) {
                let from = i.saturating_sub(context);
                let to = (i + context + 1).min(lines.len());
                hits.push(json!({
                    "file": rel,
                    "line": i + 1,
                    "text": devd_core::util::truncate(l, 300),
                    "context": if context > 0 { Some(lines[from..to].iter().map(|c| devd_core::util::truncate(c, 200)).collect::<Vec<_>>()) } else { None },
                }));
                if hits.len() >= limit {
                    break 'outer;
                }
            }
        }
    }
    Ok(json!({ "matches": hits, "files_scanned": scanned, "truncated": hits.len() >= limit }))
}

/// Create or replace a file.
pub fn write(ctx: &Ctx, path: &str, content: &str, create_dirs: bool) -> Result<Value> {
    let p = authorize(ctx, path, true)?;
    if content.len() > MAX_WRITE_BYTES {
        return Err(DevdError::invalid("content is larger than 2 MB"));
    }
    if p.is_dir() {
        return Err(DevdError::invalid(format!("{} is a directory", p.display())));
    }
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            if !create_dirs {
                return Err(DevdError::not_found(format!("{} does not exist; pass create_dirs to create it", parent.display())));
            }
            std::fs::create_dir_all(parent)?;
        }
    }
    let existed = p.exists();
    devd_core::util::atomic_write(&p, content)?;
    Ok(json!({ "path": p.display().to_string(), "created": !existed, "bytes": content.len(), "lines": content.lines().count() }))
}

/// Replace text in a file. The old text must match exactly once unless
/// `replace_all` is set.
pub fn edit(ctx: &Ctx, path: &str, old: &str, new: &str, replace_all: bool) -> Result<Value> {
    let p = authorize(ctx, path, true)?;
    if old.is_empty() {
        return Err(DevdError::invalid("`old` must not be empty; use fs.write to create or replace a whole file"));
    }
    if old == new {
        return Err(DevdError::invalid("`old` and `new` are identical"));
    }
    let text = std::fs::read_to_string(&p).map_err(|_| DevdError::not_found(format!("{} does not exist or is not text", p.display())))?;
    let count = text.matches(old).count();
    if count == 0 {
        return Err(DevdError::conflict("`old` was not found. The file may have changed; read it again with fs.read and copy the text exactly, including whitespace."));
    }
    if count > 1 && !replace_all {
        return Err(DevdError::conflict(format!("`old` matches {count} places. Include more surrounding lines to make it unique, or pass replace_all.")));
    }
    let updated = if replace_all { text.replace(old, new) } else { text.replacen(old, new, 1) };
    if updated.len() > MAX_WRITE_BYTES {
        return Err(DevdError::invalid("the result would be larger than 2 MB"));
    }
    devd_core::util::atomic_write(&p, &updated)?;
    Ok(json!({ "path": p.display().to_string(), "replacements": if replace_all { count } else { 1 } }))
}

/// Move or rename a file or directory. Never overwrites.
pub fn mv(ctx: &Ctx, from: &str, to: &str) -> Result<Value> {
    let a = authorize(ctx, from, true)?;
    let b = authorize(ctx, to, true)?;
    if !a.exists() {
        return Err(DevdError::not_found(format!("{} does not exist", a.display())));
    }
    if b.exists() {
        return Err(DevdError::conflict(format!("{} already exists; move never overwrites", b.display())));
    }
    if let Some(parent) = b.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&a, &b)?;
    Ok(json!({ "from": a.display().to_string(), "to": b.display().to_string() }))
}

/// Delete a file or an empty directory.
pub fn delete(ctx: &Ctx, path: &str, recursive: bool) -> Result<Value> {
    let p = authorize(ctx, path, true)?;
    if !p.exists() {
        return Err(DevdError::not_found(format!("{} does not exist", p.display())));
    }
    let roots = authz::roots(ctx);
    if roots.contains(&p) {
        return Err(DevdError::denied("refusing to delete a projects root"));
    }
    if p.is_dir() {
        if recursive {
            let n = WalkDir::new(&p).into_iter().filter_map(|e| e.ok()).count();
            std::fs::remove_dir_all(&p)?;
            return Ok(json!({ "deleted": p.display().to_string(), "entries": n }));
        }
        std::fs::remove_dir(&p).map_err(|_| DevdError::conflict(format!("{} is not empty; pass recursive to delete it with its contents", p.display())))?;
    } else {
        std::fs::remove_file(&p)?;
    }
    Ok(json!({ "deleted": p.display().to_string() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::capability::{Actor, Surface};
    use devd_core::config::Settings;
    use devd_core::events::EventBus;
    use devd_core::paths::Paths;
    use devd_core::state::Store;
    use std::sync::Arc;

    fn ctx(root: &Path, actor: Actor) -> Ctx {
        let paths = Paths::rooted(root.join("home"));
        paths.ensure().unwrap();
        let mut s = Settings::default();
        s.www_root = Some(root.join("www"));
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        Ctx { store: Arc::new(store), paths: Arc::new(paths), settings: Arc::new(s), events: EventBus::new(), actor, dry_run: false }
    }

    #[test]
    fn secrets_and_internals_are_off_limits() {
        for f in [".env", ".env.local", "a/.env.production", "id_rsa", "server.pem", ".npmrc", "config/master.key"] {
            assert!(is_secret_file(Path::new(f)), "{f}");
        }
        for f in [".env.example", "README.md", "src/keys.rs", "environment.ts"] {
            assert!(!is_secret_file(Path::new(f)), "{f}");
        }
    }

    #[test]
    fn a_site_inside_the_devd_home_is_project_work_not_internal_data() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let www = home.join("www");
        let project = www.join("luna");
        std::fs::create_dir_all(&project).unwrap();
        let file = project.join("report.json");
        std::fs::write(&file, "{}").unwrap();
        let uploads = home.join("uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        let upload = uploads.join("screenshot.png");
        std::fs::write(&upload, "png").unwrap();

        let paths = Paths::rooted(&home);
        paths.ensure().unwrap();
        let mut s = Settings::default();
        s.www_root = Some(www.clone());
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        let c = Ctx {
            store: Arc::new(store),
            paths: Arc::new(paths),
            settings: Arc::new(s),
            events: EventBus::new(),
            actor: Actor::agent("t"),
            dry_run: false,
        };

        assert!(authorize(&c, &file.display().to_string(), false).is_ok(), "a site under www is readable");
        assert!(authorize(&c, &upload.display().to_string(), false).is_ok(), "an upload is readable");
        assert_eq!(
            authorize(&c, &home.join("state.db").display().to_string(), false).unwrap_err().code(),
            "permission_denied",
            "devd's own state stays fenced"
        );
    }

    #[test]
    fn edit_requires_an_unambiguous_exact_match() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), Actor::agent("t"));
        let proj = d.path().join("www").join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let f = proj.join("a.txt");
        std::fs::write(&f, "one\ntwo\ntwo\n").unwrap();
        let fs = f.display().to_string();
        assert_eq!(edit(&c, &fs, "two", "2", false).unwrap_err().code(), "conflict");
        assert_eq!(edit(&c, &fs, "nine", "9", false).unwrap_err().code(), "conflict");
        edit(&c, &fs, "one", "1", false).unwrap();
        edit(&c, &fs, "two", "2", true).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "1\n2\n2\n");
    }

    #[test]
    fn agents_cannot_escape_read_secrets_or_touch_devd_data() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), Actor::agent("t"));
        let proj = d.path().join("www").join("p");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(".env"), "SECRET=1").unwrap();
        std::fs::write(proj.join("ok.txt"), "hello\nworld\n").unwrap();
        assert_eq!(read(&c, &proj.join(".env").display().to_string(), 1, 10).unwrap_err().code(), "permission_denied");
        assert_eq!(write(&c, &proj.join(".env").display().to_string(), "x", false).unwrap_err().code(), "permission_denied");
        assert!(read(&c, "/etc/passwd", 1, 10).is_err());
        assert!(read(&c, &d.path().join("home").join("state.db").display().to_string(), 1, 10).is_err());
        assert!(read(&c, &format!("{}/../../etc/passwd", proj.display()), 1, 10).is_err());
        assert!(read(&c, "relative.txt", 1, 10).is_err());
        let r = read(&c, &proj.join("ok.txt").display().to_string(), 2, 10).unwrap();
        assert_eq!(r["content"], "     2\tworld");
        // Search skips the secrets file even when it matches.
        let s = search(&c, &proj.display().to_string(), "SECRET|hello", None, 10, 0).unwrap();
        assert_eq!(s["matches"].as_array().unwrap().len(), 1);
        let l = list(&c, &proj.display().to_string(), 1, 50).unwrap();
        assert!(l["entries"].as_array().unwrap().iter().any(|e| e["path"] == ".env" && e["secret"] == true));
    }

    #[test]
    fn move_never_overwrites_and_delete_protects_roots() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path(), Actor::human(Surface::Cli));
        let proj = d.path().join("www").join("p");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("a"), "a").unwrap();
        std::fs::write(proj.join("b"), "b").unwrap();
        let p = |n: &str| proj.join(n).display().to_string();
        assert_eq!(mv(&c, &p("a"), &p("b")).unwrap_err().code(), "conflict");
        mv(&c, &p("a"), &p("c")).unwrap();
        delete(&c, &p("c"), false).unwrap();
        assert_eq!(delete(&c, &d.path().join("www").display().to_string(), true).unwrap_err().code(), "permission_denied");
    }
}
