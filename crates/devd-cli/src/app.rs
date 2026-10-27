//! Application plumbing shared by every `devd` subcommand.
//!
//! The CLI owns no business logic. It resolves the layout, opens the state
//! store, discovers Git, builds the **same** capability registry the TUI, GUI,
//! HTTP API and MCP server use, and then dispatches through
//! [`Registry::dispatch`] - which is where permissions, approvals and auditing
//! are enforced.

use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;

use serde_json::{json, Value};

use devd_core::capability::{
    Actor, Approval, Ctx, InvokeRequest, InvokeResult, Plan, Surface, Tier,
};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::{DevdError, Registry, Result};
use devd_git::GitService;

/// Everything a subcommand needs, assembled once.
pub struct App {
    /// Resolved filesystem layout.
    pub paths: Paths,
    /// State store.
    pub store: Arc<Store>,
    /// Effective settings.
    pub settings: Arc<Settings>,
    /// Event bus.
    pub events: EventBus,
    /// Git backend.
    pub git: Arc<GitService>,
    /// The shared capability registry.
    ///
    /// Shared rather than owned because the MCP server and the HTTP server each
    /// need to move it into a long-lived task while the CLI may still hold the
    /// application.
    pub registry: Arc<Registry>,
    /// The process-owning workspace the registry's new capabilities refer to.
    pub workspace: Arc<devd_workspace::Workspace>,
    /// The built-in agent and team; a long-lived command calls [`App::resume_agents`].
    pub agents: devd_bootstrap::Agents,
    /// Whether to emit JSON.
    pub json: bool,
    /// Whether to plan only.
    pub plan_only: bool,
    /// Whether operations requiring approval are approved without prompting.
    pub assume_yes: bool,
    /// Who the caller is.
    pub actor: Actor,
}

impl App {
    /// Resolve the layout, open the store, discover Git and build the registry.
    pub fn bootstrap(json: bool, plan_only: bool, assume_yes: bool) -> Result<Self> {
        Self::bootstrap_at(Paths::discover()?, json, plan_only, assume_yes)
    }

    /// The same bootstrap against an explicit layout.
    ///
    /// Separated from [`App::bootstrap`] so that a caller which needs a
    /// throwaway installation - a test, or a future `--root` flag - does not
    /// have to mutate `DEVD_HOME` in the process environment, which is global
    /// state and therefore a source of test flakiness.
    pub fn bootstrap_at(
        paths: Paths,
        json: bool,
        plan_only: bool,
        assume_yes: bool,
    ) -> Result<Self> {
        let boot = devd_bootstrap::Bootstrap::at(paths)?;
        Ok(Self {
            paths: boot.paths,
            store: boot.store,
            settings: boot.settings,
            events: boot.events,
            git: boot.git,
            registry: boot.registry,
            workspace: boot.workspace,
            agents: boot.agents,
            json,
            plan_only,
            assume_yes,
            // The CLI is a human at a keyboard until a flag says otherwise. The
            // MCP server constructs the same `App` with an agent actor, which is
            // why an agent cannot inherit a human's authority by using the CLI.
            actor: Actor::human(Surface::Cli),
        })
    }

    /// Recover conversations and runs a previous process left half done. Called
    /// only by commands that stay running and drive agents; a one-shot command
    /// must not take work from the application running beside it.
    pub fn resume_agents(&self) {
        self.agents.resume(&self.paths);
        // Cron, backups and the web-configuration watcher belong to one process. If another devd
        // already runs them, this one must not start a second set.
        if devd_bootstrap::start_ops(&self.paths, &self.store, &self.settings, &self.events) {
            devd_bootstrap::start_reconcile(&self.paths, &self.store, &self.settings, &self.events);
        }
    }

    /// Build an execution context for the configured actor.
    pub fn ctx(&self) -> Ctx {
        Ctx {
            store: self.store.clone(),
            paths: Arc::new(self.paths.clone()),
            settings: self.settings.clone(),
            events: self.events.clone(),
            actor: self.actor.clone(),
            run: None,
            dry_run: self.plan_only,
        }
    }

