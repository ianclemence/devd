//! Capabilities for servers and remote deploys.
//!
//! Anything that touches a server is `remote` tier: an agent can ask, and only a person approves. Registering a
//! server, linking a site and editing steps only change devd's own records, so they are `write`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use devd_core::capability::{Effect, Outcome, Plan, Tier};
use devd_core::{Registry, Result};

use crate::remote::{self, LinkSpec, Stack};
use crate::servers;

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
    /// No arguments.
    Nothing {}
    /// A server to register.
    ServerAddArgs {
        /** A short name, e.g. `hetzner-prod`. */ name: String,
        /** The address or hostname. */ host: String,
        /** The SSH user, e.g. `deploy`, `ploi` or `root`. */ user: String,
        /** The SSH port. Default 22. */ port: Option<u16>,
        /** The key in ~/.ssh it is reached with (see `ssh.keys`, `ssh.keygen`, `ssh.adopt`). */ key: String,
        /** A label: digitalocean, hetzner, vultr, linode, aws, ploi, forge, other. */ provider: Option<String>,
    }
    /// A server by name.
    ServerName { /** The server's name, from `server.list`. */ server: String }
    /// A command to run.
    ServerRunArgs {
        /** The server. */ server: String,
        /** One shell command line to run there. */ command: String,
    }
    /// What to install.
    ProvisionArgs {
        /** The server. */ server: String,
        /** PHP version. Default 8.4. */ php: Option<String>,
        /** `mariadb` (default), `mysql` or `none`. */ database: Option<String>,
        /** Install Redis. Default false. */ redis: Option<bool>,
        /** Install Node.js (LTS). Default true. */ node: Option<bool>,
        /** Install supervisor for queue workers. Default true. */ supervisor: Option<bool>,
    }
    /// A site linked to a place on a server.
    LinkArgs {
        /** The local site. */ site: String,
        /** The server. */ server: String,
        /** The folder on the server, e.g. `/var/www/shop`. */ path: String,
        /** `releases` (default: atomic releases with instant rollback) or `git` (the server pulls and runs the deploy script, for Ploi/Forge machines). */ mode: Option<String>,
        /** The branch. Default: the current one. */ branch: Option<String>,
        /** The web root inside a release. Default `public`. */ web_dir: Option<String>,
        /** The PHP version on the server. Default 8.4. */ php: Option<String>,
        /** Releases to keep. Default 5. */ keep: Option<u32>,
        /** The domain it answers on. */ domain: Option<String>,
        /** Local build commands run before packaging. Default `npm ci` and `npm run build` for a project that builds. */ build: Option<Vec<String>>,
        /** Ignored paths that still ship. Default `public/build` for a project that builds. */ include: Option<Vec<String>>,
        /** Commands run on the server inside the new release. Default by project type (composer install, migrate, cache). */ steps: Option<Vec<String>>,
    }
    /// A link.
    LinkId { /** The link, `<site>@<server>`, or just the site when it has one. */ link: String }
    /// Optional site.
    MaybeSite { /** Limit to one site. */ site: Option<String> }
    /// Deploy to a server.
    RemoteDeployArgs {
        /** The link, `<site>@<server>`, or just the site when it has one. */ link: String,
        /** Wait for the deploy to finish. A command-line call waits by default. */ wait: Option<bool>,
    }
    /// Roll back.
    RollbackArgs {
        /** The link. */ link: String,
        /** The release to go back to, e.g. `20261022-090000`. Default: the one before the current. */ release: Option<String>,
    }
    /// New steps.
    StepsArgs {
        /** The link. */ link: String,
        /** The commands, one per entry, run inside the new release. */ steps: Vec<String>,
    }
    /// Push the .env.
    EnvPushArgs {
        /** The link. */ link: String,
        /** Replace the file on the server if there is one. */ overwrite: Option<bool>,
    }
    /// A certificate.
    SslArgs {
        /** The link. */ link: String,
        /** The address Let's Encrypt writes to about the certificate. */ email: String,
    }
}

/// Register the capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    register_servers(reg)?;
    register_remote(reg)?;
    Ok(())
}

