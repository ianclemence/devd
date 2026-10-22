//! Named workflow commands (dev, build, test, ...), their resolution against a
//! real project, and their execution: one-shot with captured output, or as a
//! managed background process with a log.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::process::{self, SpawnSpec};
use devd_core::{util, DevdError, Result};

use crate::env::{self, EnvInput};
use crate::exec::{self, RunSpec};
use crate::model::*;
use crate::registry::StackRegistry;
use crate::template;
use crate::toolchain;
use crate::tools::ToolStatus;

/// A task as it applies to one project.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskInfo {
    /// Identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Kind.
    pub kind: String,
    /// Side-effect class.
    pub risk: Risk,
    /// Runs until stopped.
    pub long_running: bool,
    /// The command line.
    pub command: String,
    /// The argv.
    pub argv: Vec<String>,
    /// Port a long-running task usually listens on.
    pub port: Option<u16>,
    /// Can run now.
    pub available: bool,
    /// Why it cannot.
    pub unavailable_reason: Option<String>,
    /// `stack` or `project` (a script the project itself defines).
    pub origin: String,
    /// Explanation.
    pub help: String,
}

/// What the project's own manifests define.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProjectScripts {
    /// package.json scripts.
    pub npm: BTreeMap<String, String>,
    /// composer.json scripts (command or list, flattened).
    pub composer: BTreeMap<String, String>,
    /// Makefile targets.
    pub make: Vec<String>,
}

/// Read scripts from the manifests in `dir`.
pub fn read_scripts(dir: &Path) -> ProjectScripts {
    let mut s = ProjectScripts::default();
    if let Ok(t) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            if let Some(o) = v.get("scripts").and_then(|x| x.as_object()) {
                for (k, val) in o {
                    if let Some(c) = val.as_str() {
                        s.npm.insert(k.clone(), c.to_string());
                    }
                }
            }
        }
    }
    if let Ok(t) = std::fs::read_to_string(dir.join("composer.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            if let Some(o) = v.get("scripts").and_then(|x| x.as_object()) {
                for (k, val) in o {
                    let body = match val {
                        Value::String(c) => c.clone(),
                        Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" && "),
                        _ => continue,
                    };
                    s.composer.insert(k.clone(), body);
                }
            }
        }
    }
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(name)) {
            let re = Regex::new(r"(?m)^([A-Za-z0-9][A-Za-z0-9_.-]*)\s*:(?:[^=]|$)").unwrap();
            for c in re.captures_iter(&t) {
                let target = c[1].to_string();
                if !target.starts_with('.') && !s.make.contains(&target) {
                    s.make.push(target);
                }
            }
            break;
        }
    }
    s
}

/// Classify a script by its name and body.
pub fn classify_script(name: &str, body: &str) -> (String, Risk, bool) {
    let n = name.to_ascii_lowercase();
    let b = body.to_ascii_lowercase();
    let destructive = Regex::new(r"(drop|reset|wipe|destroy|purge|nuke|deploy|publish|release|rm -rf|migrate:fresh|--force|truncate|clean:all|db:push)")
        .unwrap()
        .is_match(&format!("{n} {b}"));
    let risk = if destructive { Risk::Destructive } else { Risk::Mutating };
    let has = |words: &[&str]| words.iter().any(|w| n == *w || n.starts_with(&format!("{w}:")) || n.starts_with(&format!("{w}-")));
    let (kind, long, base) = if has(&["dev", "start", "serve", "watch", "preview"]) {
        ("dev", true, Risk::Mutating)
    } else if has(&["build", "compile", "bundle", "generate"]) {
        ("build", false, Risk::Safe)
    } else if has(&["test", "tests", "spec", "e2e", "coverage"]) {
        ("test", false, Risk::Safe)
    } else if has(&["lint", "check", "verify"]) {
        ("lint", false, Risk::Safe)
    } else if has(&["typecheck", "types", "tsc"]) {
        ("typecheck", false, Risk::Safe)
    } else if has(&["format", "fmt", "prettier"]) {
        ("format", false, Risk::Mutating)
    } else {
        ("other", false, risk)
    };
    // Destructive wording always wins over the friendly name.
    let final_risk = if destructive { Risk::Destructive } else { base };
    (kind.to_string(), final_risk, long)
}

