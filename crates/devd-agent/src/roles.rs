//! The members of devd's own team: what each one is for, what it may touch, what
//! must be true before its work counts as done, and which skills it carries.
//!
//! A role is a Markdown file in `crates/devd-agent/roles/`: the front matter
//! says what the member may do and must prove, and the body is its instructions.
//! The files are embedded in the binary and parsed once. Roles are few on purpose;
//! one exists only where its permissions, its proof of completion or its way of
//! working differ from the others':
//!
//! | team       | role         | may change files | may run commands | done means                        |
//! |------------|--------------|------------------|------------------|-----------------------------------|
//! | leadership | planner      | no               | no               | a plan was submitted              |
//! | leadership | product      | yes (one doc)    | no               | the requirements file was written |
//! | design     | designer     | yes (UI)         | yes              | files changed and a check passed  |
//! | engineering| architect    | no               | no               | a report, after reading the code  |
//! | engineering| implementer  | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| frontend     | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| backend      | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| mobile       | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| data         | yes (own copy)   | yes              | files changed and a check passed  |
//! | quality    | qa           | yes (tests)      | yes              | tests were run, results reported  |
//! | quality    | reviewer     | no               | yes (read-only)  | the change was read, a verdict    |
//! | quality    | security     | no               | yes (read-only)  | the code was read, findings       |
//! | operations | infra        | no               | yes (+ approval) | the operation ran, state checked  |
//! | knowledge  | researcher   | no               | docs.fetch only  | a report with sources             |
//! | knowledge  | docs         | yes (docs)       | no               | files changed, a report           |
//!
//! "Done" is never the member's word. The harness checks the evidence in the
//! conversation's tool results (see [`crate::team::evidence`]).
//!
//! The strings are leaked once so that a [`Role`] can be `Copy` and `'static`:
//! there are a fixed handful, they live as long as the process, and nothing
//! reloads them.

use std::sync::OnceLock;

use devd_core::capability::{Capability, Tier};

/// What proof a role must leave behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// At least one successful change to files.
    pub change: bool,
    /// At least one successful verification command (test, build, lint, check).
    pub verify: bool,
    /// At least one successful read of the code.
    pub read: bool,
    /// At least one successful command of any kind.
    pub command: bool,
}

/// One member of the team.
#[derive(Debug, Clone, Copy)]
pub struct Role {
    /// Identifier used in plans.
    pub id: &'static str,
    /// Display name.
    pub title: &'static str,
    /// The part of the company the role belongs to (a key of [`TEAMS`]).
    pub team: &'static str,
    /// What the role does, for the person.
    pub summary: &'static str,
    /// Highest tier the role may invoke (see [`rank`]).
    pub max_rank: u8,
    /// May invoke capabilities whose effect is writing files.
    pub writes: bool,
    /// Capabilities allowed regardless of the tier ceiling.
    pub extra: &'static [&'static str],
    /// Works in its own copy of the project when the project is a git repository.
    pub isolated: bool,
    /// Proof required before the task counts as done.
    pub needs: Needs,
    /// The skills the member carries, by name. Only their names and one-line
    /// descriptions go into the prompt; the member reads one when it applies.
    pub skills: &'static [&'static str],
    /// The role's instructions.
    pub prompt: &'static str,
}

/// The parts of the company, in the order a roster shows them.
pub const TEAMS: &[(&str, &str)] = &[
    ("leadership", "Leadership"),
    ("design", "Design"),
    ("engineering", "Engineering"),
    ("quality", "Quality and security"),
    ("operations", "Operations"),
    ("knowledge", "Knowledge"),
];

/// Position of a tier in the ladder from harmless to dangerous.
pub fn rank(t: Tier) -> u8 {
    match t {
        Tier::Read => 0,
        Tier::Write => 1,
        Tier::Exec => 2,
        Tier::Admin => 3,
        Tier::Destructive => 4,
        Tier::Remote => 5,
    }
}

/// Capabilities every team member may use to talk back.
pub const COMMON: &[&str] = &["team.report", "inbox.post", "approval.get", "todo.create", "todo.update", "todo.list", "scratch.write", "scratch.read", "skills.get", "skills.list"];

