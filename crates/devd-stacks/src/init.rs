//! The project initialisation engine.
//!
//! One planner and one runner serve the CLI, the TUI, the window, the HTTP API
//! and agents. A request is validated against the stack definition, resolved to
//! concrete argv arrays, checked against the installed tools, and only then run.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, DevdError, Result};

use crate::authz;
use crate::env::{self, EnvInput};
use crate::exec::{self, RunSpec};
use crate::model::*;
use crate::registry::StackRegistry;
use crate::template::{self, Env};
use crate::toolchain;
use crate::tools::{satisfies, ToolStatus};

/// A request to create a project.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct InitRequest {
    /// Stack id, from `stack.list`.
    pub stack: String,
    /// Project name (also the directory name).
    pub name: String,
    /// Framework version id; defaults to the stack's default.
    #[serde(default)]
    pub version: Option<String>,
    /// Package manager id; defaults to the stack's first.
    #[serde(default)]
    pub package_manager: Option<String>,
    /// Directory that will contain the project; defaults to the projects root.
    #[serde(default)]
    pub destination: Option<String>,
    /// Option values by id; anything omitted takes its default.
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
    /// Initialise a git repository (default true).
    #[serde(default)]
    pub git: Option<bool>,
    /// Install dependencies (default true).
    #[serde(default)]
    pub install: Option<bool>,
    /// Extra arguments appended to the main scaffolding command.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Caller-chosen id so progress can be followed and the run cancelled.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// One thing wrong with a request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Issue {
    /// The request field, e.g. `options.database`.
    pub field: String,
    /// Stable machine code.
    pub code: String,
    /// Human explanation.
    pub message: String,
}

fn issue(field: &str, code: &str, message: impl Into<String>) -> Issue {
    Issue { field: field.into(), code: code.into(), message: message.into() }
}

/// The state of one prerequisite.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Prereq {
    /// Tool id.
    pub tool: String,
    /// Tool name.
    pub name: String,
    /// `ok`, `missing`, `too_old`, `too_new` or `unknown_version`.
    pub status: String,
    /// Minimum required.
    pub min: Option<String>,
    /// Maximum allowed.
    pub max: Option<String>,
    /// Version found.
    pub found: Option<String>,
    /// Why it is needed.
    pub why: String,
    /// What to do about it.
    pub fix: Option<String>,
}

/// A step as it will run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlannedStep {
    /// Step id.
    pub id: String,
    /// Title.
    pub title: String,
    /// `exec`, `write`, `fetch`, `env`, `json`, `mkdir` or `git`.
    pub kind: String,
    /// Command line, or a description for file steps.
    pub display: String,
    /// The argv, for exec steps.
    pub argv: Vec<String>,
    /// Where it runs.
    pub cwd: String,
    /// Uses the network.
    pub network: bool,
    /// Failure is only a warning.
    pub optional: bool,
    /// Receives the user's extra arguments.
    pub extensible: bool,
}

/// A service the project will need.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceNeed {
    /// Tool id.
    pub tool: String,
    /// Installed on this machine.
    pub installed: bool,
    /// Accepting connections.
    pub running: bool,
    /// Why the project needs it.
    pub why: String,
}

/// A validated, resolved plan.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InitPlan {
    /// Stack id.
    pub stack: String,
    /// Stack name.
    pub stack_name: String,
    /// Framework version id.
    pub version: String,
    /// Package manager id.
    pub package_manager: String,
    /// Project name.
    pub name: String,
    /// Final project directory.
    pub project_dir: String,
    /// Containing directory.
    pub parent_dir: String,
    /// Initialise git.
    pub git: bool,
    /// Install dependencies.
    pub install: bool,
    /// Option values after defaults.
    pub options: BTreeMap<String, Value>,
    /// The main scaffolding command, as a person could paste it.
    pub command: String,
    /// Steps in order.
    pub steps: Vec<PlannedStep>,
    /// Prerequisite checks.
    pub prerequisites: Vec<Prereq>,
    /// Services the project will need.
    pub services: Vec<ServiceNeed>,
    /// Files verification will look for.
    pub expected_files: Vec<String>,
    /// Problems with the request. Empty when valid.
    pub issues: Vec<Issue>,
    /// Request is valid.
    pub valid: bool,
    /// Valid and every prerequisite is satisfied.
    pub ready: bool,
    /// Things worth knowing.
    pub warnings: Vec<String>,
    /// Official documentation.
    pub docs: Vec<DocLink>,
}

