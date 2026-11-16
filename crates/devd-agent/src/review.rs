//! A review fork: the cheap pass after work that may propose, never decide.
//!
//! The expensive part of a run is the work. The valuable part is what the next run should not
//! have to learn again. This module keeps the second part small and safe:
//!
//! * **Staged, not written.** A proposal waits in `<devd home>/reviews/<run>.json`. It is not in
//!   any prompt until a person accepts it.
//! * **Small.** At most two memory lines and at most one skill patch per run. A fork that
//!   proposes twelve things is a fork nobody reads.
//! * **From the record.** The automatic pass reads the lessons the team already reported
//!   (`team.report`'s `lesson` field) and the run's own evidence, not a model's opinion about a
//!   model.
//! * **Only a person accepts.** `review.accept` is human-only, exactly like the persona: devd can
//!   notice a pattern, but it does not get to decide it is how everyone should work.
//!
//! This is the learning loop's low-risk half. A skill proposal is a file a person edits before it
//! becomes instructions; accepting it is `skills.save`, with the same lock rules as any other
//! skill.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use devd_core::{DevdError, Paths, Result, Store};

use crate::memory::Memory;
use crate::skills;

/// The most memory lines one review may propose.
pub const MAX_MEMORY: usize = 2;

/// A skill patch proposed for a person to read, edit, and accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillProposal {
    /// Its name, lowercase and hyphenated.
    pub name: String,
    /// One line.
    pub description: String,
    /// The stage of work; see [`crate::skills::KINDS`].
    pub kind: String,
    /// Roles it is for; empty means all of them.
    pub roles: Vec<String>,
    /// The instructions.
    pub body: String,
    /// Why this run suggested it.
    pub reason: String,
}

/// What a review fork proposed after one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staged {
    /// The run the proposals belong to.
    pub run: String,
    /// The project the run was about, when it had one.
    pub project: Option<String>,
    /// When the fork ran.
    pub created_at: String,
    /// At most [`MAX_MEMORY`] lines for the project's memory.
    pub memory: Vec<String>,
    /// At most one skill patch.
    pub skill: Option<SkillProposal>,
    /// What a person has already accepted, so it is not written twice.
    #[serde(default)]
    pub accepted: Vec<String>,
  }

impl Staged {
    /// Whether there is anything left to decide.
    pub fn pending(&self) -> bool {
        self.memory.iter().any(|m| !self.accepted.contains(m))
            || self
                .skill
                .as_ref()
                .is_some_and(|s| !self.accepted.contains(&s.name))
    }

    /// The proposal as a screen reads it.
    pub fn as_json(&self) -> Value {
        json!({
            "run": self.run,
            "project": self.project,
            "created_at": self.created_at,
            "memory": self.memory,
            "skill": self.skill,
            "accepted": self.accepted,
            "pending": self.pending(),
        })
    }
}

fn dir(paths: &Paths) -> PathBuf {
    paths.root().join("reviews")
}

fn file(paths: &Paths, run: &str) -> PathBuf {
    dir(paths).join(format!("{run}.json"))
}

