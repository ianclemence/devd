//! PHP settings, the worker pool and the extension manager, as capabilities.
//!
//! What Ploi shows under a server's PHP tab - upload size, memory, execution time, the extension
//! list, the PHP-FPM workers - is here, so a person in the window and an agent over MCP change
//! the same things through the same approval and audit path.

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};
use devd_runtime::phpconf;

use crate as services;

/// Settings of the installed PHP versions, optionally one series.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SeriesArgs {
    /// A PHP series such as `8.4`. Omit for every installed series.
    pub series: Option<String>,
}

/// What to change.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConfigureArgs {
    /// `local` (generous limits) or `production` (a typical Ploi server: 128M memory, 5M uploads, 30 s).
    pub profile: Option<String>,
    /// How many PHP processes serve each version, 1 to 16.
    pub workers: Option<u8>,
    /// Individual values, applied after the profile: `memory_limit`, `upload_max_filesize`,
    /// `post_max_size`, `max_execution_time`, `max_input_time`, `date.timezone`, `display_errors`, `opcache.enable`.
    pub values: Option<BTreeMap<String, String>>,
    /// Limit the ini changes to one series. The profile and the worker count are for all of them.
    pub series: Option<String>,
}

/// Start, stop or restart the PHP processes of a version.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PoolArgs {
    /// A PHP series such as `8.4`. Omit for every installed version.
    pub series: Option<String>,
    /// `start`, `stop` or `restart`.
    pub action: String,
}

/// A site's own limits profile.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SiteProfileArgs {
    /// The site.
    pub site: String,
    /// `local` or `production`. Omit to let the site follow the machine's profile again.
    pub profile: Option<String>,
}

/// One extension to switch.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExtensionArgs {
    /// The extension, e.g. `gd`.
    pub name: String,
    /// On or off.
    pub enabled: bool,
    /// A PHP series such as `8.4`. Omit for the primary one.
    pub series: Option<String>,
}

fn site_exists(ctx: &Ctx, site: &str) -> Result<()> {
    ctx.store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == site)
        .map(|_| ())
        .ok_or_else(|| DevdError::not_found(format!("no site named `{site}`")))
}

/// An installed PHP: series, newest patch version and where it lives.
struct Install {
    series: String,
    version: String,
    dir: PathBuf,
}

fn installs(ctx: &Ctx, only: Option<&str>) -> Result<Vec<Install>> {
    let want = only.map(services::php_series);
    let mut best: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
    for r in ctx.store.list_runtimes()? {
        if r.kind != "php" {
            continue;
        }
        let series = services::php_series(&r.version);
        if want.as_ref().is_some_and(|w| *w != series) {
            continue;
        }
        let newer = best.get(&series).is_none_or(|(v, _)| r.version.as_str() > v.as_str());
        if newer {
            best.insert(series, (r.version.clone(), PathBuf::from(&r.path)));
        }
    }
    if let (Some(w), true) = (&want, best.is_empty()) {
        return Err(DevdError::not_found(format!("PHP {w} is not installed; `devd runtime install php@{w}` adds it")));
    }
    Ok(best.into_iter().map(|(series, (version, dir))| Install { series, version, dir }).collect())
}

fn settings_now(ctx: &Ctx) -> devd_core::config::Settings {
    devd_core::config::Settings::load(&ctx.paths.settings_file()).unwrap_or_else(|_| (*ctx.settings).clone())
}

fn describe(ctx: &Ctx, only: Option<&str>) -> Result<Value> {
    let s = settings_now(ctx);
    let mut rows = Vec::new();
    for i in installs(ctx, only)? {
        let ini = i.dir.join("php.ini");
        let mut values = phpconf::effective_values(&ini, &["memory_limit", "upload_max_filesize", "post_max_size", "max_execution_time", "max_input_time", "date.timezone", "display_errors", "opcache.enable"]).unwrap_or_default();
        values.extend(phpconf::managed_values(&ini).unwrap_or_default());
        rows.push(json!({
            "series": i.series,
            "version": i.version,
            "values": values,
            "ini": i.dir.join("php.ini").display().to_string(),
        }));
    }
    let overrides: Vec<Value> = ctx
        .store
        .ops_list("site_profile", None)?
        .into_iter()
        .map(|i| json!({ "site": i.site, "profile": i.data["profile"], "own_pool": i.data["profile"].as_str() == Some(services::alt_profile(ctx)) }))
        .collect();
    Ok(json!({ "profile": s.php_profile, "workers": s.php_workers, "primary": s.primary_php, "versions": rows, "site_profiles": overrides }))
}

/// The service names of the PHP processes of a version (all versions when none is named).
fn pool_names(ctx: &Ctx, series: Option<&str>) -> Result<Vec<String>> {
    let want = series.map(services::php_series);
    let names: Vec<String> = ctx
        .store
        .list_services()?
        .into_iter()
        .filter(|s| services::php_pool_of(&s.name).is_some_and(|(v, _)| want.as_ref().is_none_or(|w| *w == v)))
        .map(|s| s.name)
        .collect();
    if names.is_empty() {
        return Err(DevdError::not_found(match series {
            Some(v) => format!("there are no PHP processes for {v}; `devd runtime install php@{v}` adds them"),
            None => "there are no PHP processes; `devd runtime install php` adds them".to_string(),
        }));
    }
    Ok(names)
}

