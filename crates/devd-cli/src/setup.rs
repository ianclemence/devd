//! `devd setup`: from an empty data directory to a site answering over HTTPS.
//!
//! Every step is an existing capability, dispatched through the registry with
//! the auditor and the tier rules intact. This module decides the order and the
//! arguments, and answers the approval question the way `--yes` does: by the
//! human who ran the command, explicitly.
//!
//! The first site is published as `localhost` on purpose. It needs no hosts-file
//! entry and no elevation, so the first run ends with something that opens in a
//! browser. Later sites use `<name>.<tld>` and are published with `devd hosts
//! sync`, which does need an elevated shell.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::capability::{Approval, InvokeRequest};
use devd_core::{DevdError, Result};

use crate::app::App;

/// First site created when the installation has none.
const FIRST_SITE: &str = "welcome";
/// Hostname of the first site. `localhost` resolves everywhere, with no hosts
/// file and no administrator.
const FIRST_HOST: &str = "localhost";

/// How long to wait when asking whether a port is already accepting connections.
const PROBE: Duration = Duration::from_millis(250);

/// Run the first-time setup.
pub fn run(json: bool, plan_only: bool, assume_yes: bool) -> Result<i32> {
    let mut app = App::bootstrap(json, plan_only, assume_yes)?;

    if plan_only {
        for line in plan_lines() {
            println!("{line}");
        }
        return Ok(0);
    }

    let mut steps: Vec<Value> = Vec::new();

    // What devd needs on this machine, and what this run will do about it.
    // The person sees the whole list before anything changes.
    say(&app, "==> Requirements");
    report_requirements(&app);

    app = settle_ports(app, &mut steps)?;

    say(&app, "==> Runtime");
    let ctx = app.ctx();
    if devd_runtime::executable(&ctx, "php", None).is_err() {
        say(
            &app,
            "    installing PHP (the vendor download is about 35 MB)",
        );
        steps.push(invoke(
            &app,
            "runtime.install",
            json!({ "kind": "php", "version": "latest" }),
        )?);
    } else {
        say(&app, "    PHP is already installed");
    }

    say(&app, "==> Services");
    let ctx = app.ctx();
    let db_port = match devd_services::mariadb_service(&ctx) {
        Some((port, _)) => port,
        None => {
            if tcp_open(3306) {
                pick_port(3307, &[])?
            } else {
                3306
            }
        }
    };
    for name in ["nginx", "mariadb"] {
        let ctx = app.ctx();
        if ctx.store.get_service(name)?.is_some() {
            say(&app, &format!("    {name} is already installed"));
            continue;
        }
        say(&app, &format!("    installing {name}"));
        let args = if name == "mariadb" {
            json!({ "name": name, "version": null, "port": db_port })
        } else {
            json!({ "name": name, "version": null })
        };
        steps.push(invoke(&app, "service.install", args)?);
    }

    say(&app, "==> Starting the stack");
    steps.push(invoke(&app, "stack.up", json!({}))?);

    let ctx = app.ctx();
    if ctx.store.list_sites()?.is_empty() {
        say(&app, "==> First site");
        steps.push(invoke(
            &app,
            "site.create",
            json!({ "name": FIRST_SITE, "host": FIRST_HOST }),
        )?);
    }

    if app.assume_yes {
        say(&app, "==> Certificate authority");
        match invoke(&app, "tls.trust", json!({ "machine": false })) {
            Ok(step) => steps.push(step),
            Err(e) => say(&app, &format!("    could not trust the CA: {e}")),
        }
    }

    // Report the URL that the site table actually holds, rather than the one
    // this module expected.
    let ctx = app.ctx();
    let url = ctx
        .store
        .get_site(FIRST_SITE)?
        .map(|site| devd_sites::url_for(&ctx, &site));

    if app.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true,
                "steps": steps,
                "site": FIRST_SITE,
                "url": url,
                "certificate_trusted": app.assume_yes,
            }))?
        );
        return Ok(0);
    }

    println!();
    println!("==> Ready");
    if let Some(url) = &url {
        println!("    site   {url}");
    }
    println!("    app    devd ui");
    if app.assume_yes {
        println!("    The certificate authority is trusted for this user.");
    } else {
        println!("    The certificate is not trusted yet: run `devd tls trust -y`.");
    }
    println!(
        "    Further sites use <name>.<tld>; publish them with `devd hosts sync` \
         from an elevated shell."
    );
    Ok(0)
}

