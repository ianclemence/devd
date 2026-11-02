//! Which prompts a turn ran under.
//!
//! devd's roles and skills are compiled into the binary on purpose: nobody can
//! edit a file and quietly change what an agent is allowed to do. The cost is
//! that "the run that worked and the run that did not behaved differently" is
//! unanswerable, because nothing records which text the agent actually read.
//!
//! This records it without giving that up. For each conversation and each turn
//! devd writes a small manifest: every role file and every skill the prompt
//! mentions, with a short hash of its contents. The *content* still comes from
//! the binary and the owner-only folder; the manifest is only a fingerprint. So a
//! build that changed a role, a person who added a skill, or an agent that loaded
//! a different playbook is visible in the run's timeline, and two runs can be
//! compared by name and hash.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use devd_core::paths::Paths;
use devd_core::util;

use crate::roles;
use crate::skills;

/// One piece of prompt text and what it hashed to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptPart {
    /// `role`, `skill` or `core`.
    pub kind: String,
    /// The role id or skill name.
    pub name: String,
    /// `builtin` or `user`; `core` for the shared instructions.
    pub source: String,
    /// The first characters of the SHA-256 of the text.
    pub hash: String,
}

impl PromptPart {
    fn of(kind: &str, name: &str, source: &str, text: &str) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            source: source.into(),
            hash: util::sha256_hex(text.as_bytes())[..12].to_string(),
        }
    }
}

/// Everything a turn's prompt was assembled from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptManifest {
    /// `answer` for the assistant, or the role id for a team member.
    pub role: String,
    /// Each part, in the order the prompt introduces it.
    pub parts: Vec<PromptPart>,
    /// One hash over all of them, so two turns can be compared with one string.
    pub hash: String,
}

impl PromptManifest {
    /// The parts as the timeline step stores them.
    pub fn to_json(&self) -> Value {
        json!({
            "role": self.role,
            "hash": self.hash,
            "parts": self.parts,
        })
    }

    /// A short line for the run view: "role implementer · 1 skill · core".
    pub fn summary(&self) -> String {
        let roles: Vec<&str> = self.parts.iter().filter(|p| p.kind == "role").map(|p| p.name.as_str()).collect();
        let skills = self.parts.iter().filter(|p| p.kind == "skill").count();
        if roles.is_empty() {
            format!("assistant prompt · {skills} skill(s) · {}", self.hash)
        } else {
            format!("role {} · {} skill(s) · {}", roles.join(", "), skills, self.hash)
        }
    }
}

/// Build the manifest for a conversation.
///
/// `role` is the specialist role driving the turn, or `None` for the assistant.
/// An `loaded` skill is one the agent read during this conversation
/// (`skills.get`), which changes what it knows even though the index did not.
pub fn manifest(paths: &Paths, role: Option<&str>, loaded: &[String]) -> PromptManifest {
    let mut parts = vec![PromptPart::of("core", "core", "builtin", crate::skills::CORE)];
    match role.and_then(roles::get) {
        Some(r) => {
            // The instruction text is what matters; the file it came from and
            // whether it is a person's own role are recorded alongside it.
            let source = if roles::is_built_in(r.id) { "builtin" } else { "user" };
            parts.push(PromptPart::of("role", r.id, source, r.prompt));
            // A role's loadout is part of what it was told to reach for.
            parts.push(PromptPart::of("role-loadout", r.id, source, &r.skills.join(",")));
        }
        None => parts.push(PromptPart::of("role", "assistant", "builtin", crate::chat::ASSISTANT_PROMPT)),
    }
    let mut named: Vec<String> = Vec::new();
    if let Some(r) = role.and_then(roles::get) {
        named.extend(r.skills.iter().map(|s| s.to_string()));
    }
    // Every `skills.get` in the conversation, in the order it happened.
    named.extend(loaded.iter().cloned());
    let mut seen = std::collections::HashSet::new();
    named.retain(|n| seen.insert(n.clone()));
    let index = skills::all(paths);
    for name in &named {
        // A skill that no longer exists is part of what changed, so its absence
        // is recorded rather than skipped.
        let part = match index.iter().find(|s| &s.name == name) {
            Some(skill) => PromptPart::of("skill", &skill.name, &skill.source, &skill.body),
            None => PromptPart::of("skill", name, "missing", "the skill this conversation read no longer exists"),
        };
        parts.push(part);
    }
    let text = parts
        .iter()
        .map(|p| format!("{}:{}:{}", p.kind, p.name, p.hash))
        .collect::<Vec<_>>()
        .join("\n");
    let hash = util::sha256_hex(text.as_bytes())[..12].to_string();
    PromptManifest {
        role: role.unwrap_or("assistant").to_string(),
        parts,
        hash,
    }
}

/// Every `skills.get` in a conversation, in the order it happened.
///
/// These are the skill bodies the agent actually read, which is part of what it
/// knows even though the prompt's index did not change.
pub fn loaded_skills(events: &[devd_core::ChatEvent]) -> Vec<String> {
    let mut out = Vec::new();
    for e in events {
        let blocks = e.data["content"].as_array().cloned().unwrap_or_default();
        for b in blocks {
            let name = b["name"].as_str().unwrap_or("");
            if b["type"] != "tool_use" {
                continue;
            }
            // A skill is read either by its direct name or through `devd_invoke`.
            let skill = if name == "skills.get" {
                b["input"]["name"].as_str().map(str::to_string)
            } else if name == "devd_invoke" && b["input"]["id"].as_str() == Some("skills.get") {
                b["input"]["args"]["name"].as_str().map(str::to_string)
            } else {
                None
            };
            if let Some(skill) = skill.filter(|s| !s.is_empty()) {
                if !out.contains(&skill) {
                    out.push(skill);
                }
            }
        }
    }
    out
}

