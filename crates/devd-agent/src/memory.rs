//! What devd and its team remember.
//!
//! Two references shaped this. TencentDB Agent Memory layers memory instead of
//! storing a flat pile: raw conversation, then the facts extracted from it, then
//! a scene that groups them, then a persona summarising the lot - each level
//! readable, and the level below kept so a summary can always be checked against
//! what actually happened. Ghost keeps its memory as Markdown files the owner can
//! read and edit, with a daily note and a digest generated from them.
//!
//! devd follows both where they agree:
//!
//! * **Files, not a vector store.** A memory a person cannot read is a memory they
//!   cannot correct. Everything here is Markdown under the devd home, so it opens
//!   in an editor and survives the database being replaced.
//! * **Layered, with the evidence kept.** L0 notes are written as they happen;
//!   L2 is the distilled view. The layers are regenerated from the ones below, so
//!   a wrong summary is a bug in the summary, never a lost fact.
//! * **Retrieval is gated.** Ghost measured an embedding call at ~2.8s on every
//!   message including "hey", and stopped doing it. devd has no embeddings at
//!   all; what it has is a cheap keyword test, and it only recalls when the
//!   message could plausibly depend on something remembered.
//!
//! What is remembered, and by whom:
//!
//! | Layer | Scope | Who writes it |
//! |---|---|---|
//! | L0 note | one project, one day | devd, after a run ends |
//! | L1 fact | one project | devd, from a run's outcome |
//! | L2 digest | one project | regenerated from L1 |
//! | persona | the person | devd proposes; **only a person accepts** |

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use devd_core::{DevdError, Result};

/// How much of a memory file is handed to a model. A digest that does not fit is
/// a digest that has stopped being a summary.
const MAX_SECTION: usize = 4000;

/// The day, for a note's date.
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// At most `n` characters, marked when something was dropped.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    format!("{cut}\n\n_(cut short)_")
}
/// The first line of a fact, for a plan's summary line.
pub fn clip_for_plan(fact: &str) -> String {
    fact.lines().next().unwrap_or("").chars().take(90).collect()
}
/// A memory store rooted at one devd home.
pub struct Memory {
    root: PathBuf,
}

impl Memory {
    /// The memory for a devd home.
    pub fn at(home: &Path) -> Self {
        Memory { root: home.join("memory") }
    }

    /// Where a project's memory lives. The project's own folder is the key, so
    /// two projects with the same display name never share memory.
    fn project_dir(&self, project: &str) -> PathBuf {
        let key = project
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>();
        let key = key.trim_matches('-').to_string();
        // Keep the last 40 characters: the tail of a path is the part that
        // identifies it, and it keeps a deep path from overflowing Windows.
        let key = if key.len() > 40 { key[key.len() - 40..].to_string() } else { key };
        self.root.join(if key.is_empty() { "unnamed".to_string() } else { key })
    }

    fn notes_path(&self, project: &str) -> PathBuf {
        self.project_dir(project).join("notes.md")
    }

    fn facts_path(&self, project: &str) -> PathBuf {
        self.project_dir(project).join("facts.md")
    }

    fn digest_path(&self, project: &str) -> PathBuf {
        self.project_dir(project).join("digest.md")
    }

    /// The person's persona for the whole of devd, not one project.
    pub fn persona_path(&self) -> PathBuf {
        self.root.join("persona.md")
    }

    /// What is remembered about how this person works.
    pub fn persona(&self) -> String {
        std::fs::read_to_string(self.persona_path()).unwrap_or_default()
    }

    /// Lines proposed for the persona but not yet accepted.
    ///
    /// Separate from the persona itself on purpose: a proposed line is not in
    /// any prompt until a person takes it. That is the whole safeguard - devd
    /// can notice a pattern, but it does not get to decide it is who you are.
    fn proposals_path(&self) -> PathBuf {
        self.root.join("persona.proposed.md")
    }

