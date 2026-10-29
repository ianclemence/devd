//! Service, web server, stack, TLS and database capabilities.
//!
//! Declared once here so that `devd service start nginx`, the same button in the
//! application and an agent asking over MCP all pass through one implementation
//! with one set of permission rules.
//!
//! Tiers follow what the operation can do to the machine:
//!
//! * Listing, reading status, reading logs, checking a configuration and listing
//!   databases are [`Tier::Read`].
//! * Installing a service, rendering configuration, creating a database and
//!   issuing a certificate are [`Tier::Write`]. An install plan names the URL,
//!   the digest and the destination before anything is downloaded.
//! * Starting, stopping, restarting and reloading are [`Tier::Exec`]: they start
//!   processes and are reversible with one more command.
//! * Removing a service and dropping a database are [`Tier::Destructive`]. They
//!   always need approval, and an agent can never approve them for itself.
//! * Trusting the certificate authority is [`Tier::Admin`]: it changes what every
//!   program on the machine treats as valid, so a person has to decide.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::download::Progress;
use devd_core::{DevdError, Registry, Result};

use crate as services;

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
    /// A service by name.
    NameArgs { name: String }
    /// A service and whether it belongs to the startup set.
    StartupArgs { name: String, enabled: bool }
    /// A service and how many log lines to read.
    LogsArgs { name: String, lines: Option<u32> }
    /// A service to install.
    InstallArgs { name: String, version: Option<String>, port: Option<u16>, force: Option<bool> }
    /// Which web server to use.
    UseWebArgs { server: String }
    /// The ports sites are served on; an omitted one keeps its current value.
    PortsArgs { http: Option<u16>, https: Option<u16> }
    /// An existing program folder to use.
    AdoptArgs { name: String, path: String, port: Option<u16> }
    /// A certificate to issue.
    HostArgs { host: String }
    /// Whether to trust the CA for every user or just this one.
    TrustArgs { machine: Option<bool> }
    /// The databases of one server.
    DbListArgs { engine: Option<String> }
    /// A database to create.
    DbCreateArgs { name: String, engine: Option<String> }
    /// A database to drop.
    DbDropArgs { name: String, engine: Option<String>, no_snapshot: Option<bool> }
    /// A snapshot to put back.
    SnapshotRef { id: String }
    /// How many snapshots to list.
    SnapshotQuery { limit: Option<usize> }
    /// A database to write to a file.
    DbDumpArgs { database: String, path: Option<String>, from_port: Option<u16>, from_user: Option<String>, from_password: Option<String>, engine: Option<String> }
    /// A file to read into a database.
    DbRestoreArgs { database: String, path: String, replace: Option<bool>, engine: Option<String> }
    /// A database to copy from another server on this machine.
    DbCopyArgs { database: String, from_port: u16, from_user: Option<String>, from_password: Option<String>, as_name: Option<String>, replace: Option<bool>, engine: Option<String> }
}

/// Register every service capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    register_service(reg)?;
    register_web(reg)?;
    register_stack(reg)?;
    register_tls(reg)?;
    register_db(reg)?;
    crate::phpcaps::register(reg)?;
    crate::redirects::register(reg)?;
    crate::siteconf::register(reg)?;
    Ok(())
}

// ------------------------------------------------------------------ services