/// The role files, in roster order: (id, contents).
const FILES: &[(&str, &str)] = &[
    ("planner", include_str!("../roles/planner.md")),
    ("product", include_str!("../roles/product.md")),
    ("designer", include_str!("../roles/designer.md")),
    ("architect", include_str!("../roles/architect.md")),
    ("implementer", include_str!("../roles/implementer.md")),
    ("frontend", include_str!("../roles/frontend.md")),
    ("backend", include_str!("../roles/backend.md")),
    ("mobile", include_str!("../roles/mobile.md")),
    ("data", include_str!("../roles/data.md")),
    ("qa", include_str!("../roles/qa.md")),
    ("reviewer", include_str!("../roles/reviewer.md")),
    ("security", include_str!("../roles/security.md")),
    ("infra", include_str!("../roles/infra.md")),
    ("researcher", include_str!("../roles/researcher.md")),
    ("docs", include_str!("../roles/docs.md")),
];

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_list(items: Vec<String>) -> &'static [&'static str] {
    let v: Vec<&'static str> = items.into_iter().map(leak).collect();
    Box::leak(v.into_boxed_slice())
}

/// Read one role file. The error says which file and what is wrong with it.
fn parse(file: &str, raw: &str) -> Result<Role, String> {
    let bad = |why: &str| format!("roles/{file}.md: {why}");
    let rest = raw.strip_prefix("---\n").ok_or_else(|| bad("must start with a front matter block"))?;
    let (front, body) = rest.split_once("\n---\n").ok_or_else(|| bad("front matter is not closed"))?;
    let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}:")).map(|v| v.trim().to_string()));
    let required = |k: &str| field(k).filter(|v| !v.is_empty()).ok_or_else(|| bad(&format!("`{k}` is missing")));
    let list = |k: &str| -> Vec<String> {
        field(k).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    };
    let flag = |k: &str| -> Result<bool, String> {
        match required(k)?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(bad(&format!("`{k}` must be true or false, not `{other}`"))),
        }
    };

    let id = required("id")?;
    if id != file || !id.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(bad("`id` must match the file name and be lower-case letters"));
    }
    let team = required("team")?;
    if !TEAMS.iter().any(|(k, _)| *k == team) {
        return Err(bad(&format!("`team` {team} is not one of {}", TEAMS.iter().map(|t| t.0).collect::<Vec<_>>().join(", "))));
    }
    let max_rank = match required("tier")?.as_str() {
        "read" => 0,
        "write" => 1,
        "exec" => 2,
        "admin" => 3,
        other => return Err(bad(&format!("`tier` must be read, write, exec or admin, not `{other}`"))),
    };
    let mut needs = Needs { change: false, verify: false, read: false, command: false };
    for p in list("proves") {
        match p.as_str() {
            "change" => needs.change = true,
            "verify" => needs.verify = true,
            "read" => needs.read = true,
            "command" => needs.command = true,
            other => return Err(bad(&format!("`proves` has an unknown item `{other}`"))),
        }
    }
    let body = body.trim();
    if body.is_empty() {
        return Err(bad("has no instructions"));
    }
    Ok(Role {
        id: leak(id),
        title: leak(required("title")?),
        team: leak(team),
        summary: leak(required("summary")?),
        max_rank,
        writes: flag("writes")?,
        extra: leak_list(list("extra")),
        isolated: flag("isolated")?,
        needs,
        skills: leak_list(list("skills")),
        prompt: leak(body.to_string()),
    })
}

/// Every role, in roster order.
pub fn all() -> &'static [Role] {
    static ROLES: OnceLock<Vec<Role>> = OnceLock::new();
    ROLES.get_or_init(|| {
        FILES
            .iter()
            .map(|(id, raw)| parse(id, raw).unwrap_or_else(|e| panic!("a built-in role is malformed: {e}")))
            .collect()
    })
}

/// Look up a role.
pub fn get(id: &str) -> Option<&'static Role> {
    all().iter().find(|r| r.id == id)
}

/// The team as a list a planner can choose from: one line per member.
pub fn roster_text() -> String {
    all().iter().map(|r| format!("- {} ({}): {}", r.id, r.title, r.summary)).collect::<Vec<_>>().join("\n")
}

