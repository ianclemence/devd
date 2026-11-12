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
//! | design     | motion       | yes (own copy)   | yes              | a composition written, a render ran |
//! | engineering| architect    | yes (task copy)  | no               | a report, after reading the code  |
//! | engineering| implementer  | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| frontend     | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| backend      | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| mobile       | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| data         | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| ml           | yes (own copy)   | yes              | files changed and a check passed  |
//! | engineering| hardware     | yes (own copy)   | yes              | files changed and a check passed  |
//! | quality    | qa           | yes (tests)      | yes              | tests were run, results reported  |
//! | quality    | reviewer     | yes (task copy)  | yes              | the change was read, a verdict    |
//! | quality    | security     | yes (task copy)  | yes              | the code was read, findings       |
//! | operations | infra        | yes (task copy)  | yes (+ approval) | the operation ran, state checked  |
//! | operations | sre          | yes (config)     | yes              | a command ran and the check passed |
//! | operations | release      | yes (notes)      | yes              | the checks ran on the final commit |
//! | knowledge  | researcher   | yes (task copy)  | docs.fetch only  | a report with sources             |
//! | knowledge  | docs         | yes (docs)       | no               | files changed, a report           |
//! | knowledge  | content      | yes (docs)       | no               | files changed, claims sourced     |
//!
//! "Done" is never the member's word. The harness checks the evidence in the
//! conversation's tool results (see [`crate::team::evidence`]).
//!
//! The strings are leaked once so that a [`Role`] can be `Copy` and `'static`:
//! there are a fixed handful, they live as long as the process, and nothing
//! reloads them.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

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
    /// Who usually receives this member's work next. It is the team's flow:
    /// the planner reads it when it lays out a plan, and `team.flow` draws it.
    /// A role may hand back as well as forward (a reviewer to the author), so
    /// the graph is not required to be acyclic; it is required to name real roles.
    pub hands_to: &'static [&'static str],
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
pub const COMMON: &[&str] = &["team.report", "team.context", "team.charter.get", "inbox.post", "approval.get", "todo.create", "todo.update", "todo.list", "scratch.write", "scratch.read", "skills.get", "skills.list"];

/// The role files, in roster order: (id, contents).
const FILES: &[(&str, &str)] = &[
    ("planner", include_str!("../roles/planner.md")),
    ("product", include_str!("../roles/product.md")),
    ("designer", include_str!("../roles/designer.md")),
    ("motion", include_str!("../roles/motion.md")),
    ("architect", include_str!("../roles/architect.md")),
    ("implementer", include_str!("../roles/implementer.md")),
    ("frontend", include_str!("../roles/frontend.md")),
    ("backend", include_str!("../roles/backend.md")),
    ("mobile", include_str!("../roles/mobile.md")),
    ("data", include_str!("../roles/data.md")),
    ("ml", include_str!("../roles/ml.md")),
    ("hardware", include_str!("../roles/hardware.md")),
    ("qa", include_str!("../roles/qa.md")),
    ("reviewer", include_str!("../roles/reviewer.md")),
    ("security", include_str!("../roles/security.md")),
    ("infra", include_str!("../roles/infra.md")),
    ("sre", include_str!("../roles/sre.md")),
    ("release", include_str!("../roles/release.md")),
    ("researcher", include_str!("../roles/researcher.md")),
    ("docs", include_str!("../roles/docs.md")),
    ("content", include_str!("../roles/content.md")),
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
    // A Windows checkout, or a role file written in Notepad, has CRLF. The
    // front matter is line-based, so normalise once rather than failing every
    // role on that platform.
    let raw = raw.replace("\r\n", "\n");
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
        hands_to: leak_list(list("hands_to")),
        prompt: leak(body.to_string()),
    })
}

static CUSTOM_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// The roster as last read: every role and why some files were not loaded.
type Roster = (&'static [Role], &'static [String]);
static ROSTER: RwLock<Option<Roster>> = RwLock::new(None);

/// Name the folder a person keeps their own roles in. The first call wins:
/// the roster is read once, and a later caller must not move it.
pub fn use_custom_dir(dir: &Path) {
    let mut slot = CUSTOM_DIR.write().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(dir.to_path_buf());
    }
}

/// Make `dir` the folder in use, because a role was just saved to it or removed from it.
fn follow(dir: &Path) {
    *CUSTOM_DIR.write().unwrap_or_else(|e| e.into_inner()) = Some(dir.to_path_buf());
}

