//! The operations capabilities.
//!
//! Reads are `read`. Changing a project's own files or devd's records is `write`. Anything that runs a
//! command (a cron job, a worker, a deploy script) is `exec`, so an agent can propose it and only a
//! person approves it. Restoring an earlier commit and emptying logs are `destructive`. Anything that
//! touches the Ploi server is `remote`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

use crate::{cron, daemons, deploy, env, limits, logs, notify, ploi};

/// No arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NoArgs {}

/// A site.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SiteArg {
    /// The site's name, as `site.list` shows it.
    pub site: String,
}

/// Optional site.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MaybeSite {
    /// Limit to one site. Omit for all.
    pub site: Option<String>,
}

/// Variables to set.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnvSetArgs {
    /// The site.
    pub site: String,
    /// Variable names and their new values, e.g. {"APP_ENV": "local"}.
    pub vars: BTreeMap<String, String>,
}

/// Variables to remove.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnvUnsetArgs {
    /// The site.
    pub site: String,
    /// Variable names to remove.
    pub names: Vec<String>,
}

/// What to compare the site's .env with.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnvDiffArgs {
    /// The site.
    pub site: String,
    /// Another env file on this machine, e.g. a copy of the production file.
    pub against_file: Option<String>,
    /// Compare with the .env of the linked Ploi site (needs `ploi.connect` and `ploi.link`).
    pub against_ploi: Option<bool>,
}

/// A cron job.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CronAddArgs {
    /// The site; the command runs in its folder with its PHP first on PATH.
    pub site: String,
    /// Five-field cron expression, or @hourly, @daily, @weekly, @monthly.
    pub schedule: String,
    /// One command line.
    pub command: String,
}

/// A job by id.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct JobArg {
    /// The job id, from `cron.list`.
    pub id: String,
}

/// Turn a job on or off.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct JobEnableArgs {
    /// The job id.
    pub id: String,
    /// On or off.
    pub enabled: bool,
}

/// A queue worker.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct QueueAddArgs {
    /// The site.
    pub site: String,
    /// A short name. Default `queue`.
    pub name: Option<String>,
    /// The queue connection (`database`, `redis`). Laravel's default when omitted.
    pub connection: Option<String>,
    /// Queue name(s), comma separated.
    pub queue: Option<String>,
    /// Attempts before a job fails. Default 3.
    pub tries: Option<u32>,
    /// Seconds to sleep when idle. Default 3.
    pub sleep: Option<u32>,
    /// Restart the worker after this many seconds so it picks up new code. Default 3600.
    pub max_time: Option<u32>,
    /// How many processes, 1 to 8. Default 1.
    pub processes: Option<u8>,
    /// A command to run instead of `queue:work`, for any other background program.
    pub command: Option<String>,
}

/// A worker by id.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DaemonArg {
    /// The worker id, from `queue.list`.
    pub id: String,
}

/// A worker's log.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DaemonLogArgs {
    /// The worker id.
    pub id: String,
    /// How many lines. Default 100.
    pub lines: Option<usize>,
}

/// Laravel log query.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LaravelLogArgs {
    /// The site.
    pub site: String,
    /// How many entries, newest first. Default 30.
    pub limit: Option<usize>,
    /// Only these levels, e.g. ["ERROR", "CRITICAL"].
    pub levels: Option<Vec<String>>,
    /// Only entries whose message contains this text.
    pub search: Option<String>,
}

/// A site log.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SiteLogArgs {
    /// The site.
    pub site: String,
    /// `nginx_access`, `nginx_error` or `php`.
    pub kind: String,
    /// How many lines. Default 100.
    pub lines: Option<usize>,
}

/// A deploy script.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ScriptArgs {
    /// The site.
    pub site: String,
    /// The script: one command per line. `{BRANCH}` and `{SITE_DIRECTORY}` are filled in.
    pub script: String,
}

/// Start a deploy.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeployArgs {
    /// The site.
    pub site: String,
    /// Wait for the deploy to finish and return how it ended. A command-line call waits by default, because the
    /// process that runs the deploy ends with the command; the window and the MCP server return at once.
    pub wait: Option<bool>,
}

/// Deployment history.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HistoryArgs {
    /// The site.
    pub site: String,
    /// How many. Default 20.
    pub limit: Option<usize>,
}

/// A deployment.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeploymentArg {
    /// The deployment id, from `deploy.history`.
    pub id: String,
    /// How many log lines. Default 200.
    pub lines: Option<usize>,
}

/// Roll back.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RollbackArgs {
    /// The site.
    pub site: String,
    /// Restore the code a particular deployment ended on. Omit to go back to before the newest deploy.
    pub deployment: Option<String>,
}

/// Link a site to its repository.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RepoLinkArgs {
    /// The site.
    pub site: String,
    /// The `origin` address, added if the folder has none.
    pub remote: Option<String>,
    /// The branch deploys follow. Default: the current branch.
    pub branch: Option<String>,
}

/// The push-to-deploy hook.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookArgs {
    /// The site.
    pub site: String,
    /// On or off. Turning it on makes a new secret address.
    pub enabled: bool,
}

/// A notification target.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NotifyAddArgs {
    /// `inbox`, `slack`, `discord` or `webhook`.
    pub channel: String,
    /// The https webhook address (not needed for the inbox).
    pub url: Option<String>,
    /// Events wanted: deploy.finished, deploy.failed, cron.failed. Default all.
    pub events: Option<Vec<String>>,
    /// Only for this site.
    pub site: Option<String>,
}