fn register_service(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("service.list", "List services")
    .description("Every service devd manages, with a live check that its files exist, its recorded \
         process is still the process it claims to be, and its port is answering.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("service.list", "list the installed services")),
        |ctx, _| {
            let rows = services::list(ctx)?;
            let mut outcome = Outcome::with_data(
                format!("{} service(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            );
            for row in &rows {
                if row.desired_state == "running" && !row.running {
                    outcome = outcome.warn(format!(
                        "{} should be running but no process was found",
                        row.name
                    ));
                }
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NameArgs>("service.available", "List service versions published upstream")
    .description("Reads the vendor's own listing (nginx.org, archive.mariadb.org) and returns the \
         versions that can be installed.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "service.available",
                "read the upstream service catalogue",
            ))
        },
        |_, args| {
            let (list, notes) = services::available(&args.name)?;
            let mut outcome = Outcome::with_data(
                format!("{} build(s) available for {}", list.len(), args.name),
                serde_json::to_value(&list).unwrap_or(Value::Null),
            );
            for note in notes {
                outcome = outcome.warn(note);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<InstallArgs>("service.install", "Install a service")
    .description("Resolves the request against the vendor's listing, downloads the official build, \
         verifies its digest where the vendor publishes one, unpacks it and records how to \
         start it. The plan names the URL, the digest and the destination before anything is \
         downloaded. A vendor that publishes no checksum file is reported as such.")
    .group("service")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, args| install_plan(args),
        |ctx, args| {
            let version = args.version.clone().unwrap_or_else(|| "latest".to_string());
            let progress = |p: &Progress| {
                ctx.events.emit_with(
                    "service.download",
                    match p.total {
                        Some(total) => format!("downloading {} of {} bytes", p.received, total),
                        None => format!("downloading {} bytes", p.received),
                    },
                    json!({ "received": p.received, "total": p.total, "fraction": p.fraction() }),
                );
                true
            };
            let done = services::install(
                ctx,
                &args.name,
                &version,
                args.port,
                args.force.unwrap_or(false),
                Some(&progress),
            )?;
            let mut outcome = Outcome::with_data(
                format!(
                    "installed {} {} at {}",
                    done.service.name, done.version, done.path
                ),
                json!({
                    "service": done.service,
                    "version": done.version,
                    "path": done.path,
                    "sha256": done.digest,
                    "bytes": done.bytes,
                    "from_cache": done.from_cache,
                }),
            );
            for warning in done.warnings {
                outcome = outcome.warn(warning);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NoArgs>("autostart.status", "Whether the stack starts when you sign in")
    .description("Reports the sign-in entry that starts the resident `devd daemon`, if there is one.")
    .group("autostart")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, _| Ok(Plan::new("autostart.status", "read the sign-in entry")),
        |_, _| {
            let entry = services::autostart::status()?;
            Ok(Outcome::with_data(
                if entry.is_some() { "the stack starts when you sign in" } else { "the stack does not start when you sign in" },
                json!({ "enabled": entry.is_some(), "command": entry }),
            ))
        },
    )?;

    reg.define::<NoArgs>("autostart.enable", "Start the stack when you sign in")
    .description("Adds a sign-in entry (a Run value in your registry hive on Windows, an autostart entry on Linux, \
         a LaunchAgent on macOS) that runs `devd stack up`: the enabled services start, detached, and devd exits. \
         No administrator rights. Human only, because it changes what runs at every sign-in.")
    .group("autostart")
    .tier(Tier::Admin)
    .effect(Effect::System)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("autostart.enable", "start the devd daemon when you sign in").effect("adds a sign-in entry for this user")),
        |_, _| {
            let command = services::autostart::enable(&devd_core::util::cli_exe()?)?;
            Ok(Outcome::with_data("the stack will start when you sign in", json!({ "command": command })))
        },
    )?;

    reg.define::<NoArgs>("autostart.disable", "Stop starting the stack when you sign in")
    .description("Removes the sign-in entry. Services that are already running keep running.")
    .group("autostart")
    .tier(Tier::Admin)
    .effect(Effect::System)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("autostart.disable", "remove the sign-in entry").effect("removes a sign-in entry for this user")),
        |_, _| {
            let was = services::autostart::disable()?;
            Ok(Outcome::ok(if was { "the sign-in entry is removed" } else { "there was no sign-in entry" }))
        },
    )?;

    reg.define::<NameArgs>("service.find_local", "Find a service's program already on this machine")
    .description("Looks for an Apache, Redis, PostgreSQL, MongoDB or Memcached folder from a portable stack, XAMPP, WampServer or a plain install on \
         every drive and reports each with its version. Nothing is changed; `service.adopt` uses one.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("service.find_local", format!("look for {} folders on this machine", a.name))),
        |_, a| {
            let folders = match a.name.as_str() {
                "apache" => services::find_local_apache(),
                "redis" => services::find_local_redis(),
                other => match services::engines::get(other) {
                    Some(engine) => services::engines::find_local(engine),
                    None => return Err(DevdError::unsupported("only Apache, Redis, PostgreSQL, MongoDB and Memcached can be found on disk")),
                },
            };
            let found: Vec<Value> = folders
                .into_iter()
                .map(|(path, version)| json!({ "path": path.display().to_string(), "version": version }))
                .collect();
            Ok(Outcome::with_data(format!("{} {} folder(s) found", found.len(), a.name), json!(found)))
        },
    )?;

    reg.define::<AdoptArgs>("service.adopt", "Use a program that is already on disk")
    .description("Records an existing Apache, Redis, PostgreSQL, MongoDB or Memcached folder (a portable stack's, XAMPP's, a plain install) as devd's. \
         The folder is not copied or modified: devd writes its own httpd.conf and starts the program with \
         it. Human only, because the program in that folder is what devd will then run.")
    .group("service")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .human_only()
    .dry_run()
    .handlers(
        |_, a| {
            Ok(Plan::new("service.adopt", format!("use {} as devd's {}", a.path, a.name))
                .step("check the folder holds a working Apache and read its version")
                .step("record it; nothing in it is changed")
                .step("render devd's own configuration for it")
                .effect("adds a service record and a generated configuration file"))
        },
        |ctx, a| {
            let done = services::adopt(ctx, &a.name, std::path::Path::new(&a.path), a.port)?;
            let mut outcome = Outcome::with_data(
                format!("{} {} at {} is now devd's {}", done.service.name, done.version, done.path, a.name),
                json!({ "service": done.service, "version": done.version, "path": done.path }),
            );
            for warning in done.warnings {
                outcome = outcome.warn(warning);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<NameArgs>("service.start", "Start a service")
    .description("Starts the service detached and records its process id, so it survives the command \
         that started it. The start is verified: the port has to open, or the service is \
         stopped again and the failure is reported with the tail of its own log.")
    .group("service")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        start_plan,
        |ctx, args| {
            let status = services::start(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<NameArgs>("service.stop", "Stop a service")
    .description("Stops the service and everything it spawned. The recorded process id is only used \
         after its image name has been confirmed, so a recycled process id cannot lead to an \
         unrelated program being killed.")
    .group("service")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        stop_plan,
        |ctx, args| {
            let status = services::stop(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<NameArgs>("service.restart", "Restart a service")
    .description("Stops the service and starts it again.")
    .group("service")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        stop_plan,
        |ctx, args| {
            let status = services::restart(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<NameArgs>("service.status", "Show one service")
    .description("The current state of a service: whether it is installed, running, which process id \
         it has, and whether its port is answering.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("service.status", "read the service state")),
        |ctx, args| {
            let status = services::status(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<StartupArgs>("service.set_startup", "Choose whether a service starts with the stack")
    .description("Adds an installed service to, or removes it from, the set that `devd stack up` and the sign-in entry start. It changes the next startup, not whether the service is running now. The web server is chosen with web.use, and PHP listeners follow the installed runtimes. Human only: this decides what runs every time the stack starts.")
    .group("service")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, a| {
            ctx.store.get_service(&a.name)?.ok_or_else(|| DevdError::not_found(format!("`{}` is not a devd service", a.name)))?;
            Ok(Plan::new(
                "service.set_startup",
                format!("{} {} in the startup set", if a.enabled { "add" } else { "remove" }, a.name),
            )
            .effect("changes which services the next stack up starts"))
        },
        |ctx, a| {
            let status = services::set_startup(ctx, &a.name, a.enabled)?;
            Ok(Outcome::with_data(
                format!("{} is {} in the startup set", a.name, if a.enabled { "now" } else { "no longer" }),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<LogsArgs>("service.logs", "Read a service log")
    .description("The last lines of a service's own log file - nginx's error output, the PHP \
         FastCGI listener's startup messages, MariaDB's error log.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("service.logs", "read the service log")),
        |ctx, args| {
            let lines = args.lines.unwrap_or(50).min(2000) as usize;
            let tail = services::logs(ctx, &args.name, lines)?;
            Ok(Outcome::with_data(
                format!("{} line(s) from {}", tail.len(), args.name),
                json!({ "name": args.name, "lines": tail }),
            ))
        },
    )?;

    reg.define::<NameArgs>("service.remove", "Remove a service")
    .description("Stops the service, deletes the installed files and forgets the record. For MariaDB \
         the data directory is deliberately kept: uninstalling the server is not the same \
         decision as discarding every database.")
    .group("service")
    .tier(Tier::Destructive)
    .effect(Effect::Write)
    .dry_run()
    .handlers(
        |ctx, args| {
            let status = services::status(ctx, &args.name)?;
            Ok(Plan::new(
                "service.remove",
                format!("remove the {} installation", args.name),
            )
            .step(format!("stop {} if it is running", args.name))
            .step(format!("delete {}", status.install_dir))
            .step(format!(
                "forget the `{}` service row{}",
                args.name,
                if args.name == "mariadb" {
                    "; the data directory is kept"
                } else {
                    ""
                }
            ))
            .effect("deletes files under the devd root")
            .destructive(true)
            .data(json!({ "name": args.name, "install_dir": status.install_dir })))
        },
        |ctx, args| {
            let result = services::remove(ctx, &args.name)?;
            Ok(Outcome::with_data(format!("removed {}", args.name), result))
        },
    )?;
    Ok(())
}

fn install_plan(args: &InstallArgs) -> Result<Plan> {
    let def = services::definition(&args.name).ok_or_else(|| {
        DevdError::unsupported(format!(
            "`{}` is not a service devd can install; installable services are nginx, apache, mariadb, redis, memcached and mailpit",
            args.name
        ))
    })?;
    let (list, _) = services::available(def.kind)?;
    let spec = args.version.clone().unwrap_or_else(|| "latest".to_string());
    // `install` will not use an nginx build whose digest devd has not recorded.
    // Resolve against the same set here, or the plan names a build the install
    // then refuses.
    let installable = services::verifiable(def.name, &list);
    let chosen = devd_runtime::catalogue::resolve(&installable, &spec, None)
        .or_else(|_| devd_runtime::catalogue::resolve(&list, &spec, None))?;
    let dest = def.name.to_string();
    let digest = match &chosen.sha256 {
        Some(d) => format!("{d} (published by the vendor)"),
        None if def.name == "nginx" => {
            "REFUSED: nginx publishes no checksum and devd has no digest recorded for this build".to_string()
        }
        None if def.name == "mariadb" => "published in the release's sha256sums.txt".to_string(),
        None => "NOT PUBLISHED by the vendor; only the HTTPS transport is checked".to_string(),
    };
    Ok(Plan::new(
        "service.install",
        format!(
            "download and install {} {} from {}",
            def.name, chosen.version, chosen.url
        ),
    )
    .step(format!("download {}", chosen.url))
    .step(format!("verify sha256: {digest}"))
    .step(format!(
        "unpack into <devd>/services/{dest}/{}",
        if args.force.unwrap_or(false) {
            " (replacing the existing installation)"
        } else {
            ""
        }
    ))
    .step(if def.name == "mariadb" {
        format!(
            "write my.ini and create the system tables on port {}",
            args.port.unwrap_or(def.default_port)
        )
    } else {
        format!("render the {} configuration from the state store", def.name)
    })
    .effect(format!("adds files under <devd>/services/{dest}"))
    .approval(false)
    .data(json!({
        "name": def.name,
        "version": chosen.version,
        "url": chosen.url,
        "sha256": chosen.sha256,
        "destination": format!("<devd>/services/{dest}"),
        "bytes": chosen.bytes,
    })))
}

fn start_plan(ctx: &Ctx, args: &NameArgs) -> Result<Plan> {
    let argv = services::planned_argv(ctx, &args.name)?;
    Ok(Plan::new(
        "service.start",
        format!("start {} as {}", args.name, argv.join(" ")),
    )
    .step(format!("run {}", argv.join(" ")))
    .step("wait until the port answers")
    .effect("starts a process")
    .data(json!({ "name": args.name, "argv": argv })))
}

fn stop_plan(ctx: &Ctx, args: &NameArgs) -> Result<Plan> {
    let status = services::status(ctx, &args.name)?;
    Ok(Plan::new(
        "service.stop",
        match status.pid {
            Some(pid) => format!("stop {} (pid {pid})", args.name),
            None => format!("stop {} (it is not running)", args.name),
        },
    )
    .step(format!("signal process {}", status.pid.unwrap_or(0)))
    .step("wait for the port to close")
    .effect("stops a process")
    .data(json!({ "name": args.name, "pid": status.pid })))
}

// --------------------------------------------------------------- web server

/// The ports `web.ports` asks for, falling back to the ones in settings.toml.
///
/// Read from disk, not `ctx.settings`, so a second change in the same window
/// builds on the first rather than on the values the window started with.
fn ports_for(ctx: &Ctx, a: &PortsArgs) -> Result<(u16, u16)> {
    let settings = devd_core::config::Settings::load(&ctx.paths.settings_file())
        .unwrap_or_else(|_| (*ctx.settings).clone());
    let http = a.http.unwrap_or(settings.http_port);
    let https = a.https.unwrap_or(settings.https_port);
    if http == 0 || https == 0 {
        return Err(DevdError::invalid("ports must be non-zero"));
    }
    if http == https {
        return Err(DevdError::invalid("the HTTP and HTTPS ports must differ"));
    }
    Ok((http, https))
}

fn register_web(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("web.render", "Render the web server configuration")
    .description("Rebuilds the web server's configuration (nginx.conf or httpd.conf, whichever the web_server \
         setting names) from the state store: one server per site, with the TLS listener, the redirect, \
         the document root and the FastCGI upstream. The file is generated, never edited, so it cannot \
         drift from what devd believes.")
    .group("web")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "web.render",
                "render the web server configuration from the sites in the state store",
            )
            .step("read the sites from the state store")
            .effect("rewrites the generated configuration file"))
        },
        |ctx, _| {
            let path = services::web::render(ctx)?;
            let sites = ctx.store.list_sites()?.len();
            Ok(Outcome::with_data(
                format!("rendered {sites} site(s) into {}", path.display()),
                json!({ "path": path.display().to_string(), "sites": sites }),
            ))
        },
    )?;

    reg.define::<NoArgs>("web.test", "Check the web server configuration")
    .description("Asks the web server (`nginx -t` or `httpd -t`) to check the generated configuration and reports its own message.")
    .group("web")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("web.test", "check the generated configuration with the web server")),
        |ctx, _| {
            let output = services::web::test(ctx)?;
            Ok(Outcome::with_data(
                "the configuration is valid",
                json!({ "server": services::web::service_name(ctx), "output": output }),
            ))
        },
    )?;

    reg.define::<PortsArgs>("web.ports", "Serve sites on different HTTP and HTTPS ports")
    .description("Sets the HTTP and HTTPS ports in settings.toml, rewrites the generated configuration and \
         restarts the web server on them when it is running. A port another program is already listening on \
         is refused. Other devd windows pick the new setting up when they restart; the server itself \
         switches immediately. Human only.")
    .group("web")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, a| {
            let (http, https) = ports_for(ctx, a)?;
            Ok(Plan::new(
                "web.ports",
                format!("serve sites on HTTP {http} and HTTPS {https}"),
            )
            .step("write the ports to settings.toml")
            .step(format!("rewrite {} for them", services::web::service_name(ctx)))
            .step("restart the web server if it is running")
            .effect("rewrites settings.toml and restarts a process")
            .data(json!({ "http": http, "https": https })))
        },
        |ctx, a| {
            let (http, https) = ports_for(ctx, a)?;
            let file = ctx.paths.settings_file();
            let before = devd_core::config::Settings::load(&file)?;
            for (port, old) in [(http, before.http_port), (https, before.https_port)] {
                if port != old && services::local_tcp_open(port) {
                    return Err(DevdError::invalid(format!(
                        "port {port} is already in use; stop what is on it or choose another port"
                    )));
                }
            }
            services::set_web_ports(ctx, http, https)?;
            let path = services::web::render(ctx)?;
            let name = services::web::service_name(ctx);
            let running = services::status(ctx, name)?.running;
            if running {
                services::restart(ctx, name)?;
            }
            let settings = devd_core::config::Settings::load(&file)?;
            Ok(Outcome::with_data(
                format!(
                    "sites are served on http://*.{}:{http} and https://*.{}:{https}{}",
                    settings.tld,
                    settings.tld,
                    if running {
                        ""
                    } else {
                        " (the web server is not running; `devd stack up` starts it)"
                    }
                ),
                json!({
                    "http": http,
                    "https": https,
                    "tld": settings.tld,
                    "restarted": running,
                    "config": path.display().to_string(),
                }),
            ))
        },
    )?;

    reg.define::<UseWebArgs>("web.use", "Choose the web server: nginx or apache")
    .description("Sets which web server fronts the sites. Apache reads .htaccess files, so a site written for shared hosting or XAMPP works unchanged; nginx is lighter. The chosen server joins the startup set and the other leaves it. The setting applies the next time devd starts: stop the running server, restart devd, then bring the stack up. The server has to be installed first. Human only.")
    .group("web")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .human_only()
    .dry_run()
    .idempotent()
    .handlers(
        |_, a| Ok(Plan::new("web.use", format!("serve sites with {}", a.server)).effect("rewrites settings.toml")),
        |ctx, a| {
            let chosen = match a.server.as_str() {
                "nginx" => devd_core::config::WebServer::Nginx,
                "apache" => devd_core::config::WebServer::Apache,
                other => return Err(DevdError::invalid(format!("web server must be nginx or apache, not {other}"))),
            };
            if ctx.store.get_service(&a.server)?.is_none() {
                return Err(DevdError::not_found(format!(
                    "{} is not installed; run `devd service install {}`{}",
                    a.server,
                    a.server,
                    if a.server == "apache" { ", or `devd service adopt apache <folder>` to use one you already have" } else { "" }
                )));
            }
            let file = ctx.paths.settings_file();
            let mut settings = devd_core::config::Settings::load(&file)?;
            settings.web_server = chosen;
            settings.validate()?;
            settings.save(&file)?;
            // Exactly one web server belongs to the stack. Choosing one also
            // makes that choice true for the next `stack up`; otherwise the
            // setting and the startup set would disagree.
            for server in ["nginx", "apache"] {
                if let Some(mut row) = ctx.store.get_service(server)? {
                    let want = server == a.server;
                    if row.enabled != want {
                        row.enabled = want;
                        ctx.store.upsert_service(&row)?;
                    }
                }
            }
            Ok(Outcome::with_data(
                format!("web_server is now {}; it is in the startup set and the other web server is not. Restart devd, stop the running server and run `devd stack up` to apply it", a.server),
                json!({ "web_server": a.server, "settings": file.display().to_string() }),
            ))
        },
    )?;

    reg.define::<NoArgs>("web.reload", "Reload the web server")
    .description("Checks the generated configuration and then has the web server read it: nginx without \
         dropping connections, Apache by a restart. A configuration that does not pass the check is never loaded.")
    .group("web")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "web.reload",
                "reload the web server with the generated configuration",
            )
            .step("check the configuration")
            .step("signal nginx to reload, or restart Apache")
            .effect("reloads a running process"))
        },
        |ctx, _| {
            let output = services::web::reload(ctx)?;
            Ok(Outcome::with_data(
                format!("{} reloaded", services::web::service_name(ctx)),
                json!({ "server": services::web::service_name(ctx), "output": output }),
            ))
        },
    )?;
    Ok(())
}

// -------------------------------------------------------------------- stack

fn register_stack(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("stack.up", "Start the development stack")
    .description("Starts every enabled service in dependency order - the PHP FastCGI listener, then \
         MariaDB, then nginx. Every service is attempted, so one failure does not hide the \
         state of the others, and the result reports exactly what started.")
    .group("stack")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, _| stack_plan(ctx, true),
        |ctx, _| {
            let started = services::stack_up(ctx)?;
            Ok(Outcome::with_data(
                format!("{} service(s) running", started.len()),
                serde_json::to_value(&started).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<NoArgs>("stack.down", "Stop the development stack")
    .description("Stops every running service, in reverse dependency order.")
    .group("stack")
    .tier(Tier::Exec)
    .effect(Effect::Process)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, _| stack_plan(ctx, false),
        |ctx, _| {
            let stopped = services::stack_down(ctx)?;
            Ok(Outcome::with_data(
                format!("{} service(s) stopped", stopped.len()),
                serde_json::to_value(&stopped).unwrap_or(Value::Null),
            ))
        },
    )?;
    Ok(())
}

fn stack_plan(ctx: &Ctx, up: bool) -> Result<Plan> {
    let services = ctx.store.list_services()?;
    let enabled: Vec<&str> = services
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.as_str())
        .collect();
    let (id, verb) = if up {
        ("stack.up", "start")
    } else {
        ("stack.down", "stop")
    };
    Ok(Plan::new(
        id,
        format!(
            "{verb} {} service(s): {}",
            enabled.len(),
            enabled.join(", ")
        ),
    )
    .step(format!("{verb} {}", enabled.join(", ")))
    .effect("changes which processes are running")
    .data(json!({ "services": enabled })))
}

// ---------------------------------------------------------------------- TLS

fn register_tls(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("tls.ca.ensure", "Create the local certificate authority")
    .description("Creates the per-installation certificate authority that signs every site \
         certificate. Idempotent: an existing authority, and its key, are left exactly as \
         they are, because replacing the key would invalidate every certificate already \
         issued from it.")
    .group("tls")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "tls.ca.ensure",
                "create the local certificate authority if it does not exist",
            )
            .step("generate an ECDSA P-256 key pair")
            .step("write the CA certificate and key under <devd>/certs")
            .effect("adds files under the devd root"))
        },
        |ctx, _| {
            let created = services::tls::ensure_ca(ctx)?;
            let path = services::tls::ca_cert_path(ctx);
            Ok(Outcome::with_data(
                if created {
                    format!("created the certificate authority at {}", path.display())
                } else {
                    format!(
                        "the certificate authority at {} already exists",
                        path.display()
                    )
                },
                json!({
                    "created": created,
                    "certificate": path.display().to_string(),
                    "key": services::tls::ca_key_path(ctx).display().to_string(),
                }),
            ))
        },
    )?;

    reg.define::<NoArgs>("tls.ca.status", "Report whether the authority is trusted")
    .description("Checks the operating system's certificate stores for the authority devd created. \
         A user-scope trust needs no administrator; a machine-scope one does. Nothing is \
         changed by checking.")
    .group("tls")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "tls.ca.status",
                "check the trust stores for the devd authority",
            ))
        },
        |ctx, _| {
            let status = services::tls::trust_status(ctx)?;
            let message = match (status.user, status.machine) {
                (_, true) => "the devd certificate authority is trusted machine-wide",
                (true, false) => "the devd certificate authority is trusted for this user",
                (false, false) if !services::tls::ca_exists(ctx) => {
                    "no certificate authority has been created yet"
                }
                (false, false) => "the devd certificate authority is not trusted",
            };
            Ok(Outcome::with_data(
                message,
                json!({
                    "exists": services::tls::ca_exists(ctx),
                    "user": status.user,
                    "machine": status.machine,
                    "certificate": services::tls::ca_cert_path(ctx).display().to_string(),
                }),
            ))
        },
    )?;

    reg.define::<NoArgs>("tls.ca.show", "Show the local certificate authority")
    .description("The public certificate of the authority that signs site certificates. This is the \
         file to import into a browser, a device or a corporate proxy to make development \
         certificates trusted without disabling verification anywhere. The private key is \
         never returned.")
    .group("tls")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "tls.ca.show",
                "read the certificate authority certificate",
            ))
        },
        |ctx, _| {
            let pem = services::tls::ca_pem_text(ctx)?;
            Ok(Outcome::with_data(
                format!(
                    "certificate authority at {}",
                    services::tls::ca_cert_path(ctx).display()
                ),
                json!({
                    "path": services::tls::ca_cert_path(ctx).display().to_string(),
                    "pem": pem,
                }),
            ))
        },
    )?;

    reg.define::<TrustArgs>("tls.trust", "Trust the local certificate authority")
    .description("Adds the authority to the operating system's trust store, so that every site devd \
         serves is accepted by a browser and by any tool that uses the system store, with \
         certificate verification left on. This changes what every program on the machine \
         trusts, which is why it needs a person to approve it.")
    .group("tls")
    .tier(Tier::Admin)
    .effect(Effect::System)
    .dry_run()
    .idempotent()
    .handlers(
        |_, args| {
            let store = if args.machine.unwrap_or(false) {
                "the machine-wide root store (an elevated shell is required)"
            } else {
                "this user's root store"
            };
            Ok(Plan::new(
                "tls.trust",
                format!("add the devd certificate authority to {store}"),
            )
            .step("run `certutil -addstore` with the CA certificate")
            .effect("changes the operating system trust store")
            .data(json!({ "machine": args.machine.unwrap_or(false) })))
        },
        |ctx, args| services::tls::trust(ctx, args.machine.unwrap_or(false)),
    )?;

    reg.define::<TrustArgs>("tls.untrust", "Stop trusting the local certificate authority")
    .description("Removes the authority from the operating system's trust store.")
    .group("tls")
    .tier(Tier::Admin)
    .effect(Effect::System)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |_, args| {
            Ok(Plan::new(
                "tls.untrust",
                "remove the devd certificate authority from the trust store",
            )
            .step("run `certutil -delstore` with the CA certificate")
            .effect("changes the operating system trust store")
            .data(json!({ "machine": args.machine.unwrap_or(false) })))
        },
        |ctx, args| services::tls::untrust(ctx, args.machine.unwrap_or(false)),
    )?;

    reg.define::<HostArgs>("tls.issue", "Issue a site certificate")
    .description("Issues a leaf certificate for a hostname, signed by the local authority, with the \
         hostname in subjectAltName and the full chain written where the web server reads \
         it. Re-issuing replaces the certificate it replaces.")
    .group("tls")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, args| {
            Ok(Plan::new(
                "tls.issue",
                format!("issue a certificate for {}", args.host),
            )
            .step(format!(
                "sign a leaf for {} from the local authority",
                args.host
            ))
            .step(format!("write <devd>/certs/{}.crt and .key", args.host))
            .effect("adds files under <devd>/certs")
            .data(
                json!({ "host": args.host, "certs_dir": ctx.paths.certs().display().to_string() }),
            ))
        },
        |ctx, args| {
            let issued = services::tls::issue(ctx, &args.host)?;
            Ok(Outcome::with_data(
                format!("issued a certificate for {}", issued.host),
                serde_json::to_value(&issued).unwrap_or(Value::Null),
            ))
        },
    )?;

    reg.define::<NoArgs>("tls.list", "List issued certificates")
    .description("Every certificate under the layout's certs directory, with the host it is for and \
         whether the key beside it is present.")
    .group("tls")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("tls.list", "list the issued certificates")),
        |ctx, _| {
            let rows = services::tls::list(ctx)?;
            Ok(Outcome::with_data(
                format!("{} certificate(s)", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            ))
        },
    )?;
    Ok(())
}

