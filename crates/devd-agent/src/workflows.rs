//! Workflows that sit on top of the team and the provider layer: reviews, a
//! second opinion, project memory, learning plans, compatibility audits,
//! verification, canvases and bounded loops.
//!
//! Every record is a JSON file under `<devd home>/workflows/`. Nothing here
//! invents a result: a review is a model's answer about a real diff, a
//! verification is a command's real exit code, and a compatibility audit reads
//! the files it reports. When a model is needed and none is configured, the
//! capability says so instead of returning a plausible-looking empty result.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{util, DevdError, Provider, Registry, Result};

use crate::providers::{self, ChatRequest};
use crate::secrets;

/// The longest diff sent to a reviewer in one request.
const DIFF_LIMIT: usize = 60_000;

// ---------------------------------------------------------------- arguments

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ReviewRun {
    /// Absolute project directory.
    project: String,
    /// What to compare. `working` (default) reviews uncommitted changes;
    /// `base:<rev>` reviews against that revision.
    #[serde(default)]
    base: Option<String>,
    /// Extra focus for the reviewer, in plain words.
    #[serde(default)]
    instructions: Option<String>,
    /// A diff to review directly, instead of reading git. Use for an
    /// uncommitted patch or a diff produced elsewhere.
    #[serde(default)]
    diff: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RecordRef {
    /// Record id, as returned by the list capability.
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ReviewQuery {
    /// Only reviews of this project.
    #[serde(default)]
    project: Option<String>,
    /// Maximum records (default 30).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct MemoryPropose {
    /// Absolute project directory.
    project: String,
    /// One durable fact worth keeping: a build command, a convention, a
    /// gotcha, a decision. One sentence.
    lesson: String,
    /// What proved it (a command, a file, a person's instruction). Shown with
    /// the proposal so the person can judge it.
    #[serde(default)]
    evidence: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct MemoryApply {
    /// Proposal id from memory.propose.
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CompatAudit {
    /// Absolute project directory.
    project: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CompatWrite {
    /// Absolute project directory.
    project: String,
}

// ------------------------------------------------------------------ storage

fn workflows_root(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("workflows")
}

fn kind_dir(ctx: &Ctx, kind: &str) -> PathBuf {
    workflows_root(ctx).join(kind)
}

fn save_record(ctx: &Ctx, kind: &str, record: &Value) -> Result<()> {
    let dir = kind_dir(ctx, kind);
    fs::create_dir_all(&dir).map_err(|e| DevdError::invalid(format!("could not create {}: {e}", dir.display())))?;
    let id = record["id"].as_str().ok_or_else(|| DevdError::invalid("a record needs an id"))?;
    let path = dir.join(format!("{id}.json"));
    let bytes = serde_json::to_vec_pretty(record)?;
    fs::write(&path, bytes).map_err(|e| DevdError::invalid(format!("could not save {}: {e}", path.display())))?;
    Ok(())
}

fn list_records(ctx: &Ctx, kind: &str) -> Vec<Value> {
    let dir = kind_dir(ctx, kind);
    let mut rows = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                rows.push(v);
            }
        }
    }
    rows.sort_by(|a, b| b["created_at"].as_str().unwrap_or("").cmp(a["created_at"].as_str().unwrap_or("")));
    rows
}

fn get_record(ctx: &Ctx, kind: &str, id: &str) -> Result<Value> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(DevdError::invalid("that is not a record id"));
    }
    let path = kind_dir(ctx, kind).join(format!("{id}.json"));
    let text = fs::read_to_string(&path).map_err(|_| DevdError::not_found(format!("no {kind} record `{id}`")))?;
    Ok(serde_json::from_str(&text)?)
}

fn delete_record(ctx: &Ctx, kind: &str, id: &str) -> Result<()> {
    get_record(ctx, kind, id)?;
    let path = kind_dir(ctx, kind).join(format!("{id}.json"));
    fs::remove_file(&path).map_err(|e| DevdError::invalid(format!("could not delete {}: {e}", path.display())))?;
    Ok(())
}

// ------------------------------------------------------------------- models

