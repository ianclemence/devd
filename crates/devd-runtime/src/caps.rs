//! Runtime capabilities.
//!
//! Declared once, here, so that `devd runtime install php@8.3` on the command
//! line, the same action in the desktop application, and an agent asking for it
//! over MCP all pass through one implementation with one set of permission
//! rules.
//!
//! Tiers are chosen by what the operation can do to the machine:
//!
//! * Listing and verifying are [`Tier::Read`]. Verifying runs the interpreter
//!   that is already installed, which is why it is not free-standing "exec",
//!   but it changes nothing.
//! * Installing is [`Tier::Write`]. It adds files under the devd root from a
//!   digest-verified vendor URL; the plan names the exact URL, digest and
//!   destination before anything is downloaded, so an agent's proposal is
//!   reviewable by a human in the approval dialog.
//! * Removing is [`Tier::Destructive`], because it deletes a directory and can
//!   take a site's interpreter with it. It always needs approval.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::download::Progress;
use devd_core::{Registry, Result};

use crate as runtimes;

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
    /// Which runtime kind to look up.
    AvailableArgs { kind: String, thread_safe: Option<bool> }
    /// A runtime to install.
    InstallArgs { kind: String, version: String, thread_safe: Option<bool>, force: Option<bool> }
    /// An installed runtime.
    VersionArgs { kind: String, version: String }
    /// A PHP version to make the default for new sites.
    UseArgs { version: String }
}

