//! Run templates: a goal written once, with blanks, that the team can be started on
//! again and again.
//!
//! A template is a Markdown file. The front matter names it, lists its parameters
//! and the definition of done every run of it carries; the body is the goal, with
//! `{{parameter}}` where a value goes. Five ship with devd (a spec to a reviewed
//! change, a bug fix, a feature, a security review, a release); your own go in
//! `<devd home>/templates/` in the same format and can add to the set but never
//! replace a built-in. Schedules start runs from templates.
//!
//! ```text
//! ---
//! id: fix-bug
//! title: Fix a bug
//! summary: Reproduce a reported bug with a test, fix the cause, prove nothing broke.
//! parallel: 1
//! params: report=What happens and how to see it; where?=Where it probably lives
//! done_when: A test that failed now passes | The whole suite passes
//! ---
//! Fix this bug: {{report}} (look in: {{where}})
//! ```
//!
//! A parameter whose name ends in `?` is optional and is empty when not given.

use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use devd_core::{DevdError, Result};

/// One parameter of a template.
#[derive(Debug, Clone, Serialize)]
pub struct Param {
    /// The name used in `{{name}}`.
    pub name: String,
    /// What to put there.
    pub help: String,
    /// Whether it may be left out.
    pub optional: bool,
}

/// A run template.
#[derive(Debug, Clone, Serialize)]
pub struct Template {
    /// Identifier, which is also the file name.
    pub id: String,
    /// Display name.
    pub title: String,
    /// One sentence.
    pub summary: String,
    /// Tasks that may run side by side.
    pub parallel: i64,
    /// The blanks.
    pub params: Vec<Param>,
    /// The definition of done every run carries.
    pub done_when: Vec<String>,
    /// The goal, with `{{blanks}}`.
    pub body: String,
    /// Whether it ships with devd.
    pub built_in: bool,
}

const FILES: &[(&str, &str)] = &[
    ("spec-to-pr", include_str!("../templates/spec-to-pr.md")),
    ("fix-bug", include_str!("../templates/fix-bug.md")),
    ("add-feature", include_str!("../templates/add-feature.md")),
    ("security-review", include_str!("../templates/security-review.md")),
    ("release-prep", include_str!("../templates/release-prep.md")),
];

/// Largest value a parameter may take.
const MAX_VALUE: usize = 20_000;

fn parse(file: &str, raw: &str, built_in: bool) -> std::result::Result<Template, String> {
    let bad = |why: &str| format!("templates/{file}.md: {why}");
    let raw = raw.replace("\r\n", "\n");
    let rest = raw.strip_prefix("---\n").ok_or_else(|| bad("must start with a front matter block"))?;
    let (front, body) = rest.split_once("\n---\n").ok_or_else(|| bad("front matter is not closed"))?;
    let field = |k: &str| front.lines().find_map(|l| l.strip_prefix(&format!("{k}:")).map(|v| v.trim().to_string()));
    let required = |k: &str| field(k).filter(|v| !v.is_empty()).ok_or_else(|| bad(&format!("`{k}` is missing")));
    let id = required("id")?;
    if id != file || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(bad("`id` must match the file name: lower-case letters, digits and hyphens"));
    }
    let parallel: i64 = field("parallel").map(|p| p.parse().map_err(|_| bad("`parallel` must be a number"))).transpose()?.unwrap_or(2);
    let mut params = Vec::new();
    for item in field("params").unwrap_or_default().split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, help) = item.split_once('=').map(|(n, h)| (n.trim(), h.trim())).unwrap_or((item, ""));
        let optional = name.ends_with('?');
        let name = name.trim_end_matches('?').to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(bad(&format!("`{name}` is not a parameter name")));
        }
        params.push(Param { name, help: help.to_string(), optional });
    }
    let body = body.trim().to_string();
    if body.is_empty() {
        return Err(bad("has no goal"));
    }
    for used in placeholders(&body) {
        if !params.iter().any(|p| p.name == used) {
            return Err(bad(&format!("the goal uses {{{{{used}}}}} but no parameter is named that")));
        }
    }
    Ok(Template {
        id,
        title: required("title")?,
        summary: required("summary")?,
        parallel: parallel.clamp(1, 6),
        params,
        done_when: field("done_when").unwrap_or_default().split('|').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect(),
        body,
        built_in,
    })
}

/// The names between double braces in a text.
fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push(after[..end].trim().to_string());
        rest = &after[end + 2..];
    }
    out
}

