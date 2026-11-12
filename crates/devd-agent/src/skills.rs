//! Skills: short, focused instructions an agent loads when the work calls for
//! them, instead of carrying every practice in its prompt all the time.
//!
//! Every skill carries a `kind`, which is the stage of work it belongs to, so an
//! agent can find the right one without reading all of them:
//!
//! * `discover` - understanding the request and the system before acting;
//! * `design` - deciding a shape;
//! * `build` - writing the change, with the methods used while doing it;
//! * `check` - proving a result and keeping it honest;
//! * `review` - reading someone else's change;
//! * `ship` - landing it;
//! * `words` - writing prose, reports and documentation;
//! * `house` - how work runs here: autonomy, ownership and staying restartable.
//!
//! A person can also write a skill of their own with any kind they like.
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

/// Every kind of skill, in the order an agent meets it, with the heading the
/// prompt index uses. The order is the order of the work itself.
pub const KINDS: &[(&str, &str)] = &[
    ("discover", "Discover"),
    ("design", "Design"),
    ("build", "Build"),
    ("check", "Check"),
    ("review", "Review"),
    ("ship", "Ship"),
    ("words", "Words"),
    ("house", "House rules"),
];

/// Whether `kind` is one devd names. A person's own skill may still use its own
/// word for it; the built-in set does not.
pub fn known_kind(kind: &str) -> bool {
    KINDS.iter().any(|(k, _)| *k == kind)
}

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
    /// The stage of work it belongs to; see [`KINDS`].
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
        kind: field("kind").unwrap_or_else(|| KINDS[2].0.to_string()),
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

/// A skill found on this machine, without its body until it is read.
#[derive(Debug, Clone, Serialize)]
pub struct Discovered {
    /// Name to pass to `skills.get` or `skills.read_external`.
    pub name: String,
    /// One line.
    pub description: String,
    /// The stage of work.
    pub kind: String,
    /// Roles it is for; empty means all.
    pub roles: Vec<String>,
    /// `builtin`, `user`, or the other agent's name.
    pub source: String,
    /// The file, for a skill that is not installed in devd.
    pub path: Option<String>,
    /// Whether `skills.get` can load it.
    pub installed: bool,
}

fn matches_skill(name: &str, description: &str, toks: &[String], roles: &[String], role: Option<&str>, kind: &str, want_kind: Option<&str>) -> bool {
    if let Some(want) = want_kind {
        if kind != want {
            return false;
        }
    }
    if let Some(want) = role {
        if !roles.is_empty() && !roles.iter().any(|r| r == want) {
            return false;
        }
    }
    toks.iter().all(|t| name.to_lowercase().contains(t) || description.to_lowercase().contains(t))
}

/// Search devd's skills and the skill folders of other agents on this machine.
///
/// A finding is a name, a description and where it lives, never the body: the
/// agent reads the one it chooses with `skills.get` or `skills.read_external`.
pub fn discover(paths: &Paths, home: &Path, query: &str, role: Option<&str>, kind: Option<&str>) -> Vec<Discovered> {
    let toks: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    let mut out: Vec<Discovered> = find(paths, query, role, kind)
        .into_iter()
        .map(|s| Discovered {
            name: s.name,
            description: s.description,
            kind: s.kind,
            roles: s.roles,
            source: s.source,
            path: None,
            installed: true,
        })
        .collect();
    for (agent, dir, _) in agent_sources(home) {
        for f in skill_files(&dir) {
            let Ok(raw) = std::fs::read_to_string(&f) else { continue };
            let Some((name, description, k, roles)) = front_matter(&raw) else { continue };
            if !matches_skill(&name, &description, &toks, &roles, role, &k, kind) {
                continue;
            }
            if out.iter().any(|x| normalise_name(&x.name) == normalise_name(&name)) {
                continue;
            }
            out.push(Discovered {
                name,
                description,
                kind: k,
                roles,
                source: agent.clone(),
                path: Some(f.display().to_string()),
                installed: false,
            });
        }
    }
    out
}

