//! Mapping from the service, site, hosts, web, stack, TLS and database command
//! surfaces to capabilities.
//!
//! Nothing here performs work. Each arm shapes typed arguments and dispatches a
//! capability, so starting nginx from the command line goes through the same
//! permission, approval and audit path as starting it from the application or
//! asking an agent over MCP.
//!
//! The command names are the ones a web developer already uses - `service
//! install`, `service start`, `web reload`, `db create`, `site create` - because
//! an interface that invents its own vocabulary for familiar operations costs
//! the user attention for nothing.

use serde_json::json;

use devd_core::Result;

use crate::app::App;
use crate::{
    DbCommand, HostsCommand, ServiceCommand, SiteCommand, StackCommand, TlsCommand, WebCommand,
};

/// Run a `devd service` subcommand.
pub fn service(app: &App, cmd: ServiceCommand) -> Result<()> {
    match cmd {
        ServiceCommand::List => app.invoke("service.list", json!({})),
        ServiceCommand::Available { name } => {
            app.invoke("service.available", json!({ "name": name }))
        }
        ServiceCommand::Install {
            name,
            version,
            port,
            force,
        } => app.invoke(
            "service.install",
            json!({ "name": name, "version": version, "port": port, "force": force }),
        ),
        ServiceCommand::Start { name } => app.invoke("service.start", json!({ "name": name })),
        ServiceCommand::Stop { name } => app.invoke("service.stop", json!({ "name": name })),
        ServiceCommand::Restart { name } => app.invoke("service.restart", json!({ "name": name })),
        ServiceCommand::Status { name } => app.invoke("service.status", json!({ "name": name })),
        ServiceCommand::Logs { name, lines } => {
            app.invoke("service.logs", json!({ "name": name, "lines": lines }))
        }
        ServiceCommand::Remove { name } => app.invoke("service.remove", json!({ "name": name })),
    }
}

/// Run a `devd web` subcommand: the generated web server configuration.
pub fn web(app: &App, cmd: WebCommand) -> Result<()> {
    match cmd {
        WebCommand::Render => app.invoke("web.render", json!({})),
        WebCommand::Test => app.invoke("web.test", json!({})),
        WebCommand::Reload => app.invoke("web.reload", json!({})),
    }
}

/// Run a `devd stack` subcommand.
pub fn stack(app: &App, cmd: StackCommand) -> Result<()> {
    match cmd {
        StackCommand::Up => app.invoke("stack.up", json!({})),
        StackCommand::Down => app.invoke("stack.down", json!({})),
    }
}

/// Run a `devd tls` subcommand.
pub fn tls(app: &App, cmd: TlsCommand) -> Result<()> {
    match cmd {
        TlsCommand::Ca => app.invoke("tls.ca.ensure", json!({})),
        TlsCommand::Show => app.invoke("tls.ca.show", json!({})),
        TlsCommand::Trust { machine } => app.invoke("tls.trust", json!({ "machine": machine })),
        TlsCommand::Untrust { machine } => app.invoke("tls.untrust", json!({ "machine": machine })),
        TlsCommand::Issue { host } => app.invoke("tls.issue", json!({ "host": host })),
        TlsCommand::List => app.invoke("tls.list", json!({})),
    }
}

/// Run a `devd db` subcommand.
pub fn db(app: &App, cmd: DbCommand) -> Result<()> {
    match cmd {
        DbCommand::List => app.invoke("db.list", json!({})),
        DbCommand::Create { name } => app.invoke("db.create", json!({ "name": name })),
        DbCommand::Drop { name } => app.invoke("db.drop", json!({ "name": name })),
    }
}

/// Run a `devd site` subcommand.
pub fn site(app: &App, cmd: SiteCommand) -> Result<()> {
    match cmd {
        SiteCommand::List => app.invoke("site.list", json!({})),
        SiteCommand::Create {
            name,
            host,
            root,
            no_tls,
            db,
        } => app.invoke(
            "site.create",
            json!({
                "name": name,
                "host": host,
                "root": root,
                "tls": !no_tls,
                "db": db,
            }),
        ),
        SiteCommand::Remove { name, delete_files } => app.invoke(
            "site.remove",
            json!({ "name": name, "delete_files": delete_files }),
        ),
    }
}

/// Run a `devd hosts` subcommand.
pub fn hosts(app: &App, cmd: HostsCommand) -> Result<()> {
    match cmd {
        HostsCommand::Show => app.invoke("hosts.show", json!({})),
        HostsCommand::Sync => app.invoke("hosts.sync", json!({})),
    }
}