/// A target by id.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TargetArg {
    /// The target id, from `notify.list`.
    pub id: String,
}

/// Connect Ploi.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiConnectArgs {
    /// A Ploi API token from https://ploi.io/profile/api-keys. Stored privately, never shown again.
    pub api_token: String,
}

/// A Ploi server.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiServerArg {
    /// The Ploi server id, from `ploi.servers`.
    pub server: u64,
}

/// A Ploi site.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiSiteArgs {
    /// The Ploi server id.
    pub server: u64,
    /// The Ploi site id, from `ploi.sites`.
    pub site: u64,
}

/// Link a local site to a Ploi site.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiLinkArgs {
    /// The local site.
    pub local: String,
    /// The Ploi server id.
    pub server: u64,
    /// The Ploi site id.
    pub site: u64,
}

/// Deploy on Ploi.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiDeployArgs {
    /// The local site that is linked to the Ploi site. Give this, or server and site.
    pub local: Option<String>,
    /// The Ploi server id.
    pub server: Option<u64>,
    /// The Ploi site id.
    pub site: Option<u64>,
    /// The branch to deploy. The site's own when omitted.
    pub branch: Option<String>,
}

/// Replace a Ploi deploy script.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiScriptArgs {
    /// The Ploi server id.
    pub server: u64,
    /// The Ploi site id.
    pub site: u64,
    /// The new script.
    pub script: String,
}

/// What the server's PHP allows.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LimitsArgs {
    /// `memory_limit` on the server, e.g. 128M.
    pub memory_limit: Option<String>,
    /// `upload_max_filesize`, e.g. 5M.
    pub upload_max_filesize: Option<String>,
    /// `post_max_size`, e.g. 8M.
    pub post_max_size: Option<String>,
    /// `max_execution_time` in seconds, e.g. 30.
    pub max_execution_time: Option<String>,
}

/// Copy a Ploi site's deploy script to a local site.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PloiPullScriptArgs {
    /// The local site, which must be linked to the Ploi site.
    pub local: String,
}

fn masked_map(vars: &BTreeMap<String, String>) -> Value {
    json!(vars.iter().map(|(k, v)| (k.clone(), env::masked(k, v))).collect::<BTreeMap<_, _>>())
}

fn env_text(root: &std::path::Path) -> String {
    std::fs::read_to_string(env::path_of(root)).unwrap_or_default()
}

fn resolve_ploi(ctx: &devd_core::capability::Ctx, local: &Option<String>, server: Option<u64>, site: Option<u64>) -> Result<(u64, u64)> {
    match (local, server, site) {
        (Some(l), _, _) => ploi::linked(ctx, l),
        (None, Some(s), Some(i)) => Ok((s, i)),
        _ => Err(DevdError::invalid("name the local site (linked with `ploi.link`) or give both the Ploi server and site ids")),
    }
}

/// Register every operations capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    register_env(reg)?;
    register_cron(reg)?;
    register_queue(reg)?;
    register_logs(reg)?;
    register_deploy(reg)?;
    register_notify(reg)?;
    register_ploi(reg)?;
    crate::caps_more::register(reg)?;
    crate::caps_shell::register(reg)?;
    Ok(())
}

// --------------------------------------------------------------------------------------- env

fn register_env(reg: &mut Registry) -> Result<()> {
    reg.define::<SiteArg>("env.get", "Show a site's .env with secrets hidden")
        .description("The variables in the site's .env file. Anything whose name suggests a secret (PASSWORD, SECRET, TOKEN, KEY, ...) is shown as hidden with its length only.")
        .group("env")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("env.get", format!("read the .env of {}", a.site))),
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let vars = env::variables(&env_text(&root));
                Ok(Outcome::with_data(format!("{} variable(s) in the .env of {}", vars.len(), a.site), json!({ "site": a.site, "exists": env::path_of(&root).is_file(), "vars": masked_map(&vars) })))
            },
        )?;

    reg.define::<SiteArg>("env.reveal", "Show a site's .env with its secrets")
        .description("Like env.get but with every value in clear. A person only: an agent never reads secrets.")
        .group("env")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("env.reveal", format!("read the .env of {} including secrets", a.site))),
            |ctx, a| {
                ctx.require_human("reading secrets")?;
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let vars = env::variables(&env_text(&root));
                Ok(Outcome::with_data(format!("{} variable(s)", vars.len()), json!({ "site": a.site, "vars": vars })))
            },
        )?;

    reg.define::<EnvSetArgs>("env.set", "Set variables in a site's .env")
        .description("Sets or adds the named variables, keeping comments, order and every other line. The previous file is kept beside it as .env.devd-backup-<time> (the last five are kept). The audit trail records the names, and hides the values of secret-looking ones.")
        .group("env")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                env::with_set(&env_text(&root), &a.vars)?;
                Ok(Plan::new("env.set", format!("set {} in the .env of {}", a.vars.keys().cloned().collect::<Vec<_>>().join(", "), a.site)).step("keep the old file as .env.devd-backup-<time>").step("write the new .env"))
            },
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let text = env::with_set(&env_text(&root), &a.vars)?;
                let backup = env::write_with_backup(&root, &text)?;
                Ok(Outcome::with_data(
                    format!("{} variable(s) set in the .env of {}", a.vars.len(), a.site),
                    json!({ "site": a.site, "set": a.vars.keys().collect::<Vec<_>>(), "backup": backup.map(|b| b.display().to_string()), "note": "A running app may cache its configuration: run `php artisan config:clear` if it does not notice." }),
                ))
            },
        )?;

    reg.define::<EnvUnsetArgs>("env.unset", "Remove variables from a site's .env")
        .description("Removes the named variables. The previous file is kept as .env.devd-backup-<time>.")
        .group("env")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("env.unset", format!("remove {} from the .env of {}", a.names.join(", "), a.site)).step("keep the old file as .env.devd-backup-<time>")),
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let text = env::without(&env_text(&root), &a.names);
                let backup = env::write_with_backup(&root, &text)?;
                Ok(Outcome::with_data(format!("{} variable(s) removed", a.names.len()), json!({ "site": a.site, "removed": a.names, "backup": backup.map(|b| b.display().to_string()) })))
            },
        )?;

    reg.define::<EnvDiffArgs>("env.diff", "Compare a site's .env with another environment")
        .description("Which variables are only here, only there, or set differently. Secrets are never shown: they are reported as differing. Warns about APP_DEBUG on in production, a local APP_URL there and a missing APP_KEY. Compare with a file (a copy of the production .env) or with the .env of the linked Ploi site.")
        .group("env")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("env.diff", format!("compare the .env of {}", a.site))),
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let here = env::variables(&env_text(&root));
                let (there, label) = if a.against_ploi.unwrap_or(false) {
                    let (server, site) = ploi::linked(ctx, &a.site)?;
                    (ploi::env(ctx, server, site)?, format!("Ploi site {site}"))
                } else if let Some(f) = &a.against_file {
                    let text = std::fs::read_to_string(f).map_err(|e| DevdError::not_found(format!("cannot read {f}: {e}")))?;
                    (env::variables(&text), f.clone())
                } else {
                    return Err(DevdError::invalid("name what to compare with: `against_file`, or `against_ploi` true"));
                };
                let d = env::diff(&here, &there);
                Ok(Outcome::with_data(
                    format!("{} only here, {} only in {label}, {} different, {} same", d.only_here.len(), d.only_there.len(), d.different.len(), d.same),
                    json!({ "site": a.site, "against": label, "diff": d }),
                ))
            },
        )?;
    Ok(())
}