fn availability(reg: &StackRegistry, argv: &[String], needs: &[String], dir: &Path, tools: &BTreeMap<String, ToolStatus>, extra: &[PathBuf]) -> (bool, Option<String>) {
    for n in needs {
        if !tools.get(n).map(|t| t.installed).unwrap_or(false) {
            let name = reg.tool(n).map(|t| t.name.clone()).unwrap_or_else(|| n.clone());
            return (false, Some(format!("{name} is not installed (toolchain.install {n})")));
        }
    }
    if let Some(p) = argv.first() {
        if let Err(e) = exec::resolve_program(p, dir, extra) {
            return (false, Some(e.to_string()));
        }
    }
    (true, None)
}

/// Resolve the tasks that apply to the project in `dir`.
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    reg: &StackRegistry,
    stack: Option<&StackDef>,
    dir: &Path,
    pm: &str,
    scripts: &ProjectScripts,
    tools: &BTreeMap<String, ToolStatus>,
    extra: &[PathBuf],
) -> Vec<TaskInfo> {
    let mut out: Vec<TaskInfo> = Vec::new();
    let mut claimed_scripts: Vec<String> = Vec::new();
    if let Some(stack) = stack {
        let version = stack.default_version();
        let options: BTreeMap<String, Value> = stack.options.iter().map(|o| (o.id.clone(), o.default.clone())).collect();
        let env = env::build(&EnvInput {
            stack,
            catalogue: &reg.catalogue,
            version,
            pm,
            name: dir.file_name().and_then(|n| n.to_str()).unwrap_or("project"),
            dir,
            parent: dir.parent().unwrap_or(dir),
            options: &options,
            git: true,
            install: true,
            init_phase: false,
            tools,
        });
        for t in &stack.tasks {
            if let Some(f) = &t.requires_file {
                if !dir.join(f).exists() {
                    continue;
                }
            }
            if let Some(sname) = &t.requires_script {
                if !scripts.npm.contains_key(sname) {
                    continue;
                }
                claimed_scripts.push(sname.clone());
            }
            let Ok(argv) = template::expand(&t.run, &env) else { continue };
            let (available, reason) = availability(reg, &argv, &t.needs, dir, tools, extra);
            out.push(TaskInfo {
                id: t.id.clone(),
                title: t.title.clone(),
                kind: t.kind.clone(),
                risk: t.risk,
                long_running: t.long_running,
                command: template::display(&argv),
                argv,
                port: t.port,
                available,
                unavailable_reason: reason,
                origin: "stack".into(),
                help: t.help.clone(),
            });
        }
    }
    // Scripts the project defines itself.
    let npm_pm = if pm.is_empty() || !["npm", "pnpm", "yarn", "bun"].contains(&pm) { "npm" } else { pm };
    let run_prefix: Vec<String> = reg
        .catalogue
        .package_managers
        .iter()
        .find(|p| p.id == npm_pm)
        .map(|p| p.run.clone())
        .unwrap_or_else(|| vec!["npm".into(), "run".into()]);
    for (name, body) in &scripts.npm {
        if claimed_scripts.contains(name) || out.iter().any(|t| t.id == format!("script:{name}")) {
            continue;
        }
        let (kind, risk, long) = classify_script(name, body);
        let mut argv = run_prefix.clone();
        argv.push(name.clone());
        let (available, reason) = availability(reg, &argv, &[], dir, tools, extra);
        out.push(TaskInfo {
            id: format!("script:{name}"),
            title: format!("package.json script `{name}`"),
            kind,
            risk,
            long_running: long,
            command: template::display(&argv),
            argv,
            port: None,
            available,
            unavailable_reason: reason,
            origin: "project".into(),
            help: format!("Runs: {}", util::truncate(body, 160)),
        });
    }
    for (name, body) in &scripts.composer {
        if name.starts_with("pre-") || name.starts_with("post-") {
            continue;
        }
        let (kind, risk, long) = classify_script(name, body);
        let argv = vec!["composer".to_string(), "run-script".to_string(), name.clone()];
        let (available, reason) = availability(reg, &argv, &[], dir, tools, extra);
        out.push(TaskInfo {
            id: format!("composer:{name}"),
            title: format!("composer script `{name}`"),
            kind,
            risk,
            long_running: long,
            command: template::display(&argv),
            argv,
            port: None,
            available,
            unavailable_reason: reason,
            origin: "project".into(),
            help: format!("Runs: {}", util::truncate(body, 160)),
        });
    }
    for target in &scripts.make {
        let (kind, risk, long) = classify_script(target, "");
        let argv = vec!["make".to_string(), target.clone()];
        let (available, reason) = availability(reg, &argv, &[], dir, tools, extra);
        out.push(TaskInfo {
            id: format!("make:{target}"),
            title: format!("make {target}"),
            kind,
            risk,
            long_running: long,
            command: template::display(&argv),
            argv,
            port: None,
            available,
            unavailable_reason: reason,
            origin: "project".into(),
            help: String::new(),
        });
    }
    out
}

