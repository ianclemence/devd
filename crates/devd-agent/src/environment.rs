//! Environment manifests and end-to-end setup workflows.
//!
//! A conversation that changes code is only half the work: the project also
//! needs the runtimes, services, variables, ports and site that make the change
//! runnable. This module turns everything devd already knows into one manifest
//! with a stable fingerprint, and then applies the missing parts in a fixed
//! order while recording each step's real result.
//!
//! Nothing here invents readiness. A runtime counts as present only when the
//! runtime table says it is installed; a service counts as running only when its
//! port accepts a connection; a site is verified from a live `devd_sites::list`
//! check. A step that could not be done is recorded as failed or skipped with
//! the reason, and the manifest after setup is the evidence.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{util, DevdError, Registry, Result};
use devd_stacks::detect::{self, ProjectContext};
use devd_stacks::registry::StackRegistry;
use devd_stacks::tasks::{RunTask, TaskInfo};

// ---------------------------------------------------------------- arguments

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ManifestArgs {
    /// Absolute project directory.
    path: String,
    /// Force a stack id instead of detecting one.
    #[serde(default)]
    stack: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SetupArgs {
    /// Absolute project directory.
    path: String,
    /// Force a stack id instead of detecting one.
    #[serde(default)]
    stack: Option<String>,
    /// Install runtimes the project asks for and the machine lacks (default true).
    #[serde(default)]
    install_runtimes: Option<bool>,
    /// Start services the project appears to need (default true).
    #[serde(default)]
    start_services: Option<bool>,
    /// Run one project task after the environment is ready, e.g. `build`,
    /// `test`, `migrate`.
    #[serde(default)]
    run_task: Option<String>,
    /// Create a site for the project. `auto` uses the project name; any other
    /// value is the site name. Omit to leave sites alone.
    #[serde(default)]
    site: Option<String>,
    /// Hostname for the new site; defaults to `<name>.<tld>`.
    #[serde(default)]
    host: Option<String>,
    /// Issue a certificate for the new site (default false).
    #[serde(default)]
    tls: Option<bool>,
    /// Create a database named after the new site (default false).
    #[serde(default)]
    database: Option<bool>,
    /// Forward to a running program instead of serving the directory.
    #[serde(default)]
    upstream: Option<String>,
    /// Re-inspect after the steps and record whether the environment is ready
    /// (default true).
    #[serde(default)]
    verify: Option<bool>,
}

// ---------------------------------------------------------------- manifest

/// A runtime the project asks for and what is installed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeState {
    /// Tool id, e.g. `node`.
    pub tool: String,
    /// The constraint as written in the project.
    pub constraint: String,
    /// The file it came from.
    pub source: String,
    /// What is installed, if anything.
    pub installed: Option<String>,
    /// Whether the installed version satisfies the constraint.
    pub satisfied: Option<bool>,
}

/// A service the project appears to need.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceState {
    /// Tool or service id.
    pub tool: String,
    /// Why it is needed.
    pub why: String,
    /// Whether devd has it installed.
    pub installed: bool,
    /// Whether it is running now.
    pub running: bool,
    /// Port it listens on, when known.
    pub port: Option<u16>,
}

/// A site that serves this project.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SiteState {
    /// Site name.
    pub name: String,
    /// Hostname.
    pub host: String,
    /// URL a person opens.
    pub url: String,
    /// Whether the project directory still exists.
    pub present: bool,
    /// Whether the web server is running.
    pub server_running: bool,
    /// Whether the hostname is in the managed hosts block.
    pub in_hosts: bool,
    /// Whether a certificate exists.
    pub certificate: bool,
    /// For a proxy site, whether the upstream accepts connections.
    pub upstream_listening: Option<bool>,
    /// What the directory serves, as devd reads it.
    pub serving: String,
}

/// A task the project defines.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskState {
    /// Task id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Kind.
    pub kind: String,
    /// Whether it can run now.
    pub available: bool,
    /// Why it cannot, when it cannot.
    pub unavailable_reason: Option<String>,
    /// Port a long-running task listens on.
    pub port: Option<u16>,
}