// -------------------------------------------------------------------------------------- cron

fn register_cron(reg: &mut Registry) -> Result<()> {
    reg.define::<CronAddArgs>("cron.add", "Add a cron job to a site")
        .description("Runs the command on the schedule while devd is open, in the site's folder with the site's PHP first on PATH. A job that was due while devd was closed is skipped, not run late. Output goes to the job's log file.")
        .group("cron")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .agent_needs_approval()
        .dry_run()
        .handlers(
            |ctx, a| {
                cron::parse(&a.schedule)?;
                crate::site_named(ctx, &a.site)?;
                let next = cron::parse(&a.schedule)?.next_after(chrono::Local::now());
                Ok(Plan::new("cron.add", format!("run `{}` for {} on `{}`", a.command, a.site, a.schedule)).step(format!("first run {}", next.map(|t| t.to_rfc3339()).unwrap_or_else(|| "never".into()))).effect("runs this command on a schedule"))
            },
            |ctx, a| {
                let job = cron::add(ctx, &a.site, &a.schedule, &a.command)?;
                Ok(Outcome::with_data(format!("cron job {} added to {}", job.id, a.site), json!(job)))
            },
        )?;

    reg.define::<SiteArg>("cron.laravel", "Run the Laravel scheduler every minute")
        .description("Adds `php artisan schedule:run` every minute to a Laravel site, which is the one cron entry Laravel's task scheduler needs. Does nothing if the job is already there.")
        .group("cron")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                if !root.join("artisan").is_file() {
                    return Err(DevdError::invalid(format!("{} is not a Laravel project (no artisan file)", a.site)));
                }
                Ok(Plan::new("cron.laravel", format!("run `php artisan schedule:run` every minute for {}", a.site)).effect("runs this command every minute while devd is open"))
            },
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                if !root.join("artisan").is_file() {
                    return Err(DevdError::invalid(format!("{} is not a Laravel project (no artisan file)", a.site)));
                }
                if let Some(existing) = cron::list(ctx, Some(&a.site))?.into_iter().find(|j| j.command.contains("schedule:run")) {
                    return Ok(Outcome::with_data(format!("{} already runs the scheduler (job {})", a.site, existing.id), json!(existing)));
                }
                let job = cron::add(ctx, &a.site, "* * * * *", "php artisan schedule:run")?;
                Ok(Outcome::with_data(format!("the Laravel scheduler now runs every minute for {}", a.site), json!(job)))
            },
        )?;

    reg.define::<MaybeSite>("cron.list", "List cron jobs")
        .description("The jobs, with when each is next due, when it last ran and how that went.")
        .group("cron")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("cron.list", "list the cron jobs")),
            |ctx, a| {
                let jobs = cron::list(ctx, a.site.as_deref())?;
                Ok(Outcome::with_data(format!("{} cron job(s)", jobs.len()), json!({ "jobs": jobs })))
            },
        )?;

    reg.define::<DaemonLogArgs>("cron.log", "Show what a cron job printed")
        .description("The last lines of the job's output file: each run's command, its output and its exit code.")
        .group("cron")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("cron.log", format!("read the log of cron job {}", a.id))),
            |ctx, a| {
                let lines = cron::log(ctx, &a.id, a.lines.unwrap_or(100))?;
                Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({ "lines": lines })))
            },
        )?;

    reg.define::<JobArg>("cron.remove", "Remove a cron job")
        .description("Stops the job from running. Its log file is kept.")
        .group("cron")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("cron.remove", format!("remove cron job {}", a.id))),
            |ctx, a| {
                cron::remove(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("cron job {} removed", a.id), json!({ "id": a.id })))
            },
        )?;

    reg.define::<JobEnableArgs>("cron.enable", "Turn a cron job on or off")
        .description("A job that is off keeps its place and schedule but does not run.")
        .group("cron")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("cron.enable", format!("turn cron job {} {}", a.id, if a.enabled { "on" } else { "off" }))),
            |ctx, a| {
                let job = cron::set_enabled(ctx, &a.id, a.enabled)?;
                Ok(Outcome::with_data(format!("cron job {} is {}", a.id, if a.enabled { "on" } else { "off" }), json!(job)))
            },
        )?;

    reg.define::<JobArg>("cron.run", "Run a cron job now")
        .description("Runs the job's command once, waits for it to finish and reports the exit code. Its output is in the job's log.")
        .group("cron")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let j = cron::show(ctx, &a.id)?;
                Ok(Plan::new("cron.run", format!("run `{}` for {} now", j.command, j.site)).effect("runs the command"))
            },
            |ctx, a| {
                let (job, ran) = cron::run_now(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("exit {} after {:.1}s", ran.exit, ran.seconds), json!({ "job": job, "exit": ran.exit, "seconds": ran.seconds, "log_tail": crate::tail_bytes(std::path::Path::new(&job.log), 4000) })))
            },
        )?;
    Ok(())
}

