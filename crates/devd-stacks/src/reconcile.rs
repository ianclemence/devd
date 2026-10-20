//! Services and tools that already exist on the machine.
//!
//! devd never assumes a blank machine and never replaces a working one. For
//! every service in the catalogue this module keeps four facts apart, because
//! collapsing them is how tools end up reporting "healthy" for something that
//! was only found:
//!
//! * **installed** - an executable was found and ran;
//! * **running** - something answers on the service's port, and its health
//!   check passes;
//! * **required** - a project devd knows says it needs the service;
//! * **managed** - a person decided how devd treats it (`adopt`, `observe`,
//!   `external`, `ignore`).
//!
//! From those it recommends one next step and lists the options with their
//! risk. Anything that would change or remove an existing installation is only
//! ever offered through the approved capabilities (`toolchain.service`,
//! `toolchain.remove`); this module changes nothing.

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{ports, Result};

use crate::registry::StackRegistry;
use crate::toolchain;
use crate::tools::Host;

/// Decisions a person can record about a service.
pub const MODES: &[&str] = &["adopt", "observe", "external", "ignore"];

/// One option offered for a service.
#[derive(Debug, Clone, Serialize)]
pub struct Option_ {
    /// Stable id: `adopt`, `observe`, `external`, `ignore`, `start`, `install`, `stop-other`, `remove`.
    pub action: String,
    /// What it does, in words.
    pub label: String,
    /// `none` (records a decision only), `approval` (changes the machine, asks first), `destructive`.
    pub risk: String,
    /// Whether devd can do it here.
    pub available: bool,
    /// Why not, when it cannot.
    pub why_not: Option<String>,
    /// The capability that does it, when there is one.
    pub capability: Option<String>,
}

fn opt(action: &str, label: &str, risk: &str, capability: Option<&str>) -> Option_ {
    Option_ { action: action.into(), label: label.into(), risk: risk.into(), available: true, why_not: None, capability: capability.map(str::to_string) }
}

fn unavailable(mut o: Option_, why: impl Into<String>) -> Option_ {
    o.available = false;
    o.why_not = Some(why.into());
    o
}

/// Projects that say they need a tool, from what `project.detect` stored.
fn required_by(ctx: &Ctx, tool: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for p in ctx.store.list_stack_projects().unwrap_or_default() {
        let Ok(c) = serde_json::from_str::<Value>(&p.context) else { continue };
        let hit = c["services"].as_array().map(|a| a.iter().find(|s| s["tool"] == tool).cloned()).unwrap_or(None);
        if let Some(s) = hit {
            out.push(json!({ "project": p.name, "path": p.path, "why": s["why"] }));
        }
    }
    out
}

