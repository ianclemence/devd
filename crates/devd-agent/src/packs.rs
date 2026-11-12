//! Packs: roles, skills and tool servers, installed as one thing.
//!
//! A person who builds a good set of specialists (a security team, a game studio) wants to
//! hand it to someone else in one step. A pack is a folder, or a GitHub repository, with a
//! `devd-pack.json` that names what is inside:
//!
//! ```text
//! devd-pack.json
//! roles/threat-modeller.md      a role file, as devd reads them
//! skills/abuse-cases.md         a skill file; a sibling folder abuse-cases/ holds its pages
//! ```
//!
//! What makes installing one safe to do:
//!
//! * **Nothing is executed.** Roles and skills are Markdown, read as text. A tool server in a
//!   pack is saved *disabled* and asking, with a note, so the person reads the command before
//!   anything can start it.
//! * **Nothing escapes the pack.** Every path is checked: absolute paths, `..`, drive letters,
//!   backslash tricks and symlinks that lead outside the folder are refused, so a pack cannot
//!   read or write anything but its own files.
//! * **Nothing is overwritten that is not its own.** A name that is built in, or that the
//!   person made, or that another pack installed, is a refusal that names the clash. Installing
//!   the same pack again (a newer version) replaces what that pack put there and nothing else.
//! * **All or nothing.** Everything is checked before anything is written, and if a write fails
//!   partway what was written is taken back.
//! * **Removal is exact.** A record keeps what each pack added, so removing it removes those
//!   files and servers and leaves everything else.
//!
//! Installing changes how every agent works, so it is a human-only capability.

use std::path::{Component, Path, PathBuf};

use devd_core::{DevdError, McpTrust, Paths, Result};
use serde::{Deserialize, Serialize};

/// The file that makes a folder a pack.
pub const MANIFEST: &str = "devd-pack.json";

/// Most of each kind in one pack, and the most text a pack may carry.
const MAX_ITEMS: usize = 50;
const MAX_FILE_BYTES: u64 = 200_000;
const MAX_TOTAL_BYTES: u64 = 4_000_000;

/// The version this build is, for a pack that says which devd it needs.
const THIS_DEVD: &str = env!("CARGO_PKG_VERSION");

/// What a pack says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Lowercase letters, digits and hyphens: what it is installed and removed by.
    pub id: String,
    /// What a person calls it.
    pub name: String,
    /// `major.minor.patch`.
    pub version: String,
    /// One sentence.
    #[serde(default)]
    pub description: String,
    /// What it needs.
    #[serde(default)]
    pub engines: Engines,
    /// Role files, relative to the pack.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Skill files, relative to the pack.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Tool servers, saved disabled.
    #[serde(default)]
    pub mcp: Vec<McpEntry>,
}

/// What a pack needs of the devd that installs it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Engines {
    /// A version requirement: `>=0.27.0`, `^0.27.0`, `0.27.7`.
    #[serde(default)]
    pub devd: Option<String>,
}

/// A tool server a pack suggests. It is saved disabled; the person turns it on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEntry {
    /// Its name.
    pub name: String,
    /// The program to start, for a local server.
    #[serde(default)]
    pub command: Option<String>,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// The address, for a remote server.
    #[serde(default)]
    pub url: Option<String>,
}

fn invalid(why: impl Into<String>) -> DevdError {
    DevdError::invalid(why.into())
}

/// Read and check a manifest.
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let m: Manifest = serde_json::from_str(text).map_err(|e| invalid(format!("{MANIFEST} is not a valid pack manifest: {e}")))?;
    if m.id.is_empty() || m.id.len() > 60 || !m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || m.id.starts_with('-') {
        return Err(invalid("the pack's `id` is lowercase letters, digits and hyphens"));
    }
    if m.name.trim().is_empty() || m.name.len() > 100 {
        return Err(invalid("the pack needs a `name`"));
    }
    if parse_version(&m.version).is_none() {
        return Err(invalid("the pack's `version` is major.minor.patch, for example 1.0.0"));
    }
    if let Some(req) = &m.engines.devd {
        if !satisfies(req, THIS_DEVD)? {
            return Err(DevdError::conflict(format!("this pack needs devd {req}; this is devd {THIS_DEVD}")));
        }
    }
    if m.roles.len() > MAX_ITEMS || m.skills.len() > MAX_ITEMS || m.mcp.len() > 10 {
        return Err(invalid("a pack holds at most 50 roles, 50 skills and 10 tool servers"));
    }
    for e in &m.mcp {
        if !devd_mcp::client::valid_name(&e.name) {
            return Err(invalid(format!("`{}` is not a valid tool server name", e.name)));
        }
        if e.command.as_deref().unwrap_or("").trim().is_empty() == e.url.as_deref().unwrap_or("").trim().is_empty() {
            return Err(invalid(format!("tool server `{}` needs a `command` or a `url`, and not both", e.name)));
        }
    }
    Ok(m)
}

