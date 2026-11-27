//! `update.check` and `update.apply`: devd updating itself from a signed release.
//!
//! The trust model and the order of checks are in [`devd_core::update`]. These
//! two capabilities are the doors to it: one only reads, the other replaces the
//! installed program and is for a person alone.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::update;
use devd_core::{Registry, Result};

/// No arguments.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Nothing {}

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<Nothing>("update.check", "Is there a newer devd?")
        .description("Asks GitHub for the latest release and compares it with the running version. Says whether the release is signed (an update needs the checksum file and its signature) and which file it would install on this platform. Changes nothing.")
        .group("update")
        .tier(Tier::Read)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .dry_run()
        .handlers(
            |_, _| Ok(Plan::new("update.check", "ask GitHub for the latest devd release")),
            |_, _| {
                let found = update::check(update::LATEST_URL, false)?;
                let message = if !found.newer {
                    format!("devd {} is the latest", found.current)
                } else if !found.signed {
                    format!("devd {} is available, but the release is not signed, so it cannot be installed from here", found.latest)
                } else if found.asset.is_none() {
                    format!("devd {} is available, but it has no program for this platform", found.latest)
                } else {
                    format!("devd {} is available (you have {}); `devd update apply` installs it", found.latest, found.current)
                };
                Ok(Outcome::with_data(message, serde_json::to_value(&found)?))
            },
        )?;

    reg.define::<Nothing>("update.apply", "Install the latest devd")
        .description("Downloads the latest release, verifies the signature on its checksum file with the key built into this program, verifies each program against the signed checksum, runs the command to confirm the version, and installs the command and the window beside it, keeping each old one as `<name>.before-<version>`. The window restarts devd into the new version; after `devd update apply`, devd uses it when it next starts. Refuses an unsigned or wrongly signed release. Human only.")
        .group("update")
        .tier(Tier::Destructive)
        .effect(Effect::System)
        .human_only()
        .dry_run()
        .handlers(
            |_, _| {
                Ok(Plan::new("update.apply", "replace the installed devd with the latest signed release")
                    .step("verify the release's signature with the built-in key")
                    .step("verify each program against the signed checksum")
                    .step("run it to confirm its version")
                    .step("install the command and the window")
                    .effect("changes the installed program")
                    .destructive(true))
            },
            |ctx, _| {
                let found = update::check(update::LATEST_URL, false)?;
                if !found.newer {
                    return Ok(Outcome::with_data(
                        format!("devd {} is the latest; nothing to install", found.current),
                        serde_json::to_value(&found)?,
                    ));
                }
                let release = update::latest(update::LATEST_URL, false)?;
                let options = update::Options {
                    public_key: update::RELEASE_PUBLIC_KEY,
                    target: devd_core::util::cli_exe()?,
                    cache: ctx.paths.cache(),
                    smoke_test: true,
                    allow_http: false,
                };
                let applied = update::apply(&release, &options)?;
                let message = if applied.window.is_empty() {
                    format!("installed devd {}; the previous program is {}", applied.version, applied.previous.display())
                } else {
                    format!("installed devd {} and its window; devd uses it when it next starts", applied.version)
                };
                Ok(Outcome::with_data(message, json!(applied)))
            },
        )?;
    Ok(())
}