    /// Invoke a capability, handling planning, approval and rendering.
    pub fn invoke(&self, id: &str, args: Value) -> Result<()> {
        let cap = self
            .registry
            .get(id)
            .ok_or_else(|| DevdError::not_found(format!("no capability `{id}`")))?;
        let ctx = self.ctx();

        if self.plan_only {
            let req = InvokeRequest {
                id: id.to_string(),
                args,
                dry_run: true,
            };
            let res = self.registry.dispatch(&ctx, &req, Approval::None)?;
            self.render(&res);
            return Ok(());
        }

        let mut approval = Approval::None;
        if cap.tier.requires_approval() {
            // Show the consequence before asking, whenever the capability can
            // describe itself without acting.
            let shown = self.show_plan(&ctx, id, &args)?;
            let decision = if self.assume_yes {
                if !self.json {
                    println!("approved (--yes)");
                }
                Ok(Approval::Granted)
            } else {
                self.prompt_approval(id, cap.tier, &cap.summary, shown)
            };
            match decision {
                Ok(granted) => approval = granted,
                Err(e) => {
                    // Record the refused attempt before reporting it. A
                    // destructive operation that a human was asked about and did
                    // not approve is exactly the kind of event the audit trail
                    // exists to capture.
                    let req = InvokeRequest {
                        id: id.to_string(),
                        args: args.clone(),
                        dry_run: false,
                    };
                    let _ = self.registry.dispatch(&ctx, &req, Approval::Denied);
                    return Err(e);
                }
            }
            if approval == Approval::Denied {
                // Dispatch anyway so a declined attempt is audited, exactly as a
                // granted one is. The dispatcher refuses to run it.
                let req = InvokeRequest {
                    id: id.to_string(),
                    args,
                    dry_run: false,
                };
                let _ = self.registry.dispatch(&ctx, &req, Approval::Denied);
                eprintln!("declined: nothing was changed");
                return Err(DevdError::denied(format!("`{id}` was declined")));
            }
        }

        let req = InvokeRequest {
            id: id.to_string(),
            args,
            dry_run: false,
        };
        let res = self.registry.dispatch(&ctx, &req, approval)?;
        self.render(&res);
        Ok(())
    }

    /// Print the plan for an operation without applying it. Returns whether a
    /// plan was available.
    fn show_plan(&self, ctx: &Ctx, id: &str, args: &Value) -> Result<bool> {
        let cap = self.registry.get(id).expect("capability was just resolved");
        if !cap.supports_dry_run {
            return Ok(false);
        }
        let req = InvokeRequest {
            id: id.to_string(),
            args: args.clone(),
            dry_run: true,
        };
        match self.registry.dispatch(ctx, &req, Approval::None) {
            Ok(res) => {
                if let Some(plan) = res.plan {
                    if !self.json {
                        print_plan(&plan);
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(e) => {
                // Being unable to pre-compute a plan is not a reason to refuse;
                // it is a reason to say so before asking for approval.
                if !self.json {
                    eprintln!("note: could not pre-compute a plan ({e})");
                }
                Ok(false)
            }
        }
    }

    /// Ask a human to approve. Interactive by construction: without a terminal
    /// the operation is refused rather than assumed.
    fn prompt_approval(
        &self,
        id: &str,
        tier: Tier,
        summary: &str,
        _plan_shown: bool,
    ) -> Result<Approval> {
        if self.json || !std::io::stdin().is_terminal() {
            return Err(DevdError::ApprovalRequired {
                capability: id.to_string(),
                reason: format!(
                    "`{id}` is a {} operation and needs explicit approval; \
                     re-run interactively, or pass --yes to approve it deliberately",
                    tier.as_str()
                ),
            });
        }
        eprint!(
            "Approve this {} operation ({summary})? [y/N] ",
            tier.as_str()
        );
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| DevdError::internal(format!("could not read the approval answer: {e}")))?;
        let yes = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        Ok(if yes {
            Approval::Granted
        } else {
            Approval::Denied
        })
    }

    fn render(&self, res: &InvokeResult) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(res).unwrap_or_else(|_| "{}".to_string())
            );
            return;
        }
        if let Some(plan) = &res.plan {
            print_plan(plan);
            return;
        }
        if let Some(outcome) = &res.outcome {
            println!("{}", outcome.message);
            for warning in &outcome.warnings {
                println!("warning: {warning}");
            }
            if !outcome.data.is_null() {
                print_value(&outcome.data, 1);
            }
        }
    }
}

