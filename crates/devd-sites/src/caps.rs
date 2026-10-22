//! Site and hosts-file capabilities.
//!
//! Two tiers matter here and they are different for a reason:
//!
//! * `site.create` is [`Tier::Write`]. It creates a directory inside the layout,
//!   issues a certificate from devd's own authority, records the site and
//!   re-renders the web server configuration. All of that is inside the devd
//!   root and reversible.
//! * `hosts.sync` is [`Tier::Admin`]. It edits a file shared with the rest of the
//!   machine, which changes how everything on the machine resolves names. That is
//!   a decision for a person, so it is a separate, explicitly approved operation
//!   rather than a hidden side effect of creating a site.
//!
//! `site.remove` is [`Tier::Destructive`] because with `delete_files` it can
//! delete a project, and an agent must never be able to approve that for itself.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

use crate as sites;

macro_rules! args {
    ($( $(#[$m:meta])* $name:ident { $( $f:ident : $t:ty ),* $(,)? } )*) => {
        $(
            $(#[$m])*
            #[derive(Debug, Clone, Deserialize, JsonSchema)]
            pub struct $name {
                $( pub $f : $t, )*
            }
        )*
    };
}

args! {
    /// No arguments.
    NoArgs {}
    /// A site to create.
    CreateArgs { name: String, host: Option<String>, root: Option<String>, tls: Option<bool>, db: Option<bool>, upstream: Option<String> }
    /// A site to remove.
    RemoveArgs { name: String, delete_files: Option<bool> }
    /// A site to point at a running program, or back at its directory.
    SetUpstreamArgs { name: String, upstream: Option<String> }
    /// A folder of projects.
    FolderArgs { path: String }
    /// A site by name.
    SiteName { name: String }
    /// Whether to ask Windows for permission when the hosts file cannot be written.
    HostsSyncArgs { elevate: Option<bool> }
    /// A site to share, and for how long.
    ShareArgs { name: String, minutes: Option<u64> }
    /// A site to pin to a PHP version.
    SetPhpArgs { name: String, version: Option<String> }
}

fn site_named(ctx: &Ctx, name: &str) -> Result<devd_core::state::Site> {
    ctx.store.list_sites()?.into_iter().find(|s| s.name == name).ok_or_else(|| DevdError::not_found(format!("no site named {name}")))
}

/// What in the project folder should not be public.
fn exposed(site: &devd_core::state::Site) -> Vec<String> {
    if site.upstream.is_some() {
        return Vec::new();
    }
    let mut out = devd_services::share::exposures(std::path::Path::new(&site.root));
    for f in devd_services::share::exposures(std::path::Path::new(&site.document_root())) {
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// Register every site capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<ShareArgs>("share.start", "Open a public link to a site")
    .description("Makes the site reachable from the internet through a Cloudflare quick tunnel at a random https address, for a limited time (default 2 hours, at most 24). Anyone with the address can open the site; it is closed when the time is up or with share.stop. The first use downloads cloudflared from Cloudflare's GitHub release and checks the SHA-256 GitHub records for it. Warns when the project folder holds a .env, .git or database dump. Human only: it publishes a site.")
    .group("share")
    .tier(Tier::Remote)
    .effect(Effect::Remote)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let site = site_named(ctx, &a.name)?;
            let minutes = a.minutes.unwrap_or(devd_services::share::DEFAULT_MINUTES);
            let mut plan = Plan::new("share.start", format!("make {} reachable from the internet for {minutes} minutes", a.name));
            if devd_services::share::installed(ctx).is_none() {
                let build = devd_services::share::latest_build()?;
                plan = plan
                    .step(format!("download cloudflared {} from {}", build.version, build.url))
                    .step(format!("verify it against SHA-256 {}", build.sha256));
            }
            plan = plan
                .step(format!("open a Cloudflare quick tunnel to {} on this computer", site.host))
                .step(format!("close it automatically after {minutes} minutes"))
                .effect("anyone with the address can open the site until then");
            for found in exposed(&site) {
                plan = plan.step(format!("WARNING: {found} is in the project folder and may be served"));
            }
            Ok(plan)
        },
        |ctx, a| {
            let site = site_named(ctx, &a.name)?;
            let minutes = a.minutes.unwrap_or(devd_services::share::DEFAULT_MINUTES);
            if devd_services::share::installed(ctx).is_none() {
                let build = devd_services::share::latest_build()?;
                devd_services::share::install(ctx, &build)?;
            }
            let share = devd_services::share::start(ctx, &site, minutes)?;
            let mut outcome = Outcome::with_data(format!("{} is public at {} until {}", a.name, share.url, share.expires_at), serde_json::to_value(&share).unwrap_or(Value::Null));
            for found in exposed(&site) {
                outcome = outcome.warn(format!("{found} is in the project folder; make sure the web server does not serve it"));
            }
            if !sites::resolves_without_hosts(&site.host) {
                outcome = outcome.warn("applications that build absolute links from their configured address (WordPress, for one) will send visitors to the local address; set the site address to the public one while the link is open");
            }
            Ok(outcome)
        },
    )?;

    reg.define::<SiteName>("share.stop", "Close a site's public link")
    .group("share")
    .tier(Tier::Write)
    .effect(Effect::Remote)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("share.stop", format!("close the public link to {}", a.name))),
        |ctx, a| {
            let was = devd_services::share::stop(ctx, &a.name)?;
            Ok(Outcome::with_data(if was { format!("{} is no longer public", a.name) } else { format!("{} had no open link", a.name) }, json!({ "site": a.name, "was_open": was })))
        },
    )?;

    reg.define::<NoArgs>("share.list", "List the open public links")
    .group("share")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("share.list", "list public links")),
        |ctx, _| {
            let open = devd_services::share::list(ctx)?;
            Ok(Outcome::with_data(format!("{} open link(s)", open.len()), json!({ "shares": open })))
        },
    )?;

    reg.define::<SiteName>("site.fix_htaccess", "Make a site's .htaccess readable by Apache")
    .description("Apache runs PHP as FastCGI, so `php_value`, `php_flag` and `php_admin_*` lines in a .htaccess answer every request with a 500. This wraps them in <IfModule mod_php.c> (the form hosts that do run mod_php read the same way), keeps the original as .htaccess.devd-backup, and carries the values into .user.ini, which PHP reads for every setting a .htaccess may change. php_admin_* values are system settings .user.ini cannot change; they are listed. Human only, because it edits a file in the project.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let site = ctx.store.list_sites()?.into_iter().find(|s| s.name == a.name).ok_or_else(|| DevdError::not_found(format!("no site named {}", a.name)))?;
            let found = std::fs::read_to_string(std::path::Path::new(&site.document_root()).join(".htaccess")).map(|t| sites::htaccess::scan(&t).len()).unwrap_or(0);
            Ok(Plan::new("site.fix_htaccess", format!("guard {found} php directive(s) in {}'s .htaccess", a.name))
                .step("keep the original as .htaccess.devd-backup")
                .step("wrap the directives in <IfModule mod_php.c>")
                .step("write the values Apache cannot read into .user.ini")
                .effect("edits .htaccess and .user.ini in the project"))
        },
        |ctx, a| {
            let site = ctx.store.list_sites()?.into_iter().find(|s| s.name == a.name).ok_or_else(|| DevdError::not_found(format!("no site named {}", a.name)))?;
            if site.upstream.is_some() {
                return Err(DevdError::invalid("a site that proxies a running program has no .htaccess"));
            }
            let done = sites::htaccess::fix(std::path::Path::new(&site.document_root()))?;
            let mut outcome = Outcome::with_data(
                format!("guarded {} directive(s); {} written to .user.ini", done.directives.len(), done.translated.len()),
                serde_json::to_value(&done).unwrap_or(Value::Null),
            );
            for name in &done.not_applied {
                outcome = outcome.warn(format!("{name} is a system setting .user.ini cannot change; set it in php.ini if it matters"));
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NoArgs>("site.parked", "List the folders whose projects become sites")
    .description("The projects root and every folder added with site.park. Each project directory in them is a site after site.sync, named after the folder, as auto virtual hosts do.")
    .group("site")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, _| Ok(Plan::new("site.parked", "list parked folders")),
        |ctx, _| {
            let rows: Vec<Value> = sites::park::folders(ctx)
                .into_iter()
                .map(|f| {
                    let projects = std::fs::read_dir(&f)
                        .map(|d| d.flatten().filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.')).count())
                        .unwrap_or(0);
                    json!({ "folder": f.display().to_string(), "projects": projects, "exists": f.is_dir() })
                })
                .collect();
            Ok(Outcome::with_data(format!("{} parked folder(s)", rows.len()), json!(rows)))
        },
    )?;

    reg.define::<FolderArgs>("site.park", "Park a folder: its projects become sites")
    .description("Adds a folder to the parked list in the settings file and makes a site of each project directory in it (site.sync). A site is <folder name>.<tld>, served from the folder, from its public/ directory when it has one. Nothing is written into the projects. Human only, because it makes everything in the folder reachable by name.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("site.park", format!("park {} and make a site of each project in it", a.path)).effect("rewrites settings.toml and the web server configuration")),
        |ctx, a| {
            let folder = sites::park::park(ctx, std::path::Path::new(&a.path))?;
            let report = sites::park::sync(ctx, std::slice::from_ref(&folder))?;
            let mut outcome = Outcome::with_data(
                format!("parked {}; {} site(s) created", folder.display(), report.created.len()),
                serde_json::to_value(&report).unwrap_or(Value::Null),
            );
            for w in &report.warnings {
                outcome = outcome.warn(w.clone());
            }
            Ok(outcome)
        },
    )?;

    reg.define::<FolderArgs>("site.unpark", "Stop parking a folder")
    .description("Removes a folder from the parked list. The sites it produced stay until site.remove deletes them.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("site.unpark", format!("stop parking {}", a.path)).effect("rewrites settings.toml")),
        |ctx, a| {
            let removed = sites::park::unpark(ctx, std::path::Path::new(&a.path))?;
            Ok(Outcome::ok(if removed { "the folder is no longer parked; its sites stay" } else { "that folder was not parked" }))
        },
    )?;

    reg.define::<NoArgs>("site.sync", "Make a site of every project in the parked folders")
    .description("For each project directory in the projects root and the parked folders that has no site, creates one (<folder>.<tld>, with a certificate) and re-renders the web server once. Sites whose folder has gone are reported, not removed. The names still need the hosts file: hosts.sync.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("site.sync", "create a site for each project directory that has none").effect("adds sites and rewrites the web server configuration")),
        |ctx, _| {
            let report = sites::park::sync(ctx, &sites::park::folders(ctx))?;
            let mut outcome = Outcome::with_data(
                format!("{} created, {} already had a site, {} skipped", report.created.len(), report.existing.len(), report.skipped.len()),
                serde_json::to_value(&report).unwrap_or(Value::Null),
            );
            for w in &report.warnings {
                outcome = outcome.warn(w.clone());
            }
            for s in &report.skipped {
                outcome = outcome.warn(format!("skipped {}: {}", s.folder, s.reason));
            }
            for g in &report.gone {
                outcome = outcome.warn(format!("{g} has no folder any more; site.remove removes the site"));
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NoArgs>("site.list", "List sites")
    .description("Every site with a live check of what is really on disk: whether its project directory \
         exists, whether its hostname is in the hosts file, whether a certificate has been \
         issued and whether the web server is running.")
    .group("site")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("site.list", "list the sites")),
        |ctx, _| {
            let rows = sites::list(ctx)?;
            let mut outcome = Outcome::with_data(
                format!("{} site(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            );
            for row in &rows {
                if !row.present {
                    outcome = outcome.warn(format!(
                        "{} has no project directory at {}",
                        row.site.name, row.site.root
                    ));
                }
                if row.htaccess_php > 0 {
                    outcome = outcome.warn(format!(
                        "{} has {} php_value-style directive(s) in .htaccess that Apache answers with a 500; `devd site fix-htaccess {}` guards them",
                        row.site.name, row.htaccess_php, row.site.name
                    ));
                }
                if !row.in_hosts && !crate::resolves_without_hosts(&row.site.host) {
                    outcome = outcome.warn(format!(
                        "{} does not resolve yet; `devd hosts sync` adds it",
                        row.site.host
                    ));
                }
            }
            Ok(outcome)
        },
    )?;

    reg.define::<CreateArgs>("site.create", "Create a site")
    .description("Creates the project directory with a starter page, issues a certificate for the \
         hostname from the local authority, records the site and re-renders the web server \
         configuration. With `upstream` (a port, or an address on this machine) the site serves \
         a program that is already running - a Node, Python or Rails dev server - instead of a \
         directory, and no directory or starter page is created. It does not edit the hosts \
         file: that is `hosts.sync`, an admin-tier operation, so creating a site does not need \
         an elevated approval.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, args| create_plan(ctx, args),
        |ctx, args| {
            let request = sites::CreateRequest {
                name: args.name.clone(),
                host: args.host.clone(),
                root: args.root.clone(),
                tls: args.tls.unwrap_or(true),
                db: args.db.unwrap_or(false),
                upstream: args.upstream.clone(),
            };
            let created = sites::create(ctx, &request)?;
            let mut outcome = Outcome::with_data(
                format!(
                    "created {} in {} ({})",
                    created.site.name, created.directory, created.url
                ),
                serde_json::to_value(&created).unwrap_or(Value::Null),
            );
            for warning in created.warnings {
                outcome = outcome.warn(warning);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<SetUpstreamArgs>("site.set_upstream", "Point a site at a running program")
    .description("Makes every request to the site's hostname go to a program that is already running \
         on this machine - `3000`, `localhost:5173`, `http://127.0.0.1:8000` - and re-renders \
         the web server configuration. Without `upstream` the site serves its project \
         directory again. Only addresses on this machine are accepted.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, args| {
            let site = ctx
                .store
                .get_site(&args.name)?
                .ok_or_else(|| DevdError::not_found(format!("no site named `{}`", args.name)))?;
            let target = match args.upstream.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                Some(address) => Some(sites::normalize_upstream(address)?),
                None => None,
            };
            let plan = Plan::new(
                "site.set_upstream",
                match &target {
                    Some(t) => format!("serve {} from {t}", site.host),
                    None => format!("serve {} from its project directory again", site.host),
                },
            )
            .step("record the change and re-render the web server configuration");
            Ok(match &target {
                Some(_) => plan.step("stop pinning a PHP version: nothing runs PHP for a proxy"),
                None => plan.step(format!("serve {}", site.root)),
            }
            .data(json!({ "site": site.name, "upstream": target })))
        },
        |ctx, args| {
            let result = sites::set_upstream(ctx, &args.name, args.upstream.as_deref())?;
            let mut outcome = Outcome::with_data(
                match result["site"]["upstream"].as_str() {
                    Some(u) => format!("{} now serves {u}", args.name),
                    None => format!("{} now serves its project directory", args.name),
                },
                result.clone(),
            );
            for warning in result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                outcome = outcome.warn(warning.to_string());
            }
            Ok(outcome)
        },
    )?;

    reg.define::<SetPhpArgs>("site.set_php", "Pin a site to a PHP version")
    .description("Pins a site to one of the installed PHP versions, so that it is served by that \
         version however the default changes, or unpins it so that it follows the default \
         again. A version that is not installed is refused, and the versions that are \
         installed are named. The web server configuration is re-rendered, and the \
         listener for the version is started if it is not running.")
    .group("site")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, args| {
            let row = sites::list(ctx)?
                .into_iter()
                .find(|r| r.site.name == args.name)
                .ok_or_else(|| DevdError::not_found(format!("no site named `{}`", args.name)))?;
            let installed = devd_services::installed_php_series(ctx)?;
            let mut plan = Plan::new(
                "site.set_php",
                match args.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    Some(v) => format!("serve {} with PHP {v}", args.name),
                    None => format!("serve {} with the default PHP version", args.name),
                },
            )
            .step("re-render the web server configuration");
            if let Some(version) = args.version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                let series = devd_services::php_series(version);
                if !installed.iter().any(|s| s == &series) {
                    plan = plan.effect(format!(
                        "PROBLEM PHP {series} is not installed; the installed versions are {}",
                        installed.join(", ")
                    ));
                } else {
                    plan = plan.step(format!("make sure the PHP {series} listener is running"));
                }
            }
            Ok(plan.data(json!({
                "site": row.site,
                "version": args.version,
                "installed": installed,
            })))
        },
        |ctx, args| {
            let installed = devd_services::installed_php_series(ctx)?;
            let version = args
                .version
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(devd_services::php_series);
            if let Some(series) = &version {
                if !installed.iter().any(|s| s == series) {
                    return Err(DevdError::invalid(format!(
                        "PHP {series} is not installed; the installed versions are {}",
                        installed.join(", ")
                    )));
                }
            }
            let result = sites::set_php(ctx, &args.name, version.as_deref())?;
            Ok(Outcome::with_data(
                match &version {
                    Some(v) => format!("{} now runs on PHP {v}", args.name),
                    None => format!("{} now follows the default PHP version", args.name),
                },
                result,
            ))
        },
    )?;

    reg.define::<RemoveArgs>("site.remove", "Remove a site")
    .description("Removes the certificate, the record and the web server configuration. The project \
         directory is kept unless `delete_files` is set, and a directory outside the layout's \
         www root is never deleted.")
    .group("site")
    .tier(Tier::Destructive)
    .effect(Effect::Write)
    .dry_run()
    .handlers(
        |ctx, args| {
            let row = sites::list(ctx)?
                .into_iter()
                .find(|r| r.site.name == args.name)
                .ok_or_else(|| DevdError::not_found(format!("no site named `{}`", args.name)))?;
            let delete = args.delete_files.unwrap_or(false);
            let mut plan = Plan::new(
                "site.remove",
                if delete {
                    format!("remove {} and delete {}", args.name, row.site.root)
                } else {
                    format!("remove {} (keeping {})", args.name, row.site.root)
                },
            )
            .step(format!("delete the certificate for {}", row.site.host))
            .step("remove the site record and re-render the web server configuration");
            if delete {
                plan = plan
                    .step(format!("delete the project directory {}", row.site.root))
                    .effect("deletes a project directory and everything in it");
            } else {
                plan = plan.step(format!("keep the project directory {}", row.site.root));
            }
            Ok(plan.destructive(delete).data(json!({
                "site": row.site,
                "delete_files": delete,
            })))
        },
        |ctx, args| {
            let result = sites::remove(ctx, &args.name, args.delete_files.unwrap_or(false))?;
            let message = format!(
                "removed {}",
                result
                    .get("site")
                    .and_then(Value::as_str)
                    .unwrap_or(&args.name)
            );
            let warnings: Vec<String> = result
                .get("warnings")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut outcome = Outcome::with_data(message, result);
            for warning in warnings {
                outcome = outcome.warn(warning);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NoArgs>("hosts.show", "Show the managed hosts block")
    .description("The hostnames devd manages in the hosts file, what is in the file now, and what a sync \
         would change. Reads the file; writes nothing.")
    .group("hosts")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, _| {
            let plan = sites::hosts::plan(ctx)?;
            Ok(Plan::new(
                "hosts.show",
                format!("read the devd block in {}", plan.file),
            ))
        },
        |ctx, _| {
            let plan = sites::hosts::plan(ctx)?;
            Ok(Outcome::with_data(
                if plan.changes {
                    format!(
                        "{} change(s) pending",
                        plan.adding.len() + plan.removing.len()
                    )
                } else {
                    "the hosts file already matches the sites".to_string()
                },
                serde_json::to_value(&plan).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<HostsSyncArgs>("hosts.sync", "Sync the hosts file")
    .description("Writes the managed block in the system hosts file so that every site hostname \
         resolves, snapshotting the file first. Everything outside the block is preserved \
         exactly, including the file's own line endings. On Windows the real hosts file needs \
         administrator rights: with elevate, devd asks Windows for them (the consent prompt you \
         know from installers) and runs the same sync in an elevated process; without it the \
         refusal is reported as the operating system gives it.")
    .group("hosts")
    .tier(Tier::Admin)
    .effect(Effect::System)
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, _| {
            let plan = sites::hosts::plan(ctx)?;
            let mut p = Plan::new(
                "hosts.sync",
                format!(
                    "write {} hostname(s) into {}",
                    plan.entries.len(),
                    plan.file
                ),
            );
            for entry in &plan.adding {
                p = p.step(format!("add {}", entry.line()));
            }
            for entry in &plan.removing {
                p = p.step(format!("remove {}", entry.line()));
            }
            if !plan.changes {
                p = p.step("nothing to change");
            }
            Ok(p.step("snapshot the current file first")
                .effect("changes how this machine resolves hostnames")
                .data(json!({
                    "file": plan.file,
                    "adding": plan.adding,
                    "removing": plan.removing,
                })))
        },
        |ctx, a| {
            let result = match sites::hosts::sync(ctx) {
                Err(DevdError::Denied(_)) if a.elevate.unwrap_or(false) && cfg!(windows) => sites::hosts::sync_elevated(ctx)?,
                other => other?,
            };
            let changed = result
                .get("changed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(Outcome::with_data(
                if changed {
                    "the hosts file was updated".to_string()
                } else {
                    "the hosts file already matched the sites".to_string()
                },
                result,
            ))
        },
    )?;
    Ok(())
}

fn create_plan(ctx: &Ctx, args: &CreateArgs) -> Result<Plan> {
    devd_core::util::validate_name(&args.name)?;
    let host = args
        .host
        .clone()
        .unwrap_or_else(|| format!("{}.{}", args.name, ctx.settings.tld));
    devd_core::util::validate_hostname(&host)?;
    let directory = args.root.clone().unwrap_or_else(|| {
        sites::default_directory(ctx, &args.name)
            .display()
            .to_string()
    });
    let tls = args.tls.unwrap_or(true);
    let upstream = match args.upstream.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(address) => Some(sites::normalize_upstream(address)?),
        None => None,
    };

    let mut plan = Plan::new(
        "site.create",
        match &upstream {
            Some(u) => format!("create the site {} served at {host}, forwarding to {u}", args.name),
            None => format!("create the site {} served at {host}", args.name),
        },
    );
    plan = match &upstream {
        Some(u) => plan.step(format!(
            "forward every request for {host} to {u}; no directory or starter page is created"
        )),
        None => plan
            .step(format!("create the project directory {directory}"))
            .step(format!(
                "write a starter index.php in {directory} if it is empty"
            )),
    };
    if tls {
        plan = plan.step(format!(
            "issue a certificate for {host} from the local authority into <devd>/certs"
        ));
    } else {
        plan = plan.step("serve over plain HTTP; no certificate is issued");
    }
    plan = plan.step("record the site and re-render the web server configuration");
    if args.db.unwrap_or(false) {
        plan = plan.step(format!(
            "create the database `{}` if MariaDB is running",
            args.name
        ));
    }
    plan = plan.step("`devd hosts sync` is a separate, admin-tier step");

    Ok(plan
        .effect("adds a project directory and a certificate under the devd root")
        .data(json!({
            "name": args.name,
            "host": host,
            "directory": directory,
            "tls": tls,
            "db": args.db.unwrap_or(false),
            "upstream": upstream,
        })))
}

