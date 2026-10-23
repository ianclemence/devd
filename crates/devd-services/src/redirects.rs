//! Redirects: send a path of a site somewhere else, rendered into the web server's configuration.
//!
//! `/old` redirects exactly that path. `/blog/*` redirects everything under `/blog/`, and the part
//! the `*` matched is `$1` in the target, so `/blog/*` to `/news/$1` sends `/blog/a/b` to `/news/a/b`.
//! The target is a path on the same site or an `https://` address.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{util, DevdError, Registry, Result};

/// The kind under which redirects are stored.
pub const KIND: &str = "redirect";

/// One redirect.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Redirect {
    /// Id.
    pub id: String,
    /// The site.
    pub site: String,
    /// The path to redirect, `/old` or `/blog/*`.
    pub from: String,
    /// Where to, a path or an https address. `$1` is what the `*` matched.
    pub to: String,
    /// 301, 302, 307 or 308.
    pub status: u16,
}

fn of(item: &devd_core::store_ops::OpsItem) -> Option<Redirect> {
    Some(Redirect {
        id: item.id.clone(),
        site: item.site.clone(),
        from: item.data["from"].as_str()?.to_string(),
        to: item.data["to"].as_str()?.to_string(),
        status: item.data["status"].as_u64().unwrap_or(301) as u16,
    })
}

/// Redirects, optionally one site's.
pub fn list(ctx: &Ctx, site: Option<&str>) -> Vec<Redirect> {
    ctx.store.ops_list(KIND, site).unwrap_or_default().iter().filter_map(of).collect()
}

/// Every site's redirects, by site name.
pub fn by_site(ctx: &Ctx) -> BTreeMap<String, Vec<Redirect>> {
    let mut m: BTreeMap<String, Vec<Redirect>> = BTreeMap::new();
    for r in list(ctx, None) {
        m.entry(r.site.clone()).or_default().push(r);
    }
    m
}

fn path_ok(from: &str) -> bool {
    from.starts_with('/') && from.len() <= 300 && from.chars().all(|c| c.is_ascii_alphanumeric() || "/_-.~%*".contains(c)) && from.matches('*').count() <= 1 && (!from.contains('*') || from.ends_with("/*"))
}

fn target_ok(to: &str) -> bool {
    let shape = to.starts_with('/') && !to.starts_with("//") || to.starts_with("https://");
    shape && to.len() <= 500 && !to.chars().any(|c| c.is_whitespace() || c.is_control() || "\"';{}\\".contains(c))
}

/// Check a redirect before it is stored.
pub fn validate(from: &str, to: &str, status: u16) -> Result<()> {
    if !path_ok(from) {
        return Err(DevdError::invalid("`from` is a path starting with `/` (letters, digits and / _ - . ~ %), or a folder ending in `/*`"));
    }
    if !target_ok(to) {
        return Err(DevdError::invalid("`to` is a path starting with `/` or an https:// address, with no spaces or quotes"));
    }
    if ![301, 302, 307, 308].contains(&status) {
        return Err(DevdError::invalid("the status is 301 or 308 (permanent), 302 or 307 (temporary)"));
    }
    Ok(())
}

fn escape_regex(s: &str) -> String {
    s.replace('.', "\\.")
}

/// nginx `location` blocks for a site's redirects. They go before `location /`.
pub fn nginx_block(list: &[Redirect]) -> String {
    let mut out = String::new();
    for r in list {
        if let Some(prefix) = r.from.strip_suffix("/*") {
            out.push_str(&format!("        location ~ ^{}/(.*)$ {{\n            return {} {};\n        }}\n\n", escape_regex(prefix), r.status, r.to));
        } else {
            out.push_str(&format!("        location = {} {{\n            return {} {};\n        }}\n\n", r.from, r.status, r.to));
        }
    }
    out
}

/// Apache directives for a site's redirects.
pub fn apache_block(list: &[Redirect]) -> String {
    let mut out = String::new();
    for r in list {
        if let Some(prefix) = r.from.strip_suffix("/*") {
            out.push_str(&format!("    RedirectMatch {} \"^{}/(.*)$\" \"{}\"\n", r.status, escape_regex(prefix), r.to));
        } else {
            out.push_str(&format!("    RedirectMatch {} \"^{}$\" \"{}\"\n", r.status, escape_regex(&r.from), r.to));
        }
    }
    out
}

/// Add a redirect, re-render the web configuration and reload the server if it is running.
pub fn add(ctx: &Ctx, site: &str, from: &str, to: &str, status: u16) -> Result<Redirect> {
    ctx.store.list_sites()?.into_iter().find(|s| s.name == site).ok_or_else(|| DevdError::not_found(format!("no site named `{site}`")))?;
    validate(from, to, status)?;
    if list(ctx, Some(site)).iter().any(|r| r.from == from) {
        return Err(DevdError::conflict(format!("{site} already redirects {from}; remove that redirect first")));
    }
    let id = util::new_id()[..8].to_string();
    ctx.store.ops_put(KIND, &id, site, &json!({ "from": from, "to": to, "status": status }))?;
    apply(ctx)?;
    list(ctx, Some(site)).into_iter().find(|r| r.id == id).ok_or_else(|| DevdError::internal("the redirect was not saved"))
}