/// Register every runtime capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("runtime.list", "List installed runtimes")
    .description("Every runtime devd has installed, with a live check that the install directory \
         and executable are still there.")
    .group("runtime")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("runtime.list", "list the installed runtimes")),
        |ctx, _| {
            let rows = runtimes::list(ctx)?;
            let missing: Vec<String> = rows
                .iter()
                .filter(|r| !r.present)
                .map(|r| format!("{} {}", r.runtime.kind, r.runtime.version))
                .collect();
            let mut outcome = Outcome::with_data(
                format!("{} runtime(s) installed", rows.len()),
                serde_json::to_value(&rows).unwrap_or(Value::Null),
            );
            for name in missing {
                outcome = outcome.warn(format!("{name} is recorded but its directory is gone"));
            }
            Ok(outcome)
        },
    )?;

    reg.define::<AvailableArgs>("runtime.available", "List runtime versions published upstream")
    .description("Reads the vendor's own version catalogue (windows.php.net for PHP, nodejs.org \
         for Node, go.dev for Go) and returns the builds that can be installed, with the \
         digest each one will be verified against.")
    .group("runtime")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "runtime.available",
                "read the upstream version catalogue",
            ))
        },
        |_, args| {
            let thread_safe = args.thread_safe.unwrap_or(false);
            let (list, notes) = runtimes::available(&args.kind, thread_safe)?;
            let mut outcome = Outcome::with_data(
                format!("{} build(s) available for {}", list.len(), args.kind),
                serde_json::to_value(&list).unwrap_or(Value::Null),
            );
            for note in notes {
                outcome = outcome.warn(note);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<InstallArgs>("runtime.install", "Install a runtime")
    .description("Resolves the request against the vendor's catalogue, downloads the official \
         build over HTTPS, verifies the vendor's published SHA-256, unpacks it under \
         runtimes/<kind>/<version>/, runs the post-install steps the build needs (PHP \
         gets a generated php.ini) and proves it starts before recording it.")
    .group("runtime")
    .tier(Tier::Write)
    .effect(Effect::Write)
    .agent_safe()
    .dry_run()
    .handlers(
        |ctx, args| install_plan(ctx, args),
        |ctx, args| {
            let thread_safe = args.thread_safe.unwrap_or(false);
            let force = args.force.unwrap_or(false);
            let progress = |p: &Progress| {
                ctx.events.emit_with(
                    "runtime.download",
                    match p.total {
                        Some(total) => format!("downloading {} of {} bytes", p.received, total),
                        None => format!("downloading {} bytes", p.received),
                    },
                    json!({ "received": p.received, "total": p.total, "fraction": p.fraction() }),
                );
                true
            };
            let done = runtimes::install(
                ctx,
                &args.kind,
                &args.version,
                thread_safe,
                force,
                Some(&progress),
            )?;
            let mut outcome = Outcome::with_data(
                format!(
                    "installed {} {} at {}",
                    done.runtime.kind, done.runtime.version, done.runtime.path
                ),
                json!({
                    "runtime": done.runtime,
                    "executable": done.executable,
                    "verified_version": done.verified_version,
                    "enabled_extensions": done.enabled,
                    "disabled_extensions": done.disabled,
                    "sha256": done.runtime.digest,
                    "bytes": done.bytes,
                    "from_cache": done.from_cache,
                }),
            );
            if !done.disabled.is_empty() {
                outcome = outcome.warn(format!(
                    "these extensions could not load on this machine and were switched off in \
                     php.ini: {}",
                    done.disabled.join(", ")
                ));
            }
            for warning in done.warnings {
                outcome = outcome.warn(warning);
            }
            Ok(outcome)
        },
    )?;

    reg.define::<VersionArgs>("runtime.verify", "Run an installed runtime and report its version")
    .description("Starts the installed interpreter with its version flag and reports what it says, \
         so a runtime that is present but cannot start is visible rather than assumed \
         healthy. For PHP it also reports the loaded modules.")
    .group("runtime")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| {
            Ok(Plan::new(
                "runtime.verify",
                "run the runtime and read its version",
            ))
        },
        |ctx, args| {
            let report = runtimes::verify(ctx, &args.kind, &args.version)?;
            let version = report
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(Outcome::with_data(
                format!("{} {} runs (reported {version})", args.kind, args.version),
                report,
            ))
        },
    )?;

    reg.define::<VersionArgs>("runtime.remove", "Remove an installed runtime")
    .description("Deletes the runtime's directory and its record. Refused while a service is \
         configured to use it, so a running web server cannot lose its interpreter.")
    .group("runtime")
    .tier(Tier::Destructive)
    .effect(Effect::Write)
    .dry_run()
    .handlers(
        |ctx, args| {
            let root = ctx.paths.runtime_version(&args.kind, &args.version);
            let mut plan = Plan::new(
                "runtime.remove",
                format!("remove {} {}", args.kind, args.version),
            )
            .destructive(true)
            .effect("delete the runtime directory")
            .effect("remove the runtime from the state store")
            .data(json!({
                "kind": args.kind,
                "version": args.version,
                "path": root.display().to_string(),
                "bytes": devd_core::util::directory_size(&root),
            }));
            if !root.is_dir() {
                plan = plan.step("nothing is installed at that version");
            } else {
                plan = plan.step(format!("delete {}", root.display()));
            }
            if args.kind == "php"
                && ctx.settings.primary_php.as_deref() == Some(args.version.as_str())
            {
                plan = plan.step(
                    "another installed PHP version becomes the default; if none is left, new \
                     sites have no interpreter until one is installed",
                );
            }
            Ok(plan)
        },
        |ctx, args| {
            let data = runtimes::remove(ctx, &args.kind, &args.version)?;
            Ok(Outcome::with_data(
                format!("removed {} {}", args.kind, args.version),
                data,
            ))
        },
    )?;

    reg.define::<UseArgs>("runtime.use", "Choose the PHP version new sites use")
    .description("Sets the default PHP version for sites that do not pin one of their own.")
    .group("runtime")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .agent_safe()
    .idempotent()
    .handlers(
        |ctx, args| {
            Ok(Plan::new(
                "runtime.use",
                format!("make PHP {} the default for new sites", args.version),
            )
            .step(format!(
                "write primary_php = \"{}\" to {}",
                args.version,
                ctx.paths.settings_file().display()
            ))
            .data(json!({ "version": args.version })))
        },
        |ctx, args| {
            runtimes::set_primary_php(ctx, &args.version)?;
            Ok(Outcome::with_data(
                format!("PHP {} is now the default for new sites", args.version),
                json!({ "primary_php": args.version }),
            ))
        },
    )?;

    Ok(())
}

