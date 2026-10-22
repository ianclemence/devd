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
    /// A service and how many log lines to read.
    LogsArgs { name: String, lines: Option<u32> }
    /// A service to install.
    InstallArgs { name: String, version: Option<String>, port: Option<u16>, force: Option<bool> }
    /// Which web server to use.
    UseWebArgs { server: String }
    /// An existing program folder to use.
    AdoptArgs { name: String, path: String, port: Option<u16> }
    /// A certificate to issue.
    HostArgs { host: String }
    /// Whether to trust the CA for every user or just this one.
    TrustArgs { machine: Option<bool> }
    /// A database to create.
    DbCreateArgs { name: String }
    /// A database to drop.
    DbDropArgs { name: String }
    /// A database to write to a file.
    DbDumpArgs { database: String, path: Option<String>, from_port: Option<u16>, from_user: Option<String>, from_password: Option<String> }
    /// A file to read into a database.
    DbRestoreArgs { database: String, path: String, replace: Option<bool> }
    /// A database to copy from another server on this machine.
    DbCopyArgs { database: String, from_port: u16, from_user: Option<String>, from_password: Option<String>, as_name: Option<String>, replace: Option<bool> }
}

/// Register every service capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    register_service(reg)?;
    register_web(reg)?;
    register_stack(reg)?;
    register_tls(reg)?;
    register_db(reg)?;
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

    reg.define::<NameArgs>("service.find_local", "Find a service's program already on this machine")
    .description("Looks for an Apache folder from Laragon, XAMPP, WampServer or a plain install on every \
         drive and reports each with its version. Nothing is changed; `service.adopt` uses one.")
    .group("service")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("service.find_local", format!("look for {} folders on this machine", a.name))),
        |_, a| {
            if a.name != "apache" {
                return Err(DevdError::unsupported("only Apache can be found on disk"));
            }
            let found: Vec<Value> = services::find_local_apache()
                .into_iter()
                .map(|(path, version)| json!({ "path": path.display().to_string(), "version": version }))
                .collect();
            Ok(Outcome::with_data(format!("{} Apache folder(s) found", found.len()), json!(found)))
        },
    )?;

    reg.define::<AdoptArgs>("service.adopt", "Use a program that is already on disk")
    .description("Records an existing Apache folder (Laragon's, XAMPP's, a plain install) as devd's Apache. \
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
        |ctx, args| start_plan(ctx, args),
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
        |ctx, args| stop_plan(ctx, args),
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
        |ctx, args| stop_plan(ctx, args),
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
            "`{}` is not a service devd can install; installable services are nginx, apache and mariadb",
            args.name
        ))
    })?;
    let (list, _) = services::available(def.kind)?;
    let spec = args.version.clone().unwrap_or_else(|| "latest".to_string());
    let chosen = devd_runtime::catalogue::resolve(&list, &spec, None)?;
    let dest = def.name.to_string();
    let digest = match &chosen.sha256 {
        Some(d) => format!("{d} (published by the vendor)"),
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

    reg.define::<UseWebArgs>("web.use", "Choose the web server: nginx or apache")
    .description("Sets which web server fronts the sites. Apache reads .htaccess files, so a site written          for Laragon or shared hosting works unchanged; nginx is lighter. The choice is a setting, so it          applies the next time devd starts: stop the running server, restart devd, then bring the stack up.          The server has to be installed first. Human only.")
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
            Ok(Outcome::with_data(
                format!("web_server is now {}; restart devd, stop the other web server and run `devd stack up` to apply it", a.server),
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

fn register_db(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("db.list", "List databases")
    .description("The databases on the local MariaDB server, excluding the system schemas.")
    .group("db")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, _| {
            let port = db_port(ctx)?;
            Ok(Plan::new(
                "db.list",
                format!("list the databases on the MariaDB server on port {port}"),
            ))
        },
        |ctx, _| {
            let port = db_port(ctx)?;
            let names = services::mariadb::databases(ctx, port)?;
            Ok(Outcome::with_data(
                format!("{} database(s)", names.len()),
                json!({ "databases": names, "port": port }),
            ))
        },
    )?;

    reg.define::<DbCreateArgs>("db.create", "Create a database")
    .description("Creates a database with the utf8mb4 character set, if it does not already exist. \
         The name is validated as a plain identifier, so it cannot carry a second statement.")
    .group("db")
    .tier(Tier::Write)
    .effect(Effect::Data)
    .agent_safe()
    .dry_run()
    .idempotent()
    .handlers(
        |ctx, args| {
            devd_core::util::validate_name(&args.name)?;
            let port = db_port(ctx)?;
            Ok(Plan::new(
                "db.create",
                format!("create the database `{}` on port {port}", args.name),
            )
            .step(format!("CREATE DATABASE IF NOT EXISTS `{}`", args.name))
            .step("character set utf8mb4, collation utf8mb4_unicode_ci")
            .effect("creates stored data"))
        },
        |ctx, args| {
            let port = db_port(ctx)?;
            let result = services::mariadb::create_database(ctx, port, &args.name)?;
            Ok(Outcome::with_data(
                format!("database `{}` exists", args.name),
                result,
            ))
        },
    )?;

    reg.define::<DbDumpArgs>("db.dump", "Write a database to a SQL file")
    .description("Dumps one database (tables, data, routines, triggers, events) to a .sql file. By default it is \
         devd's MariaDB, written under <devd home>/backups; from_port names another server on this machine \
         instead, such as Laragon's MySQL. Only a person can choose where the file goes. Servers on other \
         machines are refused.")
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
            let from = match a.from_port {
                Some(port) => services::dump::Endpoint::local(None, port, a.from_user.as_deref(), a.from_password.clone())?,
                None => services::dump::Endpoint::devd(db_port(ctx)?),
            };
            let file = match &a.path {
                Some(p) => std::path::PathBuf::from(p),
                None => services::dump::default_file(ctx, &a.database),
            };
            let report = services::dump::dump(ctx, &from, &a.database, &file)?;
            Ok(Outcome::with_data(format!("wrote {} to {}", a.database, file.display()), report))
        },
    )?;

    reg.define::<DbRestoreArgs>("db.restore", "Read a SQL file into a database")
    .description("Runs a SQL file against a database on devd's MariaDB, creating the database when it is not \
         there. A database that already has tables is refused unless replace is true, because a dump can \
         drop and recreate them. The file is run as it is: restore only files you trust. Always needs approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |_, a| Ok(Plan::new("db.restore", format!("run {} against the database `{}`", a.path, a.database)).effect("changes stored data")),
        |ctx, a| {
            let port = db_port(ctx)?;
            if !a.replace.unwrap_or(false) && services::mariadb::table_count(ctx, port, &a.database)? > 0 {
                return Err(DevdError::conflict(format!(
                    "the database `{}` already has tables; pass replace to run the file over them",
                    a.database
                )));
            }
            let report = services::dump::restore(ctx, &services::dump::Endpoint::devd(port), &a.database, std::path::Path::new(&a.path))?;
            Ok(Outcome::with_data(format!("restored {}", a.database), report))
        },
    )?;

    reg.define::<DbCopyArgs>("db.copy", "Copy a database from another server on this machine into devd")
    .description("Moves a database off Laragon's MySQL (or any server on 127.0.0.1) into devd's MariaDB: dumps it \
         to <devd home>/backups, then restores it under the same name or as_name. The source is only read. A \
         target that already has tables is refused unless replace is true. Always needs approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |_, a| {
            Ok(Plan::new("db.copy", format!("copy `{}` from the server on port {} into devd's MariaDB", a.database, a.from_port))
                .step("dump the source to the backups folder (read only)")
                .step("create the target database if it is missing")
                .step("run the dump against it")
                .effect("writes a file and changes stored data"))
        },
        |ctx, a| {
            let port = db_port(ctx)?;
            let target = a.as_name.clone().unwrap_or_else(|| a.database.clone());
            services::dump::valid_source_name(&target)?;
            if !a.replace.unwrap_or(false) && services::mariadb::table_count(ctx, port, &target)? > 0 {
                return Err(DevdError::conflict(format!(
                    "the database `{target}` already has tables; pass replace to copy over them"
                )));
            }
            let from = services::dump::Endpoint::local(None, a.from_port, a.from_user.as_deref(), a.from_password.clone())?;
            let file = services::dump::default_file(ctx, &a.database);
            let dumped = services::dump::dump(ctx, &from, &a.database, &file)?;
            let restored = services::dump::restore(ctx, &services::dump::Endpoint::devd(port), &target, &file)?;
            Ok(Outcome::with_data(
                format!("copied {} into devd as {}", a.database, target),
                json!({ "dump": dumped, "restore": restored, "tables": services::mariadb::table_count(ctx, port, &target)? }),
            ))
        },
    )?;

    reg.define::<DbDropArgs>("db.drop", "Drop a database")
    .description("Drops a database and everything in it. This cannot be undone and always requires \
         approval.")
    .group("db")
    .tier(Tier::Destructive)
    .effect(Effect::Data)
    .dry_run()
    .handlers(
        |ctx, args| {
            let port = db_port(ctx)?;
            Ok(Plan::new(
                "db.drop",
                format!("drop the database `{}` on port {port}", args.name),
            )
            .step(format!("DROP DATABASE IF EXISTS `{}`", args.name))
            .step("every table and every row in it is deleted")
            .effect("deletes stored data")
            .destructive(true)
            .data(json!({ "database": args.name, "port": port })))
        },
        |ctx, args| {
            let port = db_port(ctx)?;
            let result = services::mariadb::drop_database(ctx, port, &args.name)?;
            Ok(Outcome::with_data(
                format!("dropped the database `{}`", args.name),
                result,
            ))
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