/// Bring the listener rows, the web configuration and the running processes in line with the settings.
fn apply(ctx: &Ctx) -> Result<Vec<String>> {
    services::ensure_php_services(ctx)?;
    services::prune_php_services(ctx)?;
    services::web::render(ctx)?;
    let web_running = ctx
        .store
        .get_service(services::web::service_name(ctx))?
        .is_some_and(|s| services::running_pid(ctx, &s).is_some());
    let mut restarted = Vec::new();
    for s in ctx.store.list_services()? {
        if !s.name.starts_with(&format!("{}-", services::PHP_SERVICE)) {
            continue;
        }
        let running = services::running_pid(ctx, &s).is_some();
        if running || web_running {
            services::restart(ctx, &s.name)?;
            restarted.push(s.name);
        }
    }
    if web_running {
        services::web::reload(ctx)?;
    }
    Ok(restarted)
}

/// Register the PHP settings capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<SeriesArgs>("php.settings", "Show the PHP limits, profile and worker count")
        .description("For each installed PHP version: the values devd manages in php.ini (memory_limit, \
             upload_max_filesize, post_max_size, max_execution_time and the rest), plus the active profile \
             and how many PHP worker processes serve each version.")
        .group("php")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("php.settings", "read the PHP settings")),
            |ctx, args| {
                let data = describe(ctx, args.series.as_deref())?;
                Ok(Outcome::with_data("PHP settings".to_string(), data))
            },
        )?;

    reg.define::<ConfigureArgs>("php.configure", "Change the PHP profile, limits or worker count")
        .description("Applies a limits profile (`local` is generous; `production` matches a typical Ploi \
             server: 128M memory, 5M uploads, 30 s) and/or individual values to every installed PHP's \
             php.ini, and/or sets how many PHP worker processes serve each version. The managed values live in \
             one marked block at the end of php.ini, so they win over the rest and are replaced, not stacked. \
             Running PHP processes are restarted so the change shows at once.")
        .group("php")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, args| {
                if args.profile.is_none() && args.workers.is_none() && args.values.as_ref().is_none_or(BTreeMap::is_empty) {
                    return Err(DevdError::invalid("name a profile, a worker count or at least one value"));
                }
                let mut plan = Plan::new("php.configure", "change the PHP configuration");
                for i in installs(ctx, args.series.as_deref())? {
                    if let Some(p) = &args.profile {
                        plan = plan.step(format!("apply the `{p}` limits to PHP {} ({})", i.series, i.dir.join("php.ini").display()));
                    }
                    if let Some(v) = &args.values {
                        plan = plan.step(format!("set {} in PHP {}", v.keys().cloned().collect::<Vec<_>>().join(", "), i.series));
                    }
                }
                if let Some(n) = args.workers {
                    plan = plan.step(format!("run {n} PHP worker(s) per version and re-render the web configuration"));
                }
                Ok(plan.step("restart the PHP processes that are running").effect("restarts PHP; requests in flight are dropped"))
            },
            |ctx, args| {
                if let Some(p) = &args.profile {
                    phpconf::profile_values(p)?;
                }
                if args.workers.is_some_and(|n| n == 0 || n > 16) {
                    return Err(DevdError::invalid("workers must be between 1 and 16"));
                }
                let mut s = settings_now(ctx);
                if let Some(p) = &args.profile {
                    s.php_profile = p.clone();
                }
                if let Some(n) = args.workers {
                    s.php_workers = n;
                }
                s.validate()?;
                let targets = installs(ctx, args.series.as_deref())?;
                for i in &targets {
                    let ini = i.dir.join("php.ini");
                    if let Some(p) = &args.profile {
                        phpconf::apply_profile(&ini, p)?;
                    }
                    if let Some(v) = args.values.as_ref().filter(|v| !v.is_empty()) {
                        phpconf::set_values(&ini, v)?;
                    }
                }
                s.save(&ctx.paths.settings_file())?;
                let restarted = apply(ctx)?;
                let data = describe(ctx, args.series.as_deref())?;
                Ok(Outcome::with_data(
                    format!("PHP configured; {} process(es) restarted", restarted.len()),
                    json!({ "settings": data, "restarted": restarted }),
                ))
            },
        )?;

    reg.define::<PoolArgs>("php.pool", "Start, stop or restart the PHP processes of a version")
        .description("Controls every PHP process of a version at once: the workers, and the second pool that runs the other limits profile when \
             a site needs it. Sites served by those processes answer with an error while they are stopped.")
        .group("php")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, a| {
                let names = pool_names(ctx, a.series.as_deref())?;
                if !["start", "stop", "restart"].contains(&a.action.as_str()) {
                    return Err(DevdError::invalid("the action is start, stop or restart"));
                }
                Ok(Plan::new("php.pool", format!("{} {} PHP process(es)", a.action, names.len())).step(names.join(", ")).effect("sites served by these processes are unavailable while they restart"))
            },
            |ctx, a| {
                let names = pool_names(ctx, a.series.as_deref())?;
                for n in &names {
                    match a.action.as_str() {
                        "start" => {
                            services::start(ctx, n)?;
                        }
                        "stop" => {
                            services::stop(ctx, n)?;
                        }
                        "restart" => {
                            services::restart(ctx, n)?;
                        }
                        other => return Err(DevdError::invalid(format!("`{other}` is not start, stop or restart"))),
                    }
                }
                Ok(Outcome::with_data(format!("{} PHP process(es): {}", names.len(), a.action), json!({ "services": names, "action": a.action })))
            },
        )?;

    reg.define::<SiteProfileArgs>("php.site_profile", "Give one site its own PHP limits profile")
        .description("Serves the site with the `local` or `production` limits whatever the machine's profile is, so you can try a site against \
             the limits of your server while everything else stays generous. A site whose profile differs from the machine's is served by a \
             second pool of PHP processes started with the other profile's ini (the same workers, 50 ports higher). Omit the profile to \
             follow the machine's again. Restarts PHP.")
        .group("php")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, a| {
                site_exists(ctx, &a.site)?;
                if let Some(p) = &a.profile {
                    phpconf::profile_values(p)?;
                }
                Ok(Plan::new(
                    "php.site_profile",
                    match &a.profile {
                        Some(p) => format!("serve {} with the `{p}` PHP limits", a.site),
                        None => format!("serve {} with the machine's PHP limits again", a.site),
                    },
                )
                .step("start a second pool of PHP processes with the other profile when it is needed")
                .step("re-render the web configuration and restart the PHP processes that are running")
                .effect("restarts PHP; requests in flight are dropped"))
            },
            |ctx, a| {
                site_exists(ctx, &a.site)?;
                match &a.profile {
                    Some(p) => {
                        phpconf::profile_values(p)?;
                        ctx.store.ops_put("site_profile", &a.site, &a.site, &json!({ "profile": p }))?;
                    }
                    None => {
                        ctx.store.ops_delete("site_profile", &a.site)?;
                    }
                }
                let restarted = apply(ctx)?;
                let own_pool = services::alt_sites(ctx).contains(&a.site);
                Ok(Outcome::with_data(
                    match &a.profile {
                        Some(p) => format!("{} now runs with the `{p}` limits", a.site),
                        None => format!("{} follows the machine's limits", a.site),
                    },
                    json!({ "site": a.site, "profile": a.profile, "own_pool": own_pool, "restarted": restarted }),
                ))
            },
        )?;

    reg.define::<SeriesArgs>("php.extensions", "List a PHP version's extensions and which are on")
        .description("Every extension the PHP build ships or its php.ini names, and whether php.ini loads it.")
        .group("php")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("php.extensions", "list the PHP extensions")),
            |ctx, args| {
                let series = match args.series.clone() {
                    Some(s) => Some(s),
                    None => settings_now(ctx).primary_php,
                };
                let found = installs(ctx, series.as_deref())?;
                let i = found.first().ok_or_else(|| DevdError::not_found("no PHP is installed"))?;
                let list = phpconf::list_extensions(&i.dir)?;
                Ok(Outcome::with_data(
                    format!("{} extension(s) in PHP {}", list.len(), i.version),
                    json!({ "series": i.series, "version": i.version, "extensions": list }),
                ))
            },
        )?;

    reg.define::<ExtensionArgs>("php.extension_set", "Switch a PHP extension on or off")
        .description("Enables or disables one extension the build ships, by editing its line in php.ini, \
             then restarts the PHP processes that are running. An extension the build does not ship is refused \
             and the ones it does ship are named.")
        .group("php")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, args| {
                let series = args.series.clone().or_else(|| settings_now(ctx).primary_php);
                let found = installs(ctx, series.as_deref())?;
                let i = found.first().ok_or_else(|| DevdError::not_found("no PHP is installed"))?;
                Ok(Plan::new(
                    "php.extension_set",
                    format!("{} the {} extension in PHP {}", if args.enabled { "enable" } else { "disable" }, args.name, i.series),
                )
                .step(format!("edit {}", i.dir.join("php.ini").display()))
                .step("restart the PHP processes that are running")
                .effect("restarts PHP; requests in flight are dropped"))
            },
            |ctx, args| {
                let series = args.series.clone().or_else(|| settings_now(ctx).primary_php);
                let found = installs(ctx, series.as_deref())?;
                let i = found.first().ok_or_else(|| DevdError::not_found("no PHP is installed"))?;
                phpconf::set_extension(&i.dir, &args.name, args.enabled)?;
                let restarted = apply(ctx)?;
                Ok(Outcome::with_data(
                    format!("{} is now {} in PHP {}", args.name, if args.enabled { "on" } else { "off" }, i.series),
                    json!({ "series": i.series, "name": args.name, "enabled": args.enabled, "restarted": restarted }),
                ))
            },
        )?;
    Ok(())
}
