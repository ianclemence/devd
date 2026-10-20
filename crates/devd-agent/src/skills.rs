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

use std::path::PathBuf;

use serde::Serialize;

use devd_core::paths::Paths;
use devd_core::{DevdError, Result};

use crate::skills_data::BUILTIN;

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
    if body.trim().len() < 20 || body.len() > 20_000 {
        return Err(DevdError::invalid("the body must be between 20 and 20,000 characters"));
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

/// The index an agent sees: names and one line each, grouped by kind.
pub fn index_for(paths: &Paths, role: Option<&str>) -> String {
    let skills = all(paths);
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
