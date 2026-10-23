//! More operations capabilities: the Laravel tab, database users and backups, deploy variables, scheduled
//! deployments and staging sites, health checks and machine monitoring, SSH keys.
//!
//! Same rules as the rest of the crate: reads are `read`; anything that runs a command or changes what a
//! deploy runs is `exec`; anything that can lose work (a merge into production, dropping a user) is `destructive`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

use crate::{dbadmin, deploy, env, health, laravel, ssh};

macro_rules! args {
    ($( $(#[$m:meta])* $name:ident { $( $(#[$fm:meta])* $f:ident : $t:ty ),* $(,)? } )*) => {
        $(
            $(#[$m])*
            #[derive(Debug, Clone, Deserialize, JsonSchema)]
            pub struct $name {
                $( $(#[$fm])* pub $f : $t, )*
            }
        )*
    };
}

args! {
    /// A site.
    Site { /** The site's name, as `site.list` shows it. */ site: String }
    /// Optional site.
    MaybeSite { /** Limit to one site. Omit for all. */ site: Option<String> }
    /// No arguments.
    Nothing {}
    /// An artisan command.
    ArtisanArgs {
        /** The site. */ site: String,
        /** The command and its arguments as you would type them after `php artisan`, e.g. `migrate:status`. Quotes group words; nothing else is special, and no shell is involved. */ command: String,
    }
    /// Maintenance mode.
    DownArgs {
        /** The site. */ site: String,
        /** A secret path that still reaches the site while it is down, e.g. `let-me-in`. */ secret: Option<String>,
        /** Seconds to put in the Retry-After header. */ retry: Option<u32>,
    }
    /// Caches to clear.
    ClearArgs {
        /** The site. */ site: String,
        /** What to clear: views, config, routes, events, cache, all. Default `all` (views, config, routes and events, not the application cache). */ what: Option<Vec<String>>,
    }
    /// A database user.
    UserCreateArgs {
        /** The user name: letters, digits, underscores. */ name: String,
        /** `localhost` (default), `127.0.0.1` or `%`. */ host: Option<String>,
        /** A password of 8 to 128 printable characters. Omit to have one generated and shown once. */ password: Option<String>,
        /** Give the user every privilege on this database. */ database: Option<String>,
    }
    /// A user to remove.
    UserDropArgs {
        /** The user name. */ name: String,
        /** Its host. Default `localhost`. */ host: Option<String>,
    }
    /// A backup schedule.
    BackupAddArgs {
        /** The database. */ database: String,
        /** Five-field cron expression, or @daily, @hourly. */ schedule: String,
        /** How many backups to keep. Default 7. */ keep: Option<usize>,
    }
    /// A schedule or backup by id.
    IdArg { /** The id from the matching list. */ id: String }
    /// Backup files of a database.
    BackupFilesArgs { /** The database. */ database: String }
    /// Deploy variables.
    VarsSetArgs {
        /** The site. */ site: String,
        /** Names (capital letters, digits, underscores) and values; a null value removes the variable. Available as `{NAME}` in the deploy script and as environment variables to its commands. */ vars: BTreeMap<String, Option<String>>,
    }
    /// A deploy at a time.
    ScheduleArgs {
        /** The site. */ site: String,
        /** When: an RFC 3339 time (`2026-10-24T09:00:00+07:00`) or `in 30m`, `in 2h`, `in 1d`. */ at: String,
        /** A note to remember why. */ note: Option<String>,
    }
    /// A staging site.
    StagingArgs {
        /** The production site to copy. */ site: String,
        /** The branch the staging site follows. Default `staging`; created from the current commit when it does not exist. */ branch: Option<String>,
    }
    /// A health check.
    HealthAddArgs {
        /** The site. */ site: String,
        /** A URL to ask instead of devd's own web server, e.g. your public address. */ url: Option<String>,
    }
    /// A key name.
    KeyName { /** The key's file name in ~/.ssh, e.g. `id_ed25519`. */ name: String }
    /// A new key.
    KeygenArgs {
        /** The file name for the new key. */ name: String,
        /** The comment inside the public key. Default `devd`. */ comment: Option<String>,
    }
    /// A key to take in.
    AdoptArgs {
        /** The private key file you already have. It is copied, not moved. */ path: String,
        /** The name to give it in ~/.ssh. */ name: String,
    }
}

fn ok_text(a: &str) -> String {
    a.to_string()
}

/// Register the capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    register_laravel(reg)?;
    register_db(reg)?;
    register_deploy(reg)?;
    register_health(reg)?;
    register_ssh(reg)?;
    crate::caps_remote::register(reg)?;
    Ok(())
}

// ------------------------------------------------------------------------------------- laravel

fn register_laravel(reg: &mut Registry) -> Result<()> {
    reg.define::<Site>("laravel.status", "Show a Laravel site's state")
        .description("Maintenance mode (and its bypass path), whether `public/storage` is linked, the environment and debug settings, and whether configuration and routes are cached. Reads files; runs no PHP.")
        .group("laravel")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("laravel.status", format!("read the Laravel state of {}", a.site))),
            |ctx, a| {
                let v = laravel::status(ctx, &a.site)?;
                Ok(Outcome::with_data(if v["maintenance"].as_bool().unwrap_or(false) { "in maintenance mode".to_string() } else { "up".to_string() }, v))
            },
        )?;

    reg.define::<ArtisanArgs>("laravel.artisan", "Run an artisan command")
        .description("Runs `php artisan <command>` with the site's own PHP in its folder, as an argument list (never through a shell), waits up to ten minutes, and returns the exit code and what it printed. It can change the app's data (`migrate`), so it needs a person's approval.")
        .group("laravel")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .handlers(
            |_, a| {
                let args = laravel::split_args(&a.command)?;
                if args.is_empty() {
                    return Err(DevdError::invalid("name an artisan command"));
                }
                Ok(Plan::new("laravel.artisan", format!("run `php artisan {}` for {}", args.join(" "), a.site)).effect("runs a command that can change the application's data"))
            },
            |ctx, a| {
                let args = laravel::split_args(&a.command)?;
                let r = laravel::artisan(ctx, &a.site, &args)?;
                Ok(Outcome::with_data(format!("artisan {} exited {} after {:.1}s", r.command, r.exit, r.seconds), json!(r)))
            },
        )?;

    reg.define::<DownArgs>("laravel.down", "Put a Laravel site in maintenance mode")
        .description("Runs `php artisan down`. Visitors see the maintenance page; with a `secret`, the path `/<secret>` still reaches the site.")
        .group("laravel")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("laravel.down", format!("put {} in maintenance mode", a.site)).effect("visitors see the maintenance page")),
            |ctx, a| {
                let r = laravel::down(ctx, &a.site, a.secret.as_deref(), a.retry)?;
                Ok(Outcome::with_data(format!("{} is in maintenance mode", a.site), json!({ "run": r, "status": laravel::status(ctx, &a.site)? })))
            },
        )?;

    reg.define::<Site>("laravel.up", "Bring a Laravel site out of maintenance mode")
        .description("Runs `php artisan up`.")
        .group("laravel")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("laravel.up", format!("bring {} out of maintenance mode", a.site))),
            |ctx, a| {
                let r = laravel::up(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("{} is up", a.site), json!({ "run": r, "status": laravel::status(ctx, &a.site)? })))
            },
        )?;

    reg.define::<Site>("laravel.storage_link", "Link public/storage to storage/app/public")
        .description("Runs `php artisan storage:link`, so uploaded files are served.")
        .group("laravel")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("laravel.storage_link", format!("link the storage of {}", a.site))),
            |ctx, a| {
                let r = laravel::storage_link(ctx, &a.site)?;
                Ok(Outcome::with_data(ok_text("storage linked"), json!({ "run": r })))
            },
        )?;

    reg.define::<ClearArgs>("laravel.clear", "Clear a Laravel site's caches")
        .description("Clears compiled views, the configuration cache, the route cache, the event cache, or the application cache, or the first four together (the default). The application cache is whatever the cache driver holds (the database, Redis), so it is only cleared when asked for by name.")
        .group("laravel")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("laravel.clear", format!("clear {} for {}", a.what.clone().map(|w| w.join(", ")).unwrap_or_else(|| "views, configuration, routes and events".into()), a.site))),
            |ctx, a| {
                let runs = laravel::clear(ctx, &a.site, a.what.as_deref().unwrap_or(&[]))?;
                Ok(Outcome::with_data(format!("{} cache command(s) ran", runs.len()), json!({ "runs": runs, "clearable": laravel::CLEARABLE.iter().map(|c| json!({ "what": c.0, "clears": c.2 })).collect::<Vec<_>>() })))
            },
        )?;
    Ok(())
}