fn coerce(o: &OptionDef, v: &Value) -> std::result::Result<Value, String> {
    match o.kind {
        OptionKind::Bool => match v {
            Value::Bool(_) => Ok(v.clone()),
            Value::String(s) if s == "true" || s == "false" => Ok(json!(s == "true")),
            _ => Err("must be true or false".into()),
        },
        OptionKind::Text => match v {
            Value::String(s) => {
                if let Some(p) = &o.pattern {
                    if !Regex::new(p).map(|re| re.is_match(s)).unwrap_or(false) {
                        return Err(format!("`{s}` does not match the required format ({p})"));
                    }
                }
                if s.contains('\n') || s.contains('\0') {
                    return Err("must be a single line".into());
                }
                Ok(v.clone())
            }
            _ => Err("must be text".into()),
        },
        OptionKind::Select => {
            let s = v.as_str().ok_or("must be one of the listed choices")?;
            if o.choices.iter().any(|c| c.value == s) {
                Ok(v.clone())
            } else {
                Err(format!(
                    "`{s}` is not available; choose one of: {}",
                    o.choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>().join(", ")
                ))
            }
        }
        OptionKind::Multi => {
            let items: Vec<String> = match v {
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
                Value::String(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                _ => return Err("must be a list of choices".into()),
            };
            for i in &items {
                if !o.choices.iter().any(|c| &c.value == i) {
                    return Err(format!("`{i}` is not one of: {}", o.choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>().join(", ")));
                }
            }
            Ok(json!(items))
        }
    }
}

/// Check a list of tool requirements against probed tools.
pub fn check_requirements(reg: &StackRegistry, reqs: &[(ToolReq, String)], tools: &BTreeMap<String, ToolStatus>) -> Vec<Prereq> {
    let mut out: Vec<Prereq> = Vec::new();
    for (r, why) in reqs {
        let name = reg.tool(&r.tool).map(|t| t.name.clone()).unwrap_or_else(|| r.tool.clone());
        let st = tools.get(&r.tool);
        let (status, found, fix) = match st {
            Some(s) if s.installed => match &s.version {
                Some(v) if !satisfies(v, r.min.as_deref(), None) => (
                    "too_old",
                    Some(v.clone()),
                    Some(format!("upgrade {name} to {} or newer (toolchain.install {})", r.min.clone().unwrap_or_default(), r.tool)),
                ),
                Some(v) if !satisfies(v, None, r.max.as_deref()) => (
                    "too_new",
                    Some(v.clone()),
                    Some(format!("this version of the stack supports {name} up to {}; choose a newer framework version", r.max.clone().unwrap_or_default())),
                ),
                Some(v) => ("ok", Some(v.clone()), None),
                None => ("unknown_version", None, None),
            },
            _ => ("missing", None, Some(format!("install {name} (toolchain.install {})", r.tool))),
        };
        let p = Prereq {
            tool: r.tool.clone(),
            name,
            status: status.into(),
            min: r.min.clone(),
            max: r.max.clone(),
            found,
            why: if r.why.is_empty() { why.clone() } else { r.why.clone() },
            fix,
        };
        // Keep the strictest finding per tool.
        if let Some(prev) = out.iter_mut().find(|x| x.tool == p.tool) {
            if prev.status == "ok" && p.status != "ok" {
                *prev = p;
            } else if prev.status == "ok" && p.status == "ok" && p.min.is_some() {
                prev.min = p.min.clone().or(prev.min.clone());
            }
        } else {
            out.push(p);
        }
    }
    out
}

/// Validate a request and resolve it to a plan. Problems with the request are
/// returned in `issues`; an `Err` means devd itself could not plan.
pub fn plan(ctx: &Ctx, reg: &StackRegistry, req: &InitRequest) -> Result<InitPlan> {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let stack = reg
        .stack(&req.stack)
        .ok_or_else(|| DevdError::not_found(format!("no stack `{}`; see stack.list", req.stack)))?;

    if stack.init.steps.is_empty() {
        return Err(DevdError::unsupported(format!(
            "{} projects cannot be created by devd - it detects and runs them. {}",
            stack.name,
            stack.docs.first().map(|d| format!("See {}", d.url)).unwrap_or_default()
        )));
    }

    // Version.
    let version = match &req.version {
        Some(v) => match stack.version(v) {
            Some(x) => x,
            None => {
                issues.push(issue("version", "unknown_version", format!("`{v}` is not a version of {}; choose one of: {}", stack.name, stack.versions.iter().map(|v| v.id.as_str()).collect::<Vec<_>>().join(", "))));
                stack.default_version()
            }
        },
        None => stack.default_version(),
    };

    // Package manager.
    let pm = match &req.package_manager {
        Some(p) if stack.package_managers.contains(p) => p.clone(),
        Some(p) => {
            issues.push(issue("package_manager", "unsupported_package_manager", format!("{} does not support `{p}`; choose one of: {}", stack.name, stack.package_managers.join(", "))));
            stack.package_managers.first().cloned().unwrap_or_default()
        }
        None => stack.package_managers.first().cloned().unwrap_or_default(),
    };

    // Name.
    if !Regex::new(&stack.name_pattern).map(|re| re.is_match(&req.name)).unwrap_or(false) {
        issues.push(issue(
            "name",
            "invalid_name",
            format!(
                "`{}` is not a valid {} project name. {}",
                req.name,
                stack.name,
                if stack.name_hint.is_empty() { format!("It must match {}", stack.name_pattern) } else { stack.name_hint.clone() }
            ),
        ));
    }
    let name_ok = !issues.iter().any(|i| i.field == "name");

    // Destination.
    let parent = match &req.destination {
        Some(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => ctx.settings.effective_www_root(&ctx.paths.www()),
    };
    let parent = match authz::authorize_dir(ctx, &parent) {
        Ok(p) => p,
        Err(e) => {
            issues.push(issue("destination", "destination_not_allowed", e.to_string()));
            authz::normalise(&parent)
        }
    };
    let project_dir = parent.join(if name_ok { req.name.as_str() } else { "project" });
    if name_ok {
        if project_dir.exists() {
            let empty = project_dir.is_dir() && std::fs::read_dir(&project_dir).map(|mut d| d.next().is_none()).unwrap_or(false);
            if !empty {
                issues.push(issue("name", "destination_not_empty", format!("{} already exists and is not empty; choose another name or destination", project_dir.display())));
            }
        }
        // The projects root must be creatable for agents; for humans any parent that exists or can be made.
        if !parent.exists() {
            warnings.push(format!("{} does not exist and will be created", parent.display()));
        }
    }

    // Toggles.
    let git = req.git.unwrap_or(true);
    let install = req.install.unwrap_or(true);
    if !stack.init.install_toggle && req.install == Some(false) {
        warnings.push(format!("{} installs its dependencies as part of creation; `install: false` has no effect", stack.name));
    }

    // Options.
    let mut name_env = Env::default();
    for (k, v) in env::name_forms(&req.name) {
        name_env.vars.insert(k, v);
    }
    name_env.vars.insert("name".into(), req.name.clone());
    let mut options: BTreeMap<String, Value> = BTreeMap::new();
    for key in req.options.keys() {
        if !stack.options.iter().any(|o| &o.id == key) {
            issues.push(issue(&format!("options.{key}"), "unknown_option", format!("{} has no option `{key}`; options are: {}", stack.name, stack.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>().join(", "))));
        }
    }
    for o in &stack.options {
        let applies = o.versions.is_empty() || o.versions.contains(&version.id);
        match req.options.get(&o.id) {
            Some(v) if applies => match coerce(o, v) {
                Ok(c) => {
                    options.insert(o.id.clone(), c);
                }
                Err(m) => issues.push(issue(&format!("options.{}", o.id), "invalid_option", format!("{}: {m}", o.label))),
            },
            Some(_) => issues.push(issue(&format!("options.{}", o.id), "option_not_for_version", format!("{} does not apply to {}", o.label, version.label))),
            None => {
                let d = match &o.default {
                    Value::String(s) => json!(template::expand_text(s, &name_env).unwrap_or_else(|_| s.clone())),
                    Value::Null => match o.kind {
                        OptionKind::Bool => json!(false),
                        OptionKind::Multi => json!([]),
                        _ => json!(""),
                    },
                    other => other.clone(),
                };
                if applies {
                    options.insert(o.id.clone(), d);
                }
            }
        }
    }
    // Text defaults are validated like user input.
    for o in &stack.options {
        if o.kind == OptionKind::Text && !req.options.contains_key(&o.id) {
            if let Some(v) = options.get(&o.id).cloned() {
                if let Err(m) = coerce(o, &v) {
                    if name_ok {
                        issues.push(issue(&format!("options.{}", o.id), "invalid_option", format!("{}: default {m}; set it explicitly", o.label)));
                    }
                }
            }
        }
    }

    // Extra arguments.
    let extensible = stack.init.steps.iter().any(|s| s.extensible);
    if !req.extra_args.is_empty() {
        if !extensible {
            issues.push(issue("extra_args", "extra_args_unsupported", format!("{} has no scaffolding command that takes extra arguments", stack.name)));
        }
        if req.extra_args.len() > 50 || req.extra_args.iter().any(|a| a.is_empty() || a.contains('\0') || a.contains('\n') || a.len() > 500) {
            issues.push(issue("extra_args", "invalid_extra_args", "extra arguments must be at most 50 non-empty single-line strings"));
        }
    }

    // Tools.
    let dirs = toolchain::managed_dirs(ctx);
    let mut reqs: Vec<(ToolReq, String)> = version.requires.iter().cloned().map(|r| (r, format!("{} {}", stack.name, version.label))).collect();
    let mut need_ids: Vec<String> = Vec::new();
    if let Some(r) = &stack.runtime {
        need_ids.push(r.clone());
    }
    if let Some(p) = reg.catalogue.package_managers.iter().find(|p| p.id == pm) {
        need_ids.push(p.tool.clone());
    }
    {
        let no_tools: BTreeMap<String, ToolStatus> = BTreeMap::new();
        let env0 = env::build(&EnvInput {
            stack,
            catalogue: &reg.catalogue,
            version,
            pm: &pm,
            name: &req.name,
            dir: &project_dir,
            parent: &parent,
            options: &options,
            git,
            install,
            init_phase: true,
            tools: &no_tools,
        });
        for s in &stack.init.steps {
            if step_active_env(s, &env0) {
                need_ids.extend(s.needs.iter().cloned());
            }
        }
    }
    if git {
        need_ids.push("git".into());
    }
    for n in need_ids {
        if !reqs.iter().any(|(r, _)| r.tool == n) {
            reqs.push((ToolReq { tool: n.clone(), min: None, max: None, why: String::new() }, format!("{} needs it", stack.name)));
        }
    }
    let svc_ids: Vec<String> = stack.services.iter().map(|s| s.tool.clone()).collect();
    let mut probe_ids: Vec<String> = reqs.iter().map(|(r, _)| r.tool.clone()).collect();
    probe_ids.extend(svc_ids.iter().cloned());
    probe_ids.push("yarn".into());
    let tools = toolchain::probe(reg, &probe_ids, &dirs);

    let venv_for_env = options.clone();
    let env_input = EnvInput {
        stack,
        catalogue: &reg.catalogue,
        version,
        pm: &pm,
        name: &req.name,
        dir: &project_dir,
        parent: &parent,
        options: &venv_for_env,
        git,
        install,
        init_phase: true,
        tools: &tools,
    };
    let env = env::build(&env_input);

    // Steps.
    let mut steps = Vec::new();
    let mut main_command = String::new();
    if issues.is_empty() || name_ok {
        for s in &stack.init.steps {
            if !step_active_env(s, &env) {
                continue;
            }
            let cwd = match s.cwd {
                StepCwd::Project => project_dir.display().to_string(),
                StepCwd::Parent => {
                    if stack.init.creates_dir { project_dir.display().to_string() } else { parent.display().to_string() }
                }
            };
            let kind_name = match s.kind {
                StepKind::Exec => "exec",
                StepKind::Write => "write",
                StepKind::Fetch => "fetch",
                StepKind::Env => "env",
                StepKind::Json => "json",
                StepKind::Mkdir => "mkdir",
            };
            let (argv, display) = match s.kind {
                StepKind::Exec => {
                    let mut argv = match template::expand(&s.run, &env) {
                        Ok(a) => a,
                        Err(e) => {
                            issues.push(issue(&format!("steps.{}", s.id), "template_error", e.to_string()));
                            Vec::new()
                        }
                    };
                    if s.extensible {
                        argv.extend(req.extra_args.iter().cloned());
                    }
                    let d = template::display(&argv);
                    (argv, d)
                }
                StepKind::Write => (vec![], format!("write {}", expand_or(&s.path, &env))),
                StepKind::Fetch => (vec![], format!("download and unpack {}", expand_or(&s.path, &env))),
                StepKind::Env => (vec![], format!("update {} ({})", expand_or(&s.path, &env), s.set.keys().cloned().collect::<Vec<_>>().join(", "))),
                StepKind::Json => (vec![], format!("update {} ({})", expand_or(&s.path, &env), s.set.keys().cloned().collect::<Vec<_>>().join(", "))),
                StepKind::Mkdir => (vec![], format!("create {}", expand_or(&s.path, &env))),
            };
            if s.extensible {
                main_command = display.clone();
            }
            steps.push(PlannedStep {
                id: s.id.clone(),
                title: s.title.clone(),
                kind: kind_name.into(),
                display,
                argv,
                cwd,
                network: s.network,
                optional: s.optional,
                extensible: s.extensible,
            });
        }
        if git {
            steps.push(PlannedStep {
                id: "git-init".into(),
                title: "Initialise a git repository (skipped if the scaffolder already did)".into(),
                kind: "git".into(),
                display: format!("git init -b {}", ctx.settings.git_default_branch),
                argv: vec!["git".into(), "init".into(), "-b".into(), ctx.settings.git_default_branch.clone()],
                cwd: project_dir.display().to_string(),
                network: false,
                optional: false,
                extensible: false,
            });
        }
    }
    if main_command.is_empty() {
        main_command = steps.iter().find(|s| s.kind == "exec").map(|s| s.display.clone()).unwrap_or_default();
    }

    // Prerequisites and services.
    let prerequisites = check_requirements(reg, &reqs, &tools);
    let services: Vec<ServiceNeed> = stack
        .services
        .iter()
        .filter(|sv| sv.when.as_ref().map(|w| w.conds().iter().all(|c| template::eval_cond(c, &env))).unwrap_or(true))
        .map(|sv| {
            let st = tools.get(&sv.tool);
            let running = reg
                .tool(&sv.tool)
                .and_then(|t| t.service.as_ref())
                .map(|s| std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], s.port).into(), Duration::from_millis(300)).is_ok())
                .unwrap_or(false);
            ServiceNeed { tool: sv.tool.clone(), installed: st.map(|s| s.installed).unwrap_or(false), running, why: sv.why.clone() }
        })
        .collect();
    for sv in &services {
        if !sv.installed {
            warnings.push(format!("{} will need {} ({}), which is not installed. The project is created either way; install it from Toolchain before running migrations.", stack.name, sv.tool, sv.why));
        } else if !sv.running {
            warnings.push(format!("{} is installed but not running ({}).", sv.tool, sv.why));
        }
    }
    let expected_files: Vec<String> = stack
        .verify
        .files
        .iter()
        .filter_map(|f| template::expand_text(f, &env).ok())
        .collect();
    let ready = issues.is_empty() && prerequisites.iter().all(|p| p.status == "ok" || p.status == "unknown_version");
    Ok(InitPlan {
        stack: stack.id.clone(),
        stack_name: stack.name.clone(),
        version: version.id.clone(),
        package_manager: pm,
        name: req.name.clone(),
        project_dir: project_dir.display().to_string(),
        parent_dir: parent.display().to_string(),
        git,
        install,
        options,
        command: main_command,
        steps,
        prerequisites,
        services,
        expected_files,
        valid: issues.is_empty(),
        ready,
        issues,
        warnings,
        docs: stack.docs.clone(),
    })
}

