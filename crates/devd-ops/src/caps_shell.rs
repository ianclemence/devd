//! `php` and `composer` from a terminal: the capabilities behind `devd path`, `devd shim` and
//! `devd composer`.
//!
//! Reading is free. Changing the user's `PATH` is human only, the same rule as signing in at
//! start-up: it changes what every program the person starts can run.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{shims, Registry, Result};

use crate::shell;

/// No arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Nothing {}

/// Which command to resolve, and from where.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResolveArgs {
    /// `php` or `composer`.
    pub command: String,
    /// The folder to resolve from. Default: the folder devd is running in.
    pub cwd: Option<String>,
}

/// Replace Composer.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ComposerArgs {
    /// Replace an installed Composer with the newest release.
    pub force: Option<bool>,
}

fn status_data(ctx: &devd_core::capability::Ctx) -> Result<Value> {
    let state = shims::path_state(&ctx.paths)?;
    Ok(json!({
        "dir": state.dir,
        "on_path": state.on_path,
        "commands": state.commands,
        "php_installed": shims::has_php(&ctx.paths),
        "composer_installed": shims::composer_phar(&ctx.paths).is_file(),
    }))
}

/// Register the capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<Nothing>("shell.status", "Whether php and composer work from a terminal")
        .description("Reports devd's launcher folder, whether it is on your PATH, which commands have a launcher \
             (`php` once a PHP is installed, `composer` once Composer is), and whether PHP and Composer are installed. \
             Nothing is changed.")
        .group("shell")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("shell.status", "read the command-line setup")),
            |ctx, _| {
                let v = status_data(ctx)?;
                let msg = if v["on_path"] == true { "php and composer are available in new terminals" } else { "devd's folder is not on your PATH yet; run `devd path enable`" };
                Ok(Outcome::with_data(msg.to_string(), v))
            },
        )?;

    reg.define::<ResolveArgs>("shell.resolve", "Which program a command would run from a folder")
        .description("Says which executable `php` or `composer` runs from a folder, and why: the PHP the site there is \
             served with, a pin on the folder, or devd's default. Nothing is run.")
        .group("shell")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, args| Ok(Plan::new("shell.resolve", format!("work out what `{}` runs", args.command))),
            |ctx, args| {
                let cwd = match &args.cwd {
                    Some(c) => std::path::PathBuf::from(c),
                    None => std::env::current_dir()?,
                };
                let r = shell::resolve(ctx, &args.command, &cwd)?;
                Ok(Outcome::with_data(
                    format!("{} runs {} ({})", args.command, r.program.display(), r.reason),
                    json!({ "program": r.program.display().to_string(), "arguments": r.prefix, "reason": r.reason }),
                ))
            },
        )?;

    reg.define::<Nothing>("shell.path_enable", "Put devd's php and composer on your PATH")
        .description("Adds devd's launcher folder to the front of your user PATH (the Environment key of your registry \
             hive on Windows, a marked block in ~/.profile elsewhere). No administrator rights. Terminals that are \
             already open do not see it; new ones do. Human only, because it changes what every program you start \
             can run.")
        .group("shell")
        .tier(Tier::Admin)
        .effect(Effect::System)
        .human_only()
        .dry_run()
        .idempotent()
        .handlers(
            |ctx, _| {
                Ok(Plan::new("shell.path_enable", "put devd's launcher folder first on your PATH")
                    .step(format!("folder: {}", shims::bin_dir(&ctx.paths).display()))
                    .effect("changes your user PATH"))
            },
            |ctx, _| {
                if let Ok(exe) = devd_core::util::cli_exe() {
                    shims::ensure(&ctx.paths, &exe)?;
                }
                let state = shims::path_enable(&ctx.paths)?;
                Ok(Outcome::with_data("devd's folder is on your PATH; open a new terminal to use it".to_string(), json!(state)))
            },
        )?;

    reg.define::<Nothing>("shell.path_disable", "Take devd's php and composer off your PATH")
        .description("Removes the launcher folder from your user PATH. The launchers stay in place; every other entry is untouched.")
        .group("shell")
        .tier(Tier::Admin)
        .effect(Effect::System)
        .human_only()
        .dry_run()
        .idempotent()
        .handlers(
            |ctx, _| {
                Ok(Plan::new("shell.path_disable", "take devd's launcher folder off your PATH")
                    .step(format!("folder: {}", shims::bin_dir(&ctx.paths).display()))
                    .effect("changes your user PATH"))
            },
            |ctx, _| {
                let state = shims::path_disable(&ctx.paths)?;
                Ok(Outcome::with_data("devd's folder is off your PATH".to_string(), json!(state)))
            },
        )?;

    reg.define::<ComposerArgs>("composer.install", "Install Composer")
        .description("Downloads the newest stable composer.phar from getcomposer.org over HTTPS, checks it against the \
             SHA-256 getcomposer.org publishes for that release, puts it under tools/ and writes a `composer` launcher. \
             It runs with devd's own PHP, so it works without PHP on your PATH.")
        .group("shell")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .dry_run()
        .handlers(
            |ctx, args| {
                let mut plan = Plan::new("composer.install", "install Composer")
                    .step("read the newest stable release and its published SHA-256 from getcomposer.org")
                    .step("download composer.phar and verify it against that digest")
                    .step(format!("save it as {}", shims::composer_phar(&ctx.paths).display()))
                    .step("run it once with devd's PHP to prove it starts");
                if shims::composer_phar(&ctx.paths).is_file() && !args.force.unwrap_or(false) {
                    plan = plan.step("it is already installed; pass force to replace it");
                }
                Ok(plan)
            },
            |ctx, args| {
                let v = shell::install_composer(ctx, args.force.unwrap_or(false))?;
                let mut outcome = Outcome::with_data(
                    format!("installed Composer {} at {}", v["version"].as_str().unwrap_or("?"), v["path"].as_str().unwrap_or("")),
                    v.clone(),
                );
                if v["on_path"] == false {
                    outcome = outcome.warn("`composer` is not on your PATH yet: run `devd path enable` once to use it from any terminal");
                }
                if v["verified"].is_null() {
                    outcome = outcome.warn("Composer could not be started with devd's PHP; install a PHP first (`devd runtime install php`)");
                }
                Ok(outcome)
            },
        )?;
    Ok(())
}