    /// What devd has noticed and would like to remember about how you work.
    pub fn proposals(&self) -> Vec<String> {
        std::fs::read_to_string(self.proposals_path())
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("- "))
            .map(|l| l[2..].trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// Propose one line for the persona. Returns whether it is new.
    ///
    /// A line already in the persona, or already proposed, is not proposed again.
    pub fn propose(&self, line: &str) -> Result<bool> {
        let line = line.trim();
        if line.is_empty() || line.chars().count() > 200 {
            return Ok(false);
        }
        let norm = |s: &str| -> String {
            s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect()
        };
        let want = norm(line);
        for known in self.persona().lines().chain(std::fs::read_to_string(self.proposals_path()).unwrap_or_default().lines()) {
            if norm(known.trim_start_matches("- ")) == want {
                return Ok(false);
            }
        }
        std::fs::create_dir_all(&self.root)
            .map_err(|e| DevdError::internal(format!("could not create {}: {e}", self.root.display())))?;
        let mut body = std::fs::read_to_string(self.proposals_path()).unwrap_or_else(|_| {
            "# Proposed, not yet part of who devd thinks you are\n\nAccept one with memory.persona_accept, or drop it with memory.persona_drop.\n\n".to_string()
        });
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("- {line}\n"));
        std::fs::write(self.proposals_path(), body)
            .map_err(|e| DevdError::internal(format!("could not write the proposals: {e}")))?;
        Ok(true)
    }

    /// Take a proposal into the persona. Returns whether it was there to take.
    ///
    /// This is the only way a line becomes part of the persona, and it is
    /// reachable only by a person - see the `memory.persona_accept` capability.
    pub fn accept(&self, line: &str) -> Result<bool> {
        let line = line.trim();
        let pending = self.proposals();
        if !pending.iter().any(|p| p == line) {
            return Ok(false);
        }
        let mut persona = self.persona();
        if persona.trim().is_empty() {
            persona = "# How this person works\n\nAccepted by them, from what devd noticed. Every line here was chosen.\n\n".to_string();
        }
        if !persona.ends_with('\n') {
            persona.push('\n');
        }
        persona.push_str(&format!("- {line}\n"));
        std::fs::write(self.persona_path(), &persona)
            .map_err(|e| DevdError::internal(format!("could not write the persona: {e}")))?;
        self.rewrite_proposals(&pending.into_iter().filter(|p| p != line).collect::<Vec<_>>())?;
        Ok(true)
    }

    /// Drop a proposal without taking it.
    pub fn drop_proposal(&self, line: &str) -> Result<bool> {
        let pending = self.proposals();
        if !pending.iter().any(|p| p == line.trim()) {
            return Ok(false);
        }
        self.rewrite_proposals(&pending.into_iter().filter(|p| p != line.trim()).collect::<Vec<_>>())?;
        Ok(true)
    }

    fn rewrite_proposals(&self, lines: &[String]) -> Result<()> {
        let mut body = "# Proposed, not yet part of who devd thinks you are\n\nAccept one with memory.persona_accept, or drop it with memory.persona_drop.\n\n".to_string();
        for l in lines {
            body.push_str(&format!("- {l}\n"));
        }
        std::fs::write(self.proposals_path(), body)
            .map_err(|e| DevdError::internal(format!("could not write the proposals: {e}")))
    }
    /// A project's distilled memory.
    pub fn digest(&self, project: &str) -> String {
        std::fs::read_to_string(self.digest_path(project)).unwrap_or_default()
    }

    /// Record that a run ended, and what it produced.
    ///
    /// This is the L0 write: the raw note, cheap, appended, never summarised on
    /// the way in. Everything above it is derived from these.
    pub fn note_run(&self, project: &str, run: &str, state: &str, summary: &str) -> Result<()> {
        let dir = self.project_dir(project);
        std::fs::create_dir_all(&dir).map_err(|e| DevdError::internal(format!("could not create {}: {e}", dir.display())))?;
        let path = self.notes_path(project);
        let mut existing = std::fs::read_to_string(&path).unwrap_or_else(|_| "# Notes\n\nEach line is a run that ended, oldest first.\n\n".to_string());
        let day = today();
        let line = format!(
            "- {day} · **{state}** · {run} · {}",
            summary.lines().next().unwrap_or("").chars().take(160).collect::<String>()
        );
        existing.push_str(&line);
        existing.push('\n');
        std::fs::write(&path, existing).map_err(|e| DevdError::internal(format!("could not write {}: {e}", path.display())))
    }