/// Choose a provider and model for a kind of work. Role assignments win, then
/// the default provider. A missing model is reported, never guessed.
fn pick_model(ctx: &Ctx, role: &str) -> Result<(Provider, String)> {
    let providers = ctx.store.list_providers()?;
    let usable = |p: &Provider| p.enabled;
    let assignments = ctx.store.list_role_models()?;
    for a in assignments.iter().filter(|a| a.role == role && a.scope.is_empty()) {
        if let Some(p) = providers.iter().find(|p| p.id == a.provider_id && usable(p)) {
            if !a.model.trim().is_empty() {
                return Ok((p.clone(), a.model.clone()));
            }
        }
    }
    let p = providers
        .iter()
        .find(|p| usable(p) && p.is_default)
        .or_else(|| providers.iter().find(|p| usable(p)))
        .ok_or_else(|| DevdError::invalid("no model provider is configured; add one in Models first"))?;
    let model = p
        .default_model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| DevdError::invalid(format!("provider `{}` has no default model; choose one in Models", p.label)))?;
    Ok((p.clone(), model))
}

/// One question, one answer, no tools. The caller decides what to do with it.
fn one_shot(ctx: &Ctx, role: &str, system: &str, prompt: &str, max_tokens: u32) -> Result<(String, String, String)> {
    let (provider, model) = pick_model(ctx, role)?;
    let key = secrets::read_key(&ctx.paths, &provider);
    let request = ChatRequest {
        model: model.clone(),
        system: system.to_string(),
        messages: vec![json!({ "role": "user", "content": [{ "type": "text", "text": prompt }] })],
        tools: Vec::new(),
        tool_choice: None,
        max_tokens,
    };
    let response = providers::chat(&provider, key.as_deref(), &request)?;
    let text = response
        .content
        .iter()
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(DevdError::invalid(format!("{} returned no text", provider.label)));
    }
    Ok((text, provider.label.clone(), model))
}

// ---------------------------------------------------------------- file work

fn absolute_project(project: &str) -> Result<PathBuf> {
    let path = PathBuf::from(project.trim());
    if !path.is_absolute() || !path.is_dir() {
        return Err(DevdError::invalid("the project must be an existing folder, given as an absolute path"));
    }
    Ok(path)
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string().replace('\\', "/")
}

fn read_text(path: &Path, limit: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").current_dir(cwd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The text a reviewer should read: the working tree's changes by default, or
/// changes against a revision. An empty answer is reported as empty.
fn collect_diff(project: &Path, base: &str) -> Result<String> {
    let range = if base == "working" {
        "HEAD".to_string()
    } else if let Some(rev) = base.strip_prefix("base:") {
        rev.trim().to_string()
    } else {
        base.trim().to_string()
    };
    if range.is_empty() {
        return Err(DevdError::invalid("name a revision, or use `working`"));
    }
    let mut text = git(project, &["diff", "--no-color", &range]).unwrap_or_default();
    if text.trim().is_empty() {
        // A repository without a first commit has no HEAD; show what is staged
        // and unstaged instead of pretending there is nothing to review.
        let unstaged = git(project, &["diff", "--no-color"]).unwrap_or_default();
        let staged = git(project, &["diff", "--cached", "--no-color"]).unwrap_or_default();
        text = format!("{staged}{unstaged}");
    }
    if text.trim().is_empty() {
        return Err(DevdError::invalid("there are no uncommitted changes to review; commit them, or pass a diff"));
    }
    Ok(text)
}

// ------------------------------------------------------------------- memory

const MEMORY_MARKER: &str = "## Project memory (devd)";

/// Replace the managed section in AGENTS.md, or add it, keeping every other
/// line exactly as the person wrote it.
fn with_memory(base: &str, bullet: &str) -> String {
    let mut lines: Vec<String> = base.lines().map(str::to_string).collect();
    if let Some(start) = lines.iter().position(|l| l.trim() == MEMORY_MARKER) {
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, l)| l.starts_with("## "))
            .map(|(i, _)| i)
            .unwrap_or(lines.len());
        let mut section: Vec<String> = lines[start..end].to_vec();
        while section.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            section.pop();
        }
        if !section.iter().any(|l| l.trim() == format!("- {bullet}").as_str()) {
            section.push(format!("- {bullet}"));
        }
        let tail = lines.split_off(end);
        lines.truncate(start);
        lines.extend(section);
        if !tail.is_empty() {
            lines.push(String::new());
            lines.extend(tail);
        }
    } else {
        while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.pop();
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(MEMORY_MARKER.to_string());
        lines.push(String::new());
        lines.push(format!("- {bullet}"));
        lines.push(String::new());
    }
    let mut text = lines.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// A line-based diff with a little context. Enough to see exactly what applying