// ------------------------------------------------------------------------- database users, backups

fn register_db(reg: &mut Registry) -> Result<()> {
    reg.define::<Nothing>("db.users", "List database users")
        .description("The users of devd's MariaDB other than the system's own (root and the internal accounts).")
        .group("db")
        .tier(Tier::Read)
        .effect(Effect::Data)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("db.users", "list the database users")),
            |ctx, _| {
                let u = dbadmin::users(ctx)?;
                Ok(Outcome::with_data(format!("{} user(s)", u.len()), json!({ "users": u })))
            },
        )?;

    reg.define::<UserCreateArgs>("db.user_create", "Create a database user")
        .description("Makes a MariaDB user, with every privilege on one database when you name it. The password is generated when you do not give one, and it is returned once and not stored by devd. Put it in the app's .env.")
        .group("db")
        .tier(Tier::Write)
        .effect(Effect::Data)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("db.user_create", format!("create the database user {}{}", a.name, a.database.as_ref().map(|d| format!(" with every privilege on {d}")).unwrap_or_default())).effect("adds an account to the database server")),
            |ctx, a| {
                let host = a.host.clone().unwrap_or_else(|| "localhost".to_string());
                let generated = a.password.is_none();
                let pw = dbadmin::create_user(ctx, &a.name, &host, a.password.as_deref(), a.database.as_deref())?;
                Ok(Outcome::with_data(
                    format!("user {}@{host} created", a.name),
                    json!({ "user": a.name, "host": host, "database": a.database, "password": if generated { json!(pw) } else { Value::Null }, "note": if generated { "This password is shown once; devd does not keep it." } else { "The password is the one you gave." } }),
                ))
            },
        )?;

    reg.define::<UserDropArgs>("db.user_drop", "Drop a database user")
        .description("Removes a MariaDB user and its privileges. The databases it could use are not touched.")
        .group("db")
        .tier(Tier::Destructive)
        .effect(Effect::Data)
        .handlers(
            |_, a| Ok(Plan::new("db.user_drop", format!("drop the database user {}", a.name)).effect("apps that connect as this user stop working")),
            |ctx, a| {
                let host = a.host.clone().unwrap_or_else(|| "localhost".to_string());
                dbadmin::drop_user(ctx, &a.name, &host)?;
                Ok(Outcome::with_data(format!("user {}@{host} dropped", a.name), json!({ "user": a.name, "host": host })))
            },
        )?;

    reg.define::<BackupAddArgs>("backup.add", "Back up a database on a schedule")
        .description("Dumps the database on a cron schedule while devd is open, into <devd home>/backups/<database>/, and keeps the newest `keep` (default 7). A backup due while devd was closed is skipped. A failed backup tells the alert targets. To restore one, give its file to `db.restore`.")
        .group("backup")
        .tier(Tier::Write)
        .effect(Effect::Data)
        .agent_safe()
        .handlers(
            |_, a| {
                crate::cron::parse(&a.schedule)?;
                Ok(Plan::new("backup.add", format!("back up {} on `{}`, keeping {}", a.database, a.schedule, a.keep.unwrap_or(7))).effect("writes dump files under the devd backups folder"))
            },
            |ctx, a| {
                let v = dbadmin::add(ctx, &a.database, &a.schedule, a.keep.unwrap_or(7))?;
                Ok(Outcome::with_data(format!("{} is backed up on {}", a.database, a.schedule), v))
            },
        )?;

    reg.define::<Nothing>("backup.list", "List database backup schedules")
        .description("Each schedule, when it is next due, and how the last backup went.")
        .group("backup")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("backup.list", "list the backup schedules")),
            |ctx, _| {
                let l = dbadmin::list(ctx)?;
                Ok(Outcome::with_data(format!("{} schedule(s)", l.len()), json!({ "schedules": l })))
            },
        )?;

    reg.define::<IdArg>("backup.run", "Take a scheduled backup now")
        .description("Runs the schedule's backup once, prunes to the number kept, and returns the file.")
        .group("backup")
        .tier(Tier::Write)
        .effect(Effect::Data)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("backup.run", format!("take backup {} now", a.id)).effect("writes a dump file")),
            |ctx, a| {
                let v = dbadmin::run_now(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("backup written to {}", v["file"].as_str().unwrap_or("")), v))
            },
        )?;

    reg.define::<IdArg>("backup.remove", "Stop a database backup schedule")
        .description("Stops the schedule. The backup files already taken stay.")
        .group("backup")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("backup.remove", format!("stop backup schedule {}", a.id))),
            |ctx, a| {
                dbadmin::remove(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("backup schedule {} removed", a.id), json!({ "id": a.id })))
            },
        )?;

    reg.define::<BackupFilesArgs>("backup.files", "List the backup files of a database")
        .description("Newest first, with their size and time. Give a file's path to `db.restore` to restore it.")
        .group("backup")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("backup.files", format!("list the backups of {}", a.database))),
            |ctx, a| {
                let f = dbadmin::files(ctx, &a.database)?;
                Ok(Outcome::with_data(format!("{} backup file(s)", f.len()), json!({ "files": f })))
            },
        )?;
    Ok(())
}

