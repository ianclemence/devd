//! A teammate's soul: what a role knows about *itself*, kept across runs.
//!
//! The role files say who someone is and how they work - "You are QA. Establish
//! whether the work meets its acceptance criteria." That is identity, and it is
//! the same for every project, so it lives in the binary.
//!
//! What a role file cannot say is what *this* QA has learned: that this project's
//! test suite needs a database on a port, that the linter rejects a construct the
//! framework docs recommend, that last time the fix that worked was to run the
//! migration first. Ghost calls the equivalent file a soul, and the name is apt:
//! it is the part that accumulates.
//!
//! The separation matters. Identity is compiled in and reviewed like code; the
//! soul is written at runtime, per role and per project, and a person can read it,
//! correct it, or throw it away. An agent that writes its own soul is not
//! learning, it is drifting - so only devd writes here, and only from what a run
//! actually proved.
//!
//! ```text
//! <devd home>/souls/<project>/<role>.md
//! ```

use std::path::{Path, PathBuf};

use devd_core::Result;

/// How much of a soul travels into a prompt. A soul that does not fit is one that
/// has stopped being a distillation.
const MAX_SOUL: usize = 1500;

/// Where a role's accumulated experience lives.
pub struct Souls {
    root: PathBuf,
}

impl Souls {
    /// The souls for a devd home.
    pub fn at(home: &Path) -> Self {
        Souls { root: home.join("souls") }
    }

    fn dir(&self, project: &str) -> PathBuf {
        let key: String = project
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let key = key.trim_matches('-').to_string();
        let key = if key.len() > 40 { key[key.len() - 40..].to_string() } else { key };
        self.root.join(if key.is_empty() { "unnamed".to_string() } else { key })
    }

    /// The file for one role in one project.
    pub fn path(&self, project: &str, role: &str) -> PathBuf {
        let role = role.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>();
        self.dir(project).join(format!("{}.md", if role.is_empty() { "role".to_string() } else { role }))
    }

    /// What this teammate has learned here, ready to paste into its brief.
    ///
    /// Empty when it has learned nothing, so a first run is not handed an empty
    /// heading to interpret as an instruction.
    pub fn read(&self, project: &str, role: &str) -> String {
        let text = std::fs::read_to_string(self.path(project, role)).unwrap_or_default();
        let body = text.trim();
        if body.is_empty() {
            return String::new();
        }
        if body.chars().count() <= MAX_SOUL {
            body.to_string()
        } else {
            body.chars().take(MAX_SOUL).collect::<String>() + "\n_(older lines cut)_"
        }
    }