/// `major.minor.patch` as numbers.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().split('.');
    let n = |p: Option<&str>| p?.parse::<u64>().ok();
    let out = (n(parts.next())?, n(parts.next())?, n(parts.next())?);
    parts.next().is_none().then_some(out)
}

/// Whether `version` meets a requirement: `>=x.y.z`, `>x.y.z`, `<=`, `<`, `^x.y.z` (same major; for 0.y, same minor), `=x.y.z` or `x.y.z`.
pub fn satisfies(req: &str, version: &str) -> Result<bool> {
    let req = req.trim();
    let have = parse_version(version).ok_or_else(|| invalid(format!("`{version}` is not a version")))?;
    let (op, rest) = ["<=", ">=", "<", ">", "^", "="].iter().find_map(|o| req.strip_prefix(o).map(|r| (*o, r))).unwrap_or(("=", req));
    let want = parse_version(rest).ok_or_else(|| invalid(format!("`{req}` is not a version requirement: write >=0.27.0, ^0.27.0 or 0.27.0")))?;
    Ok(match op {
        ">=" => have >= want,
        ">" => have > want,
        "<=" => have <= want,
        "<" => have < want,
        "^" => have >= want && if want.0 == 0 { have.0 == 0 && have.1 == want.1 } else { have.0 == want.0 },
        _ => have == want,
    })
}

/// A path inside the pack, checked: relative, forward-slashed, with nothing that climbs out.
pub fn safe_relative(rel: &str) -> Result<String> {
    let bad = |why: &str| Err(invalid(format!("`{rel}` {why}")));
    if rel.is_empty() || rel.len() > 200 {
        return bad("is not a usable path");
    }
    if rel.contains('\\') || rel.contains(':') || rel.contains('\0') || rel.starts_with('/') {
        return bad("must be a relative path with forward slashes");
    }
    let mut parts = Vec::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return bad("leaves the pack"),
        }
    }
    if parts.is_empty() || parts.iter().any(|p| p.starts_with('.')) {
        return bad("is not a usable path");
    }
    Ok(parts.join("/"))
}

/// A file in the pack, read only if it really is inside it.
fn read_inside(root: &Path, rel: &str, spent: &mut u64) -> Result<String> {
    let rel = safe_relative(rel)?;
    let canon_root = root.canonicalize().map_err(|e| invalid(format!("the pack folder cannot be read: {e}")))?;
    let file = root.join(&rel).canonicalize().map_err(|_| DevdError::not_found(format!("the pack names `{rel}`, which is not in it")))?;
    // A link that leads out of the folder is the way a pack would read someone else's files.
    if !file.starts_with(&canon_root) {
        return Err(invalid(format!("`{rel}` leads outside the pack")));
    }
    let meta = std::fs::metadata(&file).map_err(|e| invalid(format!("`{rel}`: {e}")))?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(invalid(format!("`{rel}` is not a file under {MAX_FILE_BYTES} bytes")));
    }
    *spent += meta.len();
    if *spent > MAX_TOTAL_BYTES {
        return Err(invalid("the pack carries more than 4 MB of text"));
    }
    std::fs::read_to_string(&file).map_err(|_| invalid(format!("`{rel}` is not UTF-8 text")))
}

/// A skill a pack brings.
#[derive(Debug, Clone)]
pub struct PackSkill {
    /// Its name.
    pub name: String,
    /// The whole file.
    pub raw: String,
    /// Reference pages from the folder beside it.
    pub pages: Vec<(String, String)>,
}

