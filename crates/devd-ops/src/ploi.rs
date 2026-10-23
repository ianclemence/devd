//! Ploi: the same operations on a real server, through Ploi's API.
//!
//! The API token is stored the way provider keys are: a file only the current user can read,
//! outside the database, the settings, the audit trail and every response. Reading the remote
//! `.env` returns its variables with secrets masked; deploying and changing the deploy script
//! are `remote` tier, so an agent can ask for them and only a person approves.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

/// The API.
const BASE: &str = "https://ploi.io/api";

/// The kind under which a local site's link to a Ploi site is stored.
pub const LINK: &str = "ploi_link";

fn token_file(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("secrets").join("ploi").join("token")
}

/// Store the API token with owner-only permissions.
pub fn save_token(ctx: &Ctx, token: &str) -> Result<()> {
    let token = token.trim();
    if token.len() < 20 || token.len() > 600 || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(DevdError::invalid("that does not look like a Ploi API token (20-600 characters, no spaces); create one at https://ploi.io/profile/api-keys"));
    }
    let path = token_file(ctx);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Forget the token.
pub fn forget_token(ctx: &Ctx) {
    let _ = std::fs::remove_file(token_file(ctx));
}

fn token(ctx: &Ctx) -> Result<String> {
    std::env::var("PLOI_API_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| std::fs::read_to_string(token_file(ctx)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
        .ok_or_else(|| DevdError::invalid("Ploi is not connected: create an API token at https://ploi.io/profile/api-keys and give it to `ploi.connect`"))
}

/// Whether a token is stored, and where it comes from.
pub fn source(ctx: &Ctx) -> &'static str {
    if std::env::var("PLOI_API_TOKEN").map(|t| !t.trim().is_empty()).unwrap_or(false) {
        "environment"
    } else if token_file(ctx).is_file() {
        "file"
    } else {
        "none"
    }
}

fn request(ctx: &Ctx, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    let token = token(ctx)?;
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(60)).build();
    let req = agent
        .request(method, &format!("{BASE}{path}"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .set("Content-Type", "application/json");
    let res = match body {
        Some(b) => req.send_string(&b.to_string()),
        None => req.call(),
    };
    match res {
        Ok(r) => {
            let text = r.into_string().map_err(|e| DevdError::internal(format!("Ploi's answer could not be read: {e}")))?;
            if text.trim().is_empty() {
                return Ok(json!({}));
            }
            serde_json::from_str(&text).map_err(|e| DevdError::internal(format!("Ploi's answer is not JSON: {e}")))
        }
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["message"].as_str().map(str::to_string)).unwrap_or_default();
            Err(match code {
                401 => DevdError::invalid("Ploi did not accept the API token; make a new one at https://ploi.io/profile/api-keys"),
                403 => DevdError::invalid(format!("the API token has no permission for this: give it the scope for this action in Ploi. {detail}")),
                404 => DevdError::not_found(format!("Ploi has no such server or site. {detail}")),
                429 => DevdError::unsupported("Ploi's rate limit was reached; wait a minute and try again"),
                other => DevdError::unsupported(format!("Ploi answered {other}. {detail}")),
            })
        }
        Err(e) => Err(DevdError::unsupported(format!("could not reach Ploi: {e}"))),
    }
}

fn id_ok(id: u64) -> Result<u64> {
    if id == 0 {
        return Err(DevdError::invalid("an id is a positive number"));
    }
    Ok(id)
}

/// The account behind the token.
pub fn user(ctx: &Ctx) -> Result<Value> {
    let v = request(ctx, "GET", "/user", None)?;
    Ok(v["data"].clone())
}

/// The servers.
pub fn servers(ctx: &Ctx) -> Result<Value> {
    let v = request(ctx, "GET", "/servers", None)?;
    let rows: Vec<Value> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({ "id": s["id"], "name": s["name"], "ip": s["ip_address"], "php": s["php_version"], "mysql": s["mysql_version"], "sites": s["sites_count"], "status": s["status"] }))
        .collect();
    Ok(json!(rows))
}