/// Whether two turns ran under text that differs in a way worth showing.
pub fn differs(a: &PromptManifest, b: &PromptManifest) -> Option<String> {
    if a.hash == b.hash {
        return None;
    }
    let key = |p: &PromptPart| format!("{}:{}", p.kind, p.name);
    let a_names: Vec<String> = a.parts.iter().map(key).collect();
    let b_names: Vec<String> = b.parts.iter().map(key).collect();
    let added: Vec<&String> = b_names.iter().filter(|n| !a_names.contains(n)).collect();
    let removed: Vec<&String> = a_names.iter().filter(|n| !b_names.contains(n)).collect();
    let changed: Vec<String> = a
        .parts
        .iter()
        .filter_map(|p| {
            b.parts
                .iter()
                .find(|q| key(q) == key(p))
                .filter(|q| q.hash != p.hash)
                .map(|_| key(p))
        })
        .collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("added {}", added.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if !removed.is_empty() {
        parts.push(format!("removed {}", removed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    if !changed.is_empty() {
        parts.push(format!("changed {}", changed.join(", ")));
    }
    if parts.is_empty() {
        parts.push("the assembled prompt differs".to_string());
    }
    Some(parts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        (d, p)
    }

    #[test]
    fn a_specialist_prompt_records_the_role_and_its_skills() {
        let (_d, p) = paths();
        let m = manifest(&p, Some("implementer"), &[]);
        assert_eq!(m.role, "implementer");
        assert!(m.parts.iter().any(|x| x.kind == "role" && x.name == "implementer"));
        let role = roles::get("implementer").unwrap();
        for skill in role.skills.iter().take(1) {
            assert!(
                m.parts.iter().any(|x| x.kind == "skill" && &x.name == skill),
                "the loadout's skills are part of the manifest"
            );
        }
        assert_eq!(m.hash.len(), 12);
    }

    #[test]
    fn the_assistant_and_a_specialist_do_not_share_a_hash() {
        let (_d, p) = paths();
        let a = manifest(&p, None, &[]);
        let b = manifest(&p, Some("qa"), &[]);
        assert_ne!(a.hash, b.hash);
        assert_eq!(a.role, "assistant");
    }

    #[test]
    fn a_skill_read_during_the_conversation_changes_the_manifest() {
        let (_d, p) = paths();
        // A skill the role does not already carry: reading it is new information.
        let implementer = roles::get("implementer").unwrap();
        let extra = skills::all(&p)
            .into_iter()
            .find(|s| !implementer.skills.contains(&s.name.as_str()))
            .expect("there is a skill outside the implementer's loadout");
        let before = manifest(&p, Some("implementer"), &[]);
        let after = manifest(&p, Some("implementer"), std::slice::from_ref(&extra.name));
        assert_ne!(before.hash, after.hash);
        let why = differs(&before, &after).expect("they differ");
        assert!(why.contains(&format!("added skill:{}", extra.name)), "{why}");
        // A skill already in the loadout changes nothing, because it was in the
        // prompt from the first turn.
        let already = implementer.skills[0];
        assert_eq!(
            manifest(&p, Some("implementer"), &[already.to_string()]).hash,
            before.hash
        );
    }

    #[test]
    fn a_persons_own_skill_is_marked_as_theirs() {
        let (_d, p) = paths();
        crate::skills::save(
            &p,
            "house-style",
            "how this codebase is written",
            "words",
            &[],
            "Short sentences. Comments only where the code cannot say it.",
        )
        .unwrap();
        let m = manifest(&p, None, &["house-style".to_string()]);
        let part = m.parts.iter().find(|x| x.name == "house-style").unwrap();
        assert_eq!(part.source, "user");
    }

    #[test]
    fn editing_a_skill_changes_its_hash_and_the_manifest() {
        let (_d, p) = paths();
        crate::skills::save(&p, "one", "d", "words", &[], "The first version of the house rule, written out.").unwrap();
        let a = manifest(&p, None, &["one".to_string()]);
        crate::skills::save(&p, "one", "d", "words", &[], "The second version of the house rule, written out.").unwrap();
        let b = manifest(&p, None, &["one".to_string()]);
        assert_ne!(a.hash, b.hash);
        // Re-saving the same text leaves it where it was.
        let c = manifest(&p, None, &["one".to_string()]);
        assert_eq!(b.hash, c.hash);
    }

    #[test]
    fn the_same_prompt_twice_is_not_reported_as_a_difference() {
        let (_d, p) = paths();
        let a = manifest(&p, Some("qa"), &[]);
        let b = manifest(&p, Some("qa"), &[]);
        assert_eq!(a.hash, b.hash);
        assert!(differs(&a, &b).is_none());
    }

    #[test]
    fn a_new_build_that_changed_a_role_is_visible_as_a_changed_part() {
        let (_d, p) = paths();
        let before = manifest(&p, Some("planner"), &[]);
        let mut altered = before.clone();
        // Stand in for a different build: the same name, different text.
        for part in altered.parts.iter_mut().filter(|x| x.kind == "role") {
            part.hash = "000000000000".into();
        }
        altered.hash = "ffffffffffff".into();
        let why = differs(&altered, &before).unwrap();
        assert!(why.contains("changed role:planner"), "{why}");
    }

    #[test]
    fn the_manifest_is_small_enough_to_store_on_every_turn() {
        let (_d, p) = paths();
        let text = manifest(&p, Some("implementer"), &[]).to_json().to_string();
        assert!(text.len() < 4000, "{} bytes", text.len());
    }
}
