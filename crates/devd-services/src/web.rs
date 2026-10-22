//! The web server devd drives: the one the `web_server` setting names.
//!
//! Sites, TLS and PHP listeners are the same whichever server fronts them; only
//! the configuration language and the way a running server is told to re-read it
//! differ. Callers say "render", "test" and "reload" and do not care which.

use std::path::PathBuf;

use devd_core::capability::Ctx;
use devd_core::config::WebServer;
use devd_core::Result;

use crate::{apache, nginx};

/// The service name of the configured web server.
pub fn service_name(ctx: &Ctx) -> &'static str {
    match ctx.settings.web_server {
        WebServer::Nginx => "nginx",
        WebServer::Apache => "apache",
    }
}

/// Render the configured server's configuration from the state store.
pub fn render(ctx: &Ctx) -> Result<PathBuf> {
    match ctx.settings.web_server {
        WebServer::Nginx => nginx::render(ctx),
        WebServer::Apache => apache::render(ctx),
    }
}

/// Ask the configured server to check its generated configuration.
pub fn test(ctx: &Ctx) -> Result<String> {
    match ctx.settings.web_server {
        WebServer::Nginx => nginx::test(ctx),
        WebServer::Apache => apache::test(ctx),
    }
}

/// Make the running server read its configuration.
///
/// nginx re-reads it without dropping connections. Apache for Windows has no
/// signal a console process listens to, so it is restarted: the same
/// configuration, a second of downtime.
pub fn reload(ctx: &Ctx) -> Result<String> {
    match ctx.settings.web_server {
        WebServer::Nginx => nginx::reload(ctx),
        WebServer::Apache => {
            apache::test(ctx)?;
            let status = crate::restart(ctx, "apache")?;
            Ok(status.message)
        }
    }
}
