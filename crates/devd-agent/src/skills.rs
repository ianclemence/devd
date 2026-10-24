//! Skills: short, focused instructions an agent loads when the work calls for
//! them, instead of carrying every practice in its prompt all the time.
//!
//! Three kinds:
//!
//! * `playbook` - a procedure for a type of work (fix a bug, build a feature);
//! * `technique` - a method used inside one (architect, interrogate, tdd);
//! * `principle` - one engineering rule, stated once.
//!
//! The built-in set ships inside devd. A person can add their own as Markdown
//! files in `<devd home>/skills/` (or through `skills.save`); those can add
//! skills but never replace a built-in one, so an agent cannot be made to lose
//! a safeguard by dropping a file with the same name.
//!
//! Only names and one-line descriptions go into an agent's prompt; the body is
//! read on demand with `skills.get` (progressive disclosure keeps the context
//! small).

use std::path::{Path, PathBuf};

use serde::Serialize;

use devd_core::paths::Paths;
use devd_core::{DevdError, Result};

use crate::skills_data::BUILTIN;

/// The one-line description of a built-in skill.
pub fn builtin_description(name: &str) -> Option<String> {
    BUILTIN
        .iter()
        .filter_map(|(_, raw)| parse(raw, "builtin"))
        .find(|s| s.name == name)
        .map(|s| s.description)
}

/// One skill.
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    /// Identifier.
    pub name: String,
    /// One line.
    pub description: String,
    /// `playbook`, `technique` or `principle`.
    pub kind: String,
    /// Roles the skill is meant for.
    pub roles: Vec<String>,
    /// `builtin` or `user`.
    pub source: String,
    /// The instructions.
    pub body: String,
}

fn parse(raw: &str, source: &str) -> Option<Skill> {
    let raw = raw.replace("\r\n", "\n");
    let rest = raw.strip_prefix("---\n")?;
    let (front, body) = rest.split_once("\n---\n")?;
    let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}:")).map(|v| v.trim().to_string()));
    let name = field("name")?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some(Skill {
        name,
        description: field("description")?,
        kind: field("kind").unwrap_or_else(|| "technique".into()),
        roles: field("roles").map(|r| r.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        source: source.into(),
        body: body.trim().to_string(),
    })
}

/// The directory for a person's own skills.
pub fn user_dir(paths: &Paths) -> PathBuf {
    paths.root().join("skills")
}

/// Every skill: built-ins first, then the person's.
pub fn all(paths: &Paths) -> Vec<Skill> {
    let mut out: Vec<Skill> = BUILTIN.iter().filter_map(|(_, raw)| parse(raw, "builtin")).collect();
    if let Ok(rd) = std::fs::read_dir(user_dir(paths)) {
        let mut files: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e == "md").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            if let Some(s) = std::fs::read_to_string(&f).ok().and_then(|t| parse(&t, "user")) {
                if !out.iter().any(|x| x.name == s.name) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// One skill by name.
pub fn get(paths: &Paths, name: &str) -> Result<Skill> {
    all(paths).into_iter().find(|s| s.name == name).ok_or_else(|| DevdError::not_found(format!("no skill `{name}`; skills.list shows what exists")))
}

/// Skills matching a query and/or aimed at a role.
pub fn find(paths: &Paths, query: &str, role: Option<&str>, kind: Option<&str>) -> Vec<Skill> {
    let toks: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    all(paths)
        .into_iter()
        .filter(|s| role.map(|r| s.roles.iter().any(|x| x == r)).unwrap_or(true))
        .filter(|s| kind.map(|k| s.kind == k).unwrap_or(true))
        .filter(|s| toks.iter().all(|t| s.name.contains(t.as_str()) || s.description.to_lowercase().contains(t.as_str())))
        .collect()
}

/// Save a person's own skill.
pub fn save(paths: &Paths, name: &str, description: &str, kind: &str, roles: &[String], body: &str) -> Result<()> {
    if name.is_empty() || name.len() > 60 || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(DevdError::invalid("a skill name is lowercase letters, digits and hyphens"));
    }
    if BUILTIN.iter().any(|(n, _)| *n == name) {
        return Err(DevdError::conflict(format!("`{name}` is built in and cannot be replaced; choose another name")));
    }
    if !matches!(kind, "playbook" | "technique" | "principle") {
        return Err(DevdError::invalid("kind must be playbook, technique or principle"));
    }
    if description.trim().is_empty() || description.contains('\n') {
        return Err(DevdError::invalid("give a one-line description"));
    }
    // A skill's body can be long: the published ones run to tens of
    // thousands of characters. The bound is there to stop a runaway, not to
    // keep skills short - only the name and the description go into a
    // prompt, and the body is read on demand.
    if body.trim().len() < 20 || body.len() > 200_000 {
        return Err(DevdError::invalid("the body must be between 20 and 200,000 characters"));
    }
    let dir = user_dir(paths);
    std::fs::create_dir_all(&dir)?;
    let text = format!("---\nname: {name}\ndescription: {}\nkind: {kind}\nroles: {}\n---\n\n{}\n", description.trim(), roles.join(","), body.trim());
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(format!("{name}.md")))?;
    Ok(())
}