    /// Add a fact the team learned, if it is not already known.
    ///
    /// A fact is one short sentence about the project: a convention, a command
    /// that works, a trap. Duplicates are dropped by comparing the text, because
    /// a memory that repeats itself is a memory nobody reads.
    pub fn remember(&self, project: &str, fact: &str) -> Result<bool> {
        let fact = fact.trim();
        if fact.is_empty() || fact.chars().count() > 300 {
            return Ok(false);
        }
        let dir = self.project_dir(project);
        std::fs::create_dir_all(&dir).map_err(|e| DevdError::internal(format!("could not create {}: {e}", dir.display())))?;
        let path = self.facts_path(project);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect::<String>();
        let want = norm(fact);
        for line in existing.lines() {
            if norm(line.trim_start_matches("- ")) == want {
                return Ok(false);
            }
        }
        let mut body = if existing.is_empty() { "# Facts\n\nWhat the team learned about this project.\n\n".to_string() } else { existing };
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("- {fact}\n"));
        std::fs::write(&path, body).map_err(|e| DevdError::internal(format!("could not write {}: {e}", path.display())))?;
        Ok(true)
    }

    /// Rebuild the digest from the facts and the recent notes.
    ///
    /// L2 is derived, never edited by hand: a wrong digest is a bug in this
    /// function, and the facts it came from are still on disk to prove it.
    pub fn rebuild_digest(&self, project: &str) -> Result<String> {
        let facts = std::fs::read_to_string(self.facts_path(project)).unwrap_or_default();
        let notes = std::fs::read_to_string(self.notes_path(project)).unwrap_or_default();
        let mut out = String::from("# What devd knows about this project\n\n");
        out.push_str("Derived from `facts.md` and `notes.md`; edit those, not this.\n\n");
        let fact_lines: Vec<&str> = facts.lines().filter(|l| l.starts_with("- ")).collect();
        out.push_str("## Conventions and traps\n\n");
        if fact_lines.is_empty() {
            out.push_str("_Nothing learned yet._\n");
        } else {
            for l in fact_lines.iter().take(24) {
                out.push_str(l);
                out.push('\n');
            }
        }
        let note_lines: Vec<&str> = notes.lines().filter(|l| l.starts_with("- ")).collect();
        out.push_str("\n## Recent work\n\n");
        if note_lines.is_empty() {
            out.push_str("_No runs yet._\n");
        } else {
            // The last eight, newest last, read top-down.
            let start = note_lines.len().saturating_sub(8);
            for l in &note_lines[start..] {
                out.push_str(l);
                out.push('\n');
            }
        }
        let out = clip(&out, MAX_SECTION);
        let path = self.digest_path(project);
        std::fs::create_dir_all(self.project_dir(project)).ok();
        std::fs::write(&path, &out).map_err(|e| DevdError::internal(format!("could not write {}: {e}", path.display())))?;
        Ok(out)
    }

    /// Whether a message could plausibly depend on something remembered.
    ///
    /// This is the gate, and it is the point of the whole module: recalling on
    /// every message costs time and pollutes the prompt with context that does not
    /// apply. It is deliberately generous - a missed recall costs one tool call, a
    /// false recall costs the model's attention on every turn.
    pub fn should_recall(message: &str) -> bool {
        let m = message.to_lowercase();
        const CUES: &[&str] = &[
            "remember", "last time", "before", "previously", "again", "used to", "we tried",
            "didn't work", "does not work", "doesn't work", "keeps", "always", "never",
            "convention", "we use", "we decided", "as usual", "same as", "that thing",
            "why did", "what happened", "history", "earlier",
        ];
        CUES.iter().any(|c| m.contains(c))
    }

    /// The memory to put in front of a model for this message in this project.
    ///
    /// Empty when nothing is known or nothing is asked for, because a prompt full
    /// of inapplicable context is worse than an empty one.
    pub fn context_for(&self, project: Option<&str>, message: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        let persona = self.persona();
        if !persona.trim().is_empty() {
            parts.push(format!("## How this person works\n\n{}", clip(persona.trim(), 1200)));
        }
        if let Some(p) = project {
            let digest = self.digest(p);
            let relevant = Self::should_recall(message);
            if !digest.trim().is_empty() {
                let body = if relevant {
                    digest.trim().to_string()
                } else {
                    // Without a recall cue only the conventions travel, which are
                    // the part that applies to everything; the recent work does not.
                    digest.split("## Recent work").next().unwrap_or("").trim().to_string()
                };
                if !body.is_empty() {
                    parts.push(body);
                }
            }
        }
        parts.join("\n\n")
    }

    /// Everything on disk for one project, for the screen that shows it.
    pub fn describe(&self, project: &str) -> Value {
        let read = |p: PathBuf| std::fs::read_to_string(&p).unwrap_or_default();
        json!({
            "project": project,
            "dir": self.project_dir(project).display().to_string(),
            "digest": read(self.digest_path(project)),
            "facts": read(self.facts_path(project)),
            "notes": read(self.notes_path(project)),
            "persona": self.persona(),
            "proposed": std::fs::read_to_string(self.proposals_path()).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn mem() -> (tempfile::TempDir, Memory) {
        let d = tempfile::tempdir().unwrap();
        let m = Memory::at(d.path());
        (d, m)
    }

    #[test]
    fn a_run_is_noted_and_the_digest_is_built_from_the_notes() {
        let (_d, m) = mem();
        let p = "D:/work/shop";
        m.note_run(p, "Add search", "done", "The search box is in and the tests pass.").unwrap();
        m.note_run(p, "Add login", "paused", "No credential for the mail server.").unwrap();
        assert!(m.note_run(p, "Add search", "done", "The search box is in and the tests pass.").is_ok(), "a note is idempotent enough");
        let digest = m.rebuild_digest(p).unwrap();
        assert!(digest.contains("Add search"), "{digest}");
        assert!(digest.contains("Add login"), "{digest}");
        assert!(digest.contains("Recent work"), "{digest}");
    }

    #[test]
    fn a_fact_is_remembered_once() {
        let (_d, m) = mem();
        let p = "D:/work/shop";
        assert!(m.remember(p, "Tests run with `composer test`.").unwrap(), "first time");
        assert!(!m.remember(p, "tests run with `composer test`").unwrap(), "the same fact again, differently cased");
        let facts = std::fs::read_to_string(m.facts_path(p)).unwrap();
        assert_eq!(facts.matches("composer test").count(), 1, "{facts}");
    }

    #[test]
    fn the_gate_recalls_only_when_the_message_could_depend_on_memory() {
        assert!(Memory::should_recall("why did we always use pnpm here?"));
        assert!(Memory::should_recall("last time this broke it was the port"));
        assert!(Memory::should_recall("remember the convention for tests"));
        assert!(!Memory::should_recall("add a button to the header"));
        assert!(!Memory::should_recall("what is 2 + 2"));
    }

    #[test]
    fn without_a_cue_the_conventions_travel_but_the_recent_work_does_not() {
        let (_d, m) = mem();
        let p = "D:/work/shop";
        m.remember(p, "The API is under /api/v1.").unwrap();
        m.note_run(p, "Rename the router", "done", "Renamed it.").unwrap();
        m.rebuild_digest(p).unwrap();

        let plain = m.context_for(Some(p), "add a button");
        assert!(plain.contains("/api/v1"), "conventions apply to everything: {plain}");
        assert!(!plain.contains("Rename the router"), "recent work is not dumped in: {plain}");

        let recall = m.context_for(Some(p), "what did we change last time?");
        assert!(recall.contains("Rename the router"), "a recall cue brings the recent work: {recall}");
    }

    #[test]
    fn an_empty_memory_adds_nothing_to_a_prompt() {
        let (_d, m) = mem();
        assert_eq!(m.context_for(Some("D:/work/none"), "add a button"), "");
        assert_eq!(m.context_for(None, "hello"), "");
    }

    #[test]
    fn two_projects_with_similar_paths_do_not_share_memory() {
        let (_d, m) = mem();
        m.remember("D:/work/shop", "Shop fact.").unwrap();
        m.remember("D:/work/shop-api", "Api fact.").unwrap();
        let a = m.digest_path("D:/work/shop");
        let b = m.digest_path("D:/work/shop-api");
        assert_ne!(a, b, "different projects, different files");
        let _ = Arc::new(());
    }

    #[test]
    fn the_cycle_is_read_write_read() {
        // The point of the module: a run leaves something behind, and the next
        // question gets it. This walks the cycle end to end.
        let (_d, m) = mem();
        let p = "D:/work/shop";

        // Before any work, nothing is claimed.
        assert_eq!(m.context_for(Some(p), "why do we use pnpm?"), "");

        // A run finishes and leaves a convention and a note.
        m.note_run(p, "Set up the project", "done", "Chose pnpm; npm broke on the workspace.").unwrap();
        m.remember(p, "Use pnpm, not npm: the workspace needs it.").unwrap();
        m.rebuild_digest(p).unwrap();

        // The next turn that could depend on it is told.
        let ctx = m.context_for(Some(p), "why do we use pnpm again?");
        assert!(ctx.contains("Use pnpm"), "{ctx}");

        // And a turn that could not depend on it is not.
        let plain = m.context_for(Some(p), "add a button");
        assert!(!plain.contains("Set up the project"), "recent work is not forced in: {plain}");
    }

    #[test]
    fn a_member_of_the_team_inherits_what_the_team_learned() {
        // The difference between a team and a queue of strangers: the next member
        // starts from what the last one wrote down, not from nothing.
        let (_d, m) = mem();
        let p = "D:/work/shop";
        m.remember(p, "Migrations run with `make db-migrate`; `cargo run` skips them.").unwrap();
        m.rebuild_digest(p).unwrap();

        // This is what `task_brief` appends to every member's instructions.
        let digest = m.digest(p);
        assert!(digest.contains("make db-migrate"), "the brief carries the convention: {digest}");

        // A project with no memory adds no section at all.
        assert!(m.digest("D:/work/other").trim().is_empty());
    }

    #[test]
    fn a_proposal_is_not_part_of_who_you_are_until_it_is_accepted() {
        // The safeguard: devd may notice a pattern, but it does not get to decide
        // it is a fact about the person. Until a person takes the line, it is not
        // in any prompt.
        let (_d, m) = mem();
        assert!(m.propose("Prefers short answers with the reason first.").unwrap());
        assert!(!m.propose("prefers short answers with the reason first").unwrap(), "the same line twice");
        assert!(m.proposals().len() == 1);

        // Proposed is not accepted: the prompt says nothing yet.
        assert_eq!(m.context_for(None, "hello"), "", "a proposal never reaches a prompt");

        // Accepted, it becomes part of how the assistant addresses them.
        assert!(m.accept("Prefers short answers with the reason first.").unwrap());
        assert!(m.proposals().is_empty(), "taken lines leave the pending list");
        let ctx = m.context_for(None, "hello");
        assert!(ctx.contains("short answers"), "{ctx}");
    }

    #[test]
    fn a_proposal_can_be_dropped_and_a_line_not_proposed_cannot_be_accepted() {
        let (_d, m) = mem();
        m.propose("Likes long explanations.").unwrap();
        assert!(m.drop_proposal("Likes long explanations.").unwrap());
        assert!(m.proposals().is_empty());
        assert!(m.persona().trim().is_empty(), "dropping it wrote nothing");
        assert!(!m.accept("something that was never proposed").unwrap());
    }

    #[test]
    fn a_line_already_in_the_persona_is_not_proposed_again() {
        let (_d, m) = mem();
        m.propose("Wants tests before merging.").unwrap();
        m.accept("Wants tests before merging.").unwrap();
        assert!(!m.propose("wants tests before merging").unwrap(), "already known");
        assert!(m.proposals().is_empty());
    }
}