/// A server's sites.
pub fn sites(ctx: &Ctx, server: u64) -> Result<Value> {
    let v = request(ctx, "GET", &format!("/servers/{}/sites", id_ok(server)?), None)?;
    let rows: Vec<Value> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| json!({ "id": s["id"], "domain": s["domain"], "status": s["status"], "php": s["php_version"], "project_type": s["project_type"], "web_directory": s["web_directory"], "last_deploy_at": s["last_deploy_at"], "quick_deploy": s["quick_deploy"], "has_repository": s["has_repository"], "repository": s["repository"], "branch": s["branch"] }))
        .collect();
    Ok(json!(rows))
}

/// One site.
pub fn site(ctx: &Ctx, server: u64, site: u64) -> Result<Value> {
    Ok(request(ctx, "GET", &format!("/servers/{}/sites/{}", id_ok(server)?, id_ok(site)?), None)?["data"].clone())
}

/// Start a deployment of a site on the server.
pub fn deploy(ctx: &Ctx, server: u64, site: u64, branch: Option<&str>) -> Result<Value> {
    let body = branch.map(|b| json!({ "variables": { "branch": b } }));
    request(ctx, "POST", &format!("/servers/{}/sites/{}/deploy", id_ok(server)?, id_ok(site)?), body)
}

/// The site's `.env` as variables, secrets masked.
pub fn env(ctx: &Ctx, server: u64, site: u64) -> Result<std::collections::BTreeMap<String, String>> {
    let v = request(ctx, "GET", &format!("/servers/{}/sites/{}/env", id_ok(server)?, id_ok(site)?), None)?;
    Ok(crate::env::variables(v["content"].as_str().or_else(|| v["data"]["content"].as_str()).unwrap_or("")))
}

/// The site's deploy script on the server.
pub fn deploy_script(ctx: &Ctx, server: u64, site: u64) -> Result<String> {
    let v = request(ctx, "GET", &format!("/servers/{}/sites/{}/deploy/script", id_ok(server)?, id_ok(site)?), None)?;
    Ok(v["deploy_script"].as_str().or_else(|| v["data"]["deploy_script"].as_str()).unwrap_or("").to_string())
}

/// Replace the site's deploy script on the server.
pub fn set_deploy_script(ctx: &Ctx, server: u64, site: u64, script: &str) -> Result<Value> {
    request(ctx, "PATCH", &format!("/servers/{}/sites/{}/deploy/script", id_ok(server)?, id_ok(site)?), Some(json!({ "deploy_script": script })))
}

/// The cron jobs of a server.
pub fn crontabs(ctx: &Ctx, server: u64) -> Result<Value> {
    Ok(request(ctx, "GET", &format!("/servers/{}/crontabs", id_ok(server)?), None)?["data"].clone())
}

/// The queue workers of a site.
pub fn queues(ctx: &Ctx, server: u64, site: u64) -> Result<Value> {
    Ok(request(ctx, "GET", &format!("/servers/{}/sites/{}/queues", id_ok(server)?, id_ok(site)?), None)?["data"].clone())
}

/// The redirects of a site.
pub fn redirects(ctx: &Ctx, server: u64, site: u64) -> Result<Value> {
    Ok(request(ctx, "GET", &format!("/servers/{}/sites/{}/redirects", id_ok(server)?, id_ok(site)?), None)?["data"].clone())
}

/// Remember which Ploi site a local site corresponds to.
pub fn link(ctx: &Ctx, local: &str, server: u64, site: u64) -> Result<Value> {
    crate::site_named(ctx, local)?;
    let remote = self::site(ctx, server, site)?;
    ctx.store.ops_put(LINK, local, local, &json!({ "server": server, "site": site, "domain": remote["domain"] }))?;
    Ok(json!({ "local": local, "server": server, "site": site, "domain": remote["domain"] }))
}

/// The linked Ploi site of a local site.
pub fn linked(ctx: &Ctx, local: &str) -> Result<(u64, u64)> {
    let item = ctx.store.ops_get(LINK, local)?.ok_or_else(|| DevdError::invalid(format!("{local} is not linked to a Ploi site; `ploi.link` links it")))?;
    Ok((item.data["server"].as_u64().unwrap_or(0), item.data["site"].as_u64().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_must_be_positive() {
        assert!(id_ok(0).is_err());
        assert_eq!(id_ok(42).unwrap(), 42);
    }
}