fn body_of(raw: &str) -> String {
    let raw = raw.replace("\r\n", "\n");
    raw.split_once("\n---")
        .map(|(_, rest)| rest.trim_start_matches('-').trim_start_matches('\n').trim().to_string())
        .unwrap_or_default()
}

/// Read a skill another agent on this machine already has, without installing
/// it. The file must be inside the person's home; nothing is executed.
pub fn read_external(home: &Path, source: &str, name: &str) -> Result<Skill> {
    let dir = if let Some((_, d, _)) = agent_sources(home)
        .into_iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(source.trim()))
    {
        d
    } else {
        let p = PathBuf::from(source.trim());
        let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
        let real = p.canonicalize().ok();
        match real {
            Some(r) if r.is_dir() && r.starts_with(&real_home) => r,
            _ => return Err(DevdError::not_found(format!(
                "`{source}` is not a known agent and not a skill folder inside your home; skills.sources lists the agents devd can read"
            ))),
        }
    };
    let real_home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let want = normalise_name(name);
    let mut available = Vec::new();
    for f in skill_files(&dir) {
        let Ok(raw) = std::fs::read_to_string(&f) else { continue };
        let Some((n, description, kind, roles)) = front_matter(&raw) else { continue };
        if normalise_name(&n) != want {
            available.push(n);
            continue;
        }
        if let Ok(real) = f.canonicalize() {
            if !real.starts_with(&real_home) {
                continue;
            }
        }
        let body = body_of(&raw);
        if body.len() < 20 {
            return Err(DevdError::invalid(format!("`{n}` has no usable instructions")));
        }
        return Ok(Skill {
            name: n,
            description,
            kind,
            roles,
            source: format!("{source} (external, not installed)"),
            body,
        });
    }
    available.sort();
    available.dedup();
    Err(DevdError::not_found(format!(
        "no skill `{name}` in {source}; available: {}",
        if available.is_empty() { "none".to_string() } else { available.join(", ") }
    )))
}