fn expand_or(p: &Option<String>, env: &Env) -> String {
    p.as_ref().map(|s| template::expand_text(s, env).unwrap_or_else(|_| s.clone())).unwrap_or_default()
}

fn step_active_env(s: &Step, env: &Env) -> bool {
    s.when.as_ref().map(|w| w.conds().iter().all(|c| template::eval_cond(c, env))).unwrap_or(true)
}

/// One executed step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepResult {
    /// Step id.
    pub id: String,
    /// Title.
    pub title: String,
    /// `ok`, `failed`, `warning`, `skipped` or `cancelled`.
    pub status: String,
    /// Exit code for exec steps.
    pub exit_code: Option<i32>,
    /// Duration in milliseconds.
    pub duration_ms: u128,
    /// Explanation or the end of the output on failure.
    pub detail: String,
}

/// One verification check.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    /// What was checked.
    pub title: String,
    /// `ok`, `failed` or `skipped`.
    pub status: String,
    /// Evidence.
    pub detail: String,
}

/// The outcome of an initialisation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InitReport {
    /// Run id.
    pub run_id: String,
    /// `ok`, `partial` (created but a check failed), `failed` or `cancelled`.
    pub status: String,
    /// Project directory.
    pub project_dir: String,
    /// Step results.
    pub steps: Vec<StepResult>,
    /// Verification results.
    pub checks: Vec<Check>,
    /// A `.git` directory exists.
    pub git_initialised: bool,
    /// Notes.
    pub warnings: Vec<String>,
    /// What to do next.
    pub next: Vec<String>,
    /// Total time.
    pub duration_ms: u128,
    /// Full output log.
    pub log_path: String,
    /// Why it did not succeed.
    pub error: Option<String>,
    /// Detected project context, when the project exists.
    pub context: Option<Value>,
}

fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir)) {
        return Err(DevdError::invalid(format!("`{rel}` must be a relative path inside the project")));
    }
    Ok(base.join(p))
}

fn upsert_env(path: &Path, set: &BTreeMap<String, String>) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    for (k, v) in set {
        let quoted = if v.chars().any(|c| c.is_whitespace() || c == '#' || c == '"') { format!("\"{}\"", v.replace('"', "\\\"")) } else { v.clone() };
        let new = format!("{k}={quoted}");
        let active = format!("{k}=");
        let commented = format!("# {k}=");
        let commented2 = format!("#{k}=");
        if let Some(i) = lines.iter().position(|l| l.starts_with(&active)) {
            lines[i] = new;
        } else if let Some(i) = lines.iter().position(|l| l.starts_with(&commented) || l.starts_with(&commented2)) {
            lines[i] = new;
        } else {
            lines.push(new);
        }
    }
    util::atomic_write(path, &(lines.join("\n") + "\n"))
}

fn set_json(path: &Path, set: &BTreeMap<String, String>) -> Result<()> {
    let text = std::fs::read_to_string(path).map_err(|e| DevdError::not_found(format!("{}: {e}", path.display())))?;
    let mut root: Value = serde_json::from_str(&text)?;
    for (dotted, val) in set {
        let mut cur = &mut root;
        let parts: Vec<&str> = dotted.split('.').collect();
        for (i, part) in parts.iter().enumerate() {
            if i + 1 == parts.len() {
                cur[*part] = json!(val);
            } else {
                if !cur[*part].is_object() {
                    cur[*part] = json!({});
                }
                cur = &mut cur[*part];
            }
        }
    }
    util::atomic_write(path, &(serde_json::to_string_pretty(&root)? + "\n"))
}