// ------------------------------------------------------------------------------------- queue

fn register_queue(reg: &mut Registry) -> Result<()> {
    reg.define::<QueueAddArgs>("queue.add", "Add a queue worker (or other background program) to a site")
        .description("For a Laravel site this is `php artisan queue:work` with the site's own PHP, `--max-time` so the worker restarts and picks up new code, and the processes you ask for. Give a `command` to supervise any other program in the site's folder. It is a devd service: restarted when it exits, listed with the services. It is created stopped; start it with `queue.start`.")
        .group("queue")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .agent_needs_approval()
        .dry_run()
        .handlers(
            |ctx, a| {
                let spec = spec_of(a);
                let argv = daemons::argv(ctx, &spec)?;
                Ok(Plan::new("queue.add", format!("supervise a worker for {}", a.site)).step(format!("command: {}", argv.join(" "))).step(format!("{} process(es)", spec.processes)).effect("creates a service that runs this command"))
            },
            |ctx, a| {
                let d = daemons::add(ctx, &spec_of(a))?;
                Ok(Outcome::with_data(format!("worker {} created (stopped)", d.id), json!(d)))
            },
        )?;

    reg.define::<MaybeSite>("queue.list", "List queue workers")
        .description("The workers, with how many of their processes are running.")
        .group("queue")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("queue.list", "list the workers")),
            |ctx, a| {
                let list = daemons::list(ctx, a.site.as_deref())?;
                Ok(Outcome::with_data(format!("{} worker(s)", list.len()), json!({ "workers": list })))
            },
        )?;

    for (id, summary, verb) in [("queue.start", "Start a queue worker", "start"), ("queue.stop", "Stop a queue worker", "stop"), ("queue.restart", "Restart a queue worker", "restart")] {
        reg.define::<DaemonArg>(id, summary)
            .description("Starts, stops or restarts every process of the worker. Restart after a deploy so the worker runs the new code.")
            .group("queue")
            .tier(Tier::Exec)
            .effect(Effect::Process)
            .agent_safe()
            .idempotent()
            .handlers(
                move |ctx, a| {
                    let d = daemons::show(ctx, &a.id)?;
                    Ok(Plan::new(id, format!("{verb} worker {} ({} process(es))", d.id, d.processes)))
                },
                move |ctx, a| {
                    let d = daemons::control(ctx, &a.id, verb)?;
                    Ok(Outcome::with_data(format!("worker {}: {}/{} running", d.id, d.running, d.processes), json!(d)))
                },
            )?;
    }

    reg.define::<DaemonArg>("queue.remove", "Remove a queue worker")
        .description("Stops the worker and removes its service. A running job is cut off.")
        .group("queue")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("queue.remove", format!("stop and remove worker {}", a.id)).effect("stops the process")),
            |ctx, a| {
                daemons::remove(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("worker {} removed", a.id), json!({ "id": a.id })))
            },
        )?;

    reg.define::<DaemonLogArgs>("queue.logs", "Show a queue worker's log")
        .description("The last lines the worker printed.")
        .group("queue")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("queue.logs", format!("read the log of worker {}", a.id))),
            |ctx, a| {
                let lines = daemons::logs(ctx, &a.id, a.lines.unwrap_or(100))?;
                Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({ "lines": lines })))
            },
        )?;
    Ok(())
}

fn spec_of(a: &QueueAddArgs) -> daemons::Spec {
    daemons::Spec {
        site: a.site.clone(),
        name: a.name.clone().unwrap_or_else(|| if a.command.is_some() { "daemon".to_string() } else { "queue".to_string() }),
        command: a.command.clone(),
        connection: a.connection.clone(),
        queue: a.queue.clone(),
        tries: a.tries,
        sleep: a.sleep,
        max_time: a.max_time,
        processes: a.processes.unwrap_or(1),
    }
}

// -------------------------------------------------------------------------------------- logs