/// The plan for an install: resolve first, so the URL and digest are visible
/// before approval rather than after the download has already happened.
fn install_plan(ctx: &Ctx, args: &InstallArgs) -> Result<Plan> {
    let thread_safe = args.thread_safe.unwrap_or(false);
    let (list, notes) = runtimes::available(&args.kind, thread_safe)?;
    let dest_kind = args.kind.clone();
    let mut plan = Plan::new(
        "runtime.install",
        format!("install {} {}", args.kind, args.version),
    );
    if list.is_empty() {
        for note in &notes {
            plan = plan.step(note.clone());
        }
        plan = plan.step("no build can be installed on this platform right now");
        return Ok(plan);
    }

    let chosen = match crate::catalogue::resolve(
        &list,
        &args.version,
        Some(if thread_safe { "ts" } else { "nts" }),
    ) {
        Ok(c) => c,
        Err(_) => crate::catalogue::resolve(&list, &args.version, None)?,
    };
    let dest = ctx.paths.runtime_version(&dest_kind, &chosen.version);
    plan = plan
        .step(format!("download {}", chosen.url))
        .step(match &chosen.sha256 {
            Some(digest) => format!("verify the download against SHA-256 {digest}"),
            None => "no vendor digest is published for this build; it is checked for length only"
                .to_string(),
        })
        .step(format!("unpack into {}", dest.display()));
    if args.kind == "php" {
        plan = plan.step("generate php.ini with the extensions this build ships");
    }
    plan = plan.step("run the installed runtime to prove it starts");
    if dest.exists() && !args.force.unwrap_or(false) {
        plan = plan.step("it is already installed; pass force to replace it");
    }
    Ok(plan.data(json!({
        "kind": chosen.kind,
        "version": chosen.version,
        "channel": chosen.channel,
        "url": chosen.url,
        "sha256": chosen.sha256,
        "bytes": chosen.bytes,
        "destination": dest.display().to_string(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn registry() -> Registry {
        let mut reg = Registry::new();
        register(&mut reg).unwrap();
        reg
    }

    #[test]
    fn every_runtime_capability_is_declared_once() {
        let reg = registry();
        let mut ids: Vec<String> = reg
            .list()
            .iter()
            .filter(|c| c.group == "runtime")
            .map(|c| c.id.clone())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "runtime.available",
                "runtime.install",
                "runtime.list",
                "runtime.remove",
                "runtime.use",
                "runtime.verify",
            ]
        );
    }

    #[test]
    fn removal_is_destructive_and_never_agent_approvable() {
        let reg = registry();
        let cap = reg.get("runtime.remove").unwrap();
        assert_eq!(cap.tier, Tier::Destructive);
        assert!(cap.tier.requires_approval());
        assert!(cap.supports_dry_run);
        assert!(!cap.agent_safe);
    }

    #[test]
    fn installing_is_agent_reachable_but_always_plannable() {
        let reg = registry();
        let cap = reg.get("runtime.install").unwrap();
        assert_eq!(cap.tier, Tier::Write);
        assert!(!cap.tier.requires_approval());
        assert!(cap.supports_dry_run);
        assert!(cap.agent_safe);
    }

    #[test]
    fn arguments_are_typed_and_reject_nonsense() {
        let reg = registry();
        let cap = reg.get("runtime.install").unwrap();
        let ctx = Ctx {
            store: Arc::new(devd_core::state::Store::open_in_memory().unwrap()),
            paths: Arc::new(devd_core::paths::Paths::rooted(
                std::env::temp_dir().join("devd-rt-caps"),
            )),
            settings: Arc::new(devd_core::config::Settings::default()),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: true,
        };
        // A wrong type is a validation error, not a crash.
        let err = cap
            .handler
            .plan(&ctx, &json!({ "kind": 7, "version": [] }))
            .unwrap_err();
        assert_eq!(err.code(), "invalid_input", "{err}");
    }
}