/// Why a role file in the custom folder was not loaded, one line each.
pub fn custom_errors() -> &'static [String] {
    current().1
}

/// Roles whose `hands_to` names a role that does not exist, or themselves.
fn flow_errors(roles: &[Role]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for r in roles {
        for to in r.hands_to {
            if *to == r.id {
                out.push((r.id.to_string(), format!("roles/{}.md: `hands_to` names the role itself", r.id)));
            } else if !roles.iter().any(|x| x.id == *to) {
                out.push((r.id.to_string(), format!("roles/{}.md: `hands_to` names {to}, which is not a role", r.id)));
            }
        }
    }
    out
}

/// The roles in a folder: every `*.md` that parses, and a message for each that
/// does not. A person's role can add to the team but never replace a built-in one.
fn load_custom(dir: &Path, taken: &[Role]) -> (Vec<Role>, Vec<String>) {
    let mut roles: Vec<Role> = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return (roles, errors) };
    let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "md").unwrap_or(false)).collect();
    files.sort();
    for path in files {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r.replace("\r\n", "\n"),
            Err(e) => {
                errors.push(format!("roles/{stem}.md: cannot be read: {e}"));
                continue;
            }
        };
        match parse(&stem, &raw) {
            Err(e) => errors.push(e),
            Ok(r) if taken.iter().chain(roles.iter()).any(|t| t.id == r.id) => errors.push(format!("roles/{stem}.md: a role named {} already exists", r.id)),
            Ok(r) if r.max_rank > 2 || !r.extra.is_empty() => errors.push(format!("roles/{stem}.md: your own roles may use up to the exec tier and no extra capabilities")),
            Ok(r) => match r.skills.iter().find(|name| crate::skills::builtin_description(name).is_none()) {
                Some(name) => errors.push(format!("roles/{stem}.md: the skill {name} does not exist")),
                None => roles.push(r),
            },
        }
    }
    // Hand-offs are checked once every role is known, so two of a person's own
    // roles can name each other. A role that points nowhere is not loaded.
    let everyone: Vec<Role> = taken.iter().chain(roles.iter()).copied().collect();
    for (id, why) in flow_errors(&everyone) {
        if roles.iter().any(|r| r.id == id) {
            roles.retain(|r| r.id != id);
            errors.push(why);
        }
    }
    (roles, errors)
}

fn build() -> Roster {
    let mut roles: Vec<Role> = FILES
        .iter()
        .map(|(id, raw)| parse(id, raw).unwrap_or_else(|e| panic!("a built-in role is malformed: {e}")))
        .collect();
    let mut errors = Vec::new();
    let dir = CUSTOM_DIR.read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(dir) = dir {
        let (custom, found) = load_custom(&dir, &roles);
        roles.extend(custom);
        errors = found;
    }
    (Box::leak(roles.into_boxed_slice()), Box::leak(errors.into_boxed_slice()))
}

fn current() -> Roster {
    if let Some(r) = *ROSTER.read().unwrap_or_else(|e| e.into_inner()) {
        return r;
    }
    let built = build();
    let mut slot = ROSTER.write().unwrap_or_else(|e| e.into_inner());
    *slot.get_or_insert(built)
}

/// Read the roles again, after a person added, changed or removed one. Callers
/// that already hold the old roster keep it until they ask again; the few
/// hundred bytes it leaves behind are the price of never taking a lock to read a role.
pub fn reload() {
    *ROSTER.write().unwrap_or_else(|e| e.into_inner()) = Some(build());
}

/// Every role, in roster order: the built-in team, then the person's own.
pub fn all() -> &'static [Role] {
    current().0
}

/// What a person may say about a role of their own.
#[derive(Debug, Clone)]
pub struct Draft {
    /// The id: lower-case letters and hyphens, and the file name.
    pub id: String,
    /// The name shown on the roster.
    pub title: String,
    /// The part of the company it sits in (a key of [`TEAMS`]).
    pub team: String,
    /// One line for the roster.
    pub summary: String,
    /// `read`, `write` or `exec`. A role of a person's own never goes higher.
    pub tier: String,
    /// May it change files in its own copy of the project.
    pub writes: bool,
    /// Works in its own copy of the project.
    pub isolated: bool,
    /// What it must leave behind: `change`, `verify`, `read`, `command`.
    pub proves: Vec<String>,
    /// Skills it carries.
    pub skills: Vec<String>,
    /// Who usually receives its work next.
    pub hands_to: Vec<String>,
    /// Its instructions.
    pub instructions: String,
}

