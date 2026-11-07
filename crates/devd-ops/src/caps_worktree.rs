//! `worktree.sites`, `worktree.serve`, `worktree.create` and `worktree.unserve`: a site for every
//! git worktree, so each branch (and each agent working on one) has an address of its own.
//!
//! The behaviour is in [`crate::worktree`]; these are the doors to it, one per question a person
//! or an agent asks.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{Registry, Result};

use crate::worktree::{self, Database};

/// A site, by name.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SiteArgs {
    /// The site the worktrees belong to.
    pub site: String,
}

/// Serve a worktree that already exists.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ServeArgs {
    /// The site whose repository the worktree is a checkout of.
    pub site: String,
    /// The worktree's folder.
    pub path: String,
    /// `share` (the default: use the parent's database), `clone` (a copy under a new name) or `none`.
    #[serde(default)]
    pub database: Option<String>,
}

/// Check out a branch as a worktree and serve it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateArgs {
    /// The site whose repository to branch from.
    pub site: String,
    /// A branch to check out, or a new name to create. Leave it out for a made-up name like `brave-otter`.
    #[serde(default)]
    pub branch: Option<String>,
    /// Where a new branch starts: a branch, tag or commit. Defaults to the site's current HEAD.
    #[serde(default)]
    pub from: Option<String>,
    /// `share` (the default), `clone` or `none`.
    #[serde(default)]
    pub database: Option<String>,
}

/// Stop serving a worktree.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UnserveArgs {
    /// The site the worktree belongs to.
    pub site: String,
    /// The worktree's own site, from `worktree.sites`.
    pub worktree_site: String,
}

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<SiteArgs>("worktree.sites", "List a site's git worktrees and the address each is served at")
        .description("Every worktree of the site's repository: its branch, folder, and the site that serves it (with its URL) if there is one, whether it has a .env, and which of vendor/ and node_modules/ the main checkout has that it lacks. Reads only.")
        .group("worktree")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .dry_run()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("worktree.sites", format!("list the worktrees of {}", a.site))),
            |ctx, a| {
                let v = worktree::list(ctx, &a.site)?;
                let n = v["worktrees"].as_array().map(|w| w.len()).unwrap_or(0);
                Ok(Outcome::with_data(format!("{n} worktree(s)"), v))
            },
        )?;

    reg.define::<ServeArgs>("worktree.serve", "Serve an existing git worktree as a site")
        .description("Gives a worktree its own site at <branch>.<site-host>, with the same HTTPS and web root layout as the parent, a .env copied from the parent and pointed at the new address, and optionally its own copy of the database. A worktree made any other way (git worktree add, another tool) is served the same way. Dependencies are not installed: the result lists what is missing. The hosts file is not changed; `hosts.sync` does that, and asks.")
        .group("worktree")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| {
                Ok(Plan::new("worktree.serve", format!("serve {} as a worktree site of {}", a.path, a.site))
                    .step("create a site named <site>-<branch> at <branch>.<site-host>")
                    .step("write the worktree's .env if it has none, pointed at its own address")
                    .step("copy the database when asked to")
                    .step("re-render the web server configuration"))
            },
            |ctx, a| {
                let db = Database::parse(a.database.as_deref())?;
                let v = worktree::serve(ctx, &a.site, std::path::Path::new(&a.path), db)?;
                Ok(Outcome::with_data(format!("served at {}", v["url"].as_str().unwrap_or("")), v))
            },
        )?;

    reg.define::<CreateArgs>("worktree.create", "Check out a branch as a worktree and serve it at its own address")
        .description("Creates a git worktree of the site's repository (a branch that exists is checked out; a new name is created from `from`, or from HEAD) under devd's own worktrees folder, then serves it as worktree.serve does. With no branch it makes up a name like brave-otter. This is how several agents can each work on one project and each have a URL to show what they built.")
        .group("worktree")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| {
                Ok(Plan::new(
                    "worktree.create",
                    format!("make a worktree of {}{}", a.site, a.branch.as_deref().map(|b| format!(" on {b}")).unwrap_or_default()),
                )
                .step("git worktree add, under devd's worktrees folder")
                .step("serve it as a site and set it up"))
            },
            |ctx, a| {
                let db = Database::parse(a.database.as_deref())?;
                let v = worktree::create(ctx, &a.site, a.branch.as_deref(), a.from.as_deref(), db)?;
                Ok(Outcome::with_data(format!("The worktree {} is ready at {}", v["branch"].as_str().unwrap_or("the worktree"), v["url"].as_str().unwrap_or("")), v))
            },
        )?;

    reg.define::<UnserveArgs>("worktree.unserve", "Stop serving a worktree")
        .description("Removes a worktree's site: its address, certificate and web server entry. The checkout and its files are kept, because they hold work; git.worktree.remove deletes a checkout, and asks first.")
        .group("worktree")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("worktree.unserve", format!("stop serving {}", a.worktree_site)).step("the checkout is kept")),
            |ctx, a| {
                let v = worktree::unserve(ctx, &a.site, &a.worktree_site)?;
                Ok(Outcome::with_data(format!("The worktree {} is no longer served", a.worktree_site), json!(v)))
            },
        )?;
    Ok(())
}