// ----------------------------------------------------------------------------------------- deploy

fn register_deploy(reg: &mut Registry) -> Result<()> {
    reg.define::<Site>("deploy.vars", "Show a site's deploy variables")
        .description("The names of the variables a deploy gets, with secret-looking values hidden. They are `{NAME}` in the script and environment variables to its commands.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.vars", format!("read the deploy variables of {}", a.site))),
            |ctx, a| {
                let v = deploy::vars(ctx, &a.site)?;
                let shown: BTreeMap<String, String> = v.iter().map(|(k, x)| (k.clone(), env::masked(k, x))).collect();
                Ok(Outcome::with_data(format!("{} variable(s)", shown.len()), json!({ "site": a.site, "vars": shown })))
            },
        )?;

    reg.define::<VarsSetArgs>("deploy.vars_set", "Set or remove a site's deploy variables")
        .description("Sets the named variables for every future deploy of the site, and removes the ones given as null. They change what the deploy's commands see, so this is an exec action.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("deploy.vars_set", format!("change {} deploy variable(s) of {}", a.vars.len(), a.site)).effect("future deploys run with these values")),
            |ctx, a| {
                let now = deploy::set_vars(ctx, &a.site, &a.vars)?;
                Ok(Outcome::with_data(format!("{} deploy variable(s) set on {}", now.len(), a.site), json!({ "names": now.keys().collect::<Vec<_>>() })))
            },
        )?;

    reg.define::<ScheduleArgs>("deploy.schedule", "Deploy a site at a set time")
        .description("Runs the site's deploy at the time you give, while devd is open. A deploy due while devd was closed is marked missed and the alert targets are told, never run at a time nobody chose.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .handlers(
            |_, a| {
                let at = deploy::parse_when(&a.at)?;
                Ok(Plan::new("deploy.schedule", format!("deploy {} at {}", a.site, at.format("%Y-%m-%d %H:%M"))).effect("runs the deploy script at that time"))
            },
            |ctx, a| {
                let v = deploy::schedule(ctx, &a.site, &a.at, a.note.as_deref())?;
                Ok(Outcome::with_data(format!("{} will deploy at {}", a.site, v["at"].as_str().unwrap_or("")), v))
            },
        )?;

    reg.define::<MaybeSite>("deploy.scheduled", "List scheduled deployments")
        .description("Pending, started, missed and cancelled scheduled deployments.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("deploy.scheduled", "list the scheduled deployments")),
            |ctx, a| {
                let l = deploy::scheduled(ctx, a.site.as_deref())?;
                Ok(Outcome::with_data(format!("{} scheduled deployment(s)", l.len()), json!({ "scheduled": l })))
            },
        )?;

    reg.define::<IdArg>("deploy.unschedule", "Cancel a scheduled deployment")
        .description("Cancels one that has not run.")
        .group("deploy")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("deploy.unschedule", format!("cancel scheduled deployment {}", a.id))),
            |ctx, a| {
                deploy::unschedule(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("scheduled deployment {} cancelled", a.id), json!({ "id": a.id })))
            },
        )?;

    reg.define::<StagingArgs>("deploy.staging_create", "Make a staging copy of a site")
        .description("A second working folder of the same Git repository on its own branch (default `staging`), served as `<site>-staging` with its own certificate, the production `.env` with APP_ENV and APP_URL changed, and a deploy script without the pull. Deploy it with `deploy.run`; ship it with `deploy.staging_promote`.")
        .group("deploy")
        .tier(Tier::Exec)
        .effect(Effect::Write)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("deploy.staging_create", format!("make {}-staging on branch {}", a.site, a.branch.clone().unwrap_or_else(|| "staging".into()))).step("git worktree add").step("create the site and its certificate").step("copy the .env with APP_ENV=staging").effect("adds a folder, a site and a branch")),
            |ctx, a| {
                let v = deploy::staging_create(ctx, &a.site, a.branch.as_deref())?;
                Ok(Outcome::with_data(format!("{}-staging is ready", a.site), v))
            },
        )?;

    reg.define::<Site>("deploy.staging_promote", "Ship the staging branch to production")
        .description("Merges the staging branch into the production site's branch (fast-forward only, so it refuses a diverged history) and deploys production. Refuses while production has uncommitted changes.")
        .group("deploy")
        .tier(Tier::Destructive)
        .effect(Effect::Process)
        .handlers(
            |_, a| Ok(Plan::new("deploy.staging_promote", format!("merge the staging branch into production for {} and deploy it", a.site)).step("git merge --ff-only").step("run the production deploy script").effect("production's working tree moves to the staging commit")),
            |ctx, a| {
                let d = deploy::staging_promote(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("promotion started: deployment {}", d.id), json!(d)))
            },
        )?;

    reg.define::<Site>("deploy.staging_remove", "Remove a staging site")
        .description("Removes the staging site and its working folder, including what is only there (the copied .env, installed dependencies, build output). Refused while the folder has uncommitted changes to tracked files. The branch stays in the repository.")
        .group("deploy")
        .tier(Tier::Destructive)
        .effect(Effect::Write)
        .handlers(
            |_, a| Ok(Plan::new("deploy.staging_remove", format!("remove the staging site of {}", a.site)).effect("removes a working folder; the branch stays")),
            |ctx, a| {
                let v = deploy::staging_remove(ctx, &a.site)?;
                Ok(Outcome::with_data("staging site removed".to_string(), v))
            },
        )?;

    reg.define::<Nothing>("deploy.staging", "List staging sites")
        .description("Each staging site, the site it copies, its branch and folder.")
        .group("deploy")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("deploy.staging", "list the staging sites")),
            |ctx, _| {
                let l = deploy::staging_list(ctx)?;
                Ok(Outcome::with_data(format!("{} staging site(s)", l.len()), json!({ "staging": l })))
            },
        )?;
    Ok(())
}