/// a proposal changes; not a replacement for git.
fn diff_text(before: &str, after: &str, name: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len().saturating_sub(pre) && suf < b.len().saturating_sub(pre) && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let mut out = vec![format!("--- a/{name}"), format!("+++ b/{name}")];
    let ctx_start = pre.saturating_sub(3);
    out.push(format!("@@ -{},{} +{},{} @@", ctx_start + 1, a.len() - ctx_start - suf, ctx_start + 1, b.len() - ctx_start - suf));
    for line in &a[ctx_start..pre] {
        out.push(format!(" {line}"));
    }
    for line in &a[pre..a.len() - suf] {
        out.push(format!("-{line}"));
    }
    for line in &b[pre..b.len() - suf] {
        out.push(format!("+{line}"));
    }
    for line in &a[a.len() - suf..] {
        out.push(format!(" {line}"));
    }
    out.join("\n")
}

fn backup_file(project: &Path, name: &str) -> Result<Option<String>> {
    let path = project.join(name);
    if !path.is_file() {
        return Ok(None);
    }
    let dir = project.join(".devd").join("backups");
    fs::create_dir_all(&dir).map_err(|e| DevdError::invalid(format!("could not create {}: {e}", dir.display())))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = dir.join(format!("{name}.{stamp}.bak"));
    fs::copy(&path, &dest).map_err(|e| DevdError::invalid(format!("could not back up {}: {e}", path.display())))?;
    Ok(Some(relative(&dest, project)))
}

// ------------------------------------------------------------------- compat

const COMPAT_FILES: &[(&str, &str)] = &[
    ("AGENTS.md", "the file devd and most agents read"),
    ("CLAUDE.md", "Claude Code"),
    ("GEMINI.md", "Gemini CLI"),
    (".cursorrules", "Cursor (legacy)"),
    (".windsurfrules", "Windsurf"),
    (".clinerules", "Cline"),
    (".github/copilot-instructions.md", "GitHub Copilot"),
];