/// Remove a redirect and re-render.
pub fn remove(ctx: &Ctx, id: &str) -> Result<()> {
    if !ctx.store.ops_delete(KIND, id)? {
        return Err(DevdError::not_found(format!("no redirect `{id}`; `redirects.list` shows them")));
    }
    apply(ctx)
}

fn apply(ctx: &Ctx) -> Result<()> {
    crate::web::render(ctx)?;
    let running = crate::status(ctx, crate::web::service_name(ctx)).map(|s| s.running).unwrap_or(false);
    if running {
        crate::web::reload(ctx)?;
    }
    Ok(())
}

/// Arguments of `redirects.add`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddArgs {
    /// The site.
    pub site: String,
    /// The path to redirect: `/old`, or `/blog/*` for everything under it.
    pub from: String,
    /// Where to: a path (`/new`, `/news/$1`) or an https:// address.
    pub to: String,
    /// 301 (default, permanent), 302, 307 or 308.
    pub status: Option<u16>,
}

/// Arguments of `redirects.list`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListArgs {
    /// Limit to one site.
    pub site: Option<String>,
}

/// Arguments of `redirects.remove`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RemoveArgs {
    /// The redirect id, from `redirects.list`.
    pub id: String,
}

/// Register the redirect capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<AddArgs>("redirects.add", "Redirect a path of a site")
        .description("Sends visitors of one path to another path or an https address, from the web server itself. `/blog/*` redirects everything under /blog/ and `$1` in the target is what the star matched. Re-renders the web server configuration and reloads it.")
        .group("redirects")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| {
                validate(&a.from, &a.to, a.status.unwrap_or(301))?;
                Ok(Plan::new("redirects.add", format!("redirect {} to {} on {}", a.from, a.to, a.site)).step("re-render the web server configuration").step("reload the web server"))
            },
            |ctx, a| {
                let r = add(ctx, &a.site, &a.from, &a.to, a.status.unwrap_or(301))?;
                Ok(Outcome::with_data(format!("{} now redirects to {}", r.from, r.to), json!(r)))
            },
        )?;

    reg.define::<ListArgs>("redirects.list", "List redirects")
        .description("The redirects of a site, or of all sites.")
        .group("redirects")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("redirects.list", "list the redirects")),
            |ctx, a| {
                let l = list(ctx, a.site.as_deref());
                Ok(Outcome::with_data(format!("{} redirect(s)", l.len()), json!({ "redirects": l })))
            },
        )?;

    reg.define::<RemoveArgs>("redirects.remove", "Remove a redirect")
        .description("Takes the redirect out of the web server configuration and reloads it.")
        .group("redirects")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("redirects.remove", format!("remove redirect {}", a.id)).step("re-render the web server configuration")),
            |ctx, a| {
                remove(ctx, &a.id)?;
                Ok(Outcome::with_data(format!("redirect {} removed", a.id), json!({ "id": a.id })))
            },
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(from: &str, to: &str, status: u16) -> Redirect {
        Redirect { id: "x".into(), site: "shop".into(), from: from.into(), to: to.into(), status }
    }

    #[test]
    fn an_exact_path_and_a_folder_render_for_nginx() {
        let text = nginx_block(&[r("/old", "/new", 301), r("/blog/*", "/news/$1", 302), r("/a.html", "https://example.com/", 308)]);
        assert!(text.contains("location = /old {\n            return 301 /new;"), "{text}");
        assert!(text.contains("location ~ ^/blog/(.*)$ {\n            return 302 /news/$1;"), "{text}");
        assert!(text.contains("location = /a.html"), "{text}");
    }

    #[test]
    fn they_render_for_apache_too() {
        let text = apache_block(&[r("/old", "/new", 301), r("/v1.0/*", "/v2/$1", 301)]);
        assert!(text.contains("RedirectMatch 301 \"^/old$\" \"/new\""), "{text}");
        assert!(text.contains("RedirectMatch 301 \"^/v1\\.0/(.*)$\" \"/v2/$1\""), "{text}");
    }

    #[test]
    fn anything_that_could_break_the_configuration_is_refused() {
        assert!(validate("/old", "/new", 301).is_ok());
        assert!(validate("/blog/*", "https://example.com/$1", 308).is_ok());
        for (from, to) in [("old", "/new"), ("/a b", "/n"), ("/a;b", "/n"), ("/a", "/n;\nreturn 200"), ("/a", "//evil.com"), ("/a", "http://plain.example"), ("/a/*/b", "/n"), ("/a", "/n\"x")] {
            assert!(validate(from, to, 301).is_err(), "{from} -> {to}");
        }
        assert!(validate("/a", "/b", 200).is_err());
    }
}