// ------------------------------------------------------------------------------------- health

fn register_health(reg: &mut Registry) -> Result<()> {
    reg.define::<HealthAddArgs>("health.add", "Watch a site's health")
        .description("Asks the site for its front page every minute while devd is open and records whether it answered below 500 and how long it took. With no URL it asks devd's own web server for the site (without following the redirect to HTTPS); give a URL to watch your public address instead. The alert targets are told when it goes down and when it is back.")
        .group("health")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("health.add", format!("watch the health of {}", a.site))),
            |ctx, a| {
                let v = health::add(ctx, &a.site, a.url.as_deref())?;
                Ok(Outcome::with_data(format!("{} is {}", a.site, v["state"].as_str().unwrap_or("unknown")), v))
            },
        )?;

    reg.define::<Nothing>("health.list", "List health checks")
        .description("Each watched site: up or down, the last status, how long it took, and since when.")
        .group("health")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("health.list", "list the health checks")),
            |ctx, _| {
                let l = health::list(ctx)?;
                Ok(Outcome::with_data(format!("{} check(s)", l.len()), json!({ "checks": l })))
            },
        )?;

    reg.define::<Site>("health.check", "Check a site's health now")
        .description("Asks once and records the answer, whether or not the site is on the watch list.")
        .group("health")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("health.check", format!("ask {} for its front page", a.site))),
            |ctx, a| {
                let v = health::check(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("{} is {}", a.site, v["state"].as_str().unwrap_or("unknown")), v))
            },
        )?;

    reg.define::<Site>("health.remove", "Stop watching a site's health")
        .description("Removes the check.")
        .group("health")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("health.remove", format!("stop watching {}", a.site))),
            |ctx, a| {
                health::remove(ctx, &a.site)?;
                Ok(Outcome::with_data(format!("{} is no longer watched", a.site), json!({ "site": a.site })))
            },
        )?;

    reg.define::<Nothing>("monitor.stats", "CPU, memory and disks of this machine")
        .description("The processor load, memory used and free, each disk's size and free space, uptime, and how many of devd's services are running.")
        .group("monitor")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("monitor.stats", "read the machine's CPU, memory and disks")),
            |ctx, _| {
                let v = health::monitor(ctx)?;
                Ok(Outcome::with_data("machine stats".to_string(), v))
            },
        )?;
    Ok(())
}

