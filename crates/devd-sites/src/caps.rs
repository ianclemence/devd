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

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
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
    CreateArgs { name: String, host: Option<String>, root: Option<String>, tls: Option<bool>, db: Option<bool> }
    /// A site to remove.
    RemoveArgs { name: String, delete_files: Option<bool> }
}

/// Register every site capability.
pub fn register(reg: &mut Registry) -> Result<()> {
    add::<NoArgs, _, _>(
        reg,
        "site.list",
        "List sites",
        "Every site with a live check of what is really on disk: whether its project directory \
         exists, whether its hostname is in the hosts file, whether a certificate has been \
         issued and whether the web server is running.",
        "site",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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
                if !row.in_hosts {
                    outcome = outcome.warn(format!(
                        "{} does not resolve yet; `devd hosts sync` adds it",
                        row.site.host
                    ));
                }
            }
            Ok(outcome)
        },
    )?;

    add::<CreateArgs, _, _>(
        reg,
        "site.create",
        "Create a site",
        "Creates the project directory with a starter page, issues a certificate for the \
         hostname from the local authority, records the site and re-renders the web server \
         configuration. It does not edit the hosts file: that is `hosts.sync`, an admin-tier \
         operation, so creating a site does not need an elevated approval.",
        "site",
        Tier::Write,
        Effect::Write,
        true,
        true,
        false,
        |ctx, args| create_plan(ctx, args),
        |ctx, args| {
            let request = sites::CreateRequest {
                name: args.name.clone(),
                host: args.host.clone(),
                root: args.root.clone(),
                tls: args.tls.unwrap_or(true),
                db: args.db.unwrap_or(false),
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

    add::<RemoveArgs, _, _>(
        reg,
        "site.remove",
        "Remove a site",
        "Removes the certificate, the record and the web server configuration. The project \
         directory is kept unless `delete_files` is set, and a directory outside the layout's \
         www root is never deleted.",
        "site",
        Tier::Destructive,
        Effect::Write,
        false,
        true,
        false,
        |ctx, args| {
            let row = sites::list(ctx)?
                .into_iter()
                .find(|r| r.site.name == args.name)
                .ok_or_else(|| {
                    DevdError::not_found(format!("no site named `{}`", args.name))
                })?;
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

    add::<NoArgs, _, _>(
        reg,
        "hosts.show",
        "Show the managed hosts block",
        "The hostnames devd manages in the hosts file, what is in the file now, and what a sync \
         would change. Reads the file; writes nothing.",
        "hosts",
        Tier::Read,
        Effect::Read,
        true,
        false,
        true,
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
                    format!("{} change(s) pending", plan.adding.len() + plan.removing.len())
                } else {
                    "the hosts file already matches the sites".to_string()
                },
                serde_json::to_value(&plan).unwrap_or(Value::Null),
            ))
        },
    )?;

    add::<NoArgs, _, _>(
        reg,
        "hosts.sync",
        "Sync the hosts file",
        "Writes the managed block in the system hosts file so that every site hostname \
         resolves, snapshotting the file first. Everything outside the block is preserved \
         exactly, including the file's own line endings. On Windows the real hosts file needs \
         an elevated shell, and the refusal is reported as the operating system gives it.",
        "hosts",
        Tier::Admin,
        Effect::System,
        false,
        true,
        true,
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
            Ok(p
                .step("snapshot the current file first")
                .effect("changes how this machine resolves hostnames")
                .data(json!({
                    "file": plan.file,
                    "adding": plan.adding,
                    "removing": plan.removing,
                })))
        },
        |ctx, _| {
            let result = sites::hosts::sync(ctx)?;
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
    let directory = args
        .root
        .clone()
        .unwrap_or_else(|| sites::default_directory(ctx, &args.name).display().to_string());
    let tls = args.tls.unwrap_or(true);

    let mut plan = Plan::new(
        "site.create",
        format!("create the site {} served at {host}", args.name),
    )
    .step(format!("create the project directory {directory}"))
    .step(format!("write a starter index.php in {directory} if it is empty"));
    if tls {
        plan = plan.step(format!(
            "issue a certificate for {host} from the local authority into <devd>/certs"
        ));
    } else {
        plan = plan.step("serve over plain HTTP; no certificate is issued");
    }
    plan = plan.step("record the site and re-render the web server configuration");
    if args.db.unwrap_or(false) {
        plan = plan.step(format!("create the database `{}` if MariaDB is running", args.name));
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
        })))
}

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