impl Draft {
    /// The role file this draft is.
    pub fn markdown(&self) -> String {
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        format!(
            "---\nid: {}\ntitle: {}\nteam: {}\nsummary: {}\ntier: {}\nwrites: {}\nisolated: {}\nextra:\nproves: {}\nskills: {}\nhands_to: {}\n---\n{}\n",
            self.id.trim(),
            one_line(&self.title),
            self.team.trim(),
            one_line(&self.summary),
            self.tier.trim(),
            self.writes,
            self.isolated,
            self.proves.join(", "),
            self.skills.join(", "),
            self.hands_to.join(", "),
            self.instructions.trim()
        )
    }
}

/// Save a role of a person's own, check it the way a file in the folder is
/// checked, and make it part of the team at once.
///
/// It is written only if it loads: a role that would be refused at the next
/// start is refused now, with the reason, and nothing changes. A built-in role
/// cannot be replaced; one of the person's own can be changed by saving it again.
pub fn save_custom(dir: &Path, draft: &Draft) -> Result<Role, String> {
    let role = save_custom_in(dir, draft, all())?;
    follow(dir);
    reload();
    Ok(get(role.id).copied().unwrap_or(role))
}

/// [`save_custom`] against a given roster, without touching the one in use.
fn save_custom_in(dir: &Path, draft: &Draft, roster: &[Role]) -> Result<Role, String> {
    let id = draft.id.trim();
    if is_built_in(id) {
        return Err(format!("{id} is one of devd's own roles and cannot be replaced; choose another id"));
    }
    if draft.instructions.trim().chars().count() < 40 {
        return Err("give the role instructions: what it does, what it reads first, how it proves its work".to_string());
    }
    // Checked in a folder of its own inside the roles folder, so the check is exactly what a start would do.
    let scratch = dir.join(".check");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("could not check the role: {e}"))?;
    std::fs::write(scratch.join(format!("{id}.md")), draft.markdown()).map_err(|e| format!("could not check the role: {e}"))?;
    // The roster without this role, so saving it again is a change and not a clash.
    let others: Vec<Role> = roster.iter().filter(|r| r.id != id).copied().collect();
    let (loaded, errors) = load_custom(&scratch, &others);
    let _ = std::fs::remove_dir_all(&scratch);
    if let Some(why) = errors.into_iter().next() {
        return Err(why);
    }
    let Some(role) = loaded.into_iter().next() else { return Err("the role did not load".to_string()) };
    std::fs::create_dir_all(dir).map_err(|e| format!("could not save the role: {e}"))?;
    let tmp = dir.join(format!(".{id}.tmp"));
    std::fs::write(&tmp, draft.markdown()).map_err(|e| format!("could not save the role: {e}"))?;
    std::fs::rename(&tmp, dir.join(format!("{id}.md"))).map_err(|e| format!("could not save the role: {e}"))?;
    Ok(role)
}

/// Remove a role of a person's own.
///
/// A role that another still hands work to is refused: removing it would leave
/// a hand-off that points nowhere. Change those roles first.
pub fn delete_custom(dir: &Path, id: &str) -> Result<(), String> {
    delete_custom_in(dir, id, all())?;
    follow(dir);
    reload();
    Ok(())
}

/// [`delete_custom`] against a given roster, without touching the one in use.
fn delete_custom_in(dir: &Path, id: &str, roster: &[Role]) -> Result<(), String> {
    if is_built_in(id) {
        return Err(format!("{id} is one of devd's own roles and cannot be removed"));
    }
    let file = dir.join(format!("{id}.md"));
    if !file.is_file() {
        return Err(format!("you have no role named {id}"));
    }
    if let Some(r) = roster.iter().find(|r| r.id != id && r.hands_to.contains(&id)) {
        return Err(format!("{} still hands work to {id}; change that role first", r.id));
    }
    std::fs::remove_file(&file).map_err(|e| format!("could not remove the role: {e}"))?;
    Ok(())
}

/// The team's flow as a graph: each role with who it hands work to and who hands work to it.
pub fn flow() -> Vec<(&'static Role, Vec<&'static str>)> {
    all()
        .iter()
        .map(|r| (r, all().iter().filter(|x| x.hands_to.contains(&r.id)).map(|x| x.id).collect()))
        .collect()
}