// ----------------------------------------------------------------------------------------- ssh

fn register_ssh(reg: &mut Registry) -> Result<()> {
    reg.define::<Nothing>("ssh.keys", "List SSH keys")
        .description("The keys in ~/.ssh that have a public half: name, type, comment, fingerprint, and whether the private half is there. Private keys are never shown.")
        .group("ssh")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("ssh.keys", "list the SSH keys")),
            |_, _| {
                let k = ssh::keys()?;
                Ok(Outcome::with_data(format!("{} key(s)", k.len()), json!({ "keys": k })))
            },
        )?;

    reg.define::<KeyName>("ssh.public", "Show an SSH public key")
        .description("The public half of a key, the part you paste into GitHub or Ploi.")
        .group("ssh")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("ssh.public", format!("read the public key {}", a.name))),
            |_, a| Ok(Outcome::with_data(format!("public key {}", a.name), json!({ "name": a.name, "public_key": ssh::public(&a.name)? }))),
        )?;

    reg.define::<KeygenArgs>("ssh.keygen", "Make an SSH key")
        .description("Makes an ed25519 key in ~/.ssh with no passphrase (add one with `ssh-keygen -p`), and returns the public half. Refuses to touch a key that exists.")
        .group("ssh")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("ssh.keygen", format!("make the SSH key {}", a.name)).effect("adds files to ~/.ssh")),
            |_, a| {
                let p = ssh::generate(&a.name, a.comment.as_deref())?;
                Ok(Outcome::with_data(format!("key {} made", a.name), json!({ "name": a.name, "public_key": p })))
            },
        )?;

    reg.define::<AdoptArgs>("ssh.adopt", "Take in a private key you already have")
        .description("Copies a private key file into ~/.ssh under a name, locks its permissions so ssh accepts it, and writes the public half when the key has no passphrase. The original is left where it is. A person only: it reads a private key.")
        .group("ssh")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .handlers(
            |_, a| Ok(Plan::new("ssh.adopt", format!("copy {} into ~/.ssh as {}", a.path, a.name)).effect("adds a private key to ~/.ssh")),
            |ctx, a| {
                ctx.require_human("taking in a private key")?;
                let v = ssh::adopt(std::path::Path::new(&a.path), &a.name)?;
                Ok(Outcome::with_data(format!("key {} is in ~/.ssh", a.name), v))
            },
        )?;
    Ok(())
}