/// Print what devd needs on this machine and what would fix each thing.
///
/// This is the same list the window and the terminal show, so a person is
/// never told one thing by the installer and another by the app.
fn report_requirements(app: &App) {
    let ctx = app.ctx();
    let request = InvokeRequest {
        id: "setup.requirements".into(),
        args: json!({}),
        dry_run: false,
    };
    let Ok(result) = app.registry.dispatch(&ctx, &request, Approval::None) else {
        return;
    };
    let Some(outcome) = result.outcome else { return };
    let requirements = outcome.data["requirements"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let missing: Vec<&Value> = requirements
        .iter()
        .filter(|r| r["state"] != "ready")
        .collect();
    if missing.is_empty() {
        say(app, "    everything devd needs is already here");
        return;
    }
    for r in missing {
        let title = r["title"].as_str().unwrap_or("requirement");
        let why = r["why"].as_str().unwrap_or("");
        say(app, &format!("    {title} — {why}"));
        let fix = &r["fix"];
        if let Some(capability) = fix["capability"].as_str() {
            say(
                app,
                &format!("      this run installs it with {capability}"),
            );
        } else if let Some(view) = fix["view"].as_str() {
            say(app, &format!("      open {view} in the app to finish this"));
        } else if let Some(command) = fix["command"].as_str() {
            say(app, &format!("      run this yourself: {command}"));
        }
    }
}

/// The steps, for `--plan`.
fn plan_lines() -> Vec<&'static str> {
    vec![
        "choose free ports when 80/443 are taken by another program",
        "install PHP (latest stable) when no PHP runtime is installed",
        "install nginx and MariaDB when they are not installed",
        "start php-cgi, MariaDB and nginx",
        "create the site `welcome` at https://localhost:<https-port>",
        "trust the local certificate authority for this user (with --yes)",
    ]
}

/// Choose ports when the configured ones are held by another program.
///
/// When devd's own nginx is running, the configured ports are by definition
/// ours, and changing them would break the running configuration.
fn settle_ports(mut app: App, steps: &mut Vec<Value>) -> Result<App> {
    let ctx = app.ctx();
    let ours = devd_services::status(&ctx, "nginx")
        .map(|status| status.running)
        .unwrap_or(false);
    if ours {
        return Ok(app);
    }

    let mut settings = (*app.settings).clone();
    let mut changed = false;

    if tcp_open(settings.http_port) {
        let picked = pick_port(8080, &[])?;
        eprintln!(
            "note: port {} is held by another program; using {} for HTTP",
            settings.http_port, picked
        );
        settings.http_port = picked;
        changed = true;
    }
    if tcp_open(settings.https_port) {
        let picked = pick_port(8443, &[settings.http_port])?;
        eprintln!(
            "note: port {} is held by another program; using {} for HTTPS",
            settings.https_port, picked
        );
        settings.https_port = picked;
        changed = true;
    }

    if changed {
        settings.save(&app.paths.settings_file())?;
        steps.push(json!({
            "capability": "settings",
            "message": "moved the listeners to free ports",
            "data": { "http_port": settings.http_port, "https_port": settings.https_port },
        }));
        app = App::bootstrap(app.json, app.plan_only, app.assume_yes)?;
    }
    Ok(app)
}

/// Whether something is accepting connections on `port` on loopback.
fn tcp_open(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), PROBE).is_ok()
}

/// The first free port at or after `start`, skipping `avoid`.
fn pick_port(start: u16, avoid: &[u16]) -> Result<u16> {
    for port in start..=start.saturating_add(200) {
        if !avoid.contains(&port) && !tcp_open(port) {
            return Ok(port);
        }
    }
    Err(DevdError::conflict(format!(
        "no free port was found between {start} and {}",
        start.saturating_add(200)
    )))
}

/// Invoke one capability, with the approval rule `--yes` stands for.
fn invoke(app: &App, id: &str, args: Value) -> Result<Value> {
    let cap = app
        .registry
        .get(id)
        .ok_or_else(|| DevdError::not_found(format!("no capability `{id}`")))?;
    let approval = if cap.tier.requires_approval() {
        if !app.assume_yes {
            return Err(DevdError::denied(format!(
                "`{id}` needs approval; run `devd setup --yes` to approve the remaining steps"
            )));
        }
        Approval::Granted
    } else {
        Approval::None
    };

    let ctx = app.ctx();
    let request = InvokeRequest {
        id: id.to_string(),
        args,
        dry_run: false,
    };
    let result = app.registry.dispatch(&ctx, &request, approval)?;
    let outcome = result.outcome.ok_or_else(|| {
        DevdError::internal(format!("`{id}` returned a plan when it was asked to run"))
    })?;

    for warning in &outcome.warnings {
        if !app.json {
            eprintln!("    warning: {warning}");
        }
    }
    if !app.json {
        println!("    {}", outcome.message);
    }

    Ok(json!({
        "capability": id,
        "message": outcome.message,
        "data": outcome.data,
        "warnings": outcome.warnings,
    }))
}

fn say(app: &App, text: &str) {
    if !app.json {
        println!("{text}");
    }
}