/// Arguments to run a task.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct RunTask {
    /// Project directory.
    pub path: String,
    /// Task id from `project.tasks`.
    pub task: String,
    /// Extra arguments appended to the task's command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Start in the background and return immediately (for servers).
    #[serde(default)]
    pub background: Option<bool>,
    /// Seconds before a foreground task is stopped (default 600).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Caller-chosen id so output can be followed and the run cancelled.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Outcome of a foreground task.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TaskResult {
    /// Run id.
    pub run_id: String,
    /// Task id.
    pub task: String,
    /// The command that ran.
    pub command: String,
    /// Exit code.
    pub exit_code: Option<i32>,
    /// Succeeded.
    pub ok: bool,
    /// Stopped by cancel.
    pub cancelled: bool,
    /// Stopped by timeout.
    pub timed_out: bool,
    /// Duration.
    pub duration_ms: u128,
    /// End of the output, secrets removed.
    pub output_tail: Vec<String>,
    /// Full log.
    pub log_path: String,
    /// Whether repeating the task is safe.
    pub safe_to_retry: bool,
}

fn validate_args(args: &[String]) -> Result<()> {
    if args.len() > 64 || args.iter().any(|a| a.contains('\0') || a.contains('\n') || a.len() > 2000) {
        return Err(DevdError::invalid("arguments must be at most 64 single-line strings"));
    }
    Ok(())
}

/// Name for a background process.
pub fn process_name(dir: &Path, task: &str) -> String {
    let base = dir.file_name().and_then(|n| n.to_str()).unwrap_or("project");
    format!("task:{base}:{task}")
}