    /// Add one line of experience, if it is not already there.
    ///
    /// Returns whether it was new. Comparison is on the words, so the same lesson
    /// written twice with different punctuation does not stack up.
    pub fn remember(&self, project: &str, role: &str, lesson: &str) -> Result<bool> {
        let lesson = lesson.trim();
        if lesson.is_empty() || lesson.chars().count() > 240 {
            return Ok(false);
        }
        let path = self.path(project, role);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let norm = |s: &str| -> String {
            s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ').collect()
        };
        let want = norm(lesson);
        for line in existing.lines() {
            if norm(line.trim_start_matches("- ")) == want {
                return Ok(false);
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| devd_core::DevdError::internal(format!("could not create {}: {e}", parent.display())))?;
        }
        let mut body = if existing.is_empty() {
            "# What this teammate has learned here\n\nWritten by devd from what a run proved. A person may edit or delete this file; nothing regenerates it from scratch.\n\n".to_string()
        } else {
            existing
        };
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("- {lesson}\n"));
        std::fs::write(&path, body)
            .map_err(|e| devd_core::DevdError::internal(format!("could not write {}: {e}", path.display())))?;
        Ok(true)
    }

    /// The section a brief carries, or empty.
    pub fn section(&self, project: &str, role: &str) -> String {
        let soul = self.read(project, role);
        if soul.is_empty() {
            String::new()
        } else {
            format!("\n# What you have learned in this project (from your own past work)\n{soul}\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn souls() -> (tempfile::TempDir, Souls) {
        let d = tempfile::tempdir().unwrap();
        let s = Souls::at(d.path());
        (d, s)
    }

    #[test]
    fn a_first_run_has_no_soul_and_says_nothing() {
        let (_d, s) = souls();
        assert_eq!(s.read("D:/work/shop", "qa"), "");
        assert_eq!(s.section("D:/work/shop", "qa"), "", "an empty soul adds no heading");
    }

    #[test]
    fn what_a_role_learns_persists_and_is_written_once() {
        let (_d, s) = souls();
        let p = "D:/work/shop";
        assert!(s.remember(p, "qa", "The suite needs TEST_DATABASE_URL set.").unwrap());
        // "QA " and "qa" are different files on a case-sensitive filesystem and
        // the same file on Windows, so the test uses the exact role name rather
        // than depending on which of the two it is running on.
        assert!(!s.remember(p, "qa", "the suite needs TEST_DATABASE_URL set").unwrap(), "the same lesson twice");
        let soul = s.read(p, "qa");
        assert_eq!(soul.matches("TEST_DATABASE_URL").count(), 1, "{soul}");
    }

    #[test]
    fn one_role_does_not_inherit_anothers_experience() {
        let (_d, s) = souls();
        let p = "D:/work/shop";
        s.remember(p, "qa", "Tests need DATABASE_URL.").unwrap();
        s.remember(p, "frontend", "The design tokens live in src/theme.").unwrap();
        assert!(s.read(p, "qa").contains("DATABASE_URL"));
        assert!(!s.read(p, "qa").contains("design tokens"), "a tester is not told the frontend's habits");
        assert!(s.read(p, "frontend").contains("design tokens"));
    }

    #[test]
    fn two_projects_do_not_share_a_role() {
        let (_d, s) = souls();
        s.remember("D:/work/shop", "qa", "Shop fact.").unwrap();
        s.remember("D:/work/shop-api", "qa", "Api fact.").unwrap();
        assert_ne!(s.path("D:/work/shop", "qa"), s.path("D:/work/shop-api", "qa"));
        assert!(s.read("D:/work/shop", "qa").contains("Shop fact"));
        assert!(s.read("D:/work/shop-api", "qa").contains("Api fact"));
        assert!(!s.read("D:/work/shop", "qa").contains("Api fact"));
    }

    #[test]
    fn the_section_is_a_heading_plus_the_lessons() {
        let (_d, s) = souls();
        s.remember("D:/work/shop", "implementer", "Migrations run before tests.").unwrap();
        let sec = s.section("D:/work/shop", "implementer");
        assert!(sec.contains("What you have learned"), "{sec}");
        assert!(sec.contains("Migrations run before tests."), "{sec}");
    }

    #[test]
    fn a_soul_that_grows_past_the_limit_is_cut_not_dropped() {
        let (_d, s) = souls();
        for i in 0..60 {
            s.remember("D:/work/shop", "qa", &format!("lesson number {i} about this project’s behaviour")).unwrap();
        }
        let soul = s.read("D:/work/shop", "qa");
        assert!(soul.chars().count() <= MAX_SOUL + 40, "kept bounded: {}", soul.chars().count());
        assert!(soul.contains("lesson number 0"), "the oldest is kept, not the newest only");
        assert!(soul.contains("cut"), "and it says it was cut");
    }

    #[test]
    fn a_role_name_cannot_become_a_path() {
        let (_d, s) = souls();
        let p = s.path("D:/work/shop", "../../etc/passwd");
        assert!(p.to_string_lossy().contains("etcpasswd"), "the separators are gone: {p:?}");
        assert!(!p.to_string_lossy().contains(".."), "{p:?}");
    }
}