fn compat_scan(project: &Path) -> (Vec<Value>, Vec<Value>) {
    let mut files = Vec::new();
    let mut issues = Vec::new();
    let mut managers: Vec<(String, String)> = Vec::new();
    for (name, owner) in COMPAT_FILES {
        let path = project.join(name);
        if !path.is_file() {
            continue;
        }
        let text = read_text(&path, 512_000);
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        files.push(json!({
            "path": name,
            "owner": owner,
            "bytes": size,
            "lines": text.as_ref().map(|t| t.lines().count()).unwrap_or(0),
            "readable": text.is_some(),
        }));
        if let Some(text) = &text {
            for word in ["pnpm", "yarn", "npm", "bun"] {
                if text.contains(&format!("{word} test")) || text.contains(&format!("{word} run")) {
                    managers.push((name.to_string(), word.to_string()));
                }
            }
            if text.lines().count() > 400 {
                issues.push(json!({ "severity": "low", "path": name, "message": format!("{name} is {} lines; long instruction files crowd the context window", text.lines().count()) }));
            }
        }
    }
    for (dir, owner) in [(".cursor/rules", "Cursor rules"), (".claude/commands", "Claude Code commands")] {
        let path = project.join(dir);
        if let Ok(entries) = fs::read_dir(&path) {
            let count = entries.flatten().filter(|e| e.path().is_file()).count();
            if count > 0 {
                files.push(json!({ "path": dir, "owner": owner, "bytes": Value::Null, "lines": Value::Null, "readable": true, "files": count }));
            }
        }
    }
    let has_agents = files.iter().any(|f| f["path"] == "AGENTS.md");
    let has_claude = files.iter().any(|f| f["path"] == "CLAUDE.md");
    if !has_agents {
        issues.push(json!({ "severity": "medium", "path": "AGENTS.md", "message": "no AGENTS.md: agents that do not know this project's build and test commands will guess" }));
    }
    if has_agents && has_claude {
        let a = fs::read_to_string(project.join("AGENTS.md")).unwrap_or_default();
        let c = fs::read_to_string(project.join("CLAUDE.md")).unwrap_or_default();
        if a.trim() != c.trim() {
            issues.push(json!({ "severity": "medium", "path": "CLAUDE.md", "message": "AGENTS.md and CLAUDE.md differ; two instruction files drift apart and agents follow whichever they find first" }));
        }
    }
    let distinct: Vec<&str> = {
        let mut v: Vec<&str> = managers.iter().map(|(_, m)| m.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if distinct.len() > 1 {
        issues.push(json!({ "severity": "high", "path": "", "message": format!("instruction files name different package managers ({}); an agent cannot tell which one the project uses", distinct.join(", ")) }));
    }
    (files, issues)
}

/// A minimal, truthful AGENTS.md built from what the project contains.
fn proposed_agents(project: &Path) -> String {
    let name = project.file_name().and_then(|n| n.to_str()).unwrap_or("this project");
    let mut out = format!("# {name}\n\n");
    out.push_str("Guidance for coding agents working in this repository.\n\n## Commands\n\n");
    match detect_check(project) {
        Some(check) => {
            out.push_str(&format!("- Check: `{}` ({})\n", check.command, check.why));
        }
        None => out.push_str("- No test or build command was detected. Add the real command here before asking an agent to verify work.\n"),
    }
    out.push_str("\n## Layout\n\n");
    let mut dirs: Vec<String> = fs::read_dir(project)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter(|e| !matches!(e.file_name().to_string_lossy().as_ref(), ".git" | "node_modules" | "target" | "vendor" | "dist" | "build" | ".devd"))
                .map(|e| format!("- `{}/`", e.file_name().to_string_lossy()))
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    if dirs.is_empty() {
        out.push_str("- The project is a single directory.\n");
    } else {
        out.push_str(&dirs.join("\n"));
        out.push('\n');
    }
    out.push_str("\n## Rules\n\n- Read this file before changing anything.\n- Run the check above before reporting work as done, and say what it printed.\n- Do not commit or push unless a person asks.\n- Keep secrets out of the repository.\n");
    out
}

// ---------------------------------------------------------------- the check

/// The command that proves a project works, read from its own manifests.
struct Check {
    command: String,
    why: String,
}

fn detect_check(project: &Path) -> Option<Check> {
    let has = |name: &str| project.join(name).is_file();
    if has("Cargo.toml") {
        return Some(Check { command: "cargo test --quiet".into(), why: "Cargo.toml".into() });
    }
    if has("package.json") {
        let manifest = read_text(&project.join("package.json"), 200_000).unwrap_or_default();
        let json: Value = serde_json::from_str(&manifest).unwrap_or(Value::Null);
        let script = ["test", "check", "build"].into_iter().find(|s| json["scripts"][*s].is_string())?;
        let runner = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else if has("bun.lock") || has("bun.lockb") {
            "bun"
        } else {
            "npm"
        };
        return Some(Check { command: format!("{runner} run {script}"), why: format!("package.json script `{script}`") });
    }
    if has("go.mod") {
        return Some(Check { command: "go test ./...".into(), why: "go.mod".into() });
    }
    if has("composer.json") {
        return Some(Check { command: "composer test".into(), why: "composer.json".into() });
    }
    if has("pyproject.toml") || has("pytest.ini") || has("requirements.txt") {
        return Some(Check { command: "python -m pytest -q".into(), why: "Python project files".into() });
    }
    None
}

fn simple(id: &str, text: impl Into<String>) -> Result<Plan> {
    Ok(Plan::new(id, text))
}

// ------------------------------------------------------------- registration

pub fn register(reg: &mut Registry) -> Result<()> {
    // ---------------------------------------------------------- review
    reg.define::<ReviewRun>("review.run", "Review a change")
    .description("Reads the real diff (uncommitted changes by default, or against a revision), asks the configured reviewer model for findings, and stores them with the diff so the review can be read later. Findings are the model's, not devd's; when no model is configured the capability says so rather than returning an empty review. This is the Thermos review pass and the data behind the review canvas.")
    .group("review")
    .tier(Tier::Exec)
    .effect(Effect::Remote)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, a| simple("review.run", format!("review the changes in {}", a.project)),
        |ctx, a| {
            let project = absolute_project(&a.project)?;
            let base = a.base.clone().unwrap_or_else(|| "working".into());
            let diff = match a.diff.as_ref().filter(|d| !d.trim().is_empty()) {
                Some(d) => d.clone(),
                None => collect_diff(&project, &base)?,
            };
            let truncated = diff.chars().count() > DIFF_LIMIT;
            let shown: String = diff.chars().take(DIFF_LIMIT).collect();
            let focus = a.instructions.as_deref().map(|i| format!("\nThe person asked you to focus on: {i}\n")).unwrap_or_default();
            let system = "You are a careful senior reviewer. You review a diff for correctness, security, data loss, missing tests and unclear design. You do not praise. You report only problems you can point to in the diff. If the change is fine, say so and return an empty findings list. Answer with one JSON object and nothing else: {\"summary\":\"...\",\"findings\":[{\"severity\":\"high|medium|low\",\"file\":\"path\",\"line\":123,\"title\":\"...\",\"detail\":\"...\",\"suggestion\":\"...\"}]}";
            let prompt = format!(
                "Review this diff from the project at {}.\n{focus}\nThe diff{}\n\n```diff\n{}\n```",
                project.display(),
                if truncated { " was shortened; review what is shown and say so in the summary" } else { "" },
                shown
            );
            let (raw, provider_label, model) = one_shot(ctx, "reviewer", system, &prompt, 4000)?;
            let parsed = extract_json(&raw);
            let parsed_ok = parsed.is_some();
            let (summary, findings) = match parsed {
                Some(v) => (
                    v["summary"].as_str().unwrap_or("").to_string(),
                    v["findings"].as_array().cloned().unwrap_or_default(),
                ),
                None => (raw.clone(), Vec::new()),
            };
            let id = util::new_id();
            let record = json!({
                "id": id,
                "project": project.display().to_string(),
                "base": base,
                "provider": provider_label,
                "model": model,
                "created_at": util::now(),
                "summary": summary,
                "findings": findings,
                "raw": raw,
                "parsed": parsed_ok,
                "diff": shown,
                "diff_truncated": truncated,
                "focus": a.instructions,
            });
            save_record(ctx, "review", &record)?;
            let count = record["findings"].as_array().map(|f| f.len()).unwrap_or(0);
            let message = if count == 0 {
                format!("reviewed; the reviewer reported no findings{}", if parsed_ok { "" } else { " (the answer was not valid JSON, so the raw text is kept)" })
            } else {
                format!("reviewed; {count} finding(s)")
            };
            let mut outcome = Outcome::with_data(message, record);
            if !parsed_ok {
                outcome = outcome.warn("the reviewer's answer was not the requested JSON; the raw text is stored and shown as the summary");
            }
            Ok(outcome)
        },
    )?;

    reg.define::<ReviewQuery>("review.list", "List reviews")
    .description("Stored review runs, newest first, with the provider and model that produced them, finding counts and whether the diff was shortened.")
    .group("review")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, _| simple("review.list", "list reviews"),
        |ctx, a| {
            let limit = a.limit.unwrap_or(30).clamp(1, 100);
            let rows: Vec<Value> = list_records(ctx, "review")
                .into_iter()
                .filter(|r| a.project.as_deref().map(|p| r["project"].as_str() == Some(p)).unwrap_or(true))
                .take(limit)
                .map(|mut r| {
                    if let Some(obj) = r.as_object_mut() {
                        obj.remove("diff");
                        obj.remove("raw");
                    }
                    r
                })
                .collect();
            Ok(Outcome::with_data(format!("{} review(s)", rows.len()), json!(rows)))
        },
    )?;

    reg.define::<RecordRef>("review.get", "Read a review")
    .description("One stored review with its findings and the diff it reviewed.")
    .group("review")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, a| simple("review.get", format!("read review {}", a.id)),
        |ctx, a| {
            let r = get_record(ctx, "review", &a.id)?;
            Ok(Outcome::with_data(r["summary"].as_str().unwrap_or("review").to_string(), r))
        },
    )?;

    reg.define::<RecordRef>("review.delete", "Delete a review")
    .description("Removes a stored review. The diff it held is not affected.")
    .group("review")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| simple("review.delete", format!("delete review {}", a.id)),
        |ctx, a| {
            delete_record(ctx, "review", &a.id)?;
            Ok(Outcome::ok("deleted"))
        },
    )?;

    // ---------------------------------------------------------- memory
    reg.define::<MemoryPropose>("memory.propose", "Propose a line for AGENTS.md")
    .description("Turns one durable lesson into a precise change to AGENTS.md, and returns the diff instead of writing it. The line goes in a managed `Project memory (devd)` section; every other line stays exactly as it is. Applying is a separate, human-approved step, so an agent can remember something without silently editing the instructions people rely on.")
    .group("memory")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, a| simple("memory.propose", format!("propose a memory for {}", a.project)),
        |ctx, a| {
            let project = absolute_project(&a.project)?;
            let lesson = a.lesson.trim().replace('\n', " ");
            if lesson.is_empty() {
                return Err(DevdError::invalid("say what should be remembered"));
            }
            if lesson.chars().count() > 320 {
                return Err(DevdError::invalid("a memory line is at most 320 characters; split it or say it more plainly"));
            }
            let path = project.join("AGENTS.md");
            let base = fs::read_to_string(&path).unwrap_or_default();
            if base.contains(&format!("- {lesson}")) {
                return Err(DevdError::invalid("AGENTS.md already records that"));
            }
            let after = with_memory(&base, &lesson);
            let record = json!({
                "id": util::new_id(),
                "project": project.display().to_string(),
                "lesson": lesson,
                "evidence": a.evidence,
                "created_at": util::now(),
                "base": base,
                "proposed": after,
                "diff": diff_text(&base, &after, "AGENTS.md"),
                "target": "AGENTS.md",
            });
            save_record(ctx, "memory", &record)?;
            Ok(Outcome::with_data("proposed; apply it to write AGENTS.md", record))
        },
    )?;

    reg.define::<MemoryApply>("memory.apply", "Apply a memory proposal")
    .description("Writes the proposed line into AGENTS.md. If the file changed since the proposal was made, the proposal is merged three ways (the text it was made against, the file as it is now, and the proposed text): changes to other lines combine, and a collision on the same lines is refused with the conflicting text. Keeps a timestamped backup under .devd/backups. Human only, because AGENTS.md changes how every agent behaves in this project.")
    .group("memory")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let p = get_record(ctx, "memory", &a.id)?;
            Ok(Plan::new("memory.apply", format!("write one line into {}", p["target"].as_str().unwrap_or("AGENTS.md"))).step("merge the proposal with any change made to the file since").step("keep a timestamped backup under .devd/backups").effect("AGENTS.md gains a line in the managed Project memory section"))
        },
        |ctx, a| {
            let proposal = get_record(ctx, "memory", &a.id)?;
            let project = PathBuf::from(proposal["project"].as_str().unwrap_or_default());
            let target = proposal["target"].as_str().unwrap_or("AGENTS.md");
            let path = project.join(target);
            let current = fs::read_to_string(&path).unwrap_or_default();
            let base_text = proposal["base"].as_str().unwrap_or("");
            let proposed = proposal["proposed"].as_str().ok_or_else(|| DevdError::invalid("the proposal has no text"))?;
            // The file moved on after the proposal was made. Combine the two sets of changes
            // rather than throwing the proposal away: independent memory lines are independent.
            let (text, merged) = if current == base_text {
                (proposed.to_string(), false)
            } else {
                match devd_core::merge3::merge(base_text, &current, proposed, devd_core::merge3::Inserts::Union)? {
                    devd_core::merge3::Merged::Clean(t) => (t, true),
                    devd_core::merge3::Merged::Conflicts(c) => {
                        let shown: Vec<String> = c
                            .iter()
                            .map(|x| format!("lines {}-{}: the file has {:?}, the proposal has {:?}", x.start + 1, x.end, x.ours.concat().trim_end(), x.theirs.concat().trim_end()))
                            .collect();
                        return Err(DevdError::conflict(format!("{target} changed where the proposal also changes it ({}); propose again so the new text is reviewed", shown.join("; "))));
                    }
                }
            };
            let backup = backup_file(&project, target)?;
            fs::write(&path, &text).map_err(|e| DevdError::invalid(format!("could not write {}: {e}", path.display())))?;
            let _ = delete_record(ctx, "memory", &a.id);
            let mut outcome = Outcome::with_data("AGENTS.md updated", json!({ "path": path.display().to_string(), "backup": backup, "merged": merged }));
            if merged {
                outcome = outcome.warn("AGENTS.md had changed since the proposal; the two sets of changes were merged");
            }
            if backup.is_none() {
                outcome = outcome.warn("AGENTS.md did not exist; it was created");
            }
            Ok(outcome)
        },
    )?;

    reg.define::<ReviewQuery>("memory.list", "List memory proposals")
    .description("Pending proposals for AGENTS.md, with the evidence that justified each one.")
    .group("memory")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, _| simple("memory.list", "list memory proposals"),
        |ctx, a| {
            let limit = a.limit.unwrap_or(30).clamp(1, 100);
            let rows: Vec<Value> = list_records(ctx, "memory")
                .into_iter()
                .filter(|r| a.project.as_deref().map(|p| r["project"].as_str() == Some(p)).unwrap_or(true))
                .take(limit)
                .map(|mut r| {
                    if let Some(obj) = r.as_object_mut() {
                        obj.remove("base");
                        obj.remove("proposed");
                    }
                    r
                })
                .collect();
            Ok(Outcome::with_data(format!("{} proposal(s)", rows.len()), json!(rows)))
        },
    )?;

    // ----------------------------------------------------------- compat
    reg.define::<CompatAudit>("compat.audit", "Audit agent instruction files")
    .description("Finds every instruction file agents read in this project (AGENTS.md, CLAUDE.md, Cursor rules, Copilot instructions and the rest), reports overlap and contradictions with the evidence that shows them, and proposes an AGENTS.md built from the project's real commands and folders. It writes nothing.")
    .group("compat")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, a| simple("compat.audit", format!("audit agent instructions in {}", a.project)),
        |_, a| {
            let project = absolute_project(&a.project)?;
            let (files, issues) = compat_scan(&project);
            let proposal = if project.join("AGENTS.md").is_file() { Value::Null } else { Value::String(proposed_agents(&project)) };
            let high = issues.iter().filter(|i| i["severity"] == "high").count();
            let record = json!({ "files": files, "issues": issues, "proposal": proposal, "high": high });
            Ok(Outcome::with_data(format!("{} instruction file(s), {} issue(s)", record["files"].as_array().map(|f| f.len()).unwrap_or(0), issues.len()), record))
        },
    )?;

    reg.define::<CompatWrite>("compat.write", "Write the proposed AGENTS.md")
    .description("Creates AGENTS.md from the project's real commands and layout, only when the project has none. If AGENTS.md already exists the capability refuses instead of overwriting it; use memory.propose for additions to an existing file. Human only.")
    .group("compat")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| simple("compat.write", format!("create AGENTS.md in {}", a.project)),
        |_, a| {
            let project = absolute_project(&a.project)?;
            let path = project.join("AGENTS.md");
            if path.exists() {
                return Err(DevdError::conflict("AGENTS.md already exists; propose additions with memory.propose so they are reviewed"));
            }
            let text = proposed_agents(&project);
            fs::write(&path, text).map_err(|e| DevdError::invalid(format!("could not write {}: {e}", path.display())))?;
            Ok(Outcome::with_data("AGENTS.md created", json!({ "path": path.display().to_string() })))
        },
    )?;

    Ok(())
}