/// Run a task (foreground or background). The caller has already authorised
/// the directory and the task's risk class.
pub fn run(ctx: &Ctx, dir: &Path, task: &TaskInfo, args: &[String], req: &RunTask) -> Result<Value> {
    validate_args(args)?;
    if !task.available {
        return Err(DevdError::conflict(format!(
            "task `{}` cannot run: {}",
            task.id,
            task.unavailable_reason.clone().unwrap_or_default()
        )));
    }
    let mut argv = task.argv.clone();
    if !args.is_empty() {
        // `npm run dev -- --port 4000`: the separator is needed for npm-style runners.
        let needs_sep = matches!(argv.first().map(|s| s.as_str()), Some("npm")) && argv.get(1).map(|s| s == "run").unwrap_or(false) && !args.is_empty();
        if needs_sep {
            argv.push("--".into());
        }
        argv.extend(args.iter().cloned());
    }
    let dirs = toolchain::managed_dirs_for(ctx, dir);
    let run_id = req.run_id.clone().unwrap_or_else(util::new_id);
    let log = ctx.paths.logs().join("stacks").join(format!("task-{run_id}.log"));
    let display = exec::redact_argv(&argv).join(" ");
    let hist = ctx.store.begin_stack_run("task", &dir.display().to_string(), &task.title, &display, &ctx.actor.label(), Some(&log.display().to_string())).ok();

    let background = req.background.unwrap_or(task.long_running);
    if background {
        let (program, lead) = exec::resolve_program(&argv[0], dir, &dirs)?;
        let name = process_name(dir, &task.id);
        let mut spec_args: Vec<String> = lead;
        spec_args.extend(argv[1..].iter().cloned());
        let mut envs: Vec<(String, String)> = vec![("CI".into(), "".into()), ("FORCE_COLOR".into(), "0".into()), ("NO_COLOR".into(), "1".into())];
        if !dirs.is_empty() {
            let mut p: Vec<PathBuf> = dirs.clone();
            if let Some(cur) = std::env::var_os("PATH") {
                p.extend(std::env::split_paths(&cur));
            }
            if let Ok(joined) = std::env::join_paths(p) {
                envs.push(("PATH".into(), joined.to_string_lossy().to_string()));
            }
        }
        // `CI=""` would still be "set"; remove it by not passing it at all.
        envs.retain(|(k, _)| k != "CI");
        let spec = SpawnSpec {
            name: name.clone(),
            kind: "task".into(),
            program,
            args: spec_args,
            cwd: Some(dir.to_path_buf()),
            envs,
            clear_env: false,
            log_path: log.clone(),
            restarts: 0,
        };
        let sup = process::shared();
        let pid = sup.start(spec, run_id.clone())?;
        // A server that dies immediately is a failure, not a success.
        std::thread::sleep(Duration::from_millis(900));
        let info = sup.list().into_iter().find(|p| p.name == name);
        let running = info.as_ref().map(|i| i.running).unwrap_or(false);
        let tail = sup.tail_log(&log, 30).unwrap_or_default().into_iter().map(|l| exec::redact_line(&l)).collect::<Vec<_>>();
        if let Some(h) = &hist {
            let _ = ctx.store.finish_stack_run(h, info.as_ref().and_then(|i| i.exit_code), running, 900);
        }
        if !running {
            return Err(DevdError::Command {
                program: argv[0].clone(),
                code: info.and_then(|i| i.exit_code).unwrap_or(-1),
                stderr: format!("the process exited right after starting:\n{}", tail.join("\n")),
            });
        }
        return Ok(json!({
            "mode": "background",
            "process": name,
            "pid": pid,
            "command": display,
            "log_path": log.display().to_string(),
            "port": task.port,
            "url": task.port.map(|p| format!("http://127.0.0.1:{p}")),
            "output_tail": tail,
            "note": "Running in the background. Inspect with task.list and task.logs, stop with task.stop. A listening port is not confirmed; check the log.",
        }));
    }

    let cancel = exec::runs::register(&run_id);
    let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(600).clamp(1, 3600));
    let r = exec::run_streaming(
        &RunSpec { argv: argv.clone(), cwd: dir.to_path_buf(), env: vec![], extra_path: dirs, timeout: Some(timeout) },
        &cancel,
        Some(&log),
        &mut |l| ctx.events.emit_with("stack.output", l.to_string(), json!({"run_id": run_id, "task": task.id})),
    );
    exec::runs::finish(&run_id);
    let r = r?;
    if let Some(h) = &hist {
        let _ = ctx.store.finish_stack_run(h, r.code, r.ok, r.duration_ms as i64);
    }
    let res = TaskResult {
        run_id,
        task: task.id.clone(),
        command: display,
        exit_code: r.code,
        ok: r.ok,
        cancelled: r.cancelled,
        timed_out: r.timed_out,
        duration_ms: r.duration_ms,
        output_tail: r.tail,
        log_path: log.display().to_string(),
        safe_to_retry: task.risk == Risk::Safe,
    };
    Ok(serde_json::to_value(res)?)
}

/// Background processes started by the task runner.
pub fn background() -> Vec<Value> {
    process::shared()
        .list()
        .into_iter()
        .filter(|p| p.kind == "task")
        .map(|p| serde_json::to_value(p).unwrap_or(Value::Null))
        .collect()
}

/// Stop a background task by process name.
pub fn stop(name: &str) -> Result<Option<(i32, String)>> {
    if !name.starts_with("task:") {
        return Err(DevdError::denied("only processes started by the task runner can be stopped here"));
    }
    process::shared().stop(name)
}