fn register_logs(reg: &mut Registry) -> Result<()> {
    reg.define::<LaravelLogArgs>("logs.laravel", "Read a Laravel site's log as entries")
        .description("The newest entries of storage/logs/laravel.log (or the newest daily file), newest first, each with its time, level, first message line and the first stack frame. Filter by level or by text. Reads only the end of a large log.")
        .group("logs")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("logs.laravel", format!("read the Laravel log of {}", a.site))),
            |ctx, a| {
                let v = logs::laravel(ctx, &a.site, a.limit.unwrap_or(30).clamp(1, 500), a.levels.as_deref().unwrap_or(&[]), a.search.as_deref())?;
                Ok(Outcome::with_data(format!("{} matching entr(ies)", v["matching"].as_u64().unwrap_or(0)), v))
            },
        )?;

    reg.define::<SiteLogArgs>("logs.site", "Read a site's web server or PHP log")
        .description("The last lines of the site's nginx access log, nginx error log, or its PHP error log.")
        .group("logs")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("logs.site", format!("read the {} log of {}", a.kind, a.site))),
            |ctx, a| {
                let v = logs::site_log(ctx, &a.site, &a.kind, a.lines.unwrap_or(100))?;
                Ok(Outcome::with_data(format!("{} line(s)", v["lines"].as_array().map(Vec::len).unwrap_or(0)), v))
            },
        )?;

    reg.define::<SiteArg>("logs.laravel_clear", "Empty a Laravel site's log files")
        .description("Empties storage/logs/laravel*.log. A copy of each is kept beside it as .log.bak (one generation).")
        .group("logs")
        .tier(Tier::Destructive)
        .effect(Effect::Write)
        .handlers(
            |_, a| Ok(Plan::new("logs.laravel_clear", format!("empty the Laravel log files of {}", a.site)).step("keep a .log.bak copy of each").effect("the previous .log.bak is overwritten")),
            |ctx, a| {
                let n = logs::clear_laravel(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("{n} log file(s) emptied"), json!({ "site": a.site, "files": n })))
            },
        )?;
    Ok(())
}

// ------------------------------------------------------------------------------------ deploy

