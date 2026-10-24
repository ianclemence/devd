//! Capabilities of the stack engine.
//!
//! Every surface reaches the engine through these declarations: the CLI, the
//! terminal dashboard, the window, the HTTP API and the MCP server. Tiers follow
//! what an operation can do:
//!
//! * Reading registry data, plans and project context is `Read`.
//! * Creating a project, running a project's own tasks and registering a
//!   project are `Write`/`Exec`, confined to authorised directories.
//! * Anything that can destroy data - a destructive task or command - is
//!   `Destructive` and human-only.
//! * Anything that changes the machine - installing or removing software,
//!   starting or stopping a system service - is `Admin`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

use crate::authz;
use crate::detect::{self, ProjectContext};
use crate::exec::{self, runs};
use crate::init::{self, InitRequest};
use crate::model::Risk;
use crate::reconcile;
use crate::registry::StackRegistry;
use crate::tasks::{self, RunTask, TaskInfo};
use crate::toolchain;
use crate::tools::Host;

fn stacks(ctx: &Ctx) -> StackRegistry {
    StackRegistry::with_extensions(&ctx.paths.root().join("stacks"))
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct NoArgs {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct DecideService {
    /// Tool id of a service, e.g. `postgresql`.
    tool: String,
    /// `adopt`, `observe`, `external` or `ignore`.
    #[serde(default)]
    mode: Option<String>,
    /// Port of your own instance (required for `external`).
    #[serde(default)]
    port: Option<u16>,
    /// Why.
    #[serde(default)]
    note: Option<String>,
    /// Forget the decision.
    #[serde(default)]
    clear: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ListArgs {
    /// Only this category: web, frontend, backend, mobile, language.
    #[serde(default)]
    category: Option<String>,
    /// Only this language.
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct DescribeArgs {
    /// Stack id.
    stack: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PathArgs {
    /// Absolute project directory.
    path: String,
    /// Treat the project as this stack instead of detecting it.
    #[serde(default)]
    stack: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ImportArgs {
    /// Absolute project directory (an existing checkout).
    path: String,
    /// Treat the project as this stack instead of detecting it.
    #[serde(default)]
    stack: Option<String>,
    /// Display name; defaults to the directory name.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ForgetArgs {
    /// Project directory to unregister. Files are not touched.
    path: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ExecArgs {
    /// Project directory.
    path: String,
    /// Program followed by arguments. No shell is involved.
    argv: Vec<String>,
    /// Seconds before the command is stopped (default 600).
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ToolArgs {
    /// Tool id from toolchain.list.
    tool: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ToolPlanArgs {
    /// Tool id.
    tool: String,
    /// `install` (default) or `remove`.
    #[serde(default)]
    action: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ToolListArgs {
    /// Only this category: runtime, package-manager, database, vcs, mobile, container, build, utility.
    #[serde(default)]
    category: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ServiceArgs {
    /// Service tool id, e.g. postgresql, mariadb, redis.
    tool: String,
    /// `start`, `stop` or `restart`.
    action: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CheckArgs {
    /// Stack id.
    stack: String,
    /// Framework version id.
    #[serde(default)]
    version: Option<String>,
    /// Package manager id.
    #[serde(default)]
    package_manager: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RunIdArgs {
    /// The run id given to project.init or project.run.
    run_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ProcArgs {
    /// Background process name from task.list.
    name: String,
    /// How many log lines (default 200).
    #[serde(default)]
    lines: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct RunsArgs {
    /// Only runs for this project path or tool id.
    #[serde(default)]
    subject: Option<String>,
    /// How many (default 30).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct LogArgs {
    /// Run id.
    run_id: String,
    /// Last N lines (default 200).
    #[serde(default)]
    lines: Option<usize>,
}

fn read_plan(id: &str, summary: impl Into<String>) -> Result<Plan> {
    Ok(Plan::new(id, summary))
}

fn dir_arg(ctx: &Ctx, path: &str) -> Result<PathBuf> {
    let p = authz::authorize_dir(ctx, Path::new(path))?;
    if !p.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a directory", p.display())));
    }
    Ok(p)
}

fn stack_summary(s: &crate::model::StackDef) -> Value {
    json!({
        "id": s.id, "name": s.name, "language": s.language, "category": s.category,
        "summary": s.summary, "creatable": !s.init.steps.is_empty(),
        "versions": s.versions.iter().map(|v| json!({"id": v.id, "label": v.label, "default": v.default, "note": v.note})).collect::<Vec<_>>(),
        "package_managers": s.package_managers,
        "docs": s.docs, "verified": s.verified,
    })
}

/// A JSON Schema describing a stack's `options` object, derived from its definition.
pub fn options_schema(s: &crate::model::StackDef) -> Value {
    use crate::model::OptionKind::*;
    let mut props = serde_json::Map::new();
    for o in &s.options {
        let mut p = match o.kind {
            Bool => json!({"type": "boolean"}),
            Text => {
                let mut t = json!({"type": "string"});
                if let Some(pat) = &o.pattern {
                    t["pattern"] = json!(pat);
                }
                t
            }
            Select => json!({"type": "string", "enum": o.choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>()}),
            Multi => json!({"type": "array", "items": {"type": "string", "enum": o.choices.iter().map(|c| c.value.clone()).collect::<Vec<_>>()}, "uniqueItems": true}),
        };
        p["title"] = json!(o.label);
        p["description"] = json!(o.help);
        if !o.default.is_null() {
            p["default"] = o.default.clone();
        }
        if o.advanced {
            p["x-advanced"] = json!(true);
        }
        if let Some(f) = &o.flag {
            p["x-flag"] = json!(f);
        }
        if !o.versions.is_empty() {
            p["x-versions"] = json!(o.versions);
        }
        props.insert(o.id.clone(), p);
    }
    json!({"type": "object", "additionalProperties": false, "properties": props})
}

fn describe(reg: &StackRegistry, id: &str) -> Result<Value> {
    let s = reg.stack(id).ok_or_else(|| DevdError::not_found(format!("no stack `{id}`; see stacks.list")))?;
    Ok(json!({
        "stack": s,
        "options_schema": options_schema(s),
        "request_example": {
            "stack": s.id, "name": "my-app",
            "version": s.default_version().id,
            "package_manager": s.package_managers.first(),
            "options": s.options.iter().filter(|o| !o.default.is_null()).map(|o| (o.id.clone(), o.default.clone())).collect::<serde_json::Map<_, _>>(),
        },
        "prerequisite_tools": s.versions.iter().flat_map(|v| v.requires.iter().map(|r| r.tool.clone())).collect::<std::collections::BTreeSet<_>>(),
    }))
}

fn detect_ctx(ctx: &Ctx, reg: &StackRegistry, path: &str, stack: Option<&str>) -> Result<(PathBuf, ProjectContext)> {
    let dir = dir_arg(ctx, path)?;
    let c = detect::detect(ctx, reg, &dir, stack)?;
    Ok((dir, c))
}

fn pick_task<'a>(c: &'a ProjectContext, id: &str) -> Result<&'a TaskInfo> {
    c.tasks.iter().find(|t| t.id == id).ok_or_else(|| {
        DevdError::not_found(format!(
            "this project has no task `{id}`. Available: {}",
            c.tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
        ))
    })
}

fn install_hint(dir: &Path) -> Option<&'static str> {
    if dir.join("package.json").exists() && !dir.join("node_modules").exists() {
        return Some("node_modules is missing - dependencies are probably not installed; run the `install` task first");
    }
    if dir.join("composer.json").exists() && !dir.join("vendor").exists() {
        return Some("vendor/ is missing - dependencies are probably not installed; run the `install` task first");
    }
    None
}

/// Register every stack-engine capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    // -------------------------------------------------------------- registry
    reg.define::<ListArgs>("stacks.list", "List supported stacks")
    .description("Every stack devd can create, detect and run, with its versions, package managers and official documentation. Start here to discover what is supported.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("stacks.list", "list stacks"),
        |ctx, a| {
            let r = stacks(ctx);
            let list: Vec<Value> = r.stacks.iter()
                .filter(|s| a.category.as_ref().map(|c| &s.category == c).unwrap_or(true) && a.language.as_ref().map(|l| &s.language == l).unwrap_or(true))
                .map(stack_summary).collect();
            let mut o = Outcome::with_data(format!("{} stack(s)", list.len()), json!(list));
            for rej in &r.rejected { o = o.warn(format!("extension rejected: {rej}")); }
            Ok(o)
        },
    )?;

    reg.define::<DescribeArgs>("stacks.describe", "Describe one stack")
    .description("The complete definition: versions and their tool requirements, every option with its type, choices, default and the CLI flag it maps to, tasks, services, environment variables, conventions and official documentation. Includes a JSON Schema for the options object.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("stacks.describe", format!("describe {}", a.stack)),
        |ctx, a| Ok(Outcome::with_data(format!("{} definition", a.stack), describe(&stacks(ctx), &a.stack)?)),
    )?;

    reg.define::<NoArgs>("stacks.schema", "Machine-readable schemas")
    .description("JSON Schemas for stack definitions, initialisation requests, plans (with validation issues), reports, project context, tasks and tool plans, so clients can discover the API instead of hardcoding it.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("stacks.schema", "return the schemas"),
        |_, _| {
            let s = |v: schemars::schema::RootSchema| serde_json::to_value(v).unwrap_or(Value::Null);
            Ok(Outcome::with_data("schemas", json!({
                "stack_definition": s(schemars::schema_for!(crate::model::StackDef)),
                "tool_catalogue": s(schemars::schema_for!(crate::tools::Catalogue)),
                "init_request": s(schemars::schema_for!(InitRequest)),
                "init_plan": s(schemars::schema_for!(init::InitPlan)),
                "validation_issue": s(schemars::schema_for!(init::Issue)),
                "init_report": s(schemars::schema_for!(init::InitReport)),
                "project_context": s(schemars::schema_for!(ProjectContext)),
                "task": s(schemars::schema_for!(TaskInfo)),
                "run_task_request": s(schemars::schema_for!(RunTask)),
                "task_result": s(schemars::schema_for!(tasks::TaskResult)),
                "tool_plan": s(schemars::schema_for!(crate::tools::ToolPlan)),
                "service_state": s(schemars::schema_for!(crate::tools::ServiceState)),
            })))
        },
    )?;

    reg.define::<InitRequest>("stacks.validate", "Validate an initialisation request")
    .description("Resolves a project.init request without running it: validation issues (field, code, message), the exact commands that would run, prerequisite checks against the installed tools and the services the project will need. Never changes anything.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("stacks.validate", format!("validate {} {}", a.stack, a.name)),
        |ctx, a| {
            let r = stacks(ctx);
            let p = init::plan(ctx, &r, a)?;
            let msg = if !p.valid { format!("{} problem(s) with the request", p.issues.len()) } else if !p.ready { "valid, but prerequisites are missing".into() } else { "valid and ready".to_string() };
            Ok(Outcome::with_data(msg, serde_json::to_value(&p)?))
        },
    )?;

    reg.define::<NoArgs>("stacks.matrix", "Compatibility matrix")
    .description("Which framework versions need which tool versions, and which package managers each tool can be installed with. Generated from the registry, so it cannot drift from what devd enforces.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("stacks.matrix", "return the compatibility matrix"),
        |ctx, _| {
            let r = stacks(ctx);
            Ok(Outcome::with_data("compatibility matrix", crate::matrix::json(&r)))
        },
    )?;

    // -------------------------------------------------------------- init
    reg.define::<InitRequest>("project.init", "Create a project from a stack")
    .description("Creates a new project with the stack's official scaffolder. The request is validated, prerequisites are checked, and the exact commands are shown in the plan before anything runs. Afterwards devd verifies the expected files, git state and a build/check command, registers the project and reports honestly: ok, partial (created but a check failed), failed or cancelled. Progress arrives as stack.progress events; stacks.cancel stops it.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, a| {
            let r = stacks(ctx);
            let p = init::plan(ctx, &r, a)?;
            let mut plan = Plan::new("project.init", if p.valid {
                format!("create {} project `{}` in {}", p.stack_name, p.name, p.parent_dir)
            } else {
                format!("the request has {} problem(s)", p.issues.len())
            });
            for s in &p.steps { plan = plan.step(format!("{}: {}", s.title, s.display)); }
            plan = plan.effect(format!("creates {}", p.project_dir));
            if p.steps.iter().any(|s| s.network) { plan = plan.effect("downloads packages from the network"); }
            for i in &p.issues { plan = plan.effect(format!("PROBLEM {}: {}", i.field, i.message)); }
            for q in p.prerequisites.iter().filter(|q| q.status != "ok") { plan = plan.effect(format!("PREREQUISITE {} is {}", q.name, q.status)); }
            Ok(plan.data(serde_json::to_value(&p)?))
        },
        |ctx, a| {
            let r = stacks(ctx);
            let report = init::run(ctx, &r, a)?;
            let msg = match report.status.as_str() {
                "ok" => format!("created {}", report.project_dir),
                "partial" => format!("created {} but verification failed", report.project_dir),
                "cancelled" => "cancelled".to_string(),
                _ => format!("failed: {}", report.error.clone().unwrap_or_default()),
            };
            let ok = report.status == "ok";
            let mut o = Outcome::with_data(msg, serde_json::to_value(&report)?);
            o.ok = ok;
            for w in &report.warnings { o = o.warn(w.clone()); }
            Ok(o)
        },
    )?;

    // -------------------------------------------------------------- detect / import
    reg.define::<PathArgs>("project.detect", "Understand an existing project")
    .description("Inspects a directory and returns what it is: matching stacks with evidence and confidence, package manager, runtime versions the project asks for and whether the installed ones satisfy them, entry points, scripts, tests/lint/typecheck tooling, services it needs, environment-variable names (never values), documentation files, git state, official docs and conventions, and any missing tools with the capability that fixes each. Works on projects devd did not create. When detection is uncertain it says so (ambiguous: true).")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("project.detect", format!("inspect {}", a.path)),
        |ctx, a| {
            let r = stacks(ctx);
            let (_, c) = detect_ctx(ctx, &r, &a.path, a.stack.as_deref())?;
            let mut o = Outcome::with_data(
                match &c.primary { Some(p) => format!("{} ({} confidence)", p, c.stacks.first().map(|s| s.confidence.clone()).unwrap_or_default()), None => "no known stack detected".into() },
                serde_json::to_value(&c)?,
            );
            if let Some(a) = &c.ambiguity { o = o.warn(a.clone()); }
            for w in &c.warnings { o = o.warn(w.clone()); }
            Ok(o)
        },
    )?;

    reg.define::<ImportArgs>("project.import", "Import an existing project")
    .description("Registers a directory (a Git checkout or any project) with devd without changing its files: detects the stack, package manager, tasks and services, stores the result and makes the project available to agents. Agents can only import under the projects root.")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, a| {
            let d = dir_arg(ctx, &a.path)?;
            Ok(Plan::new("project.import", format!("register {} (no files are changed)", d.display())))
        },
        |ctx, a| {
            let r = stacks(ctx);
            let (dir, c) = detect_ctx(ctx, &r, &a.path, a.stack.as_deref())?;
            let now = devd_core::util::now();
            ctx.store.upsert_stack_project(&devd_core::StackProject {
                path: dir.display().to_string(),
                name: a.name.clone().unwrap_or_else(|| c.name.clone()),
                stack_id: c.primary.clone(),
                package_manager: c.package_manager.clone(),
                source: "import".into(),
                context: serde_json::to_string(&c)?,
                created_at: now.clone(),
                updated_at: now,
            })?;
            let mut o = Outcome::with_data(format!("imported {}", dir.display()), serde_json::to_value(&c)?);
            if c.ambiguous { o = o.warn(c.ambiguity.clone().unwrap_or_default()); }
            for m in &c.missing { o = o.warn(format!("{}: {} - {}", m.tool, m.why, m.fix)); }
            Ok(o)
        },
    )?;

    reg.define::<NoArgs>("project.list", "List known projects")
    .description("Every project devd knows about: the ones it created or imported, the repositories that were indexed, and the folders a site serves even if nobody imported them. One list, so the application, the terminal and an agent all see the same projects. Each row says where the project is, what stack was detected, how it was added, whether the folder is still there, and which sites serve it.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("project.list", "list projects"),
        |ctx, _| {
            let rows = ctx.store.list_stack_projects()?;
            let mut v: Vec<Value> = Vec::new();
            let mut seen: Vec<String> = Vec::new();
            // The sites that serve each project, by the one definition of which project a site is of.
            let sites = ctx.store.list_sites()?;
            let sites_of = |path: &str| -> Vec<String> {
                let want = authz::normalise(Path::new(path));
                sites.iter().filter(|s| devd_core::util::same_folder(&authz::normalise(&s.project_dir()), &want)).map(|s| s.name.clone()).collect()
            };
            for p in &rows {
                let path = authz::normalise(Path::new(&p.path)).display().to_string();
                seen.push(path.clone());
                v.push(json!({
                    "sites": sites_of(&p.path),
                    "path": path,
                    "name": p.name,
                    "stack": p.stack_id,
                    "package_manager": p.package_manager,
                    "source": p.source,
                    "registered": true,
                    "is_repository": Path::new(&p.path).join(".git").exists(),
                    "exists": Path::new(&p.path).is_dir(),
                    "updated_at": p.updated_at,
                }));
            }
            // A repository that was indexed but never imported is still a
            // project the person works on, so it belongs in the same list.
            for r in ctx.store.list_repos()? {
                let path = authz::normalise(Path::new(&r.path)).display().to_string();
                if seen.iter().any(|s| s == &path) {
                    continue;
                }
                v.push(json!({
                    "path": path,
                    "name": r.name,
                    "stack": Value::Null,
                    "package_manager": Value::Null,
                    "source": "repository",
                    "registered": false,
                    "is_repository": true,
                    "exists": Path::new(&r.path).is_dir(),
                    "sites": sites_of(&r.path),
                    "updated_at": r.last_seen_at,
                }));
            }
            // A folder a site serves is a project the person is working on even when it was
            // never imported or indexed - which is the common case for a folder they added
            // through the Sites screen. Without this, the Sites screen would be the only place
            // those projects appeared, and Projects would look like it had lost them.
            let mut by_folder: Vec<(String, Vec<String>)> = Vec::new();
            for s in &sites {
                let folder = authz::normalise(&s.project_dir()).display().to_string();
                match by_folder.iter_mut().find(|(p, _)| *p == folder) {
                    Some((_, names)) => names.push(s.name.clone()),
                    None => by_folder.push((folder, vec![s.name.clone()])),
                }
            }
            for (path, names) in by_folder {
                if seen.iter().any(|s| s == &path) {
                    continue;
                }
                seen.push(path.clone());
                v.push(json!({
                    "path": path,
                    "name": Path::new(&path).file_name().and_then(|n| n.to_str()).unwrap_or(&path),
                    "stack": Value::Null,
                    "package_manager": Value::Null,
                    "source": "site",
                    "registered": false,
                    "is_repository": Path::new(&path).join(".git").exists(),
                    "exists": Path::new(&path).is_dir(),
                    "sites": names,
                    "updated_at": Value::Null,
                }));
            }
            v.sort_by(|a, b| {
                let an = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let bn = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                an.to_lowercase().cmp(&bn.to_lowercase())
            });
            Ok(Outcome::with_data(format!("{} project(s)", v.len()), json!(v)))
        },
    )?;

    reg.define::<ForgetArgs>("project.forget", "Remove a project from devd")
    .description("Removes a project from devd's list, whether it was imported or indexed as a repository. The folder and everything in it are never touched: this only means devd forgets about it.")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("project.forget", format!("forget {} (the folder is kept)", a.path))),
        |ctx, a| {
            // Match by folder, not by spelling: a stored path may be `D:\x`, `D:/x` or a different
            // case, and a removal that silently misses the row is reported as "not registered".
            let want = authz::normalise(Path::new(&a.path));
            let mut removed = false;
            for row in ctx.store.list_stack_projects()? {
                if devd_core::util::same_folder(&authz::normalise(Path::new(&row.path)), &want) && ctx.store.delete_stack_project(&row.path)? {
                    removed = true;
                }
            }
            for row in ctx.store.list_repos()? {
                if devd_core::util::same_folder(&authz::normalise(Path::new(&row.path)), &want) && ctx.store.delete_repo(&row.path)? {
                    removed = true;
                }
            }
            let p = want.display().to_string();
            if ctx.store.delete_stack_project(&p)? {
                removed = true;
            }
            if ctx.store.delete_repo(&p)? {
                removed = true;
            }
            Ok(Outcome::ok(if removed {
                format!("forgot {p}; the folder is untouched")
            } else {
                format!("{p} was not registered")
            }))
        },
    )?;

    // -------------------------------------------------------------- tasks
    reg.define::<PathArgs>("project.tasks", "List a project's tasks")
    .description("The workflow commands available in this project - dev, build, test, lint, format, typecheck, install, migrate - from the stack definition and from the project's own package.json/composer.json scripts and Makefile, each with its exact command, risk class (safe, mutating, destructive) and whether it can run now.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("project.tasks", format!("list tasks in {}", a.path)),
        |ctx, a| {
            let r = stacks(ctx);
            let (_, c) = detect_ctx(ctx, &r, &a.path, a.stack.as_deref())?;
            Ok(Outcome::with_data(format!("{} task(s)", c.tasks.len()), serde_json::to_value(&c.tasks)?))
        },
    )?;

    reg.define::<RunTask>("project.run", "Run a project task")
    .description("Runs a task from project.tasks in the project directory with the right executable and package manager. Output is streamed (stack.output events), secrets are removed, and the result carries exit code, duration, output tail and whether the task is safe to retry. Long-running tasks (dev servers) start in the background with a log. Destructive tasks are refused here - use project.run_destructive, which needs a human's approval.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, a| {
            let r = stacks(ctx);
            let (_, c) = detect_ctx(ctx, &r, &a.path, None)?;
            let t = pick_task(&c, &a.task)?;
            let mut p = Plan::new("project.run", format!("run `{}`", t.command)).step(format!("in {}", a.path));
            if t.risk == Risk::Destructive { p = p.effect("DESTRUCTIVE: refused here; use project.run_destructive"); }
            if t.risk == Risk::Mutating { p = p.effect("changes files or data; not necessarily safe to repeat"); }
            if t.long_running { p = p.effect("starts a background process"); }
            if !t.available { p = p.effect(format!("CANNOT RUN: {}", t.unavailable_reason.clone().unwrap_or_default())); }
            Ok(p.data(serde_json::to_value(t)?))
        },
        |ctx, a| run_task(ctx, a, false),
    )?;

    reg.define::<RunTask>("project.run_destructive", "Run a destructive project task")
    .description("Runs a task classified as destructive (drops or resets data, deploys, force-pushes, migrate:fresh, db:reset...). Always needs a human's approval and is never available to agents.")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let r = stacks(ctx);
            let (_, c) = detect_ctx(ctx, &r, &a.path, None)?;
            let t = pick_task(&c, &a.task)?;
            Ok(Plan::new("project.run_destructive", format!("run `{}`", t.command)).step(format!("in {}", a.path)).effect("can destroy data; it cannot be undone by devd").destructive(true).data(serde_json::to_value(t)?))
        },
        |ctx, a| run_task(ctx, a, true),
    )?;

    reg.define::<ExecArgs>("project.exec", "Run a framework command")
    .description("Runs a program from the stack's allow-list (for Laravel: php, composer, vendor/bin/pint...; for Django: python, django-admin...) in the project, with argv - never a shell. Use it for framework commands that are not tasks, such as `php artisan make:model Post` or `python manage.py startapp blog`. Commands containing destructive arguments (migrate:fresh, db:drop, --force...) are refused here.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, a| {
            let r = stacks(ctx);
            let (_, c) = detect_ctx(ctx, &r, &a.path, None)?;
            let stack = c.primary.as_deref().and_then(|s| r.stack(s));
            let mut p = Plan::new("project.exec", format!("run `{}`", crate::template::display(&exec::redact_argv(&a.argv)))).step(format!("in {}", a.path));
            if let Some(m) = tasks::is_destructive(stack, &a.argv) { p = p.effect(format!("DESTRUCTIVE ({m}): refused here; use project.exec_destructive")); }
            if ctx.actor.is_agent() {
                if let Some(flag) = tasks::inline_code(&a.argv) { p = p.effect(format!("REFUSED for an agent: `{flag}` gives an interpreter code to run")); }
            }
            Ok(p)
        },
        |ctx, a| exec_cmd(ctx, a, false),
    )?;

    reg.define::<PathArgs>("project.terminal", "Open a terminal in a project")
    .description("Opens the operating system's own terminal window in the project folder, with the runtimes the project is pinned to (and the devd command) first on PATH. Human only: it hands a person an interactive shell.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .human_only()
    .handlers(
        |_, a| Ok(Plan::new("project.terminal", format!("open a terminal in {}", a.path)).step("pinned runtimes and devd first on PATH")),
        |ctx, a| {
            let msg = crate::terminal::open(ctx, std::path::Path::new(&a.path))?;
            Ok(Outcome::with_data(msg, json!({ "path": a.path })))
        },
    )?;

    reg.define::<ExecArgs>("project.exec_destructive", "Run a destructive framework command")
    .description("Like project.exec for commands that can destroy data. Always needs a human's approval; never available to agents.")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("project.exec_destructive", format!("run `{}`", crate::template::display(&exec::redact_argv(&a.argv)))).step(format!("in {}", a.path)).effect("can destroy data").destructive(true)),
        |ctx, a| exec_cmd(ctx, a, true),
    )?;

    reg.define::<NoArgs>("task.list", "List background tasks")
    .description("Dev servers and other long-running tasks devd started, with pid, state and log path.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("task.list", "list background tasks"),
        |_, _| {
            let v = tasks::background();
            Ok(Outcome::with_data(format!("{} background task(s)", v.len()), json!(v)))
        },
    )?;

    reg.define::<ProcArgs>("task.stop", "Stop a background task")
    .description("Stops a background task and everything it started.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("task.stop", format!("stop {}", a.name))),
        |_, a| {
            let r = tasks::stop(&a.name)?;
            Ok(Outcome::ok(match r { Some(_) => format!("stopped {}", a.name), None => format!("{} was not running", a.name) }))
        },
    )?;

    reg.define::<ProcArgs>("task.logs", "Read a background task's log")
    .description("The last lines of a background task's output, with secrets removed.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("task.logs", format!("read the log of {}", a.name)),
        |_, a| {
            let procs = devd_core::process::shared().list();
            let p = procs.iter().find(|p| p.name == a.name).ok_or_else(|| DevdError::not_found(format!("no background task `{}`", a.name)))?;
            let lines = devd_core::process::shared().tail_log(Path::new(&p.log_path), a.lines.unwrap_or(200).min(2000))?;
            let lines: Vec<String> = lines.iter().map(|l| exec::redact_line(l)).collect();
            Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({"running": p.running, "exit_code": p.exit_code, "lines": lines})))
        },
    )?;

    reg.define::<RunIdArgs>("stacks.cancel", "Cancel a running operation")
    .description("Stops a project.init, project.run or toolchain operation that is in progress, killing the whole process tree.")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("stacks.cancel", format!("cancel run {}", a.run_id))),
        |_, a| {
            let did = runs::cancel(&a.run_id);
            Ok(Outcome::ok(if did { "cancellation requested".to_string() } else { "that run is not in progress".to_string() }))
        },
    )?;

    reg.define::<RunsArgs>("stacks.runs", "Execution history")
    .description("Recent initialisations, tasks and toolchain changes with their redacted commands, outcome and duration.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("stacks.runs", "list history"),
        |ctx, a| {
            let rows = ctx.store.list_stack_runs(a.subject.as_deref(), a.limit.unwrap_or(30).clamp(1, 200))?;
            Ok(Outcome::with_data(format!("{} run(s)", rows.len()), serde_json::to_value(rows)?))
        },
    )?;

    reg.define::<LogArgs>("stacks.log", "Read a run's output")
    .description("The last lines of the output log of a run, with secrets removed. Works while the run is in progress.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("stacks.log", format!("read the log of {}", a.run_id)),
        |ctx, a| {
            if !a.run_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(DevdError::invalid("invalid run id"));
            }
            let dir = ctx.paths.logs().join("stacks");
            let mut text = None;
            for prefix in ["init", "task", "toolchain"] {
                if let Ok(t) = std::fs::read_to_string(dir.join(format!("{prefix}-{}.log", a.run_id))) { text = Some(t); break; }
            }
            let text = text.ok_or_else(|| DevdError::not_found(format!("no log for run {}", a.run_id)))?;
            let mut lines: Vec<String> = text.lines().map(exec::redact_line).collect();
            let n = a.lines.unwrap_or(200).min(5000);
            if lines.len() > n { lines.drain(0..lines.len() - n); }
            Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({"running": runs::active().contains(&a.run_id), "lines": lines})))
        },
    )?;

    // -------------------------------------------------------------- toolchain
    reg.define::<ToolListArgs>("toolchain.list", "List tools and services")
    .description("Every tool in the catalogue (Git, Node.js and its package managers, PHP, Composer, Python, Ruby, Go, Java, .NET, Rust, MySQL, MariaDB, PostgreSQL, Redis, SQLite, Docker, GitHub CLI, adb, the Android SDK, Expo/EAS and utilities) with a live probe: installed, version, path, whether it is below the supported minimum, and for services whether the port answers and the health check passes. Also returns the host: OS, package managers found, whether devd has root or passwordless sudo.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("toolchain.list", "probe the toolchain"),
        |ctx, a| {
            let r = stacks(ctx);
            let (host, views) = toolchain::overview(ctx, &r);
            let views: Vec<_> = views.into_iter().filter(|v| a.category.as_ref().map(|c| &v.tool.category == c).unwrap_or(true)).collect();
            let installed = views.iter().filter(|v| v.status.installed).count();
            Ok(Outcome::with_data(format!("{installed} of {} installed", views.len()), json!({"host": host, "tools": views})))
        },
    )?;

    reg.define::<CheckArgs>("toolchain.check", "Check a stack's prerequisites")
    .description("Whether the installed tools satisfy a stack and framework version: each requirement is ok, missing, too_old or too_new, with the fix.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("toolchain.check", format!("check prerequisites of {}", a.stack)),
        |ctx, a| {
            let r = stacks(ctx);
            let req = InitRequest { stack: a.stack.clone(), name: "sample-app".into(), version: a.version.clone(), package_manager: a.package_manager.clone(), ..Default::default() };
            let p = init::plan(ctx, &r, &req)?;
            let bad = p.prerequisites.iter().filter(|q| q.status != "ok").count();
            Ok(Outcome::with_data(if bad == 0 { "all prerequisites satisfied".to_string() } else { format!("{bad} prerequisite(s) not satisfied") }, serde_json::to_value(&p.prerequisites)?))
        },
    )?;

    reg.define::<ToolPlanArgs>("toolchain.plan", "Plan a tool install or removal")
    .description("Exactly what installing or removing a tool would do on this machine: the package manager chosen, the commands, whether administrator rights are needed, the trust model of the source, upgrade notes, and blockers that stop it being automated here.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("toolchain.plan", format!("plan {}", a.tool)),
        |ctx, a| {
            let r = stacks(ctx);
            let p = toolchain::plan(ctx, &r, &a.tool, a.action.as_deref().unwrap_or("install"))?;
            Ok(Outcome::with_data(if p.blockers.is_empty() { "can be automated".to_string() } else { p.blockers[0].clone() }, serde_json::to_value(&p)?))
        },
    )?;

    for (id, action, summary) in [("toolchain.install", "install", "Install a tool"), ("toolchain.remove", "remove", "Remove a tool")] {
        let desc = if action == "install" {
            "Installs a tool through the host's package manager or a digest-verified devd runtime download. Already-installed tools are not reinstalled. The result is verified by running the tool, not by trusting an exit code. Needs approval; privileged installs run through `sudo -n` or root only, and are refused with the exact command when neither is available."
        } else {
            "Removes a tool through the package manager that installs it. Data directories (databases!) are never deleted. Needs approval."
        };
        reg.define::<ToolArgs>(id, summary)
    .description(desc)
    .tier(Tier::Admin)
    .effect(Effect::System)
    .agent_safe()
    .dry_run()
    .handlers(
        move |ctx, a| {
                let r = stacks(ctx);
                let p = toolchain::plan(ctx, &r, &a.tool, action)?;
                let mut plan = Plan::new(if action == "install" { "toolchain.install" } else { "toolchain.remove" }, format!("{action} {}", a.tool));
                for d in &p.display { plan = plan.step(d.clone()); }
                if p.privileged { plan = plan.effect("needs administrator rights"); }
                if let Some(v) = &p.already_installed { plan = plan.effect(format!("already installed ({v}); nothing will change")); }
                for b in &p.blockers { plan = plan.effect(format!("BLOCKED: {b}")); }
                plan = plan.effect(format!("source: {}", p.trust)).approval(true);
                Ok(plan.data(serde_json::to_value(&p)?))
            },
        move |ctx, a| {
                let r = stacks(ctx);
                let p = toolchain::plan(ctx, &r, &a.tool, action)?;
                if action == "install" {
                    if let Some(v) = &p.already_installed {
                        return Ok(Outcome::with_data(format!("{} is already installed ({v})", a.tool), serde_json::to_value(&p)?));
                    }
                }
                let run_id = devd_core::util::new_id();
                let cancel = runs::register(&run_id);
                let res = toolchain::apply(ctx, &r, &p, &cancel);
                runs::finish(&run_id);
                let change = res?;
                let mut o = Outcome::with_data(
                    if change.verified { format!("{action}ed {} (verified: {})", a.tool, change.after.version.clone().unwrap_or_else(|| "present".into())).replace("removeed", "removed") } else { format!("{action} finished but {} is not in the expected state", a.tool) },
                    serde_json::to_value(&change)?,
                );
                o.ok = change.verified;
                for n in &change.notes { o = o.warn(n.clone()); }
                Ok(o)
            },
    )?;
    }

    reg.define::<NoArgs>("services.inventory", "Services found on this machine")
    .description("Every database and service devd knows (MySQL, MariaDB, PostgreSQL, Redis, Docker): whether it is installed, whether it is actually answering (a detected service is never called healthy), who holds its port, which of your projects need it, what you decided about it, the one recommended next step and every option with its risk. Changes nothing.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("services.inventory", "inspect the services on this machine"),
        |ctx, _| {
            let r = stacks(ctx);
            let v = reconcile::inventory(ctx, &r)?;
            let n = v["services"].as_array().map(|a| a.len()).unwrap_or(0);
            let ready = v["services"].as_array().map(|a| a.iter().filter(|s| s["status"] == "ready").count()).unwrap_or(0);
            Ok(Outcome::with_data(format!("{ready} of {n} services ready"), v))
        },
    )?;

    reg.define::<DecideService>("services.decide", "Decide how devd treats a service")
    .description("Records whether devd should adopt an existing service (use and control it as it is), only observe it, treat an instance you run yourself as the project's service, or ignore it. Records a decision only: nothing is installed, stopped, replaced or reconfigured, and data is never touched. Use clear=true to forget the decision.")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| {
            if !a.clear.unwrap_or(false) && !reconcile::MODES.contains(&a.mode.as_deref().unwrap_or("")) { return Err(DevdError::invalid("mode must be adopt, observe, external or ignore")); }
            Ok(Plan::new("services.decide", format!("{} {}", if a.clear.unwrap_or(false) { "forget the decision about".to_string() } else { format!("record `{}` for", a.mode.clone().unwrap_or_default()) }, a.tool)).effect("no installation, process or data is changed"))
        },
        |ctx, a| {
            let r = stacks(ctx);
            let t = r.tool(&a.tool).ok_or_else(|| DevdError::not_found(format!("no tool `{}`", a.tool)))?;
            if t.service.is_none() { return Err(DevdError::unsupported(format!("{} is not a service", t.name))); }
            if a.clear.unwrap_or(false) {
                ctx.store.clear_service_decision(&a.tool)?;
                return Ok(Outcome::ok(format!("forgot the decision about {}", t.name)));
            }
            let mode = a.mode.clone().unwrap_or_default();
            if !reconcile::MODES.contains(&mode.as_str()) { return Err(DevdError::invalid("mode must be adopt, observe, external or ignore")); }
            if mode == "external" && a.port.is_none() { return Err(DevdError::invalid("external needs the port your instance listens on")); }
            ctx.store.set_service_decision(&a.tool, &mode, a.port.map(|p| p as i64), a.note.as_deref(), &ctx.actor.label())?;
            Ok(Outcome::ok(format!("{}: {mode}", t.name)))
        },
    )?;

    reg.define::<ToolArgs>("toolchain.status", "Service status")
    .description("State of a service tool: installed, listening, healthy, which unit controls it, port conflicts, data and log locations and its security defaults.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("toolchain.status", format!("check {}", a.tool)),
        |ctx, a| {
            let r = stacks(ctx);
            let t = r.tool(&a.tool).ok_or_else(|| DevdError::not_found(format!("no tool `{}`", a.tool)))?;
            let host = Host::detect();
            let st = t.probe(&toolchain::managed_dirs(ctx));
            let s = t.service_state(&host, &st).ok_or_else(|| DevdError::unsupported(format!("{} is not a service", t.name)))?;
            Ok(Outcome::with_data(s.detail.clone(), serde_json::to_value(&s)?))
        },
    )?;

    reg.define::<ServiceArgs>("toolchain.service", "Start, stop or restart a service")
    .description("Controls a database or other service through systemd/service (privileged) or, where the host has no service manager, the tool's own daemon mode. Waits until the port answers and the health check passes, or until it has really stopped - a zero exit code from the service manager is not accepted as proof. Refuses to start when another program holds the port.")
    .tier(Tier::Admin)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, a| {
            if !["start", "stop", "restart"].contains(&a.action.as_str()) { return Err(DevdError::invalid("action must be start, stop or restart")); }
            let r = stacks(ctx);
            let t = r.tool(&a.tool).ok_or_else(|| DevdError::not_found(format!("no tool `{}`", a.tool)))?;
            let host = Host::detect();
            let st = t.probe(&toolchain::managed_dirs(ctx));
            let s = t.service_state(&host, &st).ok_or_else(|| DevdError::unsupported(format!("{} is not a service", t.name)))?;
            let mut p = Plan::new("toolchain.service", format!("{} {}", a.action, t.name)).approval(true);
            match t.service_command(&a.action, &host, &s) {
                Ok((argv, priv_)) => { p = p.step(crate::template::display(&argv)); if priv_ { p = p.effect("needs administrator rights"); } }
                Err(e) => p = p.effect(format!("BLOCKED: {e}")),
            }
            if let Some(who) = &s.port_conflict { p = p.effect(format!("port {} is held by `{who}`", s.port)); }
            Ok(p.data(serde_json::to_value(&s)?))
        },
        |ctx, a| {
            if !["start", "stop", "restart"].contains(&a.action.as_str()) { return Err(DevdError::invalid("action must be start, stop or restart")); }
            let r = stacks(ctx);
            {
                if let Err(why) = reconcile::control_allowed(ctx, &a.tool) { return Err(DevdError::conflict(why)); }
            }
            let s = toolchain::control(ctx, &r, &a.tool, &a.action)?;
            Ok(Outcome::with_data(s.detail.clone(), serde_json::to_value(&s)?))
        },
    )?;

    reg.define::<ToolArgs>("toolchain.logs", "Service logs")
    .description("Recent log lines for a service, where the host's journal can provide them.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| read_plan("toolchain.logs", format!("read logs of {}", a.tool)),
        |ctx, a| {
            let r = stacks(ctx);
            let lines: Vec<String> = toolchain::service_logs(ctx, &r, &a.tool)?;
            Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({"lines": lines})))
        },
    )?;

    reg.define::<NoArgs>("toolchain.devices", "Connected Android devices")
    .description("Devices and emulators visible to adb, with their state. Starts the adb server if needed.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("toolchain.devices", "list adb devices"),
        |ctx, _| {
            let r = stacks(ctx);
            let adb = r.tool("adb").map(|t| t.probe(&toolchain::managed_dirs(ctx)));
            if !adb.as_ref().map(|a| a.installed).unwrap_or(false) {
                return Err(DevdError::conflict("adb is not installed; install it with toolchain.install { tool: \"adb\" }"));
            }
            let res = exec::run_streaming(
                &exec::RunSpec { argv: vec!["adb".into(), "devices".into(), "-l".into()], cwd: std::env::temp_dir(), env: vec![], extra_path: toolchain::managed_dirs(ctx), timeout: Some(std::time::Duration::from_secs(30)) },
                &AtomicBool::new(false), None, &mut |_| {})?;
            let devices: Vec<Value> = res.tail.iter()
                .filter(|l| !l.starts_with("List of devices") && !l.starts_with('*') && !l.trim().is_empty())
                .map(|l| {
                    let mut it = l.split_whitespace();
                    let serial = it.next().unwrap_or_default();
                    let state = it.next().unwrap_or_default();
                    let extra: Vec<String> = it.map(|s| s.to_string()).collect();
                    json!({"serial": serial, "state": state, "details": extra, "usable": state == "device", "note": match state { "unauthorized" => "accept the debugging prompt on the device", "offline" => "reconnect the device", _ => "" }})
                }).collect();
            Ok(Outcome::with_data(format!("{} device(s)", devices.len()), json!(devices)))
        },
    )?;

    reg.define::<NoArgs>("toolchain.eas_projects", "Expo projects linked to this machine")
    .description("The Expo projects this machine is signed in to, with the slug and the account that owns them. Needs the EAS CLI and an Expo account; if EAS is not signed in it says so rather than guessing.")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| read_plan("toolchain.eas_projects", "list Expo projects"),
        |ctx, _| {
            let r = stacks(ctx);
            let eas = r.tool("eas-cli").map(|t| t.probe(&toolchain::managed_dirs(ctx)));
            if !eas.as_ref().map(|e| e.installed).unwrap_or(false) {
                return Err(DevdError::conflict("the EAS CLI is not installed; install it with toolchain.install { tool: \"eas-cli\" }"));
            }
            let res = exec::run_streaming(
                &exec::RunSpec { argv: vec!["eas".into(), "project:list".into(), "--json".into(), "--non-interactive".into()], cwd: std::env::temp_dir(), env: vec![], extra_path: toolchain::managed_dirs(ctx), timeout: Some(std::time::Duration::from_secs(60)) },
                &AtomicBool::new(false), None, &mut |_| {})?;
            let text = res.tail.join("\n");
            // EAS answers with JSON when it can and with a sentence when it
            // cannot. Both are reported; neither is invented.
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            let projects: Vec<Value> = match &parsed {
                serde_json::Value::Array(items) => items
                    .iter()
                    .map(|p| json!({
                        "slug": p.get("slug").and_then(|v| v.as_str()).unwrap_or_default(),
                        "name": p.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
                        "owner": p.get("ownerAccount").and_then(|v| v.as_str()).unwrap_or_default(),
                    }))
                    .collect(),
                _ => Vec::new(),
            };
            if projects.is_empty() {
                return Ok(Outcome::with_data(
                    String::from("no Expo projects"),
                    json!({ "projects": [], "message": devd_core::util::truncate(text.trim(), 400), "signed_in": false }),
                ));
            }
            Ok(Outcome::with_data(
                format!("{} Expo project(s)", projects.len()),
                json!({ "projects": projects, "signed_in": true }),
            ))
        },
    )?;

    Ok(())
}