/// Environment variables, by name only. Values are never part of a manifest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvState {
    /// Example files found.
    pub example_files: Vec<String>,
    /// Whether a live env file exists.
    pub file_present: bool,
    /// Names the example defines.
    pub keys: Vec<String>,
    /// Names that look secret.
    pub secret_keys: Vec<String>,
    /// Names the live environment does not set.
    pub unset_keys: Vec<String>,
}

/// One port the project or its services use.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PortState {
    /// Port number.
    pub port: u16,
    /// What uses it.
    pub used_by: String,
}

/// What isolates work from the rest of the machine.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Isolation {
    /// Platform this manifest was produced on.
    pub platform: String,
    /// Child processes get an allowlisted environment, never the daemon's.
    pub environment_allowlist: bool,
    /// Names passed through to project processes.
    pub passed_environment: Vec<String>,
    /// Each child is placed in its own process group.
    pub process_group: bool,
    /// Windows Job Objects with kill-on-close are used (Windows only).
    pub job_object: bool,
    /// A sentence describing the guarantee, without overstating it.
    pub note: String,
}

/// One missing tool or unsatisfied requirement.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Requirement {
    /// What is missing.
    pub tool: String,
    /// Why it matters.
    pub why: String,
    /// What to do about it.
    pub fix: String,
}

/// Everything devd knows about a project's environment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentManifest {
    /// Absolute project directory.
    pub project: String,
    /// Directory name.
    pub name: String,
    /// When it was produced.
    pub generated_at: String,
    /// Stable digest of the content, ignoring the timestamp.
    pub fingerprint: String,
    /// Chosen stack.
    pub stack: Option<String>,
    /// Matching stacks, best first.
    pub stacks: Vec<String>,
    /// Detection is uncertain.
    pub ambiguous: bool,
    /// Why it is uncertain.
    pub ambiguity: Option<String>,
    /// Package manager.
    pub package_manager: Option<String>,
    /// Manifests found.
    pub manifests: Vec<String>,
    /// Lockfiles found.
    pub lockfiles: Vec<String>,
    /// Entry points found.
    pub entry_points: Vec<String>,
    /// Runtimes the project asks for.
    pub runtimes: Vec<RuntimeState>,
    /// Services the project appears to need.
    pub services: Vec<ServiceState>,
    /// Sites serving this project.
    pub sites: Vec<SiteState>,
    /// Project tasks.
    pub tasks: Vec<TaskState>,
    /// Environment-variable names.
    pub env: EnvState,
    /// Ports in use.
    pub ports: Vec<PortState>,
    /// Isolation guarantees.
    pub isolation: Isolation,
    /// Requirements that are not met.
    pub missing: Vec<Requirement>,
    /// Things a person should know that are not failures.
    pub warnings: Vec<String>,
}

/// A step of a setup workflow, with its real result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetupStep {
    /// Stable step id.
    pub id: String,
    /// What the step does.
    pub title: String,
    /// `done`, `skipped` or `failed`.
    pub state: String,
    /// What happened, in plain language.
    pub detail: String,
    /// The command or operation that produced it, when there was one.
    pub evidence: Option<String>,
}

/// The result of a setup workflow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetupReport {
    /// Project.
    pub project: String,
    /// Environment before the steps.
    pub before: EnvironmentManifest,
    /// Environment after the steps.
    pub after: EnvironmentManifest,
    /// Steps in the order they ran.
    pub steps: Vec<SetupStep>,
    /// Whether every step succeeded and the environment is ready.
    pub verified: bool,
    /// What is still missing.
    pub missing: Vec<Requirement>,
    /// Non-fatal remarks.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- helpers