/// The inventory of every service devd knows how to look at.
pub fn inventory(ctx: &Ctx, reg: &StackRegistry) -> Result<Value> {
    let host = Host::detect();
    let dirs = toolchain::managed_dirs(ctx);
    let decisions = ctx.store.list_service_decisions()?;
    let mut rows = Vec::new();
    for tool in reg.catalogue.tools.iter().filter(|t| t.service.is_some()) {
        let status = tool.probe(&dirs);
        let Some(state) = tool.service_state(&host, &status) else { continue };
        let owner = if state.listening { ports::find_listener(state.port).ok().flatten() } else { None };
        let decision = decisions.iter().find(|d| d.tool == tool.id);
        let required = required_by(ctx, &tool.id);
        let foreign = state.port_conflict.clone();

        // What is true, in the user's words. Never "healthy" without an answer.
        let (kind, headline) = if !status.installed {
            ("missing", format!("{} is not installed", tool.name))
        } else if let Some(who) = &foreign {
            ("port_conflict", format!("Port {} is held by `{who}`, not by {}", state.port, tool.name))
        } else if state.healthy {
            ("ready", format!("{} {} is installed and answering on port {}", tool.name, status.version.clone().unwrap_or_default(), state.port))
        } else if state.listening {
            ("not_operational", format!("{} accepts connections on port {} but its health check fails", tool.name, state.port))
        } else if state.control == "none" {
            ("limited", format!("{} is installed, but devd cannot start it on this machine ({})", tool.name, state.detail))
        } else {
            ("installed_stopped", format!("{} {} is installed but not running", tool.name, status.version.clone().unwrap_or_default()))
        };

        let mut options: Vec<Option_> = Vec::new();
        let (recommend, why): (&str, String) = match kind {
            "missing" => {
                options.push(opt("install", &format!("Install {} with the host's package manager", tool.name), "approval", Some("toolchain.install")));
                if required.is_empty() { ("none", "Nothing you have imported needs it yet.".into()) } else { ("install", format!("{} project(s) need it.", required.len())) }
            }
            "ready" => {
                options.push(opt("adopt", "Use it as it is. devd starts, stops and checks it; your data and configuration are untouched", "none", Some("services.decide")));
                options.push(opt("observe", "Only watch it. devd will never start or stop it", "none", Some("services.decide")));
                ("adopt", "It already works. Installing a second copy would add a port conflict and a second data directory.".into())
            }
            "installed_stopped" => {
                options.push(opt("start", &format!("Start {}", tool.name), "approval", Some("toolchain.service")));
                options.push(opt("adopt", "Use it as it is, and start it when a project needs it", "none", Some("services.decide")));
                options.push(opt("observe", "Only watch it; never start it", "none", Some("services.decide")));
                ("adopt", "It is installed and compatible with how devd runs it; starting it is a separate step that asks first.".into())
            }
            "port_conflict" => {
                options.push(opt("external", "Use the program that holds the port, or an instance you run on another port", "none", Some("services.decide")));
                options.push(unavailable(opt("stop-other", "Stop the other program", "destructive", None), "devd does not stop programs it did not start. Stop it yourself, then check again."));
                ("external", "Something else answers on this port. devd will not start a second copy on top of it.".into())
            }
            "not_operational" => {
                options.push(opt("logs", "Read its logs", "none", Some("toolchain.logs")));
                options.push(opt("observe", "Only watch it while you fix it", "none", Some("services.decide")));
                ("logs", "The port is open but the health check fails, so devd will not call it healthy.".into())
            }
            _ => {
                options.push(opt("observe", "Only watch it", "none", Some("services.decide")));
                ("observe", "devd has no way to control it here, but it can still check it.".into())
            }
        };
        // Replacement is never one click: removal is its own approved, destructive step.
        if status.installed {
            let mut o = opt("remove", &format!("Remove the installed {}", tool.name), "destructive", Some("toolchain.remove"));
            o.why_not = Some("Only when you ask. It needs approval, stops the service and keeps its data directory. devd never replaces an installation as a side effect of another task.".into());
            options.push(o);
        }
        options.push(opt("ignore", "Hide it; no project needs it", "none", Some("services.decide")));

        rows.push(json!({
            "tool": tool.id, "name": tool.name, "category": tool.category,
            "installed": status.installed, "version": status.version, "path": status.path, "source": status.source,
            "outdated": status.outdated, "problem": status.problem,
            "running": state.listening, "healthy": state.healthy, "port": state.port,
            "port_owner": owner.map(|o| json!({ "program": o.program, "pid": o.pid, "address": o.address })),
            "unit": state.unit, "control": state.control, "data_dir": state.data_dir, "logs": state.logs, "security": state.security,
            "required_by": required, "required": !required.is_empty(),
            "decision": decision,
            "status": kind, "headline": headline,
            "recommendation": { "action": recommend, "why": why },
            "options": options,
        }));
    }
    Ok(json!({ "host": host, "services": rows }))
}

/// Whether devd may start or stop this service, given what a person decided.
pub fn control_allowed(ctx: &Ctx, tool: &str) -> std::result::Result<(), String> {
    match ctx.store.list_service_decisions() {
        Ok(ds) => match ds.iter().find(|d| d.tool == tool) {
            Some(d) if d.mode == "observe" => Err(format!("you decided devd should only watch {tool}; change that decision to let devd control it")),
            Some(d) if d.mode == "external" => Err(format!("{tool} is an instance you run yourself (decision: external); devd does not control it")),
            _ => Ok(()),
        },
        Err(_) => Ok(()),
    }
}