fn run_task(ctx: &Ctx, a: &RunTask, destructive: bool) -> Result<Outcome> {
    let r = stacks(ctx);
    let (dir, c) = detect_ctx(ctx, &r, &a.path, None)?;
    let t = pick_task(&c, &a.task)?;
    if t.risk == Risk::Destructive && !destructive {
        return Err(DevdError::denied(format!("`{}` can destroy data. Use project.run_destructive, which needs a human's approval.", t.command)));
    }
    if t.risk != Risk::Destructive && destructive {
        return Err(DevdError::invalid("that task is not destructive; use project.run"));
    }
    let v = tasks::run(ctx, &dir, t, &a.args, a)?;
    let mut o;
    if v.get("mode").and_then(|m| m.as_str()) == Some("background") {
        o = Outcome::with_data(format!("started `{}` in the background", t.command), v);
    } else {
        let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
        let code = v.get("exit_code").cloned().unwrap_or(Value::Null);
        o = Outcome::with_data(
            if ok { format!("`{}` succeeded", t.command) } else if v["timed_out"].as_bool() == Some(true) { format!("`{}` timed out", t.command) } else if v["cancelled"].as_bool() == Some(true) { format!("`{}` was cancelled", t.command) } else { format!("`{}` failed with exit code {}", t.command, code) },
            v,
        );
        o.ok = ok;
        if !ok {
            if let Some(h) = install_hint(&dir) { o = o.warn(h); }
        }
    }
    Ok(o)
}

