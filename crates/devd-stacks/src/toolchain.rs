//! Machine-level tool and service management: probing, install and removal
//! plans, privileged execution through a controlled path, and service control
//! with real readiness checks.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

use crate::exec::{self, RunSpec};
use crate::registry::StackRegistry;
use crate::tools::{Host, ServiceState, ToolPlan, ToolStatus};

/// Directories of runtimes devd itself installed, searched before `PATH`.
pub fn managed_dirs(ctx: &Ctx) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for kind in devd_runtime::KINDS {
        if let Ok(exe) = devd_runtime::executable(ctx, kind, None) {
            if let Some(dir) = exe.parent() {
                let d = dir.to_path_buf();
                if !out.contains(&d) {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// Probe the named tools (unknown ids are skipped).
pub fn probe(reg: &StackRegistry, ids: &[String], dirs: &[PathBuf]) -> BTreeMap<String, ToolStatus> {
    let mut out = BTreeMap::new();
    for id in ids {
        if out.contains_key(id) {
            continue;
        }
        if let Some(t) = reg.tool(id) {
            out.insert(id.clone(), t.probe(dirs));
        }
    }
    out
}

/// Probe every tool in the catalogue, concurrently.
pub fn probe_all(reg: &StackRegistry, dirs: &[PathBuf]) -> BTreeMap<String, ToolStatus> {
    let handles: Vec<_> = reg
        .catalogue
        .tools
        .iter()
        .cloned()
        .map(|t| {
            let dirs = dirs.to_vec();
            std::thread::spawn(move || (t.id.clone(), t.probe(&dirs)))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

/// A tool with its status, plan hint and service state.
#[derive(Debug, Clone, Serialize)]
pub struct ToolView {
    /// Catalogue entry.
    pub tool: crate::tools::ToolDef,
    /// What a probe found.
    pub status: ToolStatus,
    /// Service state, for services.
    pub service: Option<ServiceState>,
}

/// Everything about the machine's toolchain.
pub fn overview(ctx: &Ctx, reg: &StackRegistry) -> (Host, Vec<ToolView>) {
    let host = Host::detect();
    let dirs = managed_dirs(ctx);
    let statuses = probe_all(reg, &dirs);
    let views = reg
        .catalogue
        .tools
        .iter()
        .map(|t| {
            let status = statuses.get(&t.id).cloned().unwrap_or_else(|| t.probe(&dirs));
            let service = t.service_state(&host, &status);
            ToolView { tool: t.clone(), status, service }
        })
        .collect();
    (host, views)
}

/// Build the install or removal plan for one tool.
pub fn plan(ctx: &Ctx, reg: &StackRegistry, id: &str, action: &str) -> Result<ToolPlan> {
    let tool = reg
        .tool(id)
        .ok_or_else(|| DevdError::not_found(format!("no tool `{id}` in the catalogue")))?;
    let host = Host::detect();
    let dirs = managed_dirs(ctx);
    let mut ids = vec![id.to_string()];
    ids.extend(tool.depends.iter().cloned());
    let statuses = probe(reg, &ids, &dirs);
    let status = statuses.get(id).cloned().unwrap_or_else(|| tool.probe(&dirs));
    Ok(tool.plan(action, &host, &status, &statuses, devd_runtime::KINDS))
}

/// Result of an install or removal.
#[derive(Debug, Clone, Serialize)]
pub struct ToolChange {
    /// Tool id.
    pub tool: String,
    /// `install` or `remove`.
    pub action: String,
    /// Manager used.
    pub manager: String,
    /// Commands run and their exit codes.
    pub commands: Vec<serde_json::Value>,
    /// Status after the change, from a fresh probe.
    pub after: ToolStatus,
    /// Whether the end state matches the request.
    pub verified: bool,
    /// Notes the user should read.
    pub notes: Vec<String>,
}

/// Apply a plan. A zero exit code is never accepted as proof: the tool must be
/// found and report a version afterwards (or be gone, for removal).
pub fn apply(ctx: &Ctx, reg: &StackRegistry, plan: &ToolPlan, cancel: &AtomicBool) -> Result<ToolChange> {
    if let Some(b) = plan.blockers.first() {
        return Err(DevdError::unsupported(format!(
            "cannot {} {} here: {b}{}",
            plan.action,
            plan.tool,
            plan.manual.as_ref().map(|m| format!(" - {m}")).unwrap_or_default()
        )));
    }
    if !plan.missing_dependencies.is_empty() {
        return Err(DevdError::conflict(format!(
            "install {} first: {} depends on it",
            plan.missing_dependencies.join(", "),
            plan.tool
        )));
    }
    let tool = reg.tool(&plan.tool).ok_or_else(|| DevdError::not_found(format!("no tool `{}`", plan.tool)))?;
    let manager = plan.manager.clone().unwrap_or_default();
    let mut ran = Vec::new();
    let mut notes = Vec::new();

    if manager == "devd-runtime" {
        let kind = tool.install.iter().find(|r| r.manager == "devd-runtime").and_then(|r| r.packages.first().cloned()).unwrap_or_default();
        let installed = devd_runtime::install(ctx, &kind, "latest", false, false, None)?;
        ran.push(json!({"command": format!("devd runtime install {kind}@latest"), "exit_code": 0, "version": installed.runtime.version}));
    } else {
        let dirs = managed_dirs(ctx);
        for argv in &plan.commands {
            let mut env = vec![
                ("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string()),
                ("HOMEBREW_NO_AUTO_UPDATE".to_string(), "1".to_string()),
            ];
            if argv.first().map(|a| a == "sudo").unwrap_or(false) {
                env.clear(); // sudo resets the environment; pass nothing it would ignore
            }
            let spec = RunSpec {
                argv: with_env_for_sudo(argv),
                cwd: std::env::temp_dir(),
                env,
                extra_path: dirs.clone(),
                timeout: Some(Duration::from_secs(60 * 30)),
            };
            let run_id = devd_core::util::new_id();
            let log = ctx.paths.logs().join("stacks").join(format!("toolchain-{run_id}.log"));
            let r = exec::run_streaming(&spec, cancel, Some(&log), &mut |l| {
                ctx.events.emit_with("stack.output", l.to_string(), json!({"tool": plan.tool}));
            })?;
            ran.push(json!({
                "command": exec::redact_argv(argv).join(" "),
                "exit_code": r.code,
                "log": log.display().to_string(),
                "tail": r.tail.iter().rev().take(15).rev().collect::<Vec<_>>(),
            }));
            if r.cancelled {
                return Err(DevdError::conflict("cancelled; the package manager was stopped and the machine may be partly changed"));
            }
            if !r.ok {
                return Err(DevdError::Command {
                    program: argv.join(" "),
                    code: r.code.unwrap_or(-1),
                    stderr: format!("{} (full log: {})", r.tail.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n"), log.display()),
                });
            }
        }
    }

    let dirs = managed_dirs(ctx);
    let after = tool.probe(&dirs);
    let verified = if plan.action == "install" { after.installed } else { !after.installed };
    if !verified {
        notes.push(if plan.action == "install" {
            format!(
                "the installer finished but `{}` is still not usable: {}. A new shell or a PATH change may be needed; devd does not treat the exit code as success.",
                tool.exes.join("/"),
                after.problem.clone().unwrap_or_default()
            )
        } else {
            format!("the package manager finished but {} is still installed at {}", tool.name, after.path.clone().unwrap_or_default())
        });
    }
    if plan.action == "install" {
        if after.outdated {
            notes.push(format!("{} {} is older than the oldest supported version", tool.name, after.version.clone().unwrap_or_default()));
        }
        if let Some(svc) = &tool.service {
            notes.push(format!(
                "{} was installed but is not started by devd automatically; start it from Toolchain (it will listen on 127.0.0.1:{}). Data lives in {}.",
                tool.name, svc.port, svc.data_dir
            ));
        }
    }
    let _ = ctx.store.begin_stack_run("toolchain", &plan.tool, &format!("{} {}", plan.action, tool.name), &plan.display.join(" && "), &ctx.actor.label(), None)
        .and_then(|id| ctx.store.finish_stack_run(&id, Some(0), verified, 0));
    Ok(ToolChange { tool: plan.tool.clone(), action: plan.action.clone(), manager, commands: ran, after, verified, notes })
}

/// `sudo -n` is already in the argv; keep it as is.
fn with_env_for_sudo(argv: &[String]) -> Vec<String> {
    argv.to_vec()
}

/// Control a service and wait until it is actually ready (or actually stopped).
pub fn control(ctx: &Ctx, reg: &StackRegistry, id: &str, action: &str) -> Result<ServiceState> {
    let tool = reg.tool(id).ok_or_else(|| DevdError::not_found(format!("no tool `{id}`")))?;
    let svc = tool.service.as_ref().ok_or_else(|| DevdError::unsupported(format!("{} is not a service", tool.name)))?;
    let host = Host::detect();
    let dirs = managed_dirs(ctx);
    let status = tool.probe(&dirs);
    let state = tool.service_state(&host, &status).ok_or_else(|| DevdError::internal("service state"))?;
    if !status.installed {
        return Err(DevdError::conflict(format!("{} is not installed; install it first", tool.name)));
    }
    if action == "start" || action == "restart" {
        if let Some(who) = state.port_conflict.as_ref().filter(|_| !state.listening || action == "start") {
            return Err(DevdError::conflict(format!(
                "port {} is already used by `{who}`; stop it or change {}'s port before starting",
                svc.port, tool.name
            )));
        }
        if state.healthy && action == "start" {
            return Ok(state);
        }
    }
    let (argv, privileged) = tool.service_command(action, &host, &state)?;
    if privileged && !host.root && !host.sudo {
        return Err(DevdError::unsupported(format!(
            "controlling {} needs administrator rights and devd has neither root nor passwordless sudo; run `{}` yourself",
            tool.name,
            crate::template::display(&argv.iter().filter(|a| *a != "-n").cloned().collect::<Vec<_>>())
        )));
    }
    let cancel = AtomicBool::new(false);
    let r = exec::run_streaming(
        &RunSpec { argv: argv.clone(), cwd: std::env::temp_dir(), env: vec![], extra_path: dirs.clone(), timeout: Some(Duration::from_secs(120)) },
        &cancel,
        None,
        &mut |_| {},
    )?;
    if !r.ok {
        return Err(DevdError::Command {
            program: argv.join(" "),
            code: r.code.unwrap_or(-1),
            stderr: r.tail.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n"),
        });
    }
    // Wait for the real condition. "systemctl start" returning is not readiness.
    let want_up = action != "stop";
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let now = tool.service_state(&host, &tool.probe(&dirs)).ok_or_else(|| DevdError::internal("service state"))?;
        if want_up && now.healthy || !want_up && !now.listening {
            let _ = ctx.store.begin_stack_run("toolchain", id, &format!("{action} {}", tool.name), &argv.join(" "), &ctx.actor.label(), None)
                .and_then(|rid| ctx.store.finish_stack_run(&rid, Some(0), true, r.duration_ms as i64));
            return Ok(now);
        }
        if Instant::now() > deadline {
            return Err(DevdError::Timeout(format!(
                "`{}` exited successfully but {} {} within 25s ({}). Check the logs: {}",
                argv.join(" "),
                tool.name,
                if want_up { "is not accepting connections" } else { "is still running" },
                now.detail,
                svc.logs
            )));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// Recent log lines for a service, where the host can provide them.
pub fn service_logs(ctx: &Ctx, reg: &StackRegistry, id: &str) -> Result<Vec<String>> {
    let tool = reg.tool(id).ok_or_else(|| DevdError::not_found(format!("no tool `{id}`")))?;
    let host = Host::detect();
    let dirs = managed_dirs(ctx);
    let status = tool.probe(&dirs);
    let state = tool.service_state(&host, &status).ok_or_else(|| DevdError::unsupported(format!("{} is not a service", tool.name)))?;
    let (argv, _) = tool.service_command("logs", &host, &state)?;
    let r = exec::run_streaming(
        &RunSpec { argv, cwd: std::env::temp_dir(), env: vec![], extra_path: dirs, timeout: Some(Duration::from_secs(20)) },
        &AtomicBool::new(false),
        None,
        &mut |_| {},
    )?;
    Ok(r.tail)
}