/// Save a person's own skill.
pub fn save(paths: &Paths, name: &str, description: &str, kind: &str, roles: &[String], body: &str) -> Result<()> {
    if name.is_empty() || name.len() > 60 || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(DevdError::invalid("a skill name is lowercase letters, digits and hyphens"));
    }
    if BUILTIN.iter().any(|(n, _)| *n == name) {
        return Err(DevdError::conflict(format!("`{name}` is built in and cannot be replaced; choose another name")));
    }
    if !known_kind(kind) {
        return Err(DevdError::invalid(format!(
            "kind must be one of {}",
            KINDS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
        )));
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
    let kind = if known_kind(&kind) { kind } else { KINDS[2].0.to_string() };
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

/// The index an agent sees: names and one line each, grouped by the stage of
/// work they belong to.
///
/// A skill a person wrote may name a stage devd does not use. It is shown under
/// its own word rather than dropped, because a skill that never appears in a
/// prompt may as well not exist.
pub fn index_for(paths: &Paths, role: Option<&str>) -> String {
    let skills = all(paths);
    let wanted = |s: &Skill| role.map(|r| s.roles.iter().any(|x| x == r)).unwrap_or(true);
    let mut out = String::new();
    for (kind, title) in KINDS {
        let items: Vec<&Skill> = skills.iter().filter(|s| s.kind == *kind && wanted(s)).collect();
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("{title}:\n"));
        for s in items {
            out.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
    }
    // Stages of a person's own, in a stable order, after the built-in ones.
    let mut others: Vec<&str> = skills
        .iter()
        .filter(|s| !known_kind(&s.kind) && wanted(s))
        .map(|s| s.kind.as_str())
        .collect();
    others.sort_unstable();
    others.dedup();
    for kind in others {
        let items: Vec<&Skill> = skills.iter().filter(|s| s.kind == kind && wanted(s)).collect();
        out.push_str(&format!("{}:\n", title_of(kind)));
        for s in items {
            out.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
    }
    out
}

/// The heading for a kind: devd's own word, or a person's word capitalized.
fn title_of(kind: &str) -> String {
    let mut chars = kind.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Other".to_string(),
    }
}

/// The handful of rules every agent follows, always in the prompt.
///
/// Each one is the short form of a skill that holds the full text, and names it,
/// so an agent that knows the rule can read the reasoning behind it.
pub const CORE: &str = "\
Rules that hold whatever the work (`skills.get` with a skill's name reads the full text):\n\
1. Understand before you change: read the real code and data first (`uncover-the-real-structure`).\n\
2. Make the narrowest change that solves the problem; delete before you add (`make-the-narrowest-change`).\n\
3. Work in small steps that each end in a checkable state (`land-one-unit-at-a-time`).\n\
4. Fix root causes, not symptoms; never hide a failure with a retry, a guard or a weakened test (`fix-the-root-cause`).\n\
5. Prove it on the real thing before you say it is done, and say exactly what you ran and saw (`prove-it-works`).\n\
6. Do reversible work without asking; stop for what cannot be undone or is the person's call (`act-then-ask`).\n\
7. Keep your context lean: search first, read the part you need, keep conclusions rather than logs (`keep-your-context-lean`).\n\
8. Report the result for what it is: say what was verified and what was not (`choose-by-the-outcome`).
9. Know what the person is looking at before you answer \"this\" or \"here\" (`app.where`), and show them rather than describe (`show-the-person`).";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name a backticked `` `identifier` `` in `text` mentions, when it
    /// looks like a skill name (lowercase words joined by hyphens).
    fn referenced(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c != '`' {
                continue;
            }
            let start = i + 1;
            let mut end = None;
            for (j, d) in chars.by_ref() {
                if d == '`' {
                    end = Some(j);
                    break;
                }
            }
            let Some(end) = end else { break };
            let word = text[start..end].trim();
            let looks_like_skill = !word.is_empty()
                && word.len() <= 60
                && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && word.contains('-')
                && !word.contains('.');
            if looks_like_skill {
                out.push(word.to_string());
            }
        }
        out
    }

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        (d, p)
    }

    #[test]
    fn every_built_in_skill_parses_and_is_named_after_its_file() {
        // `parse` returning None is a failure this test is here to report, so the
        // assertion is the filter: no Option is carried past it.
        let parsed: Vec<Skill> = BUILTIN.iter().map(|(f, raw)| {
            let s = parse(raw, "builtin").unwrap_or_else(|| panic!("skills/{f}.md does not parse"));
            assert_eq!(s.name, *f, "skills/{f}.md declares name `{}`", s.name);
            assert!(!s.body.is_empty(), "{f} has no instructions");
            s
        }).collect();
        assert_eq!(parsed.len(), BUILTIN.len());
    }

    #[test]
    fn names_and_descriptions_are_unique() {
        let mut names: Vec<&str> = Vec::new();
        let mut descriptions: Vec<&str> = Vec::new();
        for (f, raw) in BUILTIN {
            let s = parse(raw, "builtin").unwrap();
            let _ = f;
            assert!(!names.contains(&s.name.as_str()), "the name `{}` is used twice", s.name);
            assert!(!descriptions.contains(&s.description.as_str()), "the description of `{}` is used twice: {}", s.name, s.description);
            assert!(s.description.len() <= 200, "`{}` has a description over 200 characters", s.name);
            assert!(!s.description.ends_with('.'), "`{}` has a description that ends like a sentence", s.name);
            names.push(Box::leak(s.name.into_boxed_str()));
            descriptions.push(Box::leak(s.description.into_boxed_str()));
        }
        assert!(names.len() > 50, "the built-in set is unexpectedly small");
    }

    #[test]
    fn a_missing_skill_is_found_in_another_agents_folder_and_read_without_installing() {
        let (_p, paths) = temp_paths();
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/skills/accessibility-audit");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: accessibility-audit\ndescription: Check a screen with a keyboard and a screen reader\nkind: check\nroles: designer\n---\n\nStart with the keyboard: tab through every control, then check focus order, labels and contrast. Record what you could not test.\n",
        )
        .unwrap();
        let found = discover(&paths, home.path(), "keyboard accessibility", Some("designer"), None);
        let external = found.iter().find(|s| s.name == "accessibility-audit").expect("found in Claude Code's folder");
        assert!(!external.installed);
        assert_eq!(external.source, "Claude Code");
        let skill = read_external(home.path(), "Claude Code", "accessibility-audit").unwrap();
        assert!(skill.body.contains("tab through every control"));
        assert!(skill.source.contains("not installed"));

        // A folder outside the person's home is not an approved source.
        let outside = tempfile::tempdir().unwrap();
        let err = read_external(home.path(), &outside.path().display().to_string(), "x").unwrap_err();
        assert_eq!(err.code(), "not_found");
    }

    #[test]
    fn discovery_does_not_hide_an_installed_skill_behind_an_external_copy() {
        let (_p, paths) = temp_paths();
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/skills/work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: work\ndescription: A different work skill\nkind: discover\n---\n\nThis is not devd's work skill; it must not shadow the built-in one.\n").unwrap();
        let found = discover(&paths, home.path(), "work", None, None);
        let work = found.iter().find(|s| s.name == "work").unwrap();
        assert!(work.installed, "the built-in skill wins");
    }

    #[test]
    fn every_built_in_skill_names_a_known_stage_and_a_real_role() {
        for (f, raw) in BUILTIN {
            let s = parse(raw, "builtin").unwrap();
            assert!(known_kind(&s.kind), "skills/{f}.md has kind `{}`, which is not one of the stages", s.kind);
            assert!(!s.roles.is_empty(), "skills/{f}.md names no role, so it would reach everyone by accident");
            for r in &s.roles {
                assert!(crate::roles::get(r).is_some(), "skills/{f}.md names the role `{r}`, which does not exist");
            }
        }
    }

    #[test]
    fn no_built_in_skill_carries_a_prefix_in_its_name() {
        // A name is what a person types and what an agent reads; a group prefix
        // repeats what `kind` already says.
        for (f, _) in BUILTIN {
            for prefix in ["principle-", "technique-", "playbook-", "skill-"] {
                assert!(!f.starts_with(prefix), "skills/{f}.md still carries the `{prefix}` prefix");
            }
        }
    }

    #[test]
    fn every_role_a_skill_names_still_exists() {
        for (f, raw) in BUILTIN {
            let s = parse(raw, "builtin").unwrap();
            for r in &s.roles {
                assert!(crate::roles::get(r).is_some(), "skills/{f}.md names the role `{r}`, which does not exist");
            }
        }
    }

    #[test]
    fn every_skill_a_skill_refers_to_exists() {
        let names: Vec<String> = BUILTIN.iter().map(|(f, _)| (*f).to_string()).collect();
        for (f, raw) in BUILTIN {
            let s = parse(raw, "builtin").unwrap();
            for word in referenced(&s.body) {
                // A backticked word with a hyphen is either a devd capability
                // (`project.run`, `fs.read`) or a skill. Only the latter is
                // checked, and a capability always has a group prefix.
                let is_capability = word.split('-').next().map(|g| {
                    ["fs", "git", "site", "service", "runtime", "toolchain", "project", "stack",
                     "stack", "docker", "env", "db", "host", "tls", "health", "skills", "team",
                     "chat", "inbox", "approval", "provider", "models", "schedule", "template",
                     "todo", "scratch", "search", "docs", "compat", "memory", "review", "mcp",
                     "policy", "judge", "workflow", "deploy", "server", "remote", "logs", "cron",
                     "queue", "dbadmin", "update", "dictation", "autostart", "shell", "ssh",
                     "php", "nginx", "apache", "redirects", "backup", "notify", "limits",
                     "environment", "stacks", "toolchain", "intake", "observ"]
                        .contains(&g)
                }).unwrap_or(false);
                if is_capability {
                    continue;
                }
                assert!(
                    names.contains(&word),
                    "skills/{f}.md refers to `{word}`, which is not a skill and not a capability group"
                );
            }
        }
    }

    #[test]
    fn the_index_shows_each_skill_once_under_its_stage() {
        let (_d, p) = temp_paths();
        let index = index_for(&p, None);
        let headings: Vec<&str> = index.lines().filter(|l| l.ends_with(':') && !l.starts_with('-')).collect();
        assert!(headings.contains(&"Discover:") && headings.contains(&"Build:") && headings.contains(&"House rules:"), "{headings:?}");
        for (f, _) in BUILTIN {
            assert_eq!(index.matches(&format!("\n- {f}: ")).count() + usize::from(index.starts_with(&format!("- {f}: "))), 1, "`{f}` appears in the index more than once");
        }
        // The stages keep the order of the work itself.
        let pos = |h: &str| index.find(h).unwrap_or(usize::MAX);
        assert!(pos("Discover:") < pos("Design:") && pos("Design:") < pos("Build:") && pos("Build:") < pos("Check:") && pos("Check:") < pos("Ship:"));
    }

    #[test]
    fn a_roles_index_leaves_out_what_the_role_does_not_carry() {
        let (_d, p) = temp_paths();
        let docs = index_for(&p, Some("docs"));
        assert!(!docs.contains("- fix-bug: "), "docs does not write fixes");
        assert!(docs.contains("- technical-writing: "));
        let qa = index_for(&p, Some("qa"));
        assert!(qa.contains("- prove-it-works: "));
    }

    #[test]
    fn the_core_rules_name_skills_that_exist() {
        let names: Vec<&str> = BUILTIN.iter().map(|(f, _)| *f).collect();
        for word in referenced(CORE) {
            assert!(names.contains(&word.as_str()), "a core rule names `{word}`, which is not a skill");
        }
        assert_eq!(CORE.lines().count(), 10, "a heading and nine rules");
    }

    #[test]
    fn the_work_skill_points_only_at_skills_that_exist() {
        let (_f, raw) = BUILTIN.iter().find(|(f, _)| *f == "work").unwrap();
        let s = parse(raw, "builtin").unwrap();
        let names: Vec<&str> = BUILTIN.iter().map(|(f, _)| *f).collect();
        for word in referenced(&s.body) {
            assert!(names.contains(&word.as_str()), "`work` points at `{word}`, which is no skill");
        }
    }

    #[test]
    fn a_stage_with_nothing_in_it_names_no_heading() {
        // The heading list is derived from what exists, so a person who removes
        // a stage does not leave an empty heading in every prompt.
        let (_d, p) = temp_paths();
        let index = index_for(&p, None);
        for (_, title) in KINDS {
            let has_members = BUILTIN.iter().any(|(_, raw)| {
                parse(raw, "builtin").map(|s| KINDS.iter().any(|(k, t)| t == title && &s.kind == k)).unwrap_or(false)
            });
            if !has_members {
                assert!(!index.contains(&format!("{title}:")), "{title} is empty but has a heading");
            }
        }
    }

    #[test]
    fn a_persons_own_skill_may_use_a_word_of_its_own() {
        let (_d, p) = temp_paths();
        // Nothing devd ships uses an unknown kind, but a person's skill may.
        let dir = user_dir(&p);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.md"), "---\nname: mine\ndescription: my own habit\nkind: personal\nroles:\n---\n\nDo the thing the way this team likes it done.\n").unwrap();
        let mine = all(&p).into_iter().find(|s| s.name == "mine").expect("the skill loads");
        assert_eq!(mine.kind, "personal");
        // It is not lost from the index: an unknown stage is shown last, under
        // its own name, rather than dropped.
        let index = index_for(&p, None);
        assert!(index.contains("- mine: "), "{index}");
    }
}