/// The first JSON object in a model's answer, tolerating a fenced block.
fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_section_is_added_once_and_kept_in_place() {
        let first = with_memory("# Notes\n\nBe kind.\n", "Tests run with cargo test.");
        assert!(first.contains(MEMORY_MARKER));
        assert!(first.contains("- Tests run with cargo test."));
        assert!(first.starts_with("# Notes"));
        let second = with_memory(&first, "The API lives in crates/api.");
        assert_eq!(second.matches(MEMORY_MARKER).count(), 1);
        assert!(second.contains("cargo test"));
        assert!(second.contains("crates/api"));
    }

    #[test]
    fn two_proposals_made_against_the_same_file_both_survive_a_three_way_merge() {
        use devd_core::merge3::{merge, Inserts, Merged};
        let base = "# Notes\n\nBe kind.\n";
        let first = with_memory(base, "Tests run with cargo test.");
        let second = with_memory(base, "The API lives in crates/api.");
        // The first proposal was applied; the second was made against the old text.
        let Merged::Clean(text) = merge(base, &first, &second, Inserts::Union).unwrap() else {
            panic!("independent memory lines must merge");
        };
        assert_eq!(text.matches(MEMORY_MARKER).count(), 1, "{text}");
        assert!(text.contains("- Tests run with cargo test."), "{text}");
        assert!(text.contains("- The API lives in crates/api."), "{text}");
        assert!(text.contains("Be kind."));
    }

    #[test]
    fn a_proposal_that_collides_with_a_rewrite_of_the_same_lines_is_refused() {
        use devd_core::merge3::{merge, Inserts, Merged};
        let base = "# Notes\n\nBe kind.\n";
        let proposed = "# Notes\n\nBe kind and brief.\n";
        let rewritten = "# Notes\n\nBe formal.\n";
        assert!(matches!(
            merge(base, rewritten, proposed, Inserts::Union).unwrap(),
            Merged::Conflicts(_)
        ));
    }

    #[test]
    fn memory_does_not_duplicate_a_line() {
        let first = with_memory("", "Use pnpm.");
        let second = with_memory(&first, "Use pnpm.");
        assert_eq!(second.matches("- Use pnpm.").count(), 1);
    }

    #[test]
    fn diff_marks_added_line() {
        let d = diff_text("a\nb\n", "a\nb\nc\n", "AGENTS.md");
        assert!(d.contains("+c"));
        assert!(d.contains("--- a/AGENTS.md"));
    }

    #[test]
    fn detects_rust_check() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let check = detect_check(dir.path()).unwrap();
        assert!(check.command.contains("cargo test"));
    }

    #[test]
    fn detects_node_runner_from_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), r#"{"scripts":{"test":"vitest"}}"#).unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let check = detect_check(dir.path()).unwrap();
        assert_eq!(check.command, "pnpm run test");
    }

    #[test]
    fn compat_flags_missing_agents_and_package_manager_conflict() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "Run pnpm test").unwrap();
        fs::write(dir.path().join("AGENTS.md"), "Run npm test").unwrap();
        let (files, issues) = compat_scan(dir.path());
        assert_eq!(files.len(), 2);
        assert!(issues.iter().any(|i| i["severity"] == "high"));
    }

    #[test]
    fn extracts_json_from_fenced_answer() {
        let v = extract_json("Here:\n```json\n{\"summary\":\"ok\",\"findings\":[]}\n```").unwrap();
        assert_eq!(v["summary"], "ok");
    }

    #[test]
    fn proposed_agents_uses_real_check() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let text = proposed_agents(dir.path());
        assert!(text.contains("cargo test"));
        assert!(text.contains("# "));
    }
}