/// A pack, read and checked, ready to install.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Its manifest.
    pub manifest: Manifest,
    /// Role files: (id, text).
    pub roles: Vec<(String, String)>,
    /// Skills.
    pub skills: Vec<PackSkill>,
}

/// Read a pack folder and check everything in it. Nothing is written.
pub fn load(root: &Path) -> Result<Loaded> {
    let mut spent = 0u64;
    let manifest = parse_manifest(&read_inside(root, MANIFEST, &mut spent)?)?;
    let mut roles = Vec::new();
    for rel in &manifest.roles {
        let text = read_inside(root, rel, &mut spent)?;
        let id = crate::roles::declared_id(&text).ok_or_else(|| invalid(format!("`{rel}` has no `id:` in its front matter")))?;
        roles.push((id, text));
    }
    let mut skills = Vec::new();
    for rel in &manifest.skills {
        let raw = read_inside(root, rel, &mut spent)?;
        let (name, description, kind, _) = crate::skills::front_matter(&raw).ok_or_else(|| invalid(format!("`{rel}` has no front matter with a name and a description")))?;
        let name = crate::skills::normalise_name(&name);
        if name.is_empty() || description.trim().is_empty() || !crate::skills::known_kind(&kind) {
            return Err(invalid(format!("`{rel}` needs a name, a one-line description and a kind devd knows")));
        }
        // Pages sit in a folder named like the file: skills/x.md and skills/x/…md
        let mut pages = Vec::new();
        let folder = root.join(safe_relative(rel)?.trim_end_matches(".md"));
        if folder.is_dir() {
            for page in crate::skills::list_resources(&folder) {
                let text = read_inside(root, &format!("{}/{}", safe_relative(rel)?.trim_end_matches(".md"), page), &mut spent)?;
                pages.push((page, text));
            }
        }
        skills.push(PackSkill { name, raw, pages });
    }
    for (i, (id, _)) in roles.iter().enumerate() {
        if roles[..i].iter().any(|(other, _)| other == id) {
            return Err(invalid(format!("the pack has two roles called `{id}`")));
        }
    }
    for (i, s) in skills.iter().enumerate() {
        if skills[..i].iter().any(|o| o.name == s.name) {
            return Err(invalid(format!("the pack has two skills called `{}`", s.name)));
        }
    }
    Ok(Loaded { manifest, roles, skills })
}

/// What an installed pack put in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The pack's id.
    pub id: String,
    /// The version installed.
    pub version: String,
    /// Where it came from, for the record.
    pub source: String,
    /// When.
    pub installed_at: String,
    /// Role ids it added.
    pub roles: Vec<String>,
    /// Skill names it added.
    pub skills: Vec<String>,
    /// Tool server names it added (disabled).
    pub mcp: Vec<String>,
}

fn registry_file(paths: &Paths) -> PathBuf {
    paths.root().join("packs.json")
}