/// Every template: the built-in ones, then those in `dir`. The second value says
/// why a file in `dir` was not loaded.
pub fn all(dir: &Path) -> (Vec<Template>, Vec<String>) {
    let mut templates: Vec<Template> = FILES
        .iter()
        .map(|(id, raw)| parse(id, raw, true).unwrap_or_else(|e| panic!("a built-in template is malformed: {e}")))
        .collect();
    let mut errors = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "md").unwrap_or(false)).collect();
        files.sort();
        for path in files {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            match std::fs::read_to_string(&path) {
                Err(e) => errors.push(format!("templates/{stem}.md: cannot be read: {e}")),
                Ok(raw) => match parse(&stem, &raw, false) {
                    Err(e) => errors.push(e),
                    Ok(t) if templates.iter().any(|x| x.id == t.id) => errors.push(format!("templates/{stem}.md: a template named {} already exists", t.id)),
                    Ok(t) => templates.push(t),
                },
            }
        }
    }
    (templates, errors)
}

/// One template by id.
pub fn get(dir: &Path, id: &str) -> Result<Template> {
    all(dir).0.into_iter().find(|t| t.id == id).ok_or_else(|| DevdError::not_found(format!("no template named `{id}`; template.list shows them")))
}

impl Template {
    /// The goal with the values put in. A missing required value, or a value for a
    /// name the template does not have, is an error that says which.
    pub fn goal(&self, values: &Map<String, Value>) -> Result<String> {
        for key in values.keys() {
            if !self.params.iter().any(|p| &p.name == key) {
                return Err(DevdError::invalid(format!(
                    "the template `{}` has no parameter `{key}`; its parameters are {}",
                    self.id,
                    self.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
        }
        let mut goal = self.body.clone();
        let mut missing = Vec::new();
        for param in &self.params {
            let value = match values.get(&param.name) {
                Some(Value::String(s)) => s.trim().to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            if value.chars().count() > MAX_VALUE {
                return Err(DevdError::invalid(format!("`{}` is longer than {MAX_VALUE} characters; put the detail in a file and name it", param.name)));
            }
            if value.is_empty() && !param.optional {
                missing.push(param.name.clone());
            }
            goal = goal.replace(&format!("{{{{{}}}}}", param.name), if value.is_empty() { "(not given)" } else { &value });
        }
        if !missing.is_empty() {
            return Err(DevdError::invalid(format!("the template `{}` needs {}", self.id, missing.join(", "))));
        }
        Ok(goal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn every_built_in_template_parses_and_uses_only_its_own_parameters() {
        let (templates, errors) = all(Path::new("no-such-folder"));
        assert!(errors.is_empty());
        assert_eq!(templates.len(), FILES.len());
        for t in &templates {
            assert!(t.built_in && !t.done_when.is_empty(), "{} carries a definition of done", t.id);
            assert!(!t.params.is_empty(), "{}", t.id);
        }
    }

    #[test]
    fn values_go_where_the_blanks_are_and_a_required_one_cannot_be_left_out() {
        let t = get(Path::new("none"), "fix-bug").unwrap();
        let goal = t.goal(&values(json!({ "report": "login fails with a 500" }))).unwrap();
        assert!(goal.contains("login fails with a 500"));
        assert!(goal.contains("(not given)"), "an optional blank says it was not given: {goal}");
        let err = t.goal(&values(json!({}))).unwrap_err().to_string();
        assert!(err.contains("needs report"), "{err}");
        let err = t.goal(&values(json!({ "report": "x", "typo": "y" }))).unwrap_err().to_string();
        assert!(err.contains("no parameter `typo`") && err.contains("report"), "{err}");
    }

    #[test]
    fn your_own_templates_join_the_set_and_bad_ones_are_explained() {
        let dir = std::env::temp_dir().join(format!("devd-templates-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("triage.md"), "---\nid: triage\ntitle: Triage\nsummary: Sort a bug report\nparams: text=The report\ndone_when: A label was suggested\n---\nTriage this: {{text}}\n").unwrap();
        std::fs::write(dir.join("fix-bug.md"), "---\nid: fix-bug\ntitle: Mine\nsummary: x\nparams: a=b\n---\n{{a}}\n").unwrap();
        std::fs::write(dir.join("typo.md"), "---\nid: typo\ntitle: T\nsummary: x\nparams: a=b\n---\n{{b}}\n").unwrap();
        let (templates, errors) = all(&dir);
        assert!(templates.iter().any(|t| t.id == "triage" && !t.built_in));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("already exists")));
        assert!(errors.iter().any(|e| e.contains("no parameter is named that")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