fn register_deploy(reg: &mut Registry) -> Result<()> {
    reg.define::<SiteArg>("deploy.script", "Show a site's deploy script")
        .description("The script deploys run, or the one devd would use when none is saved: pull, composer install, migrate, npm build, optimize, chosen from what the project contains.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.script", format!("read the deploy script of {}", a.site))),
            |ctx, a| {
                let (script, saved) = deploy::script(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("{} script", if saved { "saved" } else { "default" }), json!({ "site": a.site, "saved": saved, "script": script })))
            },
        )?;

    reg.define::<ScriptArgs>("deploy.script_set", "Save a site's deploy script")
        .description("One command per line, run in the site's folder with its PHP first on PATH; the first failure stops the deploy. {BRANCH} and {SITE_DIRECTORY} are filled in. Lines for a server (`cd`, `sudo`, reloading PHP-FPM) are skipped, so a script copied from Ploi runs here. It runs commands, so saving it is an exec action.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Write)
        .agent_safe()
        .agent_needs_approval()
        .dry_run()
        .idempotent()
        .handlers(
            |ctx, a| {
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let l = deploy::lines(&a.script, &root, "main", false);
                let mut plan = Plan::new("deploy.script_set", format!("save the deploy script of {}", a.site));
                for (line, why) in l {
                    plan = plan.step(match why { None => format!("run: {line}"), Some(w) => format!("skip: {line} ({w})") });
                }
                Ok(plan.effect("future deploys run these commands"))
            },
            |ctx, a| {
                deploy::set_script(ctx, &a.site, &a.script)?;
                Ok(Outcome::with_data(format!("deploy script of {} saved", a.site), json!({ "site": a.site })))
            },
        )?;

    reg.define::<DeployArgs>("deploy.run", "Deploy a site")
        .description("Runs the site's deploy script line by line and returns at once with the deployment; read progress with `deploy.status` and `deploy.log`. Stops at the first failing line. Records the commit before and after, and tells the notification targets when it ends.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let (script, _) = deploy::script(ctx, &a.site)?;
                let (_, root) = crate::site_root(ctx, &a.site)?;
                let mut plan = Plan::new("deploy.run", format!("deploy {}", a.site));
                for (line, why) in deploy::lines(&script, &root, "main", false) {
                    plan = plan.step(match why { None => format!("run: {line}"), Some(w) => format!("skip: {line} ({w})") });
                }
                for w in limits::warnings(ctx, &a.site) {
                    plan = plan.effect(format!("NOTE {w}"));
                }
                Ok(plan.effect("runs these commands in the project folder"))
            },
            |ctx, a| {
                let trigger = if ctx.actor.is_human() { "manual" } else { "agent" };
                let mut d = deploy::start(ctx, &a.site, trigger, None)?;
                let wait = a.wait.unwrap_or(matches!(ctx.actor.surface, devd_core::capability::Surface::Cli));
                if wait {
                    d = deploy::wait(ctx, &d.id)?;
                }
                let message = if wait { format!("deployment {} of {} {}", d.id, a.site, d.state) } else { format!("deployment {} of {} started", d.id, a.site) };
                Ok(Outcome::with_data(message, json!({ "deployment": d, "warnings": limits::warnings(ctx, &a.site) })))
            },
        )?;

    reg.define::<HistoryArgs>("deploy.history", "List a site's deployments")
        .description("Newest first, with the commit before and after, the result and how long each took.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.history", format!("list the deployments of {}", a.site))),
            |ctx, a| {
                let h = deploy::history(ctx, &a.site, a.limit.unwrap_or(20))?;
                Ok(Outcome::with_data(format!("{} deployment(s)", h.len()), json!({ "deployments": h })))
            },
        )?;

    reg.define::<SiteArg>("deploy.check", "Compare a site's PHP limits with your server's")
        .description("Lists every PHP limit (memory, upload size, post size, execution time) that is looser here than on your server, which is where a deploy-time surprise comes from. Ploi's API does not expose a server's limits, so devd uses the ones you gave `ploi.limits_set`, and assumes a default Ploi server until then (and says so).")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.check", format!("compare the PHP limits of {} with the server", a.site))),
            |ctx, a| {
                let here = limits::local(ctx, &a.site)?;
                let (there, assumed) = limits::server(ctx)?;
                let w = limits::compare(&here, &there, assumed);
                Ok(Outcome::with_data(if w.is_empty() { "no limit is looser here than on the server".to_string() } else { format!("{} limit(s) are looser here", w.len()) }, json!({ "site": a.site, "here": here, "server": there, "server_assumed": assumed, "warnings": w })))
            },
        )?;

    reg.define::<DeploymentArg>("deploy.status", "Show one deployment")
        .description("The deployment's state (running, success, failed), commits, and the line that failed.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.status", format!("read deployment {}", a.id))),
            |ctx, a| {
                let d = deploy::get(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("{} ({})", d.state, d.site), json!(d)))
            },
        )?;

    reg.define::<DeploymentArg>("deploy.log", "Read a deployment's output")
        .description("The last lines of what the deploy script printed.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.log", format!("read the log of deployment {}", a.id))),
            |ctx, a| {
                let lines = deploy::log(ctx, &a.id, a.lines.unwrap_or(200))?;
                Ok(Outcome::with_data(format!("{} line(s)", lines.len()), json!({ "id": a.id, "lines": lines })))
            },
        )?;

    reg.define::<RollbackArgs>("deploy.rollback", "Roll a site back to an earlier deployment")
        .description("Checks out the commit the chosen deployment ended on (or the commit before the newest deployment) and runs the deploy script without pulling. Refuses while the folder has uncommitted changes. The folder is left on a detached HEAD at that commit; `git switch <branch>` goes back.")
        .group("deploy")
        .tier(Tier::Destructive)
        .effect(Effect::Process)
        .handlers(
            |ctx, a| {
                let (sha, why) = deploy::rollback_target(ctx, &a.site, a.deployment.as_deref())?;
                Ok(Plan::new("deploy.rollback", format!("roll {} back to {}", a.site, &sha[..sha.len().min(10)])).step(format!("restore {why}")).step("run the deploy script without pulling").effect("the working tree changes to that commit"))
            },
            |ctx, a| {
                let d = deploy::rollback(ctx, &a.site, a.deployment.as_deref(), "rollback")?;
                Ok(Outcome::with_data(format!("rollback {} of {} started", d.id, a.site), json!(d)))
            },
        )?;

    reg.define::<SiteArg>("repo.status", "Show a site's repository link and Git state")
        .description("Whether the folder is a Git repository, the branch deploys follow, whether push-to-deploy is on, the current branch and commit, the remote, uncommitted files and how far it is from its upstream.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("repo.status", format!("read the repository state of {}", a.site))),
            |ctx, a| {
                let v = deploy::repo_status(ctx, &a.site)?;
                Ok(Outcome::with_data(if v["linked"].as_bool().unwrap_or(false) { "linked" } else { "not linked" }.to_string(), v))
            },
        )?;

    reg.define::<RepoLinkArgs>("repo.link", "Link a site to a branch of its repository")
        .description("Records the branch deploys pull. When a remote address is given and the folder has no `origin`, adds it. The site's folder must already be a Git repository.")
        .group("deploy")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("repo.link", format!("link {} to branch {}", a.site, a.branch.clone().unwrap_or_else(|| "(current)".into())))),
            |ctx, a| {
                let v = deploy::repo_link(ctx, &a.site, a.remote.as_deref(), a.branch.as_deref())?;
                Ok(Outcome::with_data(format!("{} is linked", a.site), v))
            },
        )?;

    reg.define::<HookArgs>("deploy.hook", "Turn push-to-deploy on or off")
        .description("When on, a POST to a secret address on the devd server deploys the site. Turning it on makes a new secret (the old one stops working). To receive GitHub's push events, open a public link to the devd server and give GitHub the address. Anyone with the address can start a deploy, so it is shown once here and is not logged.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Config)
        .handlers(
            |_, a| Ok(Plan::new("deploy.hook", format!("turn the push-to-deploy hook of {} {}", a.site, if a.enabled { "on" } else { "off" })).effect("a secret address can start deploys")),
            |ctx, a| {
                let v = deploy::set_hook(ctx, &a.site, a.enabled)?;
                Ok(Outcome::with_data(format!("push-to-deploy is {} for {}", if a.enabled { "on" } else { "off" }, a.site), v))
            },
        )?;
    Ok(())
}

// ------------------------------------------------------------------------------------ notify