/// Every installed pack.
pub fn installed(paths: &Paths) -> Vec<Record> {
    std::fs::read_to_string(registry_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_records(paths: &Paths, records: &[Record]) -> Result<()> {
    let file = registry_file(paths);
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(records).map_err(|e| DevdError::internal(e.to_string()))?)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// What stands in the way of installing: names that are not this pack's to take.
pub fn clashes(paths: &Paths, store: &devd_core::Store, loaded: &Loaded) -> Vec<String> {
    let records = installed(paths);
    let mine = records.iter().find(|r| r.id == loaded.manifest.id);
    let owned_elsewhere = |name: &str, pick: fn(&Record) -> &Vec<String>| records.iter().find(|r| r.id != loaded.manifest.id && pick(r).iter().any(|n| n == name)).map(|r| r.id.clone());
    let mut out = Vec::new();
    let dir = paths.root().join("roles");
    for (id, _) in &loaded.roles {
        if crate::roles::is_built_in(id) {
            out.push(format!("the role `{id}` is one of devd's own"));
        } else if let Some(other) = owned_elsewhere(id, |r| &r.roles) {
            out.push(format!("the role `{id}` belongs to the pack `{other}`"));
        } else if dir.join(format!("{id}.md")).is_file() && !mine.is_some_and(|m| m.roles.contains(id)) {
            out.push(format!("you already have a role `{id}`"));
        }
    }
    for s in &loaded.skills {
        if crate::skills::builtin_description(&s.name).is_some() {
            out.push(format!("the skill `{}` is built in", s.name));
        } else if let Some(other) = owned_elsewhere(&s.name, |r| &r.skills) {
            out.push(format!("the skill `{}` belongs to the pack `{other}`", s.name));
        } else if crate::skills::user_dir(paths).join(format!("{}.md", s.name)).is_file() && !mine.is_some_and(|m| m.skills.contains(&s.name)) {
            out.push(format!("you already have a skill `{}`", s.name));
        }
    }
    for e in &loaded.manifest.mcp {
        let exists = store.mcp_server(&e.name).ok().flatten().is_some();
        if exists && !mine.is_some_and(|m| m.mcp.contains(&e.name)) {
            out.push(format!("you already have a tool server `{}`", e.name));
        }
    }
    out
}

/// Install a loaded pack: all of it or none of it.
pub fn install(paths: &Paths, store: &devd_core::Store, loaded: &Loaded, source: &str) -> Result<Record> {
    let clash = clashes(paths, store, loaded);
    if !clash.is_empty() {
        return Err(DevdError::conflict(format!("nothing was installed: {}", clash.join("; "))));
    }
    // A reinstall (a newer version) first takes back what this pack added before.
    if installed(paths).iter().any(|r| r.id == loaded.manifest.id) {
        remove_files(paths, store, &loaded.manifest.id)?;
    }
    let mut record = Record { id: loaded.manifest.id.clone(), version: loaded.manifest.version.clone(), source: source.to_string(), installed_at: devd_core::util::now(), roles: vec![], skills: vec![], mcp: vec![] };
    let result = (|| -> Result<()> {
        // Skills first: a role may carry them, and a role that names a skill that is not there is refused.
        for s in &loaded.skills {
            let skill = crate::skills::import_text(paths, &s.raw, &s.name)?;
            record.skills.push(skill.name.clone());
            if !s.pages.is_empty() {
                crate::skills::save_resources(paths, &skill.name, &s.pages)?;
            }
        }
        let dir = paths.root().join("roles");
        for (id, text) in &loaded.roles {
            crate::roles::save_custom_markdown(&dir, text).map_err(|why| invalid(format!("the role `{id}`: {why}")))?;
            record.roles.push(id.clone());
        }
        for e in &loaded.manifest.mcp {
            let now = devd_core::util::now();
            store.mcp_put(&devd_core::McpServer {
                id: devd_core::util::new_id(),
                name: e.name.clone(),
                transport: if e.url.is_some() { "http".into() } else { "stdio".into() },
                command: e.command.clone(),
                args: e.args.clone(),
                url: e.url.clone(),
                cwd: None,
                // Off, and asking: a pack can suggest a program, never start one.
                enabled: false,
                trust: McpTrust::Ask,
                timeout_secs: 60,
                tools: Vec::new(),
                last_error: Some(format!("added by the pack `{}`; read the command, then turn it on", loaded.manifest.id)),
                created_at: now.clone(),
                updated_at: now,
            })?;
            record.mcp.push(e.name.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        // Taken back, so a failure leaves nothing half-installed.
        undo(paths, store, &record);
        return Err(e);
    }
    let mut all: Vec<Record> = installed(paths).into_iter().filter(|r| r.id != record.id).collect();
    all.push(record.clone());
    save_records(paths, &all)?;
    Ok(record)
}

/// Take back what a record says was added. Best effort, and never more than the record names.
fn undo(paths: &Paths, store: &devd_core::Store, record: &Record) {
    let dir = paths.root().join("roles");
    for id in &record.roles {
        let _ = std::fs::remove_file(dir.join(format!("{id}.md")));
    }
    for name in &record.skills {
        let _ = crate::skills::delete(paths, name);
    }
    for name in &record.mcp {
        if let Ok(Some(s)) = store.mcp_server(name) {
            let _ = store.mcp_delete(&s.id);
        }
    }
    if !record.roles.is_empty() {
        crate::roles::reload();
    }
}

fn remove_files(paths: &Paths, store: &devd_core::Store, id: &str) -> Result<()> {
    let records = installed(paths);
    let Some(record) = records.iter().find(|r| r.id == id) else { return Err(DevdError::not_found(format!("no pack `{id}` is installed"))) };
    undo(paths, store, record);
    Ok(())
}

/// Remove an installed pack and exactly what it added.
pub fn remove(paths: &Paths, store: &devd_core::Store, id: &str) -> Result<Record> {
    let records = installed(paths);
    let record = records.iter().find(|r| r.id == id).cloned().ok_or_else(|| DevdError::not_found(format!("no pack `{id}` is installed")))?;
    undo(paths, store, &record);
    save_records(paths, &records.into_iter().filter(|r| r.id != id).collect::<Vec<_>>())?;
    Ok(record)
}

/// Find the pack folder in a source: a folder on this machine, or a GitHub repository (`owner/name`, or `owner/name/path`).
/// A fetched copy is unpacked in a scratch folder; the caller reads it and removes it.
pub fn fetch(paths: &Paths, source: &str) -> Result<(PathBuf, bool)> {
    let local = Path::new(source.trim());
    if local.is_dir() {
        return Ok((local.to_path_buf(), false));
    }
    let parts: Vec<&str> = source.trim().trim_start_matches("https://github.com/").trim_start_matches("github:").trim_matches('/').split('/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(invalid("give a folder on this machine, or a GitHub repository as owner/name (or owner/name/path)"));
    }
    let (owner, repo) = (parts[0], parts[1]);
    let url = format!("https://codeload.github.com/{owner}/{repo}/zip/HEAD");
    let file = devd_core::download::fetch(&devd_core::download::Artifact::new(url).filename(format!("{owner}-{repo}.zip")), &paths.cache(), None)?;
    let scratch = paths.cache().join(format!("pack-src-{owner}-{repo}"));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    devd_core::archive::extract(&file.path, &scratch, true)?;
    // A GitHub archive has one folder inside it, named for the repository and revision.
    let top = std::fs::read_dir(&scratch)?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.is_dir()).unwrap_or_else(|| scratch.clone());
    let root = parts[2..].iter().fold(top, |acc, p| acc.join(p));
    if !root.join(MANIFEST).is_file() {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(DevdError::not_found(format!("{owner}/{repo} has no {MANIFEST}: it is not a devd pack")));
    }
    Ok((root, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"id":"studio","name":"Studio","version":"1.2.0","engines":{"devd":">=0.1.0"},"roles":[],"skills":[]}"#;

    #[test]
    fn a_manifest_is_read_strictly() {
        assert_eq!(parse_manifest(GOOD).unwrap().id, "studio");
        for bad in [
            r#"{"id":"Studio","name":"x","version":"1.0.0"}"#,
            r#"{"id":"s","name":"x","version":"1.0"}"#,
            r#"{"id":"s","name":"x","version":"1.0.0","surprise":1}"#,
            r#"{"id":"s","name":"","version":"1.0.0"}"#,
            r#"{"id":"s","name":"x","version":"1.0.0","engines":{"devd":">=99.0.0"}}"#,
            r#"{"id":"s","name":"x","version":"1.0.0","mcp":[{"name":"t"}]}"#,
            r#"{"id":"s","name":"x","version":"1.0.0","mcp":[{"name":"t","command":"a","url":"http://x"}]}"#,
            "not json",
        ] {
            assert!(parse_manifest(bad).is_err(), "{bad}");
        }
        assert!(parse_manifest(r#"{"id":"s","name":"x","version":"1.0.0","mcp":[{"name":"t","command":"node","args":["s.js"]}]}"#).is_ok());
    }

    #[test]
    fn version_requirements_mean_what_they_say() {
        for (req, ok) in [(">=0.27.0", true), (">0.27.7", false), ("<=0.27.7", true), ("<0.27.7", false), ("^0.27.0", true), ("^0.26.0", false), ("^1.0.0", false), ("0.27.7", true), ("=0.27.6", false)] {
            assert_eq!(satisfies(req, "0.27.7").unwrap(), ok, "{req}");
        }
        assert!(satisfies("^1.2.0", "1.9.0").unwrap() && !satisfies("^1.2.0", "2.0.0").unwrap());
        assert!(satisfies("soon", "1.0.0").is_err());
    }

    #[test]
    fn a_path_that_climbs_out_of_the_pack_is_refused() {
        for bad in ["../x.md", "/etc/passwd", "a/../../x", "..", "C:/x.md", "a\\b.md", "", "a/./../b", ".hidden/x.md", "x\0y"] {
            assert!(safe_relative(bad).is_err(), "{bad}");
        }
        assert_eq!(safe_relative("roles/designer.md").unwrap(), "roles/designer.md");
        assert_eq!(safe_relative("./roles//x.md").unwrap(), "roles/x.md");
    }

    fn pack(dir: &Path, role: &str) {
        std::fs::create_dir_all(dir.join("roles")).unwrap();
        std::fs::create_dir_all(dir.join("skills/abuse-cases")).unwrap();
        std::fs::write(dir.join(MANIFEST), r#"{"id":"sec","name":"Security","version":"1.0.0","roles":["roles/modeller.md"],"skills":["skills/abuse-cases.md"],"mcp":[{"name":"scanner","command":"node","args":["scan.js"]}]}"#).unwrap();
        std::fs::write(dir.join("skills/abuse-cases.md"), "---\nname: abuse-cases\ndescription: Write the ways a feature can be misused\nkind: design\nroles: reviewer\n---\n\n# Abuse cases\n\nList who would misuse it, how, and what stops them. See the page on injection.\n").unwrap();
        std::fs::write(dir.join("skills/abuse-cases/injection.md"), "# Injection\nCheck every place text becomes a command.").unwrap();
        std::fs::write(dir.join("roles/modeller.md"), role).unwrap();
    }

    const ROLE: &str = "---\nid: threat-modeller\ntitle: Threat modeller\nteam: quality\nsummary: Finds how a design can be attacked\ntier: read\nwrites: false\nisolated: false\nextra:\nproves: read\nskills: abuse-cases\nhands_to: reviewer\n---\nYou read the design and the code around it, list who would attack it and how, and say what stops each, citing the files you read.\n";

    #[test]
    fn loading_reads_the_roles_the_skills_their_pages_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), ROLE);
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.roles[0].0, "threat-modeller");
        assert_eq!(loaded.skills[0].name, "abuse-cases");
        assert_eq!(loaded.skills[0].pages[0].0, "injection.md");
        assert_eq!(loaded.manifest.mcp[0].name, "scanner");
    }

    #[test]
    fn a_pack_cannot_name_a_file_outside_itself() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), ROLE);
        let mut m = std::fs::read_to_string(dir.path().join(MANIFEST)).unwrap();
        m = m.replace("roles/modeller.md", "../../secret.md");
        std::fs::write(dir.path().join(MANIFEST), m).unwrap();
        assert!(load(dir.path()).unwrap_err().to_string().contains("leaves the pack"));
    }

    #[test]
    fn a_missing_file_and_an_oversized_one_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), ROLE);
        std::fs::remove_file(dir.path().join("roles/modeller.md")).unwrap();
        assert!(load(dir.path()).unwrap_err().to_string().contains("not in it"));
        std::fs::write(dir.path().join("roles/modeller.md"), "x".repeat(MAX_FILE_BYTES as usize + 1)).unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn a_role_file_without_an_id_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), "---\ntitle: No id\n---\nbody");
        assert!(load(dir.path()).unwrap_err().to_string().contains("no `id:`"));
    }

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        (d, p)
    }

    fn small_pack(dir: &Path, skill_name: &str) {
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(dir.join(MANIFEST), format!(r#"{{"id":"mini","name":"Mini","version":"1.0.0","skills":["skills/{skill_name}.md"],"mcp":[{{"name":"scanner","command":"node","args":["scan.js"]}}]}}"#)).unwrap();
        std::fs::write(dir.join(format!("skills/{skill_name}.md")), format!("---
name: {skill_name}
description: Write the ways a feature can be misused
kind: design
roles: reviewer
---

# Abuse cases

List who would misuse it, how, and what stops them.
")).unwrap();
    }

    #[test]
    fn installing_adds_what_the_pack_names_saves_tool_servers_disabled_and_removal_is_exact() {
        let (_d, paths) = temp_paths();
        let store = devd_core::Store::open_in_memory().unwrap();
        let src = tempfile::tempdir().unwrap();
        small_pack(src.path(), "abuse-cases");
        // The person's own skill must survive the pack coming and going.
        crate::skills::save(&paths, "mine", "My own skill", "build", &[], "A body that is long enough to be saved.").unwrap();

        let loaded = load(src.path()).unwrap();
        let record = install(&paths, &store, &loaded, "test").unwrap();
        assert_eq!((record.skills.as_slice(), record.mcp.as_slice()), (&["abuse-cases".to_string()][..], &["scanner".to_string()][..]));
        assert!(crate::skills::get(&paths, "abuse-cases").is_ok());
        let server = store.mcp_server("scanner").unwrap().unwrap();
        assert!(!server.enabled && server.trust == McpTrust::Ask, "a pack can suggest a program, never start one");
        assert_eq!(installed(&paths).len(), 1);

        let gone = remove(&paths, &store, "mini").unwrap();
        assert_eq!(gone.id, "mini");
        assert!(crate::skills::get(&paths, "abuse-cases").is_err(), "what the pack added is gone");
        assert!(store.mcp_server("scanner").unwrap().is_none());
        assert!(crate::skills::get(&paths, "mine").is_ok(), "what the pack did not add is untouched");
        assert!(installed(&paths).is_empty());
        assert!(remove(&paths, &store, "mini").is_err());
    }

    #[test]
    fn a_name_that_is_not_the_packs_to_take_is_a_refusal_that_writes_nothing() {
        let (_d, paths) = temp_paths();
        let store = devd_core::Store::open_in_memory().unwrap();
        let src = tempfile::tempdir().unwrap();
        small_pack(src.path(), "work");
        let err = install(&paths, &store, &load(src.path()).unwrap(), "test").unwrap_err().to_string();
        assert!(err.contains("nothing was installed") && err.contains("`work` is built in"), "{err}");
        assert!(installed(&paths).is_empty() && store.mcp_server("scanner").unwrap().is_none());

        // A skill the person made is theirs too.
        crate::skills::save(&paths, "abuse-cases", "Mine first", "design", &[], "A body that is long enough to be saved.").unwrap();
        let other = tempfile::tempdir().unwrap();
        small_pack(other.path(), "abuse-cases");
        assert!(install(&paths, &store, &load(other.path()).unwrap(), "test").unwrap_err().to_string().contains("you already have a skill"));
    }

    #[test]
    fn a_newer_version_replaces_what_the_pack_added_and_leaves_the_rest() {
        let (_d, paths) = temp_paths();
        let store = devd_core::Store::open_in_memory().unwrap();
        let v1 = tempfile::tempdir().unwrap();
        small_pack(v1.path(), "abuse-cases");
        install(&paths, &store, &load(v1.path()).unwrap(), "v1").unwrap();
        let v2 = tempfile::tempdir().unwrap();
        small_pack(v2.path(), "abuse-cases");
        let m = std::fs::read_to_string(v2.path().join(MANIFEST)).unwrap().replace("1.0.0", "1.1.0");
        std::fs::write(v2.path().join(MANIFEST), m).unwrap();
        let r = install(&paths, &store, &load(v2.path()).unwrap(), "v2").unwrap();
        assert_eq!(r.version, "1.1.0");
        assert_eq!(installed(&paths).len(), 1, "one record per pack");
    }

    #[test]
    fn two_skills_with_one_name_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        pack(dir.path(), ROLE);
        std::fs::write(dir.path().join("skills/again.md"), "---\nname: abuse-cases\ndescription: A second one\nkind: design\nroles: reviewer\n---\n\n# Again\n\nA body long enough to count.\n").unwrap();
        let mut m = std::fs::read_to_string(dir.path().join(MANIFEST)).unwrap();
        m = m.replace("\"skills/abuse-cases.md\"", "\"skills/abuse-cases.md\",\"skills/again.md\"");
        std::fs::write(dir.path().join(MANIFEST), m).unwrap();
        assert!(load(dir.path()).unwrap_err().to_string().contains("two skills"));
    }
}