/// Whether a role ships with devd, as opposed to being one a person wrote.
pub fn is_built_in(id: &str) -> bool {
    FILES.iter().any(|(f, _)| *f == id)
}

/// Look up a role.
pub fn get(id: &str) -> Option<&'static Role> {
    all().iter().find(|r| r.id == id)
}

/// The team as a list a planner can choose from: one line per member.
pub fn roster_text() -> String {
    // The planner assigns work from this list. `writes` is not printed as a
    // cage: a specialist may change files inside a task whose work is to
    // build or fix, whatever its usual emphasis.
    all()
        .iter()
        .map(|r| {
            let next = if r.hands_to.is_empty() { String::new() } else { format!(" Hands to: {}.", r.hands_to.join(", ")) };
            format!("- {} ({}): {}{next}", r.id, r.title, r.summary)
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    // `writes` describes a role's usual work, not a cage. A specialist whose
    // task is to change the project must be able to change it; the task's own
    // directory is the boundary. `chat::Hub::gate` gives scoped file and git
    // work that freedom and still confines every path to the task worktree.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills_data::BUILTIN;

    #[test]
    fn a_persons_own_roles_are_loaded_and_bad_ones_are_explained() {
        let d = tempfile::tempdir().unwrap();
        let good = "---\nid: copywriter\ntitle: Copywriter\nteam: knowledge\nsummary: Writes the words in the product\ntier: read\nwrites: true\nisolated: false\nproves: change\nskills: plain-speech\n---\nYou write short, plain copy.\n";
        std::fs::write(d.path().join("copywriter.md"), good).unwrap();
        std::fs::write(d.path().join("planner.md"), good.replace("copywriter", "planner")).unwrap();
        std::fs::write(d.path().join("ghost.md"), good.replace("copywriter", "ghost").replace("plain-speech", "no-such-skill")).unwrap();
        std::fs::write(d.path().join("broken.md"), "no front matter").unwrap();
        let (roles, errors) = load_custom(d.path(), all());
        assert_eq!(roles.iter().map(|r| r.id).collect::<Vec<_>>(), vec!["copywriter"]);
        assert_eq!(errors.len(), 3, "{errors:?}");
        let admin = good.replace("copywriter", "gatekeeper").replace("tier: read", "tier: admin");
        std::fs::write(d.path().join("gatekeeper.md"), admin).unwrap();
        let (_, errors) = load_custom(d.path(), all());
        assert!(errors.iter().any(|e| e.contains("up to the exec tier")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("already exists")));
        assert!(errors.iter().any(|e| e.contains("no-such-skill")));
    }

    #[test]
    fn every_hand_off_names_a_real_role_and_every_role_is_reachable() {
        let team: Vec<Role> = all().iter().filter(|r| is_built_in(r.id)).copied().collect();
        assert!(flow_errors(&team).is_empty(), "{:?}", flow_errors(&team));
        for role in &team {
            let from: Vec<&str> = team.iter().filter(|x| x.hands_to.contains(&role.id)).map(|x| x.id).collect();
            // Only the planner starts work; every other role receives some, so none is orphaned.
            assert!(role.id == "planner" || !from.is_empty() || role.id == "researcher" || role.id == "product", "{} is never handed anything", role.id);
        }
        let planner = roster_text();
        assert!(planner.contains("- implementer (") && planner.contains("Hands to: qa, reviewer."), "{planner}");
    }

    #[test]
    fn a_hand_off_to_nobody_is_refused_for_a_persons_own_role() {
        let d = tempfile::tempdir().unwrap();
        let role = |id: &str, to: &str| format!("---\nid: {id}\ntitle: T\nteam: knowledge\nsummary: S\ntier: read\nwrites: true\nisolated: false\nproves: change\nskills: plain-speech\nhands_to: {to}\n---\nYou write short, plain copy for the product.\n");
        std::fs::write(d.path().join("scribe.md"), role("scribe", "reviewer, nobody")).unwrap();
        std::fs::write(d.path().join("loop.md"), role("loop", "loop")).unwrap();
        std::fs::write(d.path().join("fine.md"), role("fine", "reviewer, qa")).unwrap();
        let (roles, errors) = load_custom(d.path(), all());
        assert_eq!(roles.iter().map(|r| r.id).collect::<Vec<_>>(), vec!["fine"]);
        assert!(errors.iter().any(|e| e.contains("nobody") && e.contains("not a role")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("the role itself")), "{errors:?}");
    }

    #[test]
    fn a_role_saved_is_checked_first_and_cannot_be_removed_from_under_one_that_hands_to_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("roles");
        let draft = |id: &str, to: &[&str]| Draft {
            id: id.into(),
            title: "Support engineer".into(),
            team: "operations".into(),
            summary: "Answers a customer's report with a reproduction and a fix".into(),
            tier: "exec".into(),
            writes: true,
            isolated: true,
            proves: vec!["change".into(), "verify".into()],
            skills: vec!["fix-bug".into(), "plain-speech".into()],
            hands_to: to.iter().map(|s| s.to_string()).collect(),
            instructions: "You are the support engineer. Reproduce what the customer reports before you change anything, fix the cause, and prove it.".into(),
        };
        // The team as a start would read it: the built-ins, then what is in the folder.
        let builtin: Vec<Role> = all().iter().filter(|r| is_built_in(r.id)).copied().collect();
        let team = |dir: &Path| -> Vec<Role> {
            let (own, _) = load_custom(dir, &builtin);
            builtin.iter().copied().chain(own).collect()
        };
        // A built-in cannot be replaced; a bad tier, skill, hand-off or text is refused with the reason, and nothing is written.
        assert!(save_custom_in(&dir, &draft("planner", &[]), &builtin).unwrap_err().contains("devd's own"));
        assert!(save_custom_in(&dir, &Draft { tier: "admin".into(), ..draft("support", &[]) }, &builtin).unwrap_err().contains("exec tier"));
        assert!(save_custom_in(&dir, &Draft { skills: vec!["no-such-skill".into()], ..draft("support", &[]) }, &builtin).unwrap_err().contains("no-such-skill"));
        assert!(save_custom_in(&dir, &draft("support", &["ghost"]), &builtin).unwrap_err().contains("not a role"));
        assert!(save_custom_in(&dir, &Draft { instructions: "short".into(), ..draft("support", &[]) }, &builtin).unwrap_err().contains("instructions"));
        assert!(!dir.join("support.md").exists(), "a refused role writes nothing");
        assert!(!dir.join(".check").exists(), "and leaves no scratch folder behind");
        // A good one is written, and is a member of the team that is read from the folder.
        save_custom_in(&dir, &draft("support", &["qa", "reviewer"]), &team(&dir)).unwrap();
        assert!(team(&dir).iter().any(|r| r.id == "support"));
        // Saving again changes it rather than clashing with itself.
        save_custom_in(&dir, &Draft { summary: "Changed".into(), ..draft("support", &["qa"]) }, &team(&dir)).unwrap();
        assert_eq!(team(&dir).iter().find(|r| r.id == "support").unwrap().summary, "Changed");
        // Another of the person's roles may hand work to it; then it cannot be removed from under that one.
        save_custom_in(&dir, &draft("triage", &["support"]), &team(&dir)).unwrap();
        assert!(delete_custom_in(&dir, "support", &team(&dir)).unwrap_err().contains("triage still hands work to support"));
        delete_custom_in(&dir, "triage", &team(&dir)).unwrap();
        delete_custom_in(&dir, "support", &team(&dir)).unwrap();
        assert!(team(&dir).iter().all(|r| is_built_in(r.id)));
        assert!(delete_custom_in(&dir, "support", &team(&dir)).unwrap_err().contains("no role named"));
        assert!(delete_custom_in(&dir, "planner", &team(&dir)).unwrap_err().contains("devd's own"));
    }

    #[test]
    fn a_role_with_windows_line_endings_parses() {
        let raw = include_str!("../roles/planner.md").replace('\n', "\r\n");
        let role = parse("planner", &raw).expect("a CRLF role parses");
        assert_eq!(role.id, "planner");
        assert_eq!(role.title, "Planner");
    }

    #[test]
    fn every_built_in_role_parses_and_matches_its_file() {
        let roles: Vec<Role> = all().iter().filter(|r| is_built_in(r.id)).copied().collect();
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
        // Every specialist may change its own task copy. Only the planner,
        // which plans and hands work on, is barred from changing files.
        for r in all().iter().filter(|r| r.id != "planner") {
            assert!(r.writes, "{} must be able to change its task copy", r.id);
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