/// The skills a member carries, as names with one line each.
pub fn loadout_text(role: &Role) -> String {
    role.skills
        .iter()
        .filter_map(|name| crate::skills::builtin_description(name).map(|d| format!("- {name}: {d}")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether the role may invoke a capability. The reason when not.
pub fn allows(role: &Role, cap: &Capability) -> Result<(), String> {
    if COMMON.contains(&cap.id.as_str()) || role.extra.contains(&cap.id.as_str()) {
        return Ok(());
    }
    if cap.id.starts_with("team.") || cap.id.starts_with("chat.") {
        return Err(format!("`{}` is not available to the {} role", cap.id, role.title));
    }
    if rank(cap.tier) > role.max_rank {
        return Err(format!("the {} role may not use {}-tier operations like `{}`", role.title, cap.tier.as_str(), cap.id));
    }
    if !role.writes && matches!(cap.effect, devd_core::capability::Effect::Write) {
        return Err(format!("the {} role does not change files; `{}` is out of scope. Report what should change instead", role.title, cap.id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills_data::BUILTIN;

    #[test]
    fn every_built_in_role_parses_and_matches_its_file() {
        let roles = all();
        assert_eq!(roles.len(), FILES.len());
        for (role, (id, _)) in roles.iter().zip(FILES) {
            assert_eq!(role.id, *id);
            assert!(!role.prompt.is_empty() && !role.summary.is_empty() && !role.title.is_empty());
        }
        let mut ids: Vec<&str> = roles.iter().map(|r| r.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), roles.len(), "a role id is used twice");
    }

    #[test]
    fn every_skill_a_role_carries_exists() {
        let names: Vec<&str> = BUILTIN
            .iter()
            .filter_map(|(_, raw)| raw.lines().find_map(|l| l.strip_prefix("name:")).map(str::trim))
            .collect();
        for role in all() {
            for skill in role.skills {
                assert!(names.contains(skill), "{} carries the skill `{skill}`, which does not exist", role.id);
            }
            assert!(!loadout_text(role).is_empty() || role.skills.is_empty(), "{} has a loadout that renders empty", role.id);
        }
    }

    #[test]
    fn the_permissions_each_role_declares_are_coherent() {
        for r in all() {
            assert!(r.max_rank <= 3, "{} may use destructive or remote operations", r.id);
            if r.needs.change {
                assert!(r.writes, "{} must prove a change but may not change files", r.id);
            }
            if r.needs.verify {
                assert!(r.max_rank >= 2, "{} must prove a check ran but may not run commands", r.id);
            }
            if r.writes && r.isolated {
                assert!(r.needs.change, "{} works in its own copy but does not have to change anything", r.id);
            }
            assert!(TEAMS.iter().any(|(k, _)| *k == r.team), "{} is in an unknown team", r.id);
        }
        // Only the planner submits a plan, and it changes nothing.
        let planner = get("planner").unwrap();
        assert!(planner.extra.contains(&"team.plan_submit") && !planner.writes);
        assert!(all().iter().filter(|r| r.extra.contains(&"team.plan_submit")).count() == 1);
        // Roles that read or review never write.
        for id in ["architect", "reviewer", "security", "researcher", "infra"] {
            assert!(!get(id).unwrap().writes, "{id} must not change files");
        }
    }

    #[test]
    fn the_roster_covers_a_startup_and_every_team_has_someone() {
        for needed in ["product", "designer", "frontend", "backend", "mobile", "data", "security", "qa", "infra", "docs"] {
            assert!(get(needed).is_some(), "the team has no {needed}");
        }
        for (key, _) in TEAMS {
            assert!(all().iter().any(|r| r.team == *key), "nobody is in {key}");
        }
    }

    #[test]
    fn the_planner_is_told_who_is_on_the_team() {
        let text = roster_text();
        assert!(text.contains("- frontend (Frontend engineer):"));
        assert_eq!(text.lines().count(), all().len());
    }

    #[test]
    fn a_malformed_role_says_which_file_and_why() {
        let e = parse("x", "no front matter").unwrap_err();
        assert!(e.contains("roles/x.md") && e.contains("front matter"), "{e}");
        let e = parse("x", "---\nid: y\ntitle: T\nteam: design\nsummary: s\ntier: read\nwrites: false\nisolated: false\n---\nbody").unwrap_err();
        assert!(e.contains("must match the file name"), "{e}");
        let e = parse("x", "---\nid: x\ntitle: T\nteam: nowhere\nsummary: s\ntier: read\nwrites: false\nisolated: false\n---\nbody").unwrap_err();
        assert!(e.contains("nowhere"), "{e}");
        let e = parse("x", "---\nid: x\ntitle: T\nteam: design\nsummary: s\ntier: root\nwrites: false\nisolated: false\n---\nbody").unwrap_err();
        assert!(e.contains("tier"), "{e}");
        let e = parse("x", "---\nid: x\ntitle: T\nteam: design\nsummary: s\ntier: read\nwrites: maybe\nisolated: false\n---\nbody").unwrap_err();
        assert!(e.contains("true or false"), "{e}");
    }
}