fn fetch_zip(url: &str, dest: &Path, ctx: &Ctx, id: &str) -> Result<()> {
    if !url.starts_with("https://start.spring.io/") {
        return Err(DevdError::denied("downloads are limited to hosts named in the stack registry"));
    }
    let cache = ctx.paths.cache().join("stacks");
    let art = devd_core::Artifact::new(url).filename(format!("{id}.zip"));
    let fetched = devd_core::download::fetch(&art, &cache, None)?;
    let tmp = dest.with_file_name(format!(".{}-unpack", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    // `extract` strips a single top-level directory; the Initializr zip has files at its root.
    devd_core::archive::extract(&fetched.path, &tmp, false)?;
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(&tmp)? {
        let entry = entry?;
        std::fs::rename(entry.path(), dest.join(entry.file_name()))?;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_file(&fetched.path);
    Ok(())
}

/// Run an initialisation. `req` is re-validated; nothing runs unless the plan
/// is valid and every prerequisite is satisfied.
pub fn run(ctx: &Ctx, reg: &StackRegistry, req: &InitRequest) -> Result<InitReport> {
    let plan = plan(ctx, reg, req)?;
    if !plan.valid {
        return Err(DevdError::invalid(format!(
            "the request is not valid: {}",
            plan.issues.iter().map(|i| format!("{} ({})", i.message, i.field)).collect::<Vec<_>>().join("; ")
        )));
    }
    if !plan.ready {
        let bad: Vec<String> = plan
            .prerequisites
            .iter()
            .filter(|p| p.status != "ok" && p.status != "unknown_version")
            .map(|p| {
                format!(
                    "{} is {}{}{}",
                    p.name,
                    p.status.replace('_', " "),
                    p.found.as_ref().map(|f| format!(" (found {f})")).unwrap_or_default(),
                    p.min.as_ref().map(|m| format!(", needs {m} or newer")).unwrap_or_default()
                )
            })
            .collect();
        return Err(DevdError::conflict(format!("prerequisites are not met: {}. Use toolchain.install to fix them.", bad.join("; "))));
    }
    let stack = reg.stack(&plan.stack).ok_or_else(|| DevdError::internal("stack vanished"))?;
    let run_id = req.run_id.clone().unwrap_or_else(util::new_id);
    let cancel = exec::runs::register(&run_id);
    let started = Instant::now();
    let log = ctx.paths.logs().join("stacks").join(format!("init-{run_id}.log"));
    let history = ctx
        .store
        .begin_stack_run("init", &plan.project_dir, &format!("Create {} project {}", plan.stack_name, plan.name), &exec::redact_line(&plan.command), &ctx.actor.label(), Some(&log.display().to_string()))
        .ok();
    let result = run_steps(ctx, reg, stack, req, &plan, &run_id, &cancel, &log, started);
    exec::runs::finish(&run_id);
    if let (Some(h), Ok(r)) = (&history, &result) {
        let _ = ctx.store.finish_stack_run(h, None, r.status == "ok", started.elapsed().as_millis() as i64);
    } else if let Some(h) = &history {
        let _ = ctx.store.finish_stack_run(h, None, false, started.elapsed().as_millis() as i64);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_steps(
    ctx: &Ctx,
    reg: &StackRegistry,
    stack: &StackDef,
    req: &InitRequest,
    plan: &InitPlan,
    run_id: &str,
    cancel: &AtomicBool,
    log: &Path,
    started: Instant,
) -> Result<InitReport> {
    let project_dir = PathBuf::from(&plan.project_dir);
    let parent = PathBuf::from(&plan.parent_dir);
    let dirs = toolchain::managed_dirs_for(ctx, &project_dir);
    let emit = |msg: String, data: Value| ctx.events.emit_with("stack.progress", msg, data);
    std::fs::create_dir_all(&parent)?;
    let existed_before = project_dir.exists();
    if stack.init.creates_dir {
        std::fs::create_dir_all(&project_dir)?;
    }

    let tools = toolchain::probe(reg, &["yarn".to_string()], &dirs);
    let version = stack.version(&plan.version).unwrap_or_else(|| stack.default_version());
    let env_input = EnvInput {
        stack,
        catalogue: &reg.catalogue,
        version,
        pm: &plan.package_manager,
        name: &plan.name,
        dir: &project_dir,
        parent: &parent,
        options: &plan.options,
        git: plan.git,
        install: plan.install,
        init_phase: true,
        tools: &tools,
    };
    let env = env::build(&env_input);

    let mut results: Vec<StepResult> = Vec::new();
    let mut warnings = plan.warnings.clone();
    let mut error: Option<String> = None;
    let mut cancelled = false;

    let defs: Vec<&Step> = stack.init.steps.iter().filter(|s| step_active_env(s, &env)).collect();
    let mut exec_index = 0usize;
    for (n, planned) in plan.steps.iter().enumerate() {
        let t0 = Instant::now();
        emit(format!("[{}/{}] {}", n + 1, plan.steps.len(), planned.title), json!({"run_id": run_id, "step": planned.id, "index": n, "total": plan.steps.len(), "state": "start"}));
        let def = if planned.kind == "git" { None } else { defs.get(exec_index).copied() };
        if planned.kind != "git" {
            exec_index += 1;
        }
        let mut res = StepResult { id: planned.id.clone(), title: planned.title.clone(), status: "ok".into(), exit_code: None, duration_ms: 0, detail: String::new() };
        let outcome: Result<()> = (|| -> Result<()> {
            match planned.kind.as_str() {
                "exec" => {
                    let def = def.ok_or_else(|| DevdError::internal("step definition missing"))?;
                    let cwd = PathBuf::from(&planned.cwd);
                    std::fs::create_dir_all(&cwd)?;
                    let mut envv: Vec<(String, String)> = def.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    envv.push(("npm_config_yes".into(), "true".into()));
                    let r = exec::run_streaming(
                        &RunSpec { argv: planned.argv.clone(), cwd, env: envv, extra_path: dirs.clone(), timeout: Some(Duration::from_secs(60 * 20)) },
                        cancel,
                        Some(log),
                        &mut |l| emit(l.to_string(), json!({"run_id": run_id, "step": planned.id, "state": "output"})),
                    )?;
                    res.exit_code = r.code;
                    if r.cancelled {
                        res.status = "cancelled".into();
                        return Err(DevdError::conflict("cancelled"));
                    }
                    if r.timed_out {
                        return Err(DevdError::Timeout(format!("`{}` did not finish within 20 minutes", planned.display)));
                    }
                    if !r.ok {
                        res.detail = r.tail.iter().rev().take(12).rev().cloned().collect::<Vec<_>>().join("\n");
                        return Err(DevdError::Command { program: planned.argv.first().cloned().unwrap_or_default(), code: r.code.unwrap_or(-1), stderr: res.detail.clone() });
                    }
                    Ok(())
                }
                "write" => {
                    let def = def.ok_or_else(|| DevdError::internal("step definition missing"))?;
                    let rel = template::expand_text(def.path.as_deref().unwrap_or(""), &env)?;
                    let target = safe_join(&project_dir, &rel)?;
                    if let Some(p) = target.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    util::atomic_write(&target, &template::expand_text(def.content.as_deref().unwrap_or(""), &env)?)?;
                    Ok(())
                }
                "mkdir" => {
                    let def = def.ok_or_else(|| DevdError::internal("step definition missing"))?;
                    let rel = template::expand_text(def.path.as_deref().unwrap_or(""), &env)?;
                    std::fs::create_dir_all(safe_join(&project_dir, &rel)?)?;
                    Ok(())
                }
                "env" | "json" => {
                    let def = def.ok_or_else(|| DevdError::internal("step definition missing"))?;
                    let rel = template::expand_text(def.path.as_deref().unwrap_or(""), &env)?;
                    let target = safe_join(&project_dir, &rel)?;
                    let mut set = BTreeMap::new();
                    for (k, v) in &def.set {
                        set.insert(k.clone(), template::expand_text(v, &env)?);
                    }
                    if planned.kind == "env" {
                        // A missing .env is created from .env.example when the framework ships one.
                        if !target.exists() {
                            let example = project_dir.join(".env.example");
                            if example.exists() {
                                std::fs::copy(&example, &target)?;
                            }
                        }
                        upsert_env(&target, &set)
                    } else {
                        set_json(&target, &set)
                    }
                }
                "fetch" => {
                    let def = def.ok_or_else(|| DevdError::internal("step definition missing"))?;
                    let url = template::expand_text(def.path.as_deref().unwrap_or(""), &env)?;
                    fetch_zip(&url, &project_dir, ctx, run_id)
                }
                "git" => {
                    if project_dir.join(".git").exists() {
                        res.status = "skipped".into();
                        res.detail = "the scaffolder already initialised a repository".into();
                        return Ok(());
                    }
                    let r = exec::run_streaming(
                        &RunSpec { argv: planned.argv.clone(), cwd: project_dir.clone(), env: vec![], extra_path: dirs.clone(), timeout: Some(Duration::from_secs(60)) },
                        cancel,
                        Some(log),
                        &mut |_| {},
                    )?;
                    if !r.ok {
                        return Err(DevdError::Command { program: "git".into(), code: r.code.unwrap_or(-1), stderr: r.tail.join("\n") });
                    }
                    Ok(())
                }
                other => Err(DevdError::internal(format!("unknown step kind `{other}`"))),
            }
        })();
        res.duration_ms = t0.elapsed().as_millis();
        if let Err(e) = outcome {
            if res.status == "cancelled" {
                cancelled = true;
            } else if planned.optional {
                res.status = "warning".into();
                res.detail = e.to_string();
                warnings.push(format!("{}: {e}", planned.title));
            } else {
                res.status = "failed".into();
                if res.detail.is_empty() {
                    res.detail = e.to_string();
                }
                error = Some(format!("{}: {e}", planned.title));
            }
        }
        let failed = res.status == "failed" || res.status == "cancelled";
        emit(format!("{}: {}", planned.title, res.status), json!({"run_id": run_id, "step": planned.id, "state": res.status}));
        results.push(res);
        if failed {
            break;
        }
    }

    let git_initialised = project_dir.join(".git").exists();
    let mut checks = Vec::new();
    let mut status = if cancelled { "cancelled" } else if error.is_some() { "failed" } else { "ok" }.to_string();

    if status == "ok" {
        for f in &plan.expected_files {
            let ok = project_dir.join(f).exists();
            checks.push(Check { title: format!("{f} exists"), status: if ok { "ok" } else { "failed" }.into(), detail: if ok { String::new() } else { format!("{f} was not created") } });
        }
        if plan.git {
            checks.push(Check { title: "git repository".into(), status: if git_initialised { "ok" } else { "failed" }.into(), detail: String::new() });
        } else if git_initialised {
            warnings.push("git was not requested but the scaffolder created a repository".into());
        }
        let dep_check: Vec<&VerifyCommand> = stack.verify.commands.iter().collect();
        for vc in dep_check {
            if vc.needs_install && !plan.install {
                checks.push(Check { title: vc.title.clone(), status: "skipped".into(), detail: "dependencies were not installed".into() });
                continue;
            }
            let argv = match template::expand(&vc.run, &env) {
                Ok(a) => a,
                Err(e) => {
                    checks.push(Check { title: vc.title.clone(), status: "failed".into(), detail: e.to_string() });
                    continue;
                }
            };
            let r = exec::run_streaming(
                &RunSpec { argv: argv.clone(), cwd: project_dir.clone(), env: vec![], extra_path: dirs.clone(), timeout: Some(Duration::from_secs(600)) },
                cancel,
                Some(log),
                &mut |l| emit(l.to_string(), json!({"run_id": run_id, "state": "output"})),
            );
            match r {
                Ok(r) if r.ok => checks.push(Check { title: vc.title.clone(), status: "ok".into(), detail: template::display(&argv) }),
                Ok(r) => checks.push(Check { title: vc.title.clone(), status: "failed".into(), detail: r.tail.iter().rev().take(10).rev().cloned().collect::<Vec<_>>().join("\n") }),
                Err(e) => checks.push(Check { title: vc.title.clone(), status: "failed".into(), detail: e.to_string() }),
            }
        }
        if checks.iter().any(|c| c.status == "failed") {
            status = "partial".into();
            error = Some("the project was created but verification failed; see checks".into());
        }
    }

    // Register the project and capture its context whenever something exists.
    let mut context = None;
    let exists_now = project_dir.exists() && std::fs::read_dir(&project_dir).map(|mut d| d.next().is_some()).unwrap_or(false);
    if exists_now && status != "cancelled" && (status == "ok" || status == "partial") {
        if let Ok(c) = crate::detect::detect(ctx, reg, &project_dir, Some(&plan.stack)) {
            let now = util::now();
            let _ = ctx.store.upsert_stack_project(&devd_core::StackProject {
                path: project_dir.display().to_string(),
                name: plan.name.clone(),
                stack_id: Some(plan.stack.clone()),
                package_manager: Some(plan.package_manager.clone()).filter(|s| !s.is_empty()),
                source: "init".into(),
                context: serde_json::to_string(&c).unwrap_or_default(),
                created_at: now.clone(),
                updated_at: now,
            });
            context = serde_json::to_value(&c).ok();
        }
    }

    let mut next = Vec::new();
    match status.as_str() {
        "ok" | "partial" => {
            if let Some(t) = stack.tasks.iter().find(|t| t.id == "dev" || t.id == "start" || t.id == "run") {
                next.push(format!("Start it: project.run {{ path: \"{}\", task: \"{}\" }}", project_dir.display(), t.id));
            }
            if !plan.install && stack.init.install_toggle {
                next.push("Install dependencies: project.run with task \"install\"".into());
            }
            for sv in &plan.services {
                if !sv.installed {
                    next.push(format!("Install {}: toolchain.install {{ tool: \"{}\" }}", sv.tool, sv.tool));
                } else if !sv.running {
                    next.push(format!("Start {}: toolchain.service {{ tool: \"{}\", action: \"start\" }}", sv.tool, sv.tool));
                }
            }
            if let Some(d) = stack.docs.first() {
                next.push(format!("Read: {}", d.url));
            }
        }
        "failed" => {
            next.push(format!("Read the full log: {}", log.display()));
            if project_dir.exists() && !existed_before {
                next.push(format!("devd left {} in place for inspection. Delete it before retrying with the same name.", project_dir.display()));
            }
        }
        _ => next.push("Cancelled. Any files created so far were left in place.".into()),
    }

    Ok(InitReport {
        run_id: run_id.to_string(),
        status,
        project_dir: plan.project_dir.clone(),
        steps: results,
        checks,
        git_initialised,
        warnings,
        next,
        duration_ms: started.elapsed().as_millis(),
        log_path: log.display().to_string(),
        error,
        context,
    })
    .map(|mut r| {
        let _ = &req;
        if r.status == "cancelled" {
            r.error = Some("cancelled".into());
        }
        r
    })
}

/// Whether the process was asked to stop; exposed for callers that poll.
pub fn is_cancelled(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_files_update_commented_and_active_lines() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(".env");
        std::fs::write(&p, "APP=x\nDB_CONNECTION=sqlite\n# DB_HOST=127.0.0.1\n").unwrap();
        let mut set = BTreeMap::new();
        set.insert("DB_CONNECTION".to_string(), "mysql".to_string());
        set.insert("DB_HOST".to_string(), "10.0.0.1".to_string());
        set.insert("DB_NAME".to_string(), "my app".to_string());
        upsert_env(&p, &set).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("DB_CONNECTION=mysql") && t.contains("DB_HOST=10.0.0.1") && t.contains("DB_NAME=\"my app\""));
        assert!(!t.contains("sqlite") && !t.contains("# DB_HOST"));
    }

    #[test]
    fn json_paths_are_set_without_losing_siblings() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("app.json");
        std::fs::write(&p, r#"{"expo":{"name":"x","ios":{"supportsTablet":true}}}"#).unwrap();
        let mut set = BTreeMap::new();
        set.insert("expo.ios.bundleIdentifier".to_string(), "com.a.b".to_string());
        set.insert("expo.android.package".to_string(), "com.a.b".to_string());
        set_json(&p, &set).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["expo"]["ios"]["supportsTablet"], json!(true));
        assert_eq!(v["expo"]["ios"]["bundleIdentifier"], json!("com.a.b"));
        assert_eq!(v["expo"]["android"]["package"], json!("com.a.b"));
    }

    #[test]
    fn paths_cannot_escape_the_project() {
        let base = Path::new("/tmp/p");
        assert!(safe_join(base, "../x").is_err());
        assert!(safe_join(base, "/etc/passwd").is_err());
        assert!(safe_join(base, "a/b.txt").is_ok());
    }
}