/// Print a plan in the order the user needs to read it: what, then how, then
/// what it costs.
pub fn print_plan(plan: &Plan) {
    println!("{}", plan.summary);
    for step in &plan.steps {
        println!("  - {step}");
    }
    for effect in &plan.effects {
        println!("  effect: {effect}");
    }
    if !plan.data.is_null() {
        print_value(&plan.data, 1);
    }
    if plan.destructive {
        println!("  DESTRUCTIVE: this can destroy data");
    }
    if plan.requires_approval {
        println!("  requires explicit approval");
    }
}

/// A small recursive renderer so structured results are readable without a GUI.
///
/// Long strings (patches, blame output) are truncated rather than dumped, with
/// an explicit note, so the terminal stays usable.
pub fn print_value(value: &Value, indent: usize) {
    let pad = " ".repeat(indent);
    match value {
        Value::Null => println!("{pad}(none)"),
        Value::Bool(b) => println!("{pad}{b}"),
        Value::Number(n) => println!("{pad}{n}"),
        Value::String(s) => {
            if s.len() > 400 {
                println!("{pad}{}", devd_core::util::truncate(s, 400));
                println!("{pad}({} bytes total; use --json for all of it)", s.len());
            } else {
                println!("{pad}{s}");
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                println!("{pad}(none)");
                return;
            }
            let scalar = items.iter().all(|i| {
                matches!(
                    i,
                    Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                )
            });
            if scalar {
                let rendered: Vec<String> = items
                    .iter()
                    .map(|i| match i {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                println!("{pad}{}", rendered.join(", "));
            } else {
                for item in items {
                    print_value(item, indent);
                }
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                if v.is_object() || v.is_array() {
                    println!("{pad}{k}:");
                    print_value(v, indent + 2);
                } else {
                    let text = match v {
                        Value::String(s) if s.len() > 400 => format!(
                            "{} ({} bytes, use --json for all of it)",
                            devd_core::util::truncate(s, 400),
                            s.len()
                        ),
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    println!("{pad}{k}: {text}");
                }
            }
        }
    }
}

/// The Git component of the environment check.
///
/// Contributed by whichever entry point can reach the Git backend, so that
/// `devd doctor`, the application's System panel and the MCP server report the
/// same thing without `devd-core` - which must not depend on a Git
/// implementation - having to know about one. Lives in `devd-git` so all three
/// callers share one implementation rather than three copies.
pub use devd_git::doctor_check as git_check;

/// Parse `--arg key=value` pairs and a `--args` JSON object into one object.
pub fn merge_args(args: Option<String>, sets: Vec<String>) -> Result<Value> {
    let mut out = match args {
        Some(text) => {
            let v: Value = serde_json::from_str(&text)
                .map_err(|e| DevdError::invalid(format!("--args is not valid JSON: {e}")))?;
            if !v.is_object() {
                return Err(DevdError::invalid("--args must be a JSON object"));
            }
            v
        }
        None => json!({}),
    };
    let map = out
        .as_object_mut()
        .ok_or_else(|| DevdError::internal("argument object disappeared"))?;
    for set in sets {
        let (key, raw) = set
            .split_once('=')
            .ok_or_else(|| DevdError::invalid(format!("--arg needs key=value, got `{set}`")))?;
        // Accept JSON for numbers, booleans, arrays and objects; fall back to a
        // plain string so `--arg message=hello world` works as typed.
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        map.insert(key.to_string(), value);
    }
    Ok(out)
}

/// Exit code for an error, so scripts can branch on the failure class.
pub fn exit_code_for(err: &DevdError) -> i32 {
    match err {
        DevdError::Denied(_) => 3,
        DevdError::ApprovalRequired { .. } => 3,
        DevdError::NotFound(_) => 4,
        DevdError::Invalid(_) | DevdError::Unsupported(_) => 2,
        DevdError::Conflict(_) => 5,
        _ => 1,
    }
}