/// Delete a person's own skill.
pub fn delete(paths: &Paths, name: &str) -> Result<()> {
    if BUILTIN.iter().any(|(n, _)| *n == name) {
        return Err(DevdError::conflict("built-in skills cannot be deleted"));
    }
    let f = user_dir(paths).join(format!("{name}.md"));
    if !f.is_file() {
        return Err(DevdError::not_found(format!("no user skill `{name}`")));
    }
    std::fs::remove_file(f)?;
    Ok(())
}

/// The name devd gives a skill: lowercase letters, digits and hyphens.
///
/// A skill written elsewhere may be called "Next.js Development" or
/// `code_review`. devd's names are one shape, so a name is normalised on the
/// way in rather than rejected.
pub fn normalise_name(raw: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let mut name: String = trimmed.chars().take(60).collect();
    while name.ends_with('-') {
        name.pop();
    }
    name
}

/// The front matter of a skill file, whatever wrote it.
///
/// The Agent Skills format - the one skills.sh publishes and the one every
/// agent on this machine reads - has a name and a description. devd's own
/// files add a kind and the roles the skill is meant for.
///
/// A description may be folded across lines, which is how the longer skills
/// write it: `description: >` and the words on the lines below. A folded
/// description is one sentence with the line breaks turned back into spaces.
pub fn front_matter(raw: &str) -> Option<(String, String, String, Vec<String>)> {
    let raw = raw.replace("\r\n", "\n");
    let rest = raw.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];

    // Every field, with a folded block gathered into one value.
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut lines = front.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim().to_string();
        let value = value.trim();
        if value == ">" || value == "|" || value == ">-" || value == "|-" || value == ">+"
            || value == "|+"
        {
            let mut block: Vec<String> = Vec::new();
            while let Some(next) = lines.peek() {
                if next.trim().is_empty() || !next.starts_with(' ') && !next.starts_with('\t') {
                    break;
                }
                block.push(next.trim().to_string());
                lines.next();
            }
            let joined = block.join(" ");
            let joined = joined.trim().to_string();
            fields.push((key, joined));
        } else {
            let value = value.trim_matches('"').trim_matches('\'').to_string();
            fields.push((key, value));
        }
    }
    let field = |k: &str| -> Option<String> {
        fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    };
    let name = field("name")?;
    let description = field("description")?;
    let kind = field("kind").unwrap_or_else(|| "technique".into());
    let roles = field("roles")
        .map(|r| r.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    Some((name, description, kind, roles))
}