fn register_servers(reg: &mut Registry) -> Result<()> {
    reg.define::<ServerAddArgs>("server.add", "Register a server you can reach over SSH")
        .description("Remembers a machine (DigitalOcean, Hetzner, Vultr, a Ploi or Forge server, a Raspberry Pi: anything with SSH) by name, address, user, port and the key in ~/.ssh that reaches it. Nothing is contacted: run `server.connect` next, which pins the machine's host key.")
        .group("server")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("server.add", format!("register the server {} ({}@{})", a.name, a.user, a.host))),
            |ctx, a| {
                let s = servers::add(ctx, &a.name, &a.host, &a.user, a.port, &a.key, a.provider.as_deref())?;
                Ok(Outcome::with_data(format!("server {} registered; run server.connect to pin its host key", s.name), json!(s)))
            },
        )?;

    reg.define::<Nothing>("server.list", "List registered servers")
        .description("Each server, whether its host key is pinned (and the fingerprint), and when it last answered.")
        .group("server")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("server.list", "list the servers")),
            |ctx, _| {
                let l = servers::list(ctx)?;
                Ok(Outcome::with_data(format!("{} server(s)", l.len()), json!({ "servers": l, "providers": servers::PROVIDERS })))
            },
        )?;

    reg.define::<ServerName>("server.remove", "Forget a server")
        .description("Removes the server and the links that deploy to it from devd. The machine itself, and what is deployed on it, are not touched.")
        .group("server")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .handlers(
            |_, a| Ok(Plan::new("server.remove", format!("forget the server {}", a.server))),
            |ctx, a| {
                let links = servers::remove(ctx, &a.server)?;
                Ok(Outcome::with_data(format!("server {} forgotten ({links} link(s) removed)", a.server), json!({ "server": a.server, "links_removed": links })))
            },
        )?;

    reg.define::<ServerName>("server.connect", "Connect to a server and pin its host key")
        .description("Opens one SSH connection with the registered key. The first time, the server's host key is recorded in devd's own known-hosts file (never yours) and its fingerprint is returned for you to compare with your provider's console; afterwards a server whose key changed is refused. Returns the server's OS and the user it logged in as.")
        .group("server")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let s = servers::get(ctx, &a.server)?;
                Ok(Plan::new("server.connect", format!("connect to {}@{}:{}", s.user, s.host, s.port)).effect(if s.host_key_pinned { "checks the pinned host key".to_string() } else { "records the host key it sees the first time".to_string() }))
            },
            |ctx, a| {
                let v = servers::connect(ctx, &a.server)?;
                Ok(Outcome::with_data(format!("connected to {}", a.server), v))
            },
        )?;

    reg.define::<ServerName>("server.info", "Look at a server")
        .description("Reads, without changing anything: the OS, uptime, load, memory, disk, which of nginx, PHP, Node, Composer, MySQL/MariaDB, Redis, supervisor, certbot, git and ufw are installed, their versions, and whether the user has passwordless sudo.")
        .group("server")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("server.info", format!("read the state of {}", a.server)).effect("runs read-only commands on the server")),
            |ctx, a| {
                let v = servers::info(ctx, &a.server)?;
                Ok(Outcome::with_data(format!("state of {}", a.server), v))
            },
        )?;

    reg.define::<ServerRunArgs>("server.run", "Run a command on a server")
        .description("Runs one command line on the server as its SSH user and returns the exit code and output (up to 15 minutes). It is exactly as powerful as a shell on that machine, so it needs a person's approval.")
        .group("server")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("server.run", format!("run `{}` on {}", a.command, a.server)).effect("runs this command on a machine that is not this one")),
            |ctx, a| {
                let s = servers::get(ctx, &a.server)?;
                let r = servers::run(ctx, &s, &a.command, None, std::time::Duration::from_secs(900), false)?;
                Ok(Outcome::with_data(format!("exit {} after {:.1}s", r.code, r.seconds), json!(r)))
            },
        )?;

    reg.define::<ProvisionArgs>("server.provision", "Set a fresh Ubuntu or Debian server up for PHP sites")
        .description("Installs nginx, PHP-FPM with the usual extensions, Composer (checked against its published signature), Node.js, a database (MariaDB or MySQL), optionally Redis, supervisor, certbot, and turns on the firewall with SSH, HTTP and HTTPS open. Plain apt and the vendors' own repositories; idempotent; the plan shows the whole script. The SSH user must be root or have passwordless sudo.")
        .group("server")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |_, a| {
                let script = remote::provision_script(&stack_of(a))?;
                let mut plan = Plan::new("server.provision", format!("install the PHP stack on {}", a.server)).effect("installs packages and enables the firewall on the server");
                for line in script.lines().filter(|l| !l.trim().is_empty()) {
                    plan = plan.step(line.to_string());
                }
                Ok(plan)
            },
            |ctx, a| {
                let d = remote::provision(ctx, &a.server, &stack_of(a))?;
                Ok(Outcome::with_data(format!("{} is set up ({:.0}s)", a.server, d.seconds), json!(d)))
            },
        )?;
    Ok(())
}