fn register_notify(reg: &mut Registry) -> Result<()> {
    reg.define::<NotifyAddArgs>("notify.add", "Add a notification target")
        .description("Tell the inbox, a Slack or Discord webhook, or any https webhook when a deploy finishes or fails or a cron job fails. The webhook must be a public https address.")
        .group("notify")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .handlers(
            |_, a| Ok(Plan::new("notify.add", format!("notify {} on {}", a.channel, a.events.clone().map(|e| e.join(", ")).unwrap_or_else(|| "every event".into())))),
            |ctx, a| {
                let t = notify::add(ctx, &a.channel, a.url.as_deref(), a.events.as_deref().unwrap_or(&[]), a.site.as_deref())?;
                Ok(Outcome::with_data(format!("target {} added", t.id), json!(t)))
            },
        )?;

    reg.define::<NoArgs>("notify.list", "List notification targets")
        .description("Where messages go and for which events. A webhook is shown by host only.")
        .group("notify")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("notify.list", "list the notification targets")),
            |ctx, _| {
                let t = notify::list(ctx)?;
                Ok(Outcome::with_data(format!("{} target(s)", t.len()), json!({ "targets": t, "events": notify::EVENTS })))
            },
        )?;

    reg.define::<TargetArg>("notify.remove", "Remove a notification target")
        .description("Stops sending to it.")
        .group("notify")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .handlers(
            |_, a| Ok(Plan::new("notify.remove", format!("remove target {}", a.id))),
            |ctx, a| {
                notify::remove(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("target {} removed", a.id), json!({ "id": a.id })))
            },
        )?;

    reg.define::<TargetArg>("notify.test", "Send a test message to a target")
        .description("Delivers one test message and reports whether it arrived.")
        .group("notify")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .handlers(
            |_, a| Ok(Plan::new("notify.test", format!("send a test message to target {}", a.id)).effect("sends a message over the network")),
            |ctx, a| {
                notify::test(ctx, &a.id)?;
                Ok(Outcome::with_data("delivered".to_string(), json!({ "id": a.id })))
            },
        )?;
    Ok(())
}

// --------------------------------------------------------------------------------------- ploi