fn exec_cmd(ctx: &Ctx, a: &ExecArgs, destructive: bool) -> Result<Outcome> {
    let r = stacks(ctx);
    let (dir, c) = detect_ctx(ctx, &r, &a.path, None)?;
    let stack = c.primary.as_deref().and_then(|s| r.stack(s));
    if ctx.actor.is_agent() {
        if let Some(flag) = tasks::inline_code(&a.argv) {
            return Err(DevdError::denied(format!(
                "`{flag}` gives an interpreter code to run, which the program allow-list does not cover. An agent can run the project's own scripts and commands; a person can run this one"
            )));
        }
    }
    let mark = tasks::is_destructive(stack, &a.argv);
    if mark.is_some() && !destructive {
        return Err(DevdError::denied(format!("`{}` contains `{}` and can destroy data. Use project.exec_destructive, which needs a human's approval.", a.argv.join(" "), mark.unwrap_or_default())));
    }
    if mark.is_none() && destructive {
        return Err(DevdError::invalid("that command is not destructive; use project.exec"));
    }
    let res = tasks::exec_allowed(ctx, &r, stack, c.package_manager.as_deref().unwrap_or(""), &dir, &a.argv, a.timeout_secs.unwrap_or(600))?;
    let display = exec::redact_argv(&a.argv).join(" ");
    let _ = ctx.store.begin_stack_run("exec", &dir.display().to_string(), "project.exec", &display, &ctx.actor.label(), None)
        .and_then(|id| ctx.store.finish_stack_run(&id, res.code, res.ok, res.duration_ms as i64));
    let ok = res.ok;
    let mut o = Outcome::with_data(
        if ok { format!("`{display}` succeeded") } else { format!("`{display}` failed with exit code {}", res.code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())) },
        json!({"command": display, "exit_code": res.code, "ok": res.ok, "timed_out": res.timed_out, "duration_ms": res.duration_ms, "output_tail": res.tail, "safe_to_retry": false}),
    );
    o.ok = ok;
    Ok(o)
}