/// Run an arbitrary allow-listed program in a project.
pub fn exec_allowed(ctx: &Ctx, reg: &StackRegistry, stack: Option<&StackDef>, pm: &str, dir: &Path, argv: &[String], timeout: u64) -> Result<exec::RunResult> {
    let program = argv.first().ok_or_else(|| DevdError::invalid("empty command"))?;
    validate_args(argv)?;
    let base = Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or(program).trim_end_matches(".exe").to_string();
    let mut allowed: Vec<String> = vec![
        "git".into(), // read-only subcommands are policed below
    ];
    allowed.clear();
    if let Some(s) = stack {
        allowed.extend(s.exec.programs.iter().cloned());
    }
    if let Some(p) = reg.catalogue.package_managers.iter().find(|p| p.id == pm) {
        if let Some(first) = p.run.first() {
            if !first.starts_with('{') {
                allowed.push(first.clone());
            }
        }
    }
    let ok = allowed.iter().any(|a| a == program || Path::new(a).file_name().and_then(|n| n.to_str()) == Some(base.as_str()) || a == &base);
    if !ok {
        return Err(DevdError::denied(format!(
            "`{program}` is not an allowed command for this project's stack. Allowed: {}. For other tools use a task from project.tasks.",
            if allowed.is_empty() { "none (the stack could not be determined)".to_string() } else { allowed.join(", ") }
        )));
    }
    if matches!(base.as_str(), "sh" | "bash" | "zsh" | "cmd" | "powershell" | "pwsh" | "sudo" | "su") {
        return Err(DevdError::denied("shells and privilege tools are never allowed"));
    }
    let dirs = toolchain::managed_dirs_for(ctx, dir);
    let cancel = AtomicBool::new(false);
    exec::run_streaming(
        &RunSpec { argv: argv.to_vec(), cwd: dir.to_path_buf(), env: vec![], extra_path: dirs, timeout: Some(Duration::from_secs(timeout.clamp(1, 3600))) },
        &cancel,
        None,
        &mut |_| {},
    )
}

/// Whether any argument marks the command destructive for this stack.
pub fn is_destructive(stack: Option<&StackDef>, argv: &[String]) -> Option<String> {
    let joined = argv.join(" ");
    let mut generic = vec!["--force", "rm", "drop", "reset --hard", "push --force"];
    generic.retain(|g| g.contains(' ') || *g == "--force");
    let mut marks: Vec<String> = stack.map(|s| s.exec.destructive_args.clone()).unwrap_or_default();
    marks.extend(generic.into_iter().map(|g| g.to_string()));
    marks.into_iter().find(|m| argv.iter().any(|a| a == m) || (m.contains(' ') && joined.contains(m.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_classified_by_name_and_body() {
        assert_eq!(classify_script("dev", "vite").0, "dev");
        assert!(classify_script("dev", "vite").2);
        assert_eq!(classify_script("test:unit", "vitest").1, Risk::Safe);
        assert_eq!(classify_script("build", "tsc").1, Risk::Safe);
        assert_eq!(classify_script("deploy", "ssh host").1, Risk::Destructive);
        assert_eq!(classify_script("db:reset", "prisma migrate reset").1, Risk::Destructive);
        // A friendly name does not hide a destructive body.
        assert_eq!(classify_script("test", "rm -rf data && jest").1, Risk::Destructive);
        assert_eq!(classify_script("whatever", "node x.js").1, Risk::Mutating);
    }

    #[test]
    fn makefile_targets_are_found_and_variables_ignored() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Makefile"), "CC := gcc\n.PHONY: all\nall: build\nbuild:\n\tcc x.c\ntest: build\n\t./a.out\n").unwrap();
        let s = read_scripts(d.path());
        assert_eq!(s.make, vec!["all", "build", "test"]);
    }

    #[test]
    fn destructive_arguments_are_recognised() {
        let reg = StackRegistry::builtin();
        let laravel = reg.stack("laravel");
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_destructive(laravel, &argv(&["php", "artisan", "migrate:fresh"])).is_some());
        assert!(is_destructive(laravel, &argv(&["php", "artisan", "route:list"])).is_none());
    }
}