fn dir_arg(ctx: &Ctx, path: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(path);
    let dir = devd_stacks::authz::authorize_dir(ctx, &dir)?;
    if !dir.is_dir() {
        return Err(DevdError::not_found(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    Ok(dir)
}

fn registry(ctx: &Ctx) -> StackRegistry {
    StackRegistry::with_extensions(&ctx.paths.root().join("stacks"))
}

fn detect_project(ctx: &Ctx, dir: &Path, stack: Option<&str>) -> Result<ProjectContext> {
    detect::detect(ctx, &registry(ctx), dir, stack)
}

/// The environment names passed to project processes.
///
/// Kept in one place so the manifest can tell a person exactly what a project
/// command can see. The process runner uses the same list.
pub fn passed_environment() -> Vec<String> {
    devd_core::process::CHILD_ENV_ALLOWLIST
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn isolation() -> Isolation {
    Isolation {
        platform: std::env::consts::OS.to_string(),
        environment_allowlist: true,
        passed_environment: passed_environment(),
        process_group: true,
        job_object: cfg!(windows),
        note: if cfg!(windows) {
            "Project processes get an allowlisted environment, a new process group, and a Job Object \
             with kill-on-close where the job could be assigned; stopping a service terminates the \
             job, and if devd exits its handles close, which kills the job's processes. Assignment \
             is best effort when devd itself already runs inside another job (some CI runners)."
                .to_string()
        } else {
            "Project processes get an allowlisted environment and their own process group; \
             stopping a service signals the group. devd does not claim a sandbox: the process runs \
             as the same user and can reach the same files."
                .to_string()
        },
    }
}

fn site_serving_label(row: &devd_sites::SiteRow) -> String {
    match row.serving {
        devd_core::serving::Serving::Proxy => "proxy",
        devd_core::serving::Serving::Php => "php",
        devd_core::serving::Serving::Static => "static",
        devd_core::serving::Serving::NoEntry => "no entry point",
    }
    .to_string()
}

fn canonical_path(path: &str) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn upstream_port(site: &devd_core::state::Site) -> Option<u16> {
    let raw = site.upstream.as_deref()?;
    let tail = raw.rsplit(':').next().unwrap_or(raw);
    tail.trim_end_matches('/').parse::<u16>().ok()
}

fn manifest(ctx: &Ctx, dir: &Path, stack: Option<&str>) -> Result<EnvironmentManifest> {
    let d = detect_project(ctx, dir, stack)?;
    let service_rows = devd_services::list(ctx).unwrap_or_default();
    let site_rows = devd_sites::list(ctx).unwrap_or_default();

    let runtimes: Vec<RuntimeState> = d
        .runtimes
        .iter()
        .map(|r| RuntimeState {
            tool: r.tool.clone(),
            constraint: r.constraint.clone(),
            source: r.source.clone(),
            installed: r.installed.clone(),
            satisfied: r.satisfied,
        })
        .collect();

    let services: Vec<ServiceState> = d
        .services
        .iter()
        .map(|s| {
            let row = service_rows
                .iter()
                .find(|x| x.name == s.tool || x.kind == s.tool);
            ServiceState {
                tool: s.tool.clone(),
                why: s.why.clone(),
                installed: row.map(|r| r.installed).unwrap_or(s.installed),
                running: row.map(|r| r.running).unwrap_or(s.running),
                port: row.and_then(|r| r.port),
            }
        })
        .collect();

    let project = canonical_path(&dir.display().to_string()).unwrap_or_else(|| dir.to_path_buf());
    let sites: Vec<SiteState> = site_rows
        .iter()
        .filter(|row| {
            canonical_path(&row.project)
                .or_else(|| canonical_path(&row.site.root))
                .map(|p| p == project || p.starts_with(&project))
                .unwrap_or(false)
        })
        .map(|row| SiteState {
            name: row.site.name.clone(),
            host: row.site.host.clone(),
            url: row.url.clone(),
            present: row.present,
            server_running: row.server_running,
            in_hosts: row.in_hosts,
            certificate: row.certificate,
            upstream_listening: row.upstream_listening,
            serving: site_serving_label(row),
        })
        .collect();

    let tasks: Vec<TaskState> = d
        .tasks
        .iter()
        .map(|t| TaskState {
            id: t.id.clone(),
            title: t.title.clone(),
            kind: t.kind.clone(),
            available: t.available,
            unavailable_reason: t.unavailable_reason.clone(),
            port: t.port,
        })
        .collect();

    let mut ports: Vec<PortState> = Vec::new();
    for s in &services {
        if let Some(port) = s.port {
            ports.push(PortState {
                port,
                used_by: format!("service {}", s.tool),
            });
        }
    }
    for t in &tasks {
        if let Some(port) = t.port {
            ports.push(PortState {
                port,
                used_by: format!("task {}", t.id),
            });
        }
    }
    for row in &site_rows {
        if sites.iter().any(|s| s.name == row.site.name) {
            if let Some(port) = upstream_port(&row.site) {
                ports.push(PortState {
                    port,
                    used_by: format!("site {}", row.site.name),
                });
            }
            ports.push(PortState {
                port: ctx.settings.http_port,
                used_by: "web server".into(),
            });
        }
    }
    ports.sort_by_key(|p| p.port);
    ports.dedup_by_key(|p| p.port);

    let mut missing = Vec::new();
    for r in &runtimes {
        if r.satisfied == Some(false) || (r.installed.is_none() && r.satisfied.is_none()) {
            missing.push(Requirement {
                tool: r.tool.clone(),
                why: format!("{} asks for {} {}", r.source, r.tool, r.constraint),
                fix: format!("install {} {}", r.tool, r.constraint),
            });
        }
    }
    for s in &services {
        if !s.installed {
            missing.push(Requirement {
                tool: s.tool.clone(),
                why: s.why.clone(),
                fix: format!("install the {} service", s.tool),
            });
        } else if !s.running {
            missing.push(Requirement {
                tool: s.tool.clone(),
                why: s.why.clone(),
                fix: format!("start the {} service", s.tool),
            });
        }
    }
    for key in &d.env.unset_keys {
        missing.push(Requirement {
            tool: key.clone(),
            why: "the environment example requires this variable and the live environment does not set it".into(),
            fix: format!("set {key} in the project's environment file"),
        });
    }

    let mut manifest = EnvironmentManifest {
        project: dir.display().to_string(),
        name: d.name.clone(),
        generated_at: util::now(),
        fingerprint: String::new(),
        stack: d.primary.clone(),
        stacks: d.stacks.iter().map(|s| s.id.clone()).collect(),
        ambiguous: d.ambiguous,
        ambiguity: d.ambiguity.clone(),
        package_manager: d.package_manager.clone(),
        manifests: d.manifests.clone(),
        lockfiles: d.lockfiles.clone(),
        entry_points: d.entry_points.clone(),
        runtimes,
        services,
        sites,
        tasks,
        env: EnvState {
            example_files: d.env.example_files.clone(),
            file_present: d.env.env_file_present,
            keys: d.env.keys.clone(),
            secret_keys: d.env.secret_keys.clone(),
            unset_keys: d.env.unset_keys.clone(),
        },
        ports,
        isolation: isolation(),
        missing,
        warnings: Vec::new(),
    };
    if manifest.ambiguous {
        if let Some(why) = &manifest.ambiguity {
            manifest.warnings.push(why.clone());
        }
    }
    for s in &manifest.services {
        if !s.installed {
            manifest
                .warnings
                .push(format!("{} is needed but not installed", s.tool));
        }
    }
    if manifest.sites.is_empty() {
        manifest
            .warnings
            .push("no devd site serves this project; it is not reachable by hostname".into());
    }
    // The installation digest is part of the identity, but the timestamp is
    // not: two manifests of the same machine compare equal.
    let mut canonical = serde_json::to_value(&manifest)?;
    if let Value::Object(map) = &mut canonical {
        map.remove("generated_at");
        map.insert("fingerprint".into(), Value::String(String::new()));
    }
    manifest.fingerprint = util::sha256_hex(&serde_json::to_vec(&canonical)?);
    Ok(manifest)
}

fn task_of<'a>(d: &'a ProjectContext, id: &str) -> Result<&'a TaskInfo> {
    d.tasks.iter().find(|t| t.id == id).ok_or_else(|| {
        DevdError::not_found(format!(
            "this project has no task `{id}`. Available: {}",
            if d.tasks.is_empty() {
                "none".to_string()
            } else {
                d.tasks
                    .iter()
                    .map(|t| t.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ))
    })
}

/// Pull the first version-looking number out of a constraint (`^8.3`, `>=20`,
/// `8.2.*`). The runtime installer wants a version, not a range.
fn version_of(constraint: &str) -> Option<String> {
    let bytes = constraint.as_bytes();
    let mut start = None;
    for (i, b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            start = Some(i);
            break;
        }
    }
    let start = start?;
    let mut end = start;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    let raw = constraint[start..end].trim_end_matches('.');
    if raw.is_empty() || raw == "." {
        None
    } else {
        Some(raw.to_string())
    }
}

fn step(
    steps: &mut Vec<SetupStep>,
    id: &str,
    title: &str,
    state: &str,
    detail: String,
    evidence: Option<String>,
) {
    steps.push(SetupStep {
        id: id.to_string(),
        title: title.to_string(),
        state: state.to_string(),
        detail,
        evidence,
    });
}

fn verify_site(ctx: &Ctx, name: &str) -> Result<Option<devd_sites::SiteRow>> {
    Ok(devd_sites::list(ctx)?
        .into_iter()
        .find(|r| r.site.name == name))
}

fn site_ready(row: &devd_sites::SiteRow) -> bool {
    row.present
        && row.server_running
        && row.in_hosts
        && row.upstream_listening != Some(false)
        && !matches!(row.serving, devd_core::serving::Serving::NoEntry)
}

// ---------------------------------------------------------------- registration

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<ManifestArgs>("environment.manifest", "Describe a project's environment")
        .description(
            "Reads the project, its declared runtimes, the services it needs, its environment \
             variables (names only, never values), the ports in use, the devd sites that serve it, \
             and the isolation a project process gets. The result has a stable fingerprint that \
             changes only when the environment changes, so a run can record the exact environment \
             it was executed in.",
        )
        .group("environment")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .dry_run()
        .handlers(
            |_, a| {
                Ok(Plan::new(
                    "environment.manifest",
                    format!("describe the environment of {}", a.path),
                ))
            },
            |ctx, a| {
                let dir = dir_arg(ctx, &a.path)?;
                let m = manifest(ctx, &dir, a.stack.as_deref())?;
                let ready = m.missing.is_empty();
                let mut o = Outcome::with_data(
                    if ready {
                        format!(
                            "{} is ready: {} runtime(s), {} service(s), {} site(s)",
                            m.name,
                            m.runtimes.len(),
                            m.services.len(),
                            m.sites.len()
                        )
                    } else {
                        format!("{} has {} unmet requirement(s)", m.name, m.missing.len())
                    },
                    serde_json::to_value(&m)?,
                );
                o.ok = true;
                Ok(o)
            },
        )?;

    reg.define::<SetupArgs>("workflow.setup", "Set up a project's environment")
        .description(
            "Inspects a project and then applies what is missing in a fixed order: install the \
             runtimes it asks for, start the services it needs, optionally run one task, create a \
             site if asked, and re-inspect. Every step records its real result; a step that failed \
             is reported as failed rather than as success, and `verified` is true only when the \
             re-inspection finds nothing missing and every step succeeded. Installations are \
             digest-verified by the runtime and service installers. Approval is required because \
             this adds files and starts processes.",
        )
        .group("environment")
        .tier(Tier::Write)
        .effect(Effect::Process)
        .agent_safe()
        .dry_run()
        .handlers(
            |ctx, a| {
                let dir = dir_arg(ctx, &a.path)?;
                let m = manifest(ctx, &dir, a.stack.as_deref())?;
                let mut plan = Plan::new("workflow.setup", format!("set up the environment for {}", m.name))
                    .effect("installs runtimes and starts services inside the devd root")
                    .step(format!("inspect {} ({}, fingerprint {})", m.name, m.stack.clone().unwrap_or_else(|| "no stack".into()), &m.fingerprint[..12.min(m.fingerprint.len())]));
                for need in m.runtimes.iter().filter(|r| r.satisfied == Some(false) || r.installed.is_none()) {
                    if let Some(v) = version_of(&need.constraint) {
                        plan = plan.step(format!("install {} {}", need.tool, v));
                    } else {
                        plan = plan.step(format!("cannot read a version from {} `{}`; it will be reported, not installed", need.tool, need.constraint));
                    }
                }
                for s in m.services.iter().filter(|s| !s.running) {
                    plan = plan.step(if s.installed {
                        format!("start service {}", s.tool)
                    } else {
                        format!("service {} is not installed; it will be reported, not installed", s.tool)
                    });
                }
                if let Some(task) = &a.run_task {
                    plan = plan.step(format!("run task `{task}`"));
                }
                if let Some(site) = &a.site {
                    plan = plan.step(format!("create or reuse site {site}"));
                }
                if a.verify.unwrap_or(true) {
                    plan = plan.step("re-inspect and verify the environment");
                }
                let _ = ctx;
                Ok(plan.data(json!({ "before": m })))
            },
            |ctx, a| {
                let dir = dir_arg(ctx, &a.path)?;
                let before = manifest(ctx, &dir, a.stack.as_deref())?;
                let mut steps: Vec<SetupStep> = Vec::new();
                let mut warnings: Vec<String> = Vec::new();

                step(
                    &mut steps,
                    "inspect",
                    "Inspect the project",
                    "done",
                    format!("{} is a {} project", before.name, before.stack.clone().unwrap_or_else(|| "unclassified".into())),
                    Some(format!("environment.manifest fingerprint {}", before.fingerprint)),
                );

                // Runtimes.
                if a.install_runtimes.unwrap_or(true) {
                    for need in before.runtimes.clone() {
                        if need.satisfied == Some(true) {
                            step(&mut steps, &format!("runtime:{}", need.tool), &format!("Install {}", need.tool), "skipped", format!("{} {} already satisfies {}", need.tool, need.installed.clone().unwrap_or_default(), need.constraint), None);
                            continue;
                        }
                        let Some(version) = version_of(&need.constraint) else {
                            step(&mut steps, &format!("runtime:{}", need.tool), &format!("Install {}", need.tool), "failed", format!("cannot read a version from `{}`", need.constraint), None);
                            continue;
                        };
                        let title = format!("Install {} {}", need.tool, version);
                        match devd_runtime::install(ctx, &need.tool, &version, false, false, None) {
                            Ok(i) => step(&mut steps, &format!("runtime:{}", need.tool), &title, "done", format!("installed {} {} at {}", i.runtime.kind, i.runtime.version, i.executable), Some(format!("runtime install {}@{}", need.tool, version))),
                            Err(e) => step(&mut steps, &format!("runtime:{}", need.tool), &title, "failed", e.to_string(), None),
                        }
                    }
                } else if before.runtimes.iter().any(|r| r.satisfied != Some(true)) {
                    step(&mut steps, "runtimes", "Install runtimes", "skipped", "runtime installation was turned off for this workflow".into(), None);
                }

                // Services.
                if a.start_services.unwrap_or(true) {
                    for svc in before.services.clone() {
                        if svc.running {
                            step(&mut steps, &format!("service:{}", svc.tool), &format!("Start {}", svc.tool), "skipped", "already running".into(), None);
                            continue;
                        }
                        let title = format!("Start {}", svc.tool);
                        if !svc.installed {
                            step(&mut steps, &format!("service:{}", svc.tool), &title, "failed", "not installed; install it from Services, then run setup again".into(), None);
                            continue;
                        }
                        match devd_services::start(ctx, &svc.tool) {
                            Ok(s) => step(&mut steps, &format!("service:{}", svc.tool), &title, if s.running { "done" } else { "failed" }, format!("{}: {}", s.name, s.message), Some("service start".into())),
                            Err(e) => step(&mut steps, &format!("service:{}", svc.tool), &title, "failed", e.to_string(), None),
                        }
                    }
                }

                // One task, run for real.
                if let Some(id) = &a.run_task {
                    let title = format!("Run task {id}");
                    match detect_project(ctx, &dir, a.stack.as_deref()).and_then(|d| task_of(&d, id).cloned()) {
                        Ok(t) => {
                            let req = RunTask {
                                path: dir.display().to_string(),
                                task: t.id.clone(),
                                args: Vec::new(),
                                background: Some(false),
                                timeout_secs: Some(600),
                                run_id: None,
                            };
                            match devd_stacks::tasks::run(ctx, &dir, &t, &[], &req) {
                                Ok(v) => {
                                    let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
                                    let code = v.get("exit_code").and_then(Value::as_i64);
                                    step(&mut steps, "task", &title, if ok { "done" } else { "failed" }, format!("exit {}", code.map(|c| c.to_string()).unwrap_or_else(|| "?".into())), Some(format!("project.run {}", t.id)));
                                }
                                Err(e) => step(&mut steps, "task", &title, "failed", e.to_string(), None),
                            }
                        }
                        Err(e) => step(&mut steps, "task", &title, "failed", e.to_string(), None),
                    }
                }

                // A site, when asked for.
                if let Some(requested) = &a.site {
                    let name = if requested == "auto" { before.name.clone() } else { requested.clone() };
                    let title = format!("Create site {name}");
                    if let Some(existing) = verify_site(ctx, &name)? {
                        step(&mut steps, "site", &title, "skipped", format!("site {name} already exists at {}", existing.url), None);
                    } else {
                        let req = devd_sites::CreateRequest {
                            name: name.clone(),
                            host: a.host.clone().or_else(|| a.upstream.as_ref().map(|_| format!("{name}.{}", ctx.settings.tld))),
                            root: if a.upstream.is_some() { None } else { Some(dir.display().to_string()) },
                            tls: a.tls.unwrap_or(false),
                            db: a.database.unwrap_or(false),
                            upstream: a.upstream.clone(),
                            serving: None,
                        };
                        match devd_sites::create(ctx, &req) {
                            Ok(c) => step(&mut steps, "site", &title, "done", format!("{} at {}", c.site.host, c.url), Some("site.create".into())),
                            Err(e) => step(&mut steps, "site", &title, "failed", e.to_string(), None),
                        }
                    }
                }

                // Re-inspect, then report what is still wrong.
                let after = manifest(ctx, &dir, a.stack.as_deref())?;
                if a.verify.unwrap_or(true) {
                    if let Some(row) = after.sites.first() {
                        let ok = verify_site(ctx, &row.name)?.as_ref().map(site_ready).unwrap_or(false);
                        step(
                            &mut steps,
                            "verify",
                            "Verify the environment",
                            if ok { "done" } else { "failed" },
                            if ok {
                                format!("{} is served and listening at {}", row.name, row.url)
                            } else {
                                format!("{} exists but is not fully ready (hosts, server or upstream); open Sites for the live check", row.name)
                            },
                            Some("devd_sites::list".into()),
                        );
                    } else {
                        let runtimes_ok = after.runtimes.iter().all(|r| r.satisfied == Some(true));
                        let services_ok = after.services.iter().all(|s| s.installed && s.running);
                        let ok = runtimes_ok && services_ok;
                        step(
                            &mut steps,
                            "verify",
                            "Verify the environment",
                            if ok { "done" } else { "failed" },
                            if ok {
                                "every requested runtime is installed and every needed service is running".into()
                            } else {
                                format!("{} requirement(s) remain", after.missing.len())
                            },
                            Some(format!("environment.manifest fingerprint {}", after.fingerprint)),
                        );
                    }
                }

                for w in &after.warnings {
                    if !warnings.contains(w) {
                        warnings.push(w.clone());
                    }
                }
                let failed_count = steps.iter().filter(|s| s.state == "failed").count();
                let verified = failed_count == 0 && after.missing.is_empty() && a.verify.unwrap_or(true);
                for f in steps.iter().filter(|s| s.state == "failed") {
                    warnings.push(format!("{}: {}", f.title, f.detail));
                }
                let missing = after.missing.clone();

                let report = SetupReport {
                    project: dir.display().to_string(),
                    before,
                    after,
                    steps,
                    verified,
                    missing,
                    warnings,
                };
                let mut outcome = Outcome::with_data(
                    if verified { "the environment is set up and verified".to_string() } else { format!("setup finished with {} failed step(s) and {} unmet requirement(s)", failed_count, report.after.missing.len()) },
                    serde_json::to_value(&report)?,
                );
                outcome.ok = failed_count == 0 && report.after.missing.is_empty();
                for w in &report.warnings {
                    outcome = outcome.warn(w.clone());
                }
                Ok(outcome)
            },
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use devd_core::capability::{Actor, Surface};
    use devd_core::events::EventBus;
    use devd_core::paths::Paths;
    use devd_core::state::Store;

    fn fixture() -> (Ctx, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join("devd"));
        paths.ensure().unwrap();
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        let ctx = Ctx {
            store: Arc::new(store),
            paths: Arc::new(paths),
            settings: Arc::new(devd_core::Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Cli),
            run: None,
            dry_run: false,
        };
        (ctx, dir)
    }

    #[test]
    fn a_constraint_yields_the_version_the_installer_wants() {
        assert_eq!(version_of("^8.3").as_deref(), Some("8.3"));
        assert_eq!(version_of(">=20.0.0").as_deref(), Some("20.0.0"));
        assert_eq!(version_of("~18").as_deref(), Some("18"));
        assert_eq!(version_of(">= 8.2 *").as_deref(), Some("8.2"));
        assert_eq!(version_of("latest"), None);
    }

    #[test]
    fn the_allowlist_names_no_secret_material() {
        let names = passed_environment();
        for name in &names {
            let upper = name.to_ascii_uppercase();
            assert!(!upper.contains("TOKEN"), "{name} looks like a credential");
            assert!(!upper.contains("SECRET"), "{name} looks like a credential");
            assert!(
                !upper.contains("PASSWORD"),
                "{name} looks like a credential"
            );
        }
        assert!(names.iter().any(|n| n == "PATH"));
        assert!(names.iter().any(|n| n == "HOME"));
    }

    #[test]
    fn a_manifest_is_stable_and_names_variables_without_their_values() {
        let (ctx, dir) = fixture();
        let project = dir.path().join("www").join("web");
        std::fs::create_dir_all(project.join("public")).unwrap();
        std::fs::write(
            project.join("package.json"),
            r#"{"name":"web","version":"1.0.0","engines":{"node":">=20"},"scripts":{"test":"node --test"}}"#,
        )
        .unwrap();
        std::fs::write(project.join("index.js"), "console.log('web');\n").unwrap();
        std::fs::write(project.join(".env.example"), "API_TOKEN=\nAPP_URL=\n").unwrap();
        std::fs::write(project.join(".env"), "API_TOKEN=super-secret-value\n").unwrap();

        let first = manifest(&ctx, &project, None).unwrap();
        let second = manifest(&ctx, &project, None).unwrap();
        assert_eq!(
            first.fingerprint, second.fingerprint,
            "the fingerprint ignores the timestamp"
        );
        assert_eq!(first.name, "web");
        assert!(
            first.stack.as_deref() == Some("node"),
            "detected {:?}",
            first.stack
        );
        assert!(first.env.keys.iter().any(|k| k == "API_TOKEN"));
        assert!(first.env.secret_keys.iter().any(|k| k == "API_TOKEN"));
        assert!(first.env.unset_keys.iter().any(|k| k == "APP_URL"));

        let text = serde_json::to_string(&first).unwrap();
        assert!(
            !text.contains("super-secret-value"),
            "a value leaked into the manifest"
        );
        assert!(first.isolation.environment_allowlist);
        assert!(!first
            .isolation
            .passed_environment
            .iter()
            .any(|n| n.contains("TOKEN")));
    }
}