fn register_ploi(reg: &mut Registry) -> Result<()> {
    reg.define::<PloiConnectArgs>("ploi.connect", "Connect devd to Ploi with an API token")
        .description("Stores the token in a private file (never in the database, settings or audit trail) and checks it by asking Ploi who it belongs to. Create the token at https://ploi.io/profile/api-keys and give it only the scopes you want agents to be able to use. A person only.")
        .group("ploi")
        .tier(Tier::Write)
        .effect(Effect::Remote)
        .handlers(
            |_, _| Ok(Plan::new("ploi.connect", "store the Ploi API token and check it").effect("contacts ploi.io")),
            |ctx, a| {
                ctx.require_human("connecting Ploi")?;
                ploi::save_token(ctx, &a.api_token)?;
                match ploi::user(ctx) {
                    Ok(u) => Ok(Outcome::with_data(format!("connected to Ploi as {}", u["name"].as_str().or(u["email"].as_str()).unwrap_or("this account")), json!({ "user": { "name": u["name"], "email": u["email"] } }))),
                    Err(e) => {
                        ploi::forget_token(ctx);
                        Err(e)
                    }
                }
            },
        )?;

    reg.define::<NoArgs>("ploi.disconnect", "Forget the Ploi API token")
        .description("Deletes the stored token. The token still exists in Ploi until you delete it there.")
        .group("ploi")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .handlers(
            |_, _| Ok(Plan::new("ploi.disconnect", "delete the stored Ploi API token")),
            |ctx, _| {
                ctx.require_human("disconnecting Ploi")?;
                ploi::forget_token(ctx);
                Ok(Outcome::with_data("Ploi disconnected".to_string(), json!({})))
            },
        )?;

    reg.define::<NoArgs>("ploi.status", "Is Ploi connected, and as whom")
        .description("Whether a token is stored (never the token), and the account it belongs to when Ploi answers.")
        .group("ploi")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("ploi.status", "check the Ploi connection").effect("contacts ploi.io")),
            |ctx, _| {
                let source = ploi::source(ctx);
                if source == "none" {
                    return Ok(Outcome::with_data("not connected".to_string(), json!({ "connected": false, "how": "`ploi.connect` with an API token from https://ploi.io/profile/api-keys" })));
                }
                match ploi::user(ctx) {
                    Ok(u) => Ok(Outcome::with_data("connected".to_string(), json!({ "connected": true, "token_from": source, "user": { "name": u["name"], "email": u["email"] } }))),
                    Err(e) => Ok(Outcome::with_data(format!("a token is stored but Ploi refused it: {e}"), json!({ "connected": false, "token_from": source, "error": e.to_string() }))),
                }
            },
        )?;

    reg.define::<NoArgs>("ploi.servers", "List your Ploi servers")
        .description("Id, name, address, PHP and MySQL versions, number of sites and status.")
        .group("ploi")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("ploi.servers", "list the Ploi servers").effect("contacts ploi.io")),
            |ctx, _| {
                let v = ploi::servers(ctx)?;
                Ok(Outcome::with_data(format!("{} server(s)", v.as_array().map(Vec::len).unwrap_or(0)), json!({ "servers": v })))
            },
        )?;

    reg.define::<PloiServerArg>("ploi.sites", "List the sites on a Ploi server")
        .description("Id, domain, status, PHP version, project type, last deploy and whether quick deploy is on.")
        .group("ploi")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("ploi.sites", format!("list the sites on Ploi server {}", a.server)).effect("contacts ploi.io")),
            |ctx, a| {
                let v = ploi::sites(ctx, a.server)?;
                Ok(Outcome::with_data(format!("{} site(s)", v.as_array().map(Vec::len).unwrap_or(0)), json!({ "sites": v })))
            },
        )?;

    reg.define::<PloiLinkArgs>("ploi.link", "Link a local site to its Ploi site")
        .description("Remembers which Ploi site a local site is, so `ploi.deploy`, `env.diff` against Ploi and `ploi.pull_script` need only the local name.")
        .group("ploi")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("ploi.link", format!("link {} to Ploi site {} on server {}", a.local, a.site, a.server)).effect("contacts ploi.io")),
            |ctx, a| {
                let v = ploi::link(ctx, &a.local, a.server, a.site)?;
                Ok(Outcome::with_data(format!("{} is linked to {}", a.local, v["domain"].as_str().unwrap_or("the Ploi site")), v))
            },
        )?;

    reg.define::<PloiDeployArgs>("ploi.deploy", "Deploy a site on your Ploi server")
        .description("Asks Ploi to start a deployment of the site on the server, exactly as the Deploy button does. Name the linked local site, or the Ploi server and site ids. Optionally a branch. This changes a live server, so it needs a person's approval.")
        .group("ploi")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let (server, site) = resolve_ploi(ctx, &a.local, a.server, a.site)?;
                Ok(Plan::new("ploi.deploy", format!("deploy Ploi site {site} on server {server}{}", a.branch.as_ref().map(|b| format!(" from {b}")).unwrap_or_default())).effect("starts a deployment on a live server"))
            },
            |ctx, a| {
                let (server, site) = resolve_ploi(ctx, &a.local, a.server, a.site)?;
                let v = ploi::deploy(ctx, server, site, a.branch.as_deref())?;
                crate::notify::send(ctx, "deploy.finished", a.local.as_deref().unwrap_or("ploi"), &format!("Deployment started on Ploi for site {site}"), json!({}));
                Ok(Outcome::with_data(v["message"].as_str().unwrap_or("deployment started").to_string(), json!({ "server": server, "site": site, "response": v })))
            },
        )?;

    reg.define::<PloiSiteArgs>("ploi.env", "Read a Ploi site's .env, secrets hidden")
        .description("The variables of the site's .env on the server, with secrets hidden. To compare it with a local site use `env.diff` with `against_ploi`.")
        .group("ploi")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("ploi.env", format!("read the .env of Ploi site {}", a.site)).effect("contacts ploi.io")),
            |ctx, a| {
                let vars = ploi::env(ctx, a.server, a.site)?;
                Ok(Outcome::with_data(format!("{} variable(s)", vars.len()), json!({ "vars": masked_map(&vars) })))
            },
        )?;

    reg.define::<PloiSiteArgs>("ploi.script", "Read a Ploi site's deploy script")
        .description("The deploy script on the server.")
        .group("ploi")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("ploi.script", format!("read the deploy script of Ploi site {}", a.site)).effect("contacts ploi.io")),
            |ctx, a| Ok(Outcome::with_data("deploy script".to_string(), json!({ "script": ploi::deploy_script(ctx, a.server, a.site)? }))),
        )?;

    reg.define::<PloiScriptArgs>("ploi.script_set", "Replace a Ploi site's deploy script")
        .description("Overwrites the deploy script on the server. It changes what the next deploy on a live server runs, so it needs a person's approval.")
        .group("ploi")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("ploi.script_set", format!("replace the deploy script of Ploi site {}", a.site)).effect("changes what deploys run on a live server")),
            |ctx, a| {
                let v = ploi::set_deploy_script(ctx, a.server, a.site, &a.script)?;
                Ok(Outcome::with_data(v["message"].as_str().unwrap_or("deploy script updated").to_string(), json!({ "server": a.server, "site": a.site })))
            },
        )?;

    reg.define::<LimitsArgs>("ploi.limits_set", "Tell devd what your server's PHP allows")
        .description("Ploi's API does not expose a server's PHP limits, so read them from the PHP tab of your server in Ploi and give them here. \
             devd then warns, in `deploy.check` and before `deploy.run`, about every limit that is looser on this machine. Only the limits you name change.")
        .group("ploi")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("ploi.limits_set", "remember the PHP limits of your server")),
            |ctx, a| {
                let mut set = BTreeMap::new();
                for (k, v) in [("memory_limit", &a.memory_limit), ("upload_max_filesize", &a.upload_max_filesize), ("post_max_size", &a.post_max_size), ("max_execution_time", &a.max_execution_time)] {
                    if let Some(v) = v {
                        set.insert(k.to_string(), v.clone());
                    }
                }
                if set.is_empty() {
                    return Err(DevdError::invalid("name at least one limit"));
                }
                let now = limits::set_server(ctx, &set)?;
                Ok(Outcome::with_data("server limits saved".to_string(), json!({ "server": now })))
            },
        )?;

    reg.define::<PloiPullScriptArgs>("ploi.pull_script", "Copy a Ploi site's deploy script to the local site")
        .description("Reads the deploy script from the linked Ploi site and saves it as the local site's deploy script, so a local deploy does what the server does (server-only lines such as sudo and PHP-FPM reloads are skipped when run here).")
        .group("ploi")
        .tier(Tier::Exec)
        .effect(Effect::Write)
        .agent_safe()
        .agent_needs_approval()
        .dry_run()
        .handlers(
            |_, a| Ok(Plan::new("ploi.pull_script", format!("copy the Ploi deploy script to {}", a.local)).effect("contacts ploi.io; future local deploys run the script")),
            |ctx, a| {
                let (server, site) = ploi::linked(ctx, &a.local)?;
                let script = ploi::deploy_script(ctx, server, site)?;
                deploy::set_script(ctx, &a.local, &script)?;
                Ok(Outcome::with_data(format!("deploy script of {} copied from Ploi", a.local), json!({ "script": script })))
            },
        )?;
    Ok(())
}