// ----------------------------------------------------------------- database

/// The database server a `db.*` call is about.
enum Db {
    /// devd's MariaDB.
    Maria { port: u16 },
    /// devd's PostgreSQL, run from this folder.
    Postgres { root: std::path::PathBuf, port: u16 },
    /// devd's MySQL (the Oracle server), run from this folder.
    Mysql { root: std::path::PathBuf, port: u16 },
}

/// Pick the server: MariaDB unless the call says `engine: postgres` or `engine: mysql`.
///
/// `mysql` means the Oracle server when devd has one installed, and otherwise
/// MariaDB, which speaks the same protocol: a call written for "a MySQL" keeps
/// working on a machine that only has MariaDB.
fn db_target(ctx: &Ctx, engine: &Option<String>) -> Result<Db> {
    match engine.as_deref() {
        Some("mysql") if ctx.store.get_service("mysql")?.is_some() => {
            let service = ctx.store.get_service("mysql")?.expect("checked");
            let root = services::manifest_root(ctx, "mysql").ok_or_else(|| DevdError::not_found("MySQL has no recorded folder; install it again"))?;
            Ok(Db::Mysql { root, port: service.port.unwrap_or(3307) })
        }
        None | Some("mariadb") | Some("mysql") => Ok(Db::Maria { port: db_port(ctx)? }),
        Some("postgres") | Some("postgresql") => {
            let service = ctx.store.get_service("postgres")?.ok_or_else(|| DevdError::not_found("PostgreSQL is not set up; `devd service adopt postgres <folder>` uses one you already have"))?;
            let root = services::manifest_root(ctx, "postgres").ok_or_else(|| DevdError::not_found("PostgreSQL has no recorded folder; adopt it again"))?;
            Ok(Db::Postgres { root, port: service.port.unwrap_or(5432) })
        }
        Some(other) => Err(DevdError::invalid(format!("engine must be mariadb, mysql or postgres, not {other}"))),
    }
}

