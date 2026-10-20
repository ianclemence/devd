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

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
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
    /// A certificate to issue.
    HostArgs { host: String }
    /// Whether to trust the CA for every user or just this one.
    TrustArgs { machine: Option<bool> }
    /// A database to create.
    DbCreateArgs { name: String }
    /// A database to drop.
    DbDropArgs { name: String }
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
    add::<NoArgs, _, _>(
        reg,
        "service.list",
        "List services",
        "Every service devd manages, with a live check that its files exist, its recorded \
         process is still the process it claims to be, and its port is answering.",
        "service",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<NameArgs, _, _>(
        reg,
        "service.available",
        "List service versions published upstream",
        "Reads the vendor's own listing (nginx.org, archive.mariadb.org) and returns the \
         versions that can be installed.",
        "service",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<InstallArgs, _, _>(
        reg,
        "service.install",
        "Install a service",
        "Resolves the request against the vendor's listing, downloads the official build, \
         verifies its digest where the vendor publishes one, unpacks it and records how to \
         start it. The plan names the URL, the digest and the destination before anything is \
         downloaded. A vendor that publishes no checksum file is reported as such.",
        "service",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
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

    add::<NameArgs, _, _>(
        reg,
        "service.start",
        "Start a service",
        "Starts the service detached and records its process id, so it survives the command \
         that started it. The start is verified: the port has to open, or the service is \
         stopped again and the failure is reported with the tail of its own log.",
        "service",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |ctx, args| start_plan(ctx, args),
        |ctx, args| {
            let status = services::start(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<NameArgs, _, _>(
        reg,
        "service.stop",
        "Stop a service",
        "Stops the service and everything it spawned. The recorded process id is only used \
         after its image name has been confirmed, so a recycled process id cannot lead to an \
         unrelated program being killed.",
        "service",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        true,
        |ctx, args| stop_plan(ctx, args),
        |ctx, args| {
            let status = services::stop(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<NameArgs, _, _>(
        reg,
        "service.restart",
        "Restart a service",
        "Stops the service and starts it again.",
        "service",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |ctx, args| stop_plan(ctx, args),
        |ctx, args| {
            let status = services::restart(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<NameArgs, _, _>(
        reg,
        "service.status",
        "Show one service",
        "The current state of a service: whether it is installed, running, which process id \
         it has, and whether its port is answering.",
        "service",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
        |_, _| Ok(Plan::new("service.status", "read the service state")),
        |ctx, args| {
            let status = services::status(ctx, &args.name)?;
            Ok(Outcome::with_data(
                status.message.clone(),
                serde_json::to_value(&status).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<LogsArgs, _, _>(
        reg,
        "service.logs",
        "Read a service log",
        "The last lines of a service's own log file - nginx's error output, the PHP \
         FastCGI listener's startup messages, MariaDB's error log.",
        "service",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<NameArgs, _, _>(
        reg,
        "service.remove",
        "Remove a service",
        "Stops the service, deletes the installed files and forgets the record. For MariaDB \
         the data directory is deliberately kept: uninstalling the server is not the same \
         decision as discarding every database.",
        "service",
        Tier::Destructive,
        Effect::Write,
        false,
        true,
        false,
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
            "`{}` is not a service devd can install; installable services are nginx and mariadb",
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
        "render the nginx configuration from the state store".to_string()
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
    add::<NoArgs, _, _>(
        reg,
        "web.render",
        "Render the web server configuration",
        "Rebuilds nginx.conf from the state store: one server block per site, with the TLS \
         listener, the redirect, the document root and the FastCGI upstream. The file is \
         generated, never edited, so it cannot drift from what devd believes.",
        "web",
        Tier::Write,
        Effect::Config,
        true,
        true,
        true,
        |_, _| {
            Ok(Plan::new(
                "web.render",
                "render nginx.conf from the sites in the state store",
            )
            .step("read the sites from the state store")
            .effect("rewrites the generated configuration file"))
        },
        |ctx, _| {
            let path = services::nginx::render(ctx)?;
            let sites = ctx.store.list_sites()?.len();
            Ok(Outcome::with_data(
                format!("rendered {sites} site(s) into {}", path.display()),
                json!({ "path": path.display().to_string(), "sites": sites }),
            ))
        },
    )?;

    add::<NoArgs, _, _>(
        reg,
        "web.test",
        "Check the web server configuration",
        "Runs `nginx -t` against the generated configuration and reports nginx's own message.",
        "web",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
        |_, _| Ok(Plan::new("web.test", "run `nginx -t`")),
        |ctx, _| {
            let output = services::nginx::test(ctx)?;
            Ok(Outcome::with_data(
                "the configuration is valid",
                json!({ "nginx": output }),
            ))
        },
    )?;

    add::<NoArgs, _, _>(
        reg,
        "web.reload",
        "Reload the web server",
        "Checks the generated configuration and then asks nginx to read it, without dropping \
         connections. A configuration that does not pass the check is never loaded.",
        "web",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |_, _| {
            Ok(Plan::new(
                "web.reload",
                "reload nginx with the generated configuration",
            )
            .step("run `nginx -t`")
            .step("signal nginx to reload")
            .effect("reloads a running process"))
        },
        |ctx, _| {
            let output = services::nginx::reload(ctx)?;
            Ok(Outcome::with_data(
                "nginx reloaded",
                json!({ "nginx": output }),
            ))
        },
    )?;
    Ok(())
}

// -------------------------------------------------------------------- stack

fn register_stack(reg: &mut Registry) -> Result<()> {
    add::<NoArgs, _, _>(
        reg,
        "stack.up",
        "Start the development stack",
        "Starts every enabled service in dependency order - the PHP FastCGI listener, then \
         MariaDB, then nginx. Every service is attempted, so one failure does not hide the \
         state of the others, and the result reports exactly what started.",
        "stack",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
        |ctx, _| stack_plan(ctx, true),
        |ctx, _| {
            let started = services::stack_up(ctx)?;
            Ok(Outcome::with_data(
                format!("{} service(s) running", started.len()),
                serde_json::to_value(&started).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<NoArgs, _, _>(
        reg,
        "stack.down",
        "Stop the development stack",
        "Stops every running service, in reverse dependency order.",
        "stack",
        Tier::Exec,
        Effect::Process,
        true,
        true,
        false,
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
    add::<NoArgs, _, _>(
        reg,
        "tls.ca.ensure",
        "Create the local certificate authority",
        "Creates the per-installation certificate authority that signs every site \
         certificate. Idempotent: an existing authority, and its key, are left exactly as \
         they are, because replacing the key would invalidate every certificate already \
         issued from it.",
        "tls",
        Tier::Write,
        Effect::Write,
        true,
        false,
        true,
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

    add::<NoArgs, _, _>(
        reg,
        "tls.ca.status",
        "Report whether the authority is trusted",
        "Checks the operating system's certificate stores for the authority devd created. \
         A user-scope trust needs no administrator; a machine-scope one does. Nothing is \
         changed by checking.",
        "tls",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<NoArgs, _, _>(
        reg,
        "tls.ca.show",
        "Show the local certificate authority",
        "The public certificate of the authority that signs site certificates. This is the \
         file to import into a browser, a device or a corporate proxy to make development \
         certificates trusted without disabling verification anywhere. The private key is \
         never returned.",
        "tls",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<TrustArgs, _, _>(
        reg,
        "tls.trust",
        "Trust the local certificate authority",
        "Adds the authority to the operating system's trust store, so that every site devd \
         serves is accepted by a browser and by any tool that uses the system store, with \
         certificate verification left on. This changes what every program on the machine \
         trusts, which is why it needs a person to approve it.",
        "tls",
        Tier::Admin,
        Effect::System,
        false,
        true,
        true,
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

    add::<TrustArgs, _, _>(
        reg,
        "tls.untrust",
        "Stop trusting the local certificate authority",
        "Removes the authority from the operating system's trust store.",
        "tls",
        Tier::Admin,
        Effect::System,
        true,
        true,
        true,
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

    add::<HostArgs, _, _>(
        reg,
        "tls.issue",
        "Issue a site certificate",
        "Issues a leaf certificate for a hostname, signed by the local authority, with the \
         hostname in subjectAltName and the full chain written where the web server reads \
         it. Re-issuing replaces the certificate it replaces.",
        "tls",
        Tier::Write,
        Effect::Write,
        true,
        false,
        true,
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

    add::<NoArgs, _, _>(
        reg,
        "tls.list",
        "List issued certificates",
        "Every certificate under the layout's certs directory, with the host it is for and \
         whether the key beside it is present.",
        "tls",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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
    add::<NoArgs, _, _>(
        reg,
        "db.list",
        "List databases",
        "The databases on the local MariaDB server, excluding the system schemas.",
        "db",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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

    add::<DbCreateArgs, _, _>(
        reg,
        "db.create",
        "Create a database",
        "Creates a database with the utf8mb4 character set, if it does not already exist. \
         The name is validated as a plain identifier, so it cannot carry a second statement.",
        "db",
        Tier::Write,
        Effect::Data,
        true,
        true,
        true,
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

    add::<DbDropArgs, _, _>(
        reg,
        "db.drop",
        "Drop a database",
        "Drops a database and everything in it. This cannot be undone and always requires \
         approval.",
        "db",
        Tier::Destructive,
        Effect::Data,
        false,
        true,
        false,
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

#[allow(clippy::too_many_arguments)]
fn add<A, P, Q>(
    reg: &mut Registry,
    id: &str,
    summary: &str,
    description: &str,
    group: &str,
    tier: Tier,
    effect: Effect,
    agent_safe: bool,
    dry_run: bool,
    idempotent: bool,
    plan: P,
    apply: Q,
) -> Result<()>
where
    A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
    P: Fn(&Ctx, &A) -> Result<Plan> + Send + Sync + 'static,
    Q: Fn(&Ctx, &A) -> Result<Outcome> + Send + Sync + 'static,
{
    let cap = Capability::builder(id, summary)
        .description(description)
        .group(group)
        .tier(tier)
        .effect(effect)
        .agent_safe(agent_safe)
        .dry_run(dry_run)
        .idempotent(idempotent)
        .args_schema(
            serde_json::to_value(schemars::schema_for!(A))
                .map_err(|e| DevdError::internal(format!("cannot build schema for {id}: {e}")))?,
        )
        .handler(
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                plan(ctx, &typed)
            },
            move |ctx: &Ctx, args: &Value| {
                let typed: A = ctx.parse(args)?;
                apply(ctx, &typed)
            },
        )
        .build()?;
    reg.register(cap)
}