/// Read a skill written in the Agent Skills format and save it as devd's own.
///
/// The body is kept exactly as it was written: a skill is somebody's
/// instructions, and rewriting them would change what the agent does.
pub fn import_text(paths: &Paths, raw: &str, fallback_name: &str) -> Result<Skill> {
    let (raw_name, description, kind, roles) = front_matter(raw)
        .ok_or_else(|| DevdError::invalid("the file has no front matter with a name and a description"))?;
    let name = normalise_name(if raw_name.trim().is_empty() { fallback_name } else { &raw_name });
    if name.is_empty() {
        return Err(DevdError::invalid("the skill has no usable name"));
    }
    if description.trim().is_empty() {
        return Err(DevdError::invalid("the skill has no description"));
    }
    let kind = if matches!(kind.as_str(), "playbook" | "technique" | "principle") {
        kind
    } else {
        "technique".into()
    };
    let body = raw
        .split_once("\n---")
        .map(|(_, rest)| rest.trim_start_matches('-').trim_start_matches('\n'))
        .unwrap_or(raw)
        .trim();
    if body.len() < 20 {
        return Err(DevdError::invalid("the skill has no instructions"));
    }
    save(paths, &name, description.lines().next().unwrap_or("").trim(), &kind, &roles, body)?;
    get(paths, &name)
}

/// The other agents on this machine, and where each keeps its skills.
pub const AGENT_SKILL_DIRS: &[(&str, &str)] = &[
    ("Claude Code", ".claude/skills"),
    ("Cursor", ".cursor/skills"),
    ("Codex", ".codex/skills"),
    ("OpenCode", ".config/opencode/skills"),
    ("OpenCode (legacy)", ".opencode/skills"),
    ("Windsurf", ".codeium/windsurf/skills"),
    ("Cline", ".cline/skills"),
    ("Gemini", ".gemini/skills"),
    ("Amp", ".amp/skills"),
    ("Zed", ".config/zed/skills"),
];

/// The skill folders other agents keep, and how many are in each.
pub fn agent_sources(home: &Path) -> Vec<(String, PathBuf, usize)> {
    let mut out = Vec::new();
    for (name, rel) in AGENT_SKILL_DIRS {
        let dir = home.join(rel);
        if !dir.is_dir() {
            continue;
        }
        let count = skill_files(&dir).len();
        out.push(((*name).to_string(), dir, count));
    }
    out
}

/// Every skill file under a directory: a `SKILL.md` in each folder, and any
/// Markdown file that carries devd's own front matter.
fn skill_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_md = path.extension().map(|e| e == "md").unwrap_or(false);
            if !is_md {
                continue;
            }
            let named = path.file_name().map(|n| n == "SKILL.md").unwrap_or(false);
            if named {
                out.push(path);
                continue;
            }
            if std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| front_matter(&t))
                .is_some()
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Import every skill another agent keeps into devd's own folder.
pub fn import_from(paths: &Paths, dir: &Path) -> Result<Vec<String>> {
    if !dir.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a directory", dir.display())));
    }
    let files = skill_files(dir);
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for f in files {
        let raw = std::fs::read_to_string(&f)?;
        let fallback = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "skill".into());
        match import_text(paths, &raw, &fallback) {
            Ok(s) => names.push(s.name),
            Err(_) => continue,
        }
    }
    Ok(names)
}

/// The index an agent sees: names and one line each, grouped by kind.
pub fn index_for(paths: &Paths, role: Option<&str>) -> String {    let skills = all(paths);
    let mut out = String::new();
    for (kind, title) in [("playbook", "Playbooks"), ("technique", "Techniques"), ("principle", "Principles")] {
        let items: Vec<&Skill> = skills.iter().filter(|s| s.kind == kind && role.map(|r| s.roles.iter().any(|x| x == r) || s.roles.is_empty()).unwrap_or(true)).collect();
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("{title}:\n"));
        for s in items {
            out.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
    }
    out
}

/// The handful of rules every agent follows, always in the prompt.
pub const CORE: &str = "\
Working principles (load `skills.get` with the name of any skill below for the full text):\n\
1. Understand before you change: read the real code and data first.\n\
2. Make the narrowest change that solves the problem; delete before you add.\n\
3. Work in small steps that each end in a checkable state.\n\
4. Fix root causes, not symptoms; never hide a failure with a retry, a guard or a weakened test.\n\
5. Prove it on the real thing before you say it is done, and say exactly what you ran and saw.\n\
6. Proceed on reversible work without asking; stop for what cannot be undone or is the person's call.\n\
7. Keep your context lean: search first, read the part you need, keep conclusions not logs.\n\
8. Tell the truth about what was and was not verified.";