fn stack_of(a: &ProvisionArgs) -> Stack {
    let d = Stack::default();
    Stack {
        php: a.php.clone().unwrap_or(d.php),
        database: match a.database.as_deref() {
            None => d.database,
            Some("none") | Some("") => None,
            Some(x) => Some(x.to_string()),
        },
        redis: a.redis.unwrap_or(false),
        node: a.node.unwrap_or(true),
        supervisor: a.supervisor.unwrap_or(true),
    }
}

fn register_remote(reg: &mut Registry) -> Result<()> {
    reg.define::<LinkArgs>("remote.link", "Link a site to a place on a server")
        .description("Says where a local site is deployed: which server, which folder, in which mode. `releases` ships a build over SSH into `releases/<time>`, links the shared `.env` and `storage`, runs the steps, and switches `current` atomically (rollback is pointing it back). `git` has the server pull the branch and run the deploy script as written, for machines a panel manages. A site can be linked to several servers (staging and production). Only devd's records change.")
        .group("remote")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("remote.link", format!("link {} to {}:{} ({})", a.site, a.server, a.path, a.mode.clone().unwrap_or_else(|| "releases".into())))),
            |ctx, a| {
                let l = remote::make_link(ctx, &spec_of(a))?;
                Ok(Outcome::with_data(format!("{} deploys to {}:{}", l.site, l.server, l.path), json!(l)))
            },
        )?;

    reg.define::<LinkId>("remote.unlink", "Remove a site's link to a server")
        .description("Forgets the link. Nothing on the server changes.")
        .group("remote")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("remote.unlink", format!("remove the link {}", a.link))),
            |ctx, a| {
                remote::unlink(ctx, &a.link)?;
                Ok(Outcome::with_data(format!("{} unlinked", a.link), json!({ "link": a.link })))
            },
        )?;

    reg.define::<MaybeSite>("remote.links", "List where sites are deployed")
        .description("Every link: site, server, folder, mode, branch, domain, build commands and steps.")
        .group("remote")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("remote.links", "list the links")),
            |ctx, a| {
                let l = remote::links(ctx, a.site.as_deref())?;
                Ok(Outcome::with_data(format!("{} link(s)", l.len()), json!({ "links": l })))
            },
        )?;

    reg.define::<StepsArgs>("remote.steps_set", "Set the commands run on the server in a new release")
        .description("Replaces the steps a `releases` deploy runs inside the new release before switching to it (the default is composer install, migrate and the caches for a Laravel project). An empty list means the default again. They run on the server, so this is an exec action.")
        .group("remote")
        .tier(Tier::Exec)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("remote.steps_set", format!("set {} step(s) for {}", a.steps.len(), a.link)).effect("future deploys run these commands on the server")),
            |ctx, a| {
                let l = remote::set_steps(ctx, &a.link, &a.steps)?;
                Ok(Outcome::with_data(format!("{} step(s) set", l.steps.len()), json!(l)))
            },
        )?;

    reg.define::<RemoteDeployArgs>("remote.deploy", "Deploy a site to its server")
        .description("`releases` mode: runs the local build, packages the project (never the .env, never dependencies the server installs, never logs), uploads it over SSH, unpacks it as a new release, links the shared `.env` and `storage`, runs the steps, and only then switches `current` with one atomic rename and reloads PHP; a failure before the switch leaves the running site untouched. `git` mode: the server pulls the branch and runs the deploy script. Returns at once with the deployment unless asked to wait; follow it with `deploy.status` and `deploy.log`.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let l = remote::link(ctx, &a.link)?;
                let mut plan = Plan::new("remote.deploy", format!("deploy {} to {}:{} ({})", l.site, l.server, l.path, l.mode));
                if l.mode == "releases" {
                    for b in &l.build {
                        plan = plan.step(format!("locally: {b}"));
                    }
                    plan = plan.step("package the project and upload it").step("unpack it as a new release and link .env and storage");
                    let (_, root) = crate::site_root(ctx, &l.site)?;
                    for s in if l.steps.is_empty() { remote::default_steps(&root) } else { l.steps.clone() } {
                        plan = plan.step(format!("on the server: {s}"));
                    }
                    plan = plan.step("switch `current` atomically and reload PHP-FPM");
                } else {
                    plan = plan.step("git pull on the server").step("run the deploy script on the server");
                }
                for w in crate::limits::warnings(ctx, &l.site) {
                    plan = plan.effect(format!("NOTE {w}"));
                }
                Ok(plan.effect("changes what a live server serves"))
            },
            |ctx, a| {
                let trigger = if ctx.actor.is_human() { "manual" } else { "agent" };
                let mut d = remote::deploy(ctx, &a.link, trigger)?;
                let wait = a.wait.unwrap_or(matches!(ctx.actor.surface, devd_core::capability::Surface::Cli));
                if wait {
                    d = crate::deploy::wait(ctx, &d.id)?;
                }
                let message = if wait { format!("deployment {} {}", d.id, d.state) } else { format!("deployment {} started", d.id) };
                Ok(Outcome::with_data(message, json!({ "deployment": d, "warnings": remote::link(ctx, &a.link).map(|l| crate::limits::warnings(ctx, &l.site)).unwrap_or_default() })))
            },
        )?;

    reg.define::<LinkId>("remote.releases", "List the releases on the server")
        .description("The releases in `releases/` on the server, oldest first, with the commit each was built from and which one `current` points at.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("remote.releases", format!("list the releases of {}", a.link)).effect("runs a read-only command on the server")),
            |ctx, a| {
                let v = remote::releases(ctx, &a.link)?;
                Ok(Outcome::with_data(format!("current is {}", v["current"].as_str().unwrap_or("not set")), v))
            },
        )?;

    reg.define::<RollbackArgs>("remote.rollback", "Roll a server back to an earlier release")
        .description("Points `current` at an earlier release (the one before the current by default, or the one you name) with one atomic rename, reloads PHP, and restarts queue workers. Instant, because the old release is still on disk. Database migrations are not undone.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("remote.rollback", format!("roll {} back to {}", a.link, a.release.clone().unwrap_or_else(|| "the previous release".into()))).effect("changes what a live server serves; migrations are not undone")),
            |ctx, a| {
                let d = remote::rollback(ctx, &a.link, a.release.as_deref())?;
                Ok(Outcome::with_data(d.output.lines().last().unwrap_or("rolled back").to_string(), json!(d)))
            },
        )?;

    reg.define::<EnvPushArgs>("remote.env_push", "Put the site's .env on the server")
        .description("Sends the local .env to `shared/.env` on the server with APP_ENV=production, APP_DEBUG=false and APP_URL set to the domain, readable only by the server user. Refuses to replace a file that is there unless told to. A person only, because it carries secrets.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .handlers(
            |_, a| Ok(Plan::new("remote.env_push", format!("send the .env of {} to its server", a.link)).effect("copies secrets to a machine that is not this one")),
            |ctx, a| {
                ctx.require_human("sending a .env to a server")?;
                let v = remote::env_push(ctx, &a.link, a.overwrite.unwrap_or(false))?;
                Ok(Outcome::with_data("the .env is on the server".to_string(), v))
            },
        )?;

    reg.define::<LinkId>("remote.site_create", "Create the site's nginx block on the server")
        .description("Writes the nginx server block (HTTP, the web root inside `current`, PHP-FPM, dotfiles denied) for the link's domain, enables it, checks the configuration and reloads nginx. The SSH user needs passwordless sudo unless it is root.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |ctx, a| {
                let l = remote::link(ctx, &a.link)?;
                let conf = remote::nginx_vhost(&l)?;
                let mut plan = Plan::new("remote.site_create", format!("create the nginx site {} on {}", l.domain.clone().unwrap_or_default(), l.server)).effect("changes the web server configuration of a server");
                for line in conf.lines().filter(|x| !x.trim().is_empty()) {
                    plan = plan.step(line.to_string());
                }
                Ok(plan)
            },
            |ctx, a| {
                let d = remote::site_create(ctx, &a.link)?;
                Ok(Outcome::with_data("the site is configured on the server".to_string(), json!(d)))
            },
        )?;

    reg.define::<SslArgs>("remote.ssl", "Get an HTTPS certificate for the site")
        .description("Runs certbot on the server for the link's domain with the nginx plugin and turns on the redirect to HTTPS. The domain's DNS must already point at the server.")
        .group("remote")
        .tier(Tier::Remote)
        .effect(Effect::Remote)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("remote.ssl", format!("get a certificate for {}", a.link)).effect("contacts Let's Encrypt from the server and edits its nginx block")),
            |ctx, a| {
                let d = remote::ssl(ctx, &a.link, &a.email)?;
                Ok(Outcome::with_data("the certificate is installed".to_string(), json!(d)))
            },
        )?;
    Ok(())
}

fn spec_of(a: &LinkArgs) -> LinkSpec {
    LinkSpec { site: a.site.clone(), server: a.server.clone(), path: a.path.clone(), mode: a.mode.clone(), branch: a.branch.clone(), web_dir: a.web_dir.clone(), php: a.php.clone(), keep: a.keep, domain: a.domain.clone(), build: a.build.clone(), include: a.include.clone(), steps: a.steps.clone() }
}