/// Dump a database into the snapshot area before it is dropped.
fn snapshot_database(ctx: &Ctx, target: &Db, name: &str) -> Result<devd_core::snapshot::Snapshot> {
    let (id, payload) = devd_core::snapshot::allocate(ctx)?;
    let (engine, port) = match target {
        Db::Maria { port } => {
            services::dump::dump(ctx, &services::dump::Endpoint::devd(*port), name, &payload)?;
            ("mariadb", *port)
        }
        Db::Postgres { root, port } => {
            services::engines::pg_dump(root, *port, "postgres", None, name, &payload)?;
            ("postgres", *port)
        }
        Db::Mysql { root, port } => {
            services::engines::my_dump(root, *port, name, &payload)?;
            ("mysql", *port)
        }
    };
    devd_core::snapshot::record_payload(
        ctx,
        devd_core::snapshot::Kind::Database,
        &format!("{engine}:{name}"),
        "db.drop",
        &id,
        json!({ "engine": engine, "database": name, "port": port }),
    )
}

fn register_db(reg: &mut Registry) -> Result<()> {
    reg.define::<DbListArgs>("db.list", "List databases")
    .description("The databases on devd's MariaDB (or, with engine postgres, PostgreSQL), excluding the system databases.")
    .group("db")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, a| {
            db_target(ctx, &a.engine)?;
            Ok(Plan::new("db.list", "list the databases"))
        },
        |ctx, a| match db_target(ctx, &a.engine)? {
            Db::Maria { port } => {
                let names = services::mariadb::databases(ctx, port)?;
                Ok(Outcome::with_data(format!("{} database(s)", names.len()), json!({ "databases": names, "port": port })))
            }
            Db::Postgres { root, port } => {
                let names = services::engines::pg_databases(ctx, &root, port)?;
                Ok(Outcome::with_data(format!("{} database(s)", names.len()), json!({ "databases": names, "port": port })))
            }
            Db::Mysql { root, port } => {
                let names = services::engines::my_databases(&root, port)?;
                Ok(Outcome::with_data(format!("{} database(s)", names.len()), json!({ "databases": names, "port": port })))
            }
        },
    )?;

    reg.define::<DbCreateArgs>("db.create", "Create a database")
    .description("Creates a database (utf8mb4 on MariaDB, UTF-8 on PostgreSQL) if it does not already exist. The name is validated as a plain identifier, so it cannot carry a second statement.")
    .group("db")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, a| {
            match db_target(ctx, &a.engine)? {
                Db::Maria { .. } => devd_core::util::validate_name(&a.name)?,
                Db::Postgres { .. } => services::engines::valid_pg_name(&a.name)?,
                Db::Mysql { .. } => services::engines::valid_my_name(&a.name)?,
            }
            Ok(Plan::new("db.create", format!("create the database `{}`", a.name)).effect("creates stored data"))
        },
        |ctx, a| match db_target(ctx, &a.engine)? {
            Db::Maria { port } => {
                let result = services::mariadb::create_database(ctx, port, &a.name)?;
                Ok(Outcome::with_data(format!("database `{}` exists", a.name), result))
            }
            Db::Postgres { root, port } => {
                services::engines::pg_create(ctx, &root, port, &a.name)?;
                Ok(Outcome::with_data(format!("database `{}` exists", a.name), json!({ "database": a.name, "port": port })))
            }
            Db::Mysql { root, port } => {
                services::engines::my_create(&root, port, &a.name)?;
                Ok(Outcome::with_data(format!("database `{}` exists", a.name), json!({ "database": a.name, "port": port })))
            }
        },
    )?;

    reg.define::<DbDumpArgs>("db.dump", "Write a database to a SQL file")
    .description("Dumps one database to a .sql file. By default it is devd's MariaDB (or PostgreSQL with engine postgres), written under <devd home>/backups; from_port names another server of the same kind on this machine instead, such as a MySQL you already run. Only a person can choose where the file goes. Servers on other machines are refused.")
    .group("db")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .agent_safe()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("db.dump", format!("write the database `{}` to a SQL file", a.database)).effect("writes a file")),
        |ctx, a| {
            if a.path.is_some() && ctx.actor.is_agent() {
                return Err(DevdError::denied("an agent cannot choose where a dump is written; it goes under the devd backups folder"));
            }
            let file = match &a.path {
                Some(p) => std::path::PathBuf::from(p),
                None => services::dump::default_file(ctx, &a.database),
            };
            match db_target(ctx, &a.engine)? {
                Db::Maria { port } => {
                    let from = match a.from_port {
                        Some(p) => services::dump::Endpoint::local(None, p, a.from_user.as_deref(), a.from_password.clone())?,
                        None => services::dump::Endpoint::devd(port),
                    };
                    let report = services::dump::dump(ctx, &from, &a.database, &file)?;
                    Ok(Outcome::with_data(format!("wrote {} to {}", a.database, file.display()), report))
                }
                Db::Postgres { root, port } => {
                    let (port, user) = match a.from_port {
                        Some(p) => (services::dump::Endpoint::local(None, p, a.from_user.as_deref(), None)?.port, a.from_user.clone().unwrap_or_else(|| "postgres".into())),
                        None => (port, "postgres".to_string()),
                    };
                    services::engines::pg_dump(&root, port, &user, a.from_password.as_deref(), &a.database, &file)?;
                    Ok(Outcome::with_data(format!("wrote {} to {}", a.database, file.display()), json!({ "database": a.database, "file": file.display().to_string() })))
                }
                Db::Mysql { root, port } => {
                    let port = a.from_port.unwrap_or(port);
                    services::engines::my_dump(&root, port, &a.database, &file)?;
                    Ok(Outcome::with_data(format!("wrote {} to {}", a.database, file.display()), json!({ "database": a.database, "file": file.display().to_string() })))
                }
            }
        },
    )?;

    reg.define::<DbRestoreArgs>("db.restore", "Read a SQL file into a database")
    .description("Runs a SQL file against a database on devd's MariaDB (or PostgreSQL with engine postgres), creating the database when it is not there. A database that already has tables is refused unless replace is true, because a dump can drop and recreate them. The file is run as it is: restore only files you trust. Always needs approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("db.restore", format!("run {} against the database `{}`", a.path, a.database)).effect("changes stored data")),
        |ctx, a| match db_target(ctx, &a.engine)? {
            Db::Maria { port } => {
                if !a.replace.unwrap_or(false) && services::mariadb::table_count(ctx, port, &a.database)? > 0 {
                    return Err(DevdError::conflict(format!("the database `{}` already has tables; pass replace to run the file over them", a.database)));
                }
                let report = services::dump::restore(ctx, &services::dump::Endpoint::devd(port), &a.database, std::path::Path::new(&a.path))?;
                Ok(Outcome::with_data(format!("restored {}", a.database), report))
            }
            Db::Postgres { root, port } => {
                if !a.replace.unwrap_or(false) && services::engines::pg_table_count(&root, port, &a.database)? > 0 {
                    return Err(DevdError::conflict(format!("the database `{}` already has tables; pass replace to run the file over them", a.database)));
                }
                services::engines::pg_restore(ctx, &root, port, &a.database, std::path::Path::new(&a.path))?;
                Ok(Outcome::with_data(format!("restored {}", a.database), json!({ "database": a.database, "file": a.path })))
            }
            Db::Mysql { root, port } => {
                if !a.replace.unwrap_or(false) && services::engines::my_table_count(&root, port, &a.database)? > 0 {
                    return Err(DevdError::conflict(format!("the database `{}` already has tables; pass replace to run the file over them", a.database)));
                }
                services::engines::my_restore(&root, port, &a.database, std::path::Path::new(&a.path))?;
                Ok(Outcome::with_data(format!("restored {}", a.database), json!({ "database": a.database, "file": a.path })))
            }
        },
    )?;

    reg.define::<DbCopyArgs>("db.copy", "Copy a database from another server on this machine into devd")
    .description("Moves a database off another server on 127.0.0.1 (a MySQL for MariaDB, a PostgreSQL for engine postgres) into devd's: dumps it to <devd home>/backups, then restores it under the same name or as_name. MySQL 8 collation and charset names are rewritten for MariaDB and counted in the result. The source is only read. A target that already has tables is refused unless replace is true. Always needs approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |_, a| {
            Ok(Plan::new("db.copy", format!("copy `{}` from the server on port {} into devd", a.database, a.from_port))
                .step("dump the source to the backups folder (read only)")
                .step("create the target database if it is missing")
                .step("run the dump against it")
                .effect("writes a file and changes stored data"))
        },
        |ctx, a| {
            let target = a.as_name.clone().unwrap_or_else(|| a.database.clone());
            let file = services::dump::default_file(ctx, &a.database);
            match db_target(ctx, &a.engine)? {
                Db::Maria { port } => {
                    services::dump::valid_source_name(&target)?;
                    if !a.replace.unwrap_or(false) && services::mariadb::table_count(ctx, port, &target)? > 0 {
                        return Err(DevdError::conflict(format!("the database `{target}` already has tables; pass replace to copy over them")));
                    }
                    let from = services::dump::Endpoint::local(None, a.from_port, a.from_user.as_deref(), a.from_password.clone())?;
                    let dumped = services::dump::dump(ctx, &from, &a.database, &file)?;
                    let restored = services::dump::restore(ctx, &services::dump::Endpoint::devd(port), &target, &file)?;
                    Ok(Outcome::with_data(
                        format!("copied {} into devd as {}", a.database, target),
                        json!({ "dump": dumped, "restore": restored, "tables": services::mariadb::table_count(ctx, port, &target)? }),
                    ))
                }
                Db::Postgres { root, port } => {
                    services::engines::valid_pg_name(&target)?;
                    if !a.replace.unwrap_or(false) && services::engines::pg_table_count(&root, port, &target)? > 0 {
                        return Err(DevdError::conflict(format!("the database `{target}` already has tables; pass replace to copy over them")));
                    }
                    services::dump::Endpoint::local(None, a.from_port, a.from_user.as_deref(), None)?;
                    let user = a.from_user.clone().unwrap_or_else(|| "postgres".into());
                    services::engines::pg_dump(&root, a.from_port, &user, a.from_password.as_deref(), &a.database, &file)?;
                    services::engines::pg_restore(ctx, &root, port, &target, &file)?;
                    Ok(Outcome::with_data(
                        format!("copied {} into devd as {}", a.database, target),
                        json!({ "file": file.display().to_string(), "tables": services::engines::pg_table_count(&root, port, &target)? }),
                    ))
                }
                Db::Mysql { root, port } => {
                    services::engines::valid_my_name(&target)?;
                    if !a.replace.unwrap_or(false) && services::engines::my_table_count(&root, port, &target)? > 0 {
                        return Err(DevdError::conflict(format!("the database `{target}` already has tables; pass replace to copy over them")));
                    }
                    services::engines::my_dump(&root, a.from_port, &a.database, &file)?;
                    services::engines::my_restore(&root, port, &target, &file)?;
                    Ok(Outcome::with_data(
                        format!("copied {} into devd as {}", a.database, target),
                        json!({ "file": file.display().to_string(), "tables": services::engines::my_table_count(&root, port, &target)? }),
                    ))
                }
            }
        },
    )?;

    reg.define::<DbDropArgs>("db.drop", "Drop a database")
    .description("Drops a database and everything in it. A dump is saved first, and its id is in the result: `snapshot.restore` brings the database back. If the dump cannot be made the drop is refused, unless `no_snapshot` says to go ahead without one. Always requires approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |ctx, a| {
            db_target(ctx, &a.engine)?;
            Ok(Plan::new("db.drop", format!("drop the database `{}`", a.name))
                .step(if a.no_snapshot.unwrap_or(false) {
                    "NO dump is saved first: this cannot be undone"
                } else {
                    "a dump is saved first, so `snapshot.restore` can bring it back"
                })
                .step("every table and every row in it is deleted")
                .effect("deletes stored data")
                .destructive(true)
                .data(json!({ "database": a.name })))
        },
        |ctx, a| {
            let target = db_target(ctx, &a.engine)?;
            // Save what is about to be destroyed. A dump that cannot be made stops
            // the drop: an approval that said "I can get it back" must be true.
            let snapshot = if a.no_snapshot.unwrap_or(false) {
                None
            } else {
                Some(snapshot_database(ctx, &target, &a.name)?)
            };
            let mut data = match &target {
                Db::Maria { port } => services::mariadb::drop_database(ctx, *port, &a.name)?,
                Db::Postgres { root, port } => {
                    services::engines::pg_drop(ctx, root, *port, &a.name)?;
                    json!({ "database": a.name })
                }
                Db::Mysql { root, port } => {
                    services::engines::my_drop(root, *port, &a.name)?;
                    json!({ "database": a.name })
                }
            };
            if let (Some(snap), Some(object)) = (&snapshot, data.as_object_mut()) {
                object.insert("undo".into(), json!(snap.id));
            }
            let message = match &snapshot {
                Some(snap) => format!("dropped the database `{}`; `snapshot.restore {}` brings it back", a.name, snap.id),
                None => format!("dropped the database `{}`; no dump was saved", a.name),
            };
            Ok(Outcome::with_data(message, data))
        },
    )?;

    reg.define::<SnapshotQuery>("snapshot.list", "List saved states that can be restored")
    .description("What devd saved before it overwrote or deleted a file or directory, or dropped a database: what it was, which capability did it and who asked, newest first. Restore one with `snapshot.restore`. Snapshots are kept for a limited time and size, and a very large file or tree is not saved at all (the result of the operation says so).")
    .group("snapshot")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, _| Ok(Plan::new("snapshot.list", "list saved states")),
        |ctx, a| {
            let all = devd_core::snapshot::list(&ctx.paths, a.limit.unwrap_or(30).clamp(1, 500));
            Ok(Outcome::with_data(format!("{} snapshot(s)", all.len()), serde_json::to_value(&all)?))
        },
    )?;

    reg.define::<SnapshotRef>("snapshot.restore", "Put a saved state back")
    .description("Restores a file or directory to what it was before an operation changed it, or recreates a database from the dump taken before it was dropped. What is there now is saved first, so the restore can itself be undone (databases excepted: a database is restored over a fresh one, and refused if one of that name exists). Human only, because it overwrites current content.")
    .group("snapshot")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, a| {
            let snap = devd_core::snapshot::get(&ctx.paths, &a.id)?;
            Ok(Plan::new("snapshot.restore", format!("restore {} ({:?}, from {})", snap.subject, snap.kind, snap.created_at))
                .step("what is there now is saved first")
                .effect("overwrites current content with what was saved"))
        },
        |ctx, a| {
            let snap = devd_core::snapshot::get(&ctx.paths, &a.id)?;
            match snap.kind {
                devd_core::snapshot::Kind::Database => {
                    let engine = snap.meta["engine"].as_str().unwrap_or("mariadb").to_string();
                    let database = snap.meta["database"].as_str().ok_or_else(|| DevdError::invalid("the snapshot does not name its database"))?.to_string();
                    let target = db_target(ctx, &Some(engine.clone()))?;
                    let payload = devd_core::snapshot::payload_path(&ctx.paths, &a.id);
                    match target {
                        Db::Maria { port } => {
                            if services::mariadb::databases(ctx, port)?.iter().any(|d| d == &database) {
                                return Err(DevdError::conflict(format!("a database named `{database}` exists; drop or rename it before restoring over it")));
                            }
                            services::dump::restore(ctx, &services::dump::Endpoint::devd(port), &database, &payload)?;
                        }
                        Db::Postgres { root, port } => {
                            services::engines::pg_restore(ctx, &root, port, &database, &payload)?;
                        }
                        Db::Mysql { root, port } => {
                            services::engines::my_restore(&root, port, &database, &payload)?;
                        }
                    }
                    Ok(Outcome::with_data(format!("recreated the database `{database}`"), json!({ "database": database, "engine": engine })))
                }
                _ => {
                    let restored = devd_core::snapshot::restore_path(ctx, &a.id)?;
                    Ok(Outcome::with_data(
                        format!("restored {}", restored.path.display()),
                        json!({ "path": restored.path.display().to_string(), "replaced_snapshot": restored.replaced }),
                    ))
                }
            }
        },
    )?;
    Ok(())
}

/// The port the MariaDB service listens on, refusing clearly when it is absent.
fn db_port(ctx: &Ctx) -> Result<u16> {
    let service = ctx.store.get_service("mariadb")?.ok_or_else(|| {
        DevdError::not_found("no database server is installed; run `devd service install mariadb`")
    })?;
    Ok(services::mariadb::port_of(&service))
}

// -------------------------------------------------------------------- shared