/// Every staged review, newest first.
pub fn list(paths: &Paths) -> Vec<Staged> {
    let mut out: Vec<Staged> = std::fs::read_dir(dir(paths))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .filter_map(|t| serde_json::from_str::<Staged>(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// One run's staged review.
pub fn get(paths: &Paths, run: &str) -> Result<Staged> {
    let text = std::fs::read_to_string(file(paths, run)).map_err(|_| {
        DevdError::not_found(format!("there is no staged review for run {run}"))
    })?;
    serde_json::from_str(&text)
        .map_err(|e| DevdError::internal(format!("the staged review for {run} is not readable: {e}")))
}

fn save(paths: &Paths, staged: &Staged) -> Result<()> {
    let path = file(paths, &staged.run);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(staged).map_err(|e| DevdError::internal(e.to_string()))?,
    )?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// The project a run was about, if it had one.
fn project_of(store: &Store, run: &str) -> Option<String> {
    store
        .get_run(run)
        .ok()
        .flatten()
        .and_then(|r| r.project)
}

/// Stage what a fork noticed. Nothing is applied.
///
/// Refuses a fourth memory line and a second skill: the point of the cap is that a person can
/// read all of it in one sitting.
pub fn stage(
    paths: &Paths,
    store: &Store,
    run: &str,
    memory: &[String],
    skill: Option<SkillProposal>,
) -> Result<Staged> {
    if memory.len() > MAX_MEMORY {
        return Err(DevdError::invalid(format!(
            "a review proposes at most {MAX_MEMORY} memory lines; this one has {}",
            memory.len()
        )));
    }
    let mut staged = get(paths, run).unwrap_or(Staged {
        run: run.to_string(),
        project: project_of(store, run),
        created_at: devd_core::util::now(),
        memory: Vec::new(),
        skill: None,
        accepted: Vec::new(),
    });
    if let Some(new) = skill {
        if let Some(old) = &staged.skill {
            if old.name != new.name {
                return Err(DevdError::conflict(format!(
                    "this run already has a skill proposal (`{}`); accept or drop it before staging another",
                    old.name
                )));
            }
        }
        // A name that already exists is not a proposal, it is an edit. Refuse it here so the
        // person sees the clash rather than an accepted proposal that quietly overwrote a skill.
        if crate::skills::builtin_description(&new.name).is_some() {
            return Err(DevdError::conflict(format!(
                "`{}` is a built-in skill; a proposal cannot replace one",
                new.name
            )));
        }
        if crate::skills::get(paths, &new.name).is_ok() {
            return Err(DevdError::conflict(format!(
                "a skill named `{}` already exists; a proposal is a new skill, not an edit",
                new.name
            )));
        }
        if staged.accepted.iter().any(|a| a == &new.name) {
            return Err(DevdError::conflict(format!(
                "the skill proposal `{}` was already accepted",
                new.name
            )));
        }
        staged.skill = Some(new);
    }
    for line in memory {
        let line = line.trim();
        if line.is_empty() || line.chars().count() > 240 {
            continue;
        }
        if !staged
            .memory
            .iter()
            .any(|m| m.eq_ignore_ascii_case(line))
        {
            staged.memory.push(line.to_string());
        }
    }
    if staged.memory.len() > MAX_MEMORY {
        staged.memory.truncate(MAX_MEMORY);
    }
    if staged.memory.is_empty() && staged.skill.is_none() {
        return Err(DevdError::invalid(
            "a review proposes something: give 0-2 memory lines, or one skill patch",
        ));
    }
    save(paths, &staged)?;
    Ok(staged)
}

/// A lesson that reads like a rule is worth a skill; a lesson that is a fact is worth memory.
fn rule_like(line: &str) -> bool {
    let l = line.to_lowercase();
    [
        "always ",
        "never ",
        "must ",
        "before you ",
        "instead of ",
        "do not ",
        "don't ",
        "when the ",
        "when a ",
    ]
    .iter()
    .any(|m| l.contains(m))
}

/// A short name from a run id: readable, unique enough, and never a path.
fn proposal_name(run: &str) -> String {
    let tail: String = run
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!(
        "lessons-{}",
        if tail.is_empty() { "run" } else { &tail }
    )
}

/// The lessons the team reported on a run, in the order they were written.
pub fn lessons(store: &Store, run: &str) -> Vec<String> {
    store
        .list_team_events(run, 0, 5000)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.kind == "report")
        .filter_map(|e| e.data["lesson"].as_str().map(str::to_string))
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// Stage proposals from what the team already reported. Deterministic, and safe to call twice.
///
/// The fork proposes 0-2 memory lines from the run's lessons, and one skill patch only when at
/// least two lessons read like rules rather than facts. It never writes a soul, a fact or a
/// skill itself: accepting is a person's step.
pub fn after_run(
    paths: &Paths,
    store: &Store,
    run: &str,
    lessons: &[String],
) -> Result<Option<Staged>> {
    if lessons.is_empty() {
        return Ok(None);
    }
    let memory = Memory::at(paths.root());
    let known: Vec<String> = memory
        .proposals()
        .into_iter()
        .chain(
            project_of(store, run)
                .map(|p| memory.digest(&p))
                .unwrap_or_default()
                .lines()
                .map(str::to_string),
        )
        .collect();
    let mut facts: Vec<String> = Vec::new();
    let mut rules: Vec<String> = Vec::new();
    for lesson in lessons {
        let line = lesson.trim();
        if line.is_empty() || line.chars().count() > 240 {
            continue;
        }
        if known.iter().any(|k| k.eq_ignore_ascii_case(line)) {
            continue;
        }
        if rule_like(line) {
            rules.push(line.to_string());
        } else {
            facts.push(line.to_string());
        }
        if facts.len() + rules.len() >= MAX_MEMORY * 2 {
            break;
        }
    }
    let mut memory_lines: Vec<String> = facts.into_iter().chain(rules.clone()).take(MAX_MEMORY).collect();
    memory_lines.dedup();
    let skill = (rules.len() >= 2).then(|| SkillProposal {
        name: proposal_name(run),
        description: "What this project taught the team, written down as a rule".to_string(),
        kind: "house".to_string(),
        roles: Vec::new(),
        body: format!(
            "# Lessons from a run\n\n## When to use\n\nBefore starting work in this project, read what the previous run had to learn the hard way.\n\n## Procedure\n\n{}\n\n## Verification\n\nEach rule above names its own test. If a rule cannot be checked, it is an opinion and should not be here.\n",
            rules
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        reason: format!(
            "{} rule-shaped lesson(s) were reported on this run; a person decides whether they are instructions or just this run's history.",
            rules.len()
        ),
    });
    if memory_lines.is_empty() && skill.is_none() {
        return Ok(None);
    }
    // The completion gate can run more than once for one run. Staging the same proposal again
    // would add nothing and would read like new work, so say so instead.
    if let Ok(existing) = get(paths, run) {
        let same_memory = memory_lines
            .iter()
            .all(|m| existing.memory.iter().any(|e| e.eq_ignore_ascii_case(m)));
        let same_skill = match (&existing.skill, &skill) {
            (Some(a), Some(b)) => a.name == b.name,
            (None, None) => true,
            _ => false,
        };
        if same_memory && same_skill {
            return Ok(None);
        }
    }
    stage(paths, store, run, &memory_lines, skill).map(Some)
}

/// Apply a staged review. Human only: this is the step that changes every agent's memory or
/// skills.
pub fn accept(paths: &Paths, store: &Store, run: &str, which: Option<&str>) -> Result<Staged> {
    let mut staged = get(paths, run)?;
    let which = which.unwrap_or("all");
    let accept_memory = matches!(which, "all" | "memory");
    let accept_skill = matches!(which, "all" | "skill")
        || staged.skill.as_ref().is_some_and(|s| s.name == which);
    if !accept_memory && !accept_skill {
        return Err(DevdError::invalid(format!(
            "`which` is `all`, `memory`, `skill`, or a proposed skill's name; this review has {}",
            staged
                .skill
                .as_ref()
                .map(|s| format!("`{}`", s.name))
                .unwrap_or_else(|| "no skill".to_string())
        )));
    }
    let project = staged
        .project
        .clone()
        .or_else(|| project_of(store, run))
        .unwrap_or_else(|| "unnamed".to_string());
    if accept_memory {
        let memory = Memory::at(paths.root());
        for line in staged.memory.clone() {
            if staged.accepted.contains(&line) {
                continue;
            }
            memory.remember(&project, &line)?;
            staged.accepted.push(line);
        }
    }
    if accept_skill {
        if let Some(skill) = staged.skill.clone() {
            if !staged.accepted.contains(&skill.name) {
                crate::curator::ensure_editable(paths, &skill.name)?;
                if skills::get(paths, &skill.name).is_ok() {
                    return Err(DevdError::conflict(format!(
                        "a skill named `{}` appeared after this review was staged; rename or remove it before accepting",
                        skill.name
                    )));
                }
                skills::save(
                    paths,
                    &skill.name,
                    &skill.description,
                    &skill.kind,
                    &skill.roles,
                    &skill.body,
                )?;
                crate::curator::note_event(
                    paths,
                    &skill.name,
                    "accepted",
                    &format!("review {run}"),
                );
                staged.accepted.push(skill.name);
            }
        }
    }
    save(paths, &staged)?;
    Ok(staged)
}

/// Throw a staged review away. Human only, and it writes nothing else.
pub fn drop_review(paths: &Paths, run: &str) -> Result<Staged> {
    let staged = get(paths, run)?;
    let path = file(paths, run);
    std::fs::remove_file(&path)
        .map_err(|e| DevdError::internal(format!("could not remove {}: {e}", path.display())))?;
    Ok(staged)
}

/// Whether a name is already proposed by some run, so two runs do not stage the same skill.
pub fn proposed_elsewhere(paths: &Paths, name: &str, except_run: &str) -> Option<String> {
    list(paths)
        .into_iter()
        .find(|s| {
            s.run != except_run
                && s.skill.as_ref().is_some_and(|p| p.name == name)
                && !s.accepted.contains(&name.to_string())
        })
        .map(|s| s.run)
}

/// The path a proposal would become, for a screen.
pub fn proposed_skill_path(paths: &Paths, name: &str) -> PathBuf {
    skills::user_dir(paths).join(format!("{name}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp() -> (tempfile::TempDir, Paths, Store) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::rooted(d.path());
        p.ensure().unwrap();
        let s = Store::open_in_memory().unwrap();
        (d, p, s)
    }

    fn run(store: &Store, project: Option<&Path>) -> String {
        store
            .create_run(
                "t",
                "Recreate a site",
                project.map(|p| p.display().to_string()).as_deref(),
                1,
                "human:test",
            )
            .unwrap()
            .id
    }

    #[test]
    fn a_review_stages_at_most_two_facts_and_at_most_one_skill() {
        let (d, p, s) = temp();
        let project = d.path().join("shop");
        std::fs::create_dir_all(&project).unwrap();
        let run = run(&s, Some(&project));
        let skill = SkillProposal {
            name: "lessons-shop".into(),
            description: "What the shop taught".into(),
            kind: "house".into(),
            roles: vec![],
            body: "A body long enough to be read.".into(),
            reason: "two rules".into(),
        };
        let staged = stage(
            &p,
            &s,
            &run,
            &["the suite needs a database".to_string(), "run migrations first".to_string()],
            Some(skill.clone()),
        )
        .unwrap();
        assert_eq!(staged.memory.len(), 2);
        assert_eq!(staged.skill, Some(skill));
        assert!(staged.pending());
        // A third memory line is refused rather than silently kept.
        let err = stage(&p, &s, &run, &["a".into(), "b".into(), "c".into()], None).unwrap_err();
        assert!(err.to_string().contains("at most 2"));
        // A second, different skill is refused: one patch per review.
        let other = SkillProposal { name: "other".into(), ..staged.skill.clone().unwrap() };
        assert!(stage(&p, &s, &run, &[], Some(other)).is_err());
        // Nothing was written where an agent could read it.
        assert!(skills::get(&p, "lessons-shop").is_err());
        assert!(crate::memory::Memory::at(p.root()).digest(&project.display().to_string()).is_empty());
    }

    #[test]
    fn accepting_a_review_writes_the_memory_and_the_skill_once() {
        let (d, p, s) = temp();
        let project = d.path().join("shop");
        std::fs::create_dir_all(&project).unwrap();
        let run = run(&s, Some(&project));
        let skill = SkillProposal {
            name: "shop-lessons".into(),
            description: "What the shop taught".into(),
            kind: "house".into(),
            roles: vec![],
            body: "Always run the migrations first.".into(),
            reason: "a rule".into(),
        };
        stage(&p, &s, &run, &["the suite needs a database".to_string()], Some(skill)).unwrap();
        assert!(skills::get(&p, "shop-lessons").is_err(), "staged is not installed");

        let accepted = accept(&p, &s, &run, Some("all")).unwrap();
        assert_eq!(accepted.accepted.len(), 2);
        assert!(skills::get(&p, "shop-lessons").is_ok(), "accepting installs it");
        let digest = crate::memory::Memory::at(p.root())
            .rebuild_digest(&project.display().to_string())
            .unwrap();
        assert!(digest.contains("database"), "the fact was remembered: {digest}");
        // Accepting twice writes nothing twice.
        let again = accept(&p, &s, &run, Some("all")).unwrap();
        assert_eq!(again.accepted.len(), 2);
    }

    #[test]
    fn an_automatic_fork_stages_lessons_and_skips_what_is_already_known() {
        let (_d, p, s) = temp();
        let run = run(&s, Some(Path::new("D:/work/shop")));
        let lessons = vec![
            "Always run the migration before the suite.".to_string(),
            "Never commit the local database file.".to_string(),
        ];
        let staged = after_run(&p, &s, &run, &lessons).unwrap().unwrap();
        assert!(staged.memory.len() <= MAX_MEMORY);
        let skill = staged.skill.expect("two rule-shaped lessons make one proposal");
        assert_eq!(skill.kind, "house");
        assert!(skill.body.contains("migration"), "{}", skill.body);
        // Running it again stages nothing new.
        let again = after_run(&p, &s, &run, &lessons).unwrap();
        assert!(again.is_none() || again.unwrap().memory.is_empty());
    }

    #[test]
    fn a_fact_alone_is_memory_and_does_not_become_a_skill() {
        let (_d, p, s) = temp();
        let run = run(&s, Some(Path::new("D:/work/shop")));
        let staged = after_run(&p, &s, &run, &["The API listens on port 4310.".to_string()])
            .unwrap()
            .unwrap();
        assert_eq!(staged.memory, vec!["The API listens on port 4310.".to_string()]);
        assert!(staged.skill.is_none());
    }
}
