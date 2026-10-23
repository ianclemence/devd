//! Per-site nginx directives: the part of a site's `server` block that is yours.
//!
//! devd generates a site's block. What it cannot know (CORS headers, a bigger upload limit, a cache rule for static
//! files, a proxy to another program) goes in here, as nginx directives that are written into that block, checked with
//! `nginx -t`, and rolled back automatically when nginx rejects them. A few directives are refused because they would
//! break what devd itself manages (`listen`, `server_name`, the certificate) or reach outside the site (`include`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{DevdError, Registry, Result};

/// The kind under which a site's directives are stored.
pub const KIND: &str = "nginx_extra";

/// Directives that are refused, and why.
const REFUSED: &[(&str, &str)] = &[
    ("include", "it could read any file on this machine"),
    ("load_module", "it loads code into nginx"),
    ("listen", "devd owns the ports a site listens on"),
    ("server_name", "devd owns the site's name"),
    ("ssl_certificate", "devd owns the site's certificate"),
    ("ssl_certificate_key", "devd owns the site's certificate"),
    ("daemon", "it is a main-context directive"),
    ("pid", "it is a main-context directive"),
    ("user", "it is a main-context directive"),
    ("env", "it is a main-context directive"),
    ("worker_processes", "it is a main-context directive"),
    ("perl", "it runs code inside nginx"),
    ("perl_set", "it runs code inside nginx"),
    ("content_by_lua_block", "it runs code inside nginx"),
    ("access_by_lua_block", "it runs code inside nginx"),
    ("set_by_lua_block", "it runs code inside nginx"),
    ("init_by_lua_block", "it runs code inside nginx"),
];

/// Ready-made snippets: (name, what it does, directives).
pub const PRESETS: &[(&str, &str, &str)] = &[
    ("upload-100m", "accept uploads up to 100 MB", "client_max_body_size 100m;"),
    ("gzip", "compress text responses", "gzip on;\ngzip_types text/plain text/css application/json application/javascript text/xml application/xml image/svg+xml;\ngzip_min_length 1024;"),
    ("cache-static", "let browsers keep CSS, JavaScript, fonts and images for 30 days", "location ~* \\.(?:css|js|woff2?|ttf|svg|png|jpe?g|gif|webp|ico)$ {\n    expires 30d;\n    add_header Cache-Control \"public\";\n    access_log off;\n    try_files $uri =404;\n}"),
    ("cors", "allow any origin to read this site's responses (development only)", "add_header Access-Control-Allow-Origin * always;\nadd_header Access-Control-Allow-Methods \"GET, POST, PUT, PATCH, DELETE, OPTIONS\" always;\nadd_header Access-Control-Allow-Headers \"Authorization, Content-Type, X-Requested-With\" always;"),
    ("no-index", "ask search engines not to index this site", "add_header X-Robots-Tag \"noindex, nofollow\" always;"),
    ("security-headers", "the usual browser hardening headers", "add_header X-Frame-Options \"SAMEORIGIN\" always;\nadd_header X-Content-Type-Options \"nosniff\" always;\nadd_header Referrer-Policy \"strict-origin-when-cross-origin\" always;"),
    ("long-requests", "allow requests that take up to 10 minutes", "fastcgi_read_timeout 600s;\nproxy_read_timeout 600s;"),
];

/// Check directives before they go anywhere near a configuration file.
pub fn validate(text: &str) -> Result<()> {
    if text.len() > 8000 || text.contains('\0') || text.contains('\r') {
        return Err(DevdError::invalid("directives are text of at most 8000 characters, without NUL or carriage returns"));
    }
    let mut depth: i32 = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return Err(DevdError::invalid("a `}` closes a block that was never opened"));
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(DevdError::invalid("a `{` is never closed"));
    }
    // The first word of every statement and block.
    let stripped: String = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    for segment in stripped.split([';', '{', '}']) {
        let first = segment.split_whitespace().next().unwrap_or("");
        if let Some((_, why)) = REFUSED.iter().find(|(d, _)| *d == first) {
            return Err(DevdError::invalid(format!("`{first}` is not allowed here: {why}")));
        }
    }
    Ok(())
}

/// The directives of every site that has some.
pub fn extras_by_site(ctx: &Ctx) -> BTreeMap<String, String> {
    ctx.store.ops_list(KIND, None).unwrap_or_default().into_iter().filter_map(|i| i.data["directives"].as_str().map(|d| (i.site, d.to_string()))).collect()
}

/// The text to put in a site's server block, indented to sit there.
pub fn block_text(directives: &str) -> String {
    let mut out = String::from("        # your directives for this site (devd nginx.site_set)\n");
    for line in directives.lines() {
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str("        ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

/// The server blocks nginx will use for a hostname, taken from the generated configuration.
pub fn generated_blocks(conf: &str, host: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = format!("server_name  {host};");
    let mut from = 0;
    while let Some(rel) = conf[from..].find(&needle) {
        let at = from + rel;
        let Some(start) = conf[..at].rfind("server {") else { break };
        let mut depth = 0;
        let mut end = None;
        for (i, c) in conf[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push(conf[start..end].to_string());
        from = end;
    }
    out
}

fn site_named(ctx: &Ctx, site: &str) -> Result<devd_core::state::Site> {
    ctx.store.get_site(site)?.ok_or_else(|| DevdError::not_found(format!("no site named `{site}`")))
}

/// Put the saved directives live: render, check with nginx, reload when it is running. On a configuration nginx
/// rejects, put the previous directives back (or none), render again and say what nginx said.
fn apply(ctx: &Ctx, site: &str, previous: Option<String>) -> Result<()> {
    crate::web::render(ctx)?;
    if ctx.settings.web_server != devd_core::config::WebServer::Nginx {
        return Ok(());
    }
    if crate::nginx::exe_present(ctx) {
        if let Err(e) = crate::nginx::test(ctx) {
            match previous {
                Some(p) => ctx.store.ops_put(KIND, site, site, &json!({ "directives": p }))?,
                None => {
                    ctx.store.ops_delete(KIND, site)?;
                }
            }
            crate::web::render(ctx)?;
            return Err(DevdError::invalid(format!("nginx rejected those directives, so they were not kept: {e}")));
        }
        if crate::status(ctx, "nginx").map(|s| s.running).unwrap_or(false) {
            crate::web::reload(ctx)?;
        }
    }
    Ok(())
}

/// Save a site's directives (replacing any), and put them live.
pub fn set(ctx: &Ctx, site: &str, directives: &str) -> Result<()> {
    site_named(ctx, site)?;
    validate(directives)?;
    let previous = ctx.store.ops_get(KIND, site)?.and_then(|i| i.data["directives"].as_str().map(str::to_string));
    if directives.trim().is_empty() {
        ctx.store.ops_delete(KIND, site)?;
    } else {
        ctx.store.ops_put(KIND, site, site, &json!({ "directives": directives.trim_end() }))?;
    }
    apply(ctx, site, previous)
}

/// Arguments of `nginx.site_show` and `nginx.site_clear`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SiteArg {
    /// The site.
    pub site: String,
}

/// Arguments of `nginx.site_set`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetArgs {
    /// The site.
    pub site: String,
    /// nginx directives for the site's `server` block, written as in an nginx config. Replaces what the site had.
    pub directives: Option<String>,
    /// Names of presets to add to the directives (see `nginx.presets`).
    pub presets: Option<Vec<String>>,
}

/// Arguments of `nginx.presets`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NoArgs {}

fn compose(a: &SetArgs, current: &str) -> Result<String> {
    let mut text = match &a.directives {
        Some(d) => d.trim_end().to_string(),
        None if a.presets.is_some() => current.trim_end().to_string(),
        None => return Err(DevdError::invalid("give `directives`, or `presets` to add to the ones the site has")),
    };
    for name in a.presets.iter().flatten() {
        let (_, _, body) = PRESETS.iter().find(|(n, _, _)| n == name).ok_or_else(|| DevdError::invalid(format!("`{name}` is not a preset; use {}", PRESETS.iter().map(|p| p.0).collect::<Vec<_>>().join(", "))))?;
        if !text.contains(body) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(body);
        }
    }
    Ok(text)
}

/// Register the per-site nginx capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("nginx.presets", "List ready-made nginx snippets")
        .description("Snippets for a site's server block: bigger uploads, compression, long-lived caching of static files, CORS, no-index, security headers, long requests.")
        .group("nginx")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, _| Ok(Plan::new("nginx.presets", "list the presets")),
            |_, _| Ok(Outcome::with_data(format!("{} preset(s)", PRESETS.len()), json!({ "presets": PRESETS.iter().map(|(n, d, b)| json!({ "name": n, "does": d, "directives": b })).collect::<Vec<_>>(), "refused": REFUSED.iter().map(|(d, w)| json!({ "directive": d, "why": w })).collect::<Vec<_>>() }))),
        )?;

    reg.define::<SiteArg>("nginx.site_show", "Show a site's nginx configuration")
        .description("The directives that are yours, and the whole server block(s) devd generated for the site, so you can see them in place.")
        .group("nginx")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("nginx.site_show", format!("read the nginx configuration of {}", a.site))),
            |ctx, a| {
                let s = site_named(ctx, &a.site)?;
                let conf = std::fs::read_to_string(crate::nginx::config_path(ctx)).unwrap_or_default();
                let mine = extras_by_site(ctx).remove(&a.site);
                Ok(Outcome::with_data(
                    if mine.is_some() { "has its own directives".to_string() } else { "no custom directives".to_string() },
                    json!({ "site": a.site, "host": s.host, "directives": mine, "generated": generated_blocks(&conf, &s.host), "config_file": crate::nginx::config_path(ctx).display().to_string(), "web_server": ctx.settings.web_server.as_str() }),
                ))
            },
        )?;

    reg.define::<SetArgs>("nginx.site_set", "Give a site its own nginx directives")
        .description("Writes the directives (and/or presets) into the site's `server` block, checks the whole configuration with `nginx -t`, and reloads nginx. If nginx rejects them they are not kept and the previous ones are put back. `listen`, `server_name`, certificates and `include` are refused. With the Apache driver the directives are saved but not used. They change what the web server does, so this is an exec action.")
        .group("nginx")
        .tier(Tier::Exec)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |ctx, a| {
                site_named(ctx, &a.site)?;
                let current = extras_by_site(ctx).remove(&a.site).unwrap_or_default();
                let text = compose(a, &current)?;
                validate(&text)?;
                let mut plan = Plan::new("nginx.site_set", format!("change the nginx directives of {}", a.site)).step("check the configuration with nginx -t, and keep the old directives if it fails").step("reload nginx");
                for l in text.lines().filter(|l| !l.trim().is_empty()) {
                    plan = plan.step(format!("add: {l}"));
                }
                Ok(plan.effect("changes how the web server answers for this site"))
            },
            |ctx, a| {
                site_named(ctx, &a.site)?;
                let current = extras_by_site(ctx).remove(&a.site).unwrap_or_default();
                let text = compose(a, &current)?;
                set(ctx, &a.site, &text)?;
                Ok(Outcome::with_data(format!("{} has {} line(s) of its own nginx configuration", a.site, text.lines().filter(|l| !l.trim().is_empty()).count()), json!({ "site": a.site, "directives": text })))
            },
        )?;

    reg.define::<SiteArg>("nginx.site_clear", "Remove a site's own nginx directives")
        .description("Back to the generated configuration only. Checked with nginx -t and reloaded.")
        .group("nginx")
        .tier(Tier::Exec)
        .effect(Effect::Config)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("nginx.site_clear", format!("remove the custom nginx directives of {}", a.site))),
            |ctx, a| {
                set(ctx, &a.site, "")?;
                Ok(Outcome::with_data(format!("{} uses the generated configuration only", a.site), json!({ "site": a.site })))
            },
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmless_directives_pass_and_dangerous_ones_are_refused_with_a_reason() {
        for ok in ["client_max_body_size 100m;", "add_header X-A \"b\" always;", "location /api {\n    proxy_pass http://127.0.0.1:3000;\n}", "# only a comment\ngzip on;"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for (bad, word) in [("include /etc/passwd;", "include"), ("listen 8081;", "listen"), ("server_name evil.test;", "server_name"), ("ssl_certificate x.crt;", "ssl_certificate"), ("location / {\n include /x;\n}", "include"), ("load_module m.so;", "load_module"), ("perl_set $x 'sub {}';", "perl_set")] {
            let e = validate(bad).unwrap_err().to_string();
            assert!(e.contains(word), "{bad}: {e}");
        }
        assert!(validate("location / {").is_err() && validate("}").is_err() && validate("a;\0b;").is_err());
        assert!(validate(&"x;".repeat(5000)).is_err());
        // A word inside a value is not a directive: `include` here is part of a string.
        assert!(validate("add_header X-Note \"do not include secrets\";").is_ok());
    }

    #[test]
    fn the_block_is_indented_to_sit_inside_the_server_block() {
        let t = block_text("gzip on;\nlocation /a {\n    deny all;\n}");
        assert!(t.starts_with("        # your directives") && t.contains("        gzip on;\n") && t.contains("        location /a {\n            deny all;\n        }\n"), "{t}");
    }

    #[test]
    fn the_generated_block_of_a_host_is_found_by_its_name_with_nested_braces() {
        let conf = "http {\n    server {\n        listen 80;\n        server_name  a.test;\n        return 301 https://$host$request_uri;\n    }\n\n    server {\n        listen 443 ssl;\n        server_name  a.test;\n        location / {\n            try_files $uri =404;\n        }\n    }\n\n    server {\n        server_name  b.test;\n    }\n}\n";
        let a = generated_blocks(conf, "a.test");
        assert_eq!(a.len(), 2);
        assert!(a[1].contains("location /") && a[1].trim_end().ends_with('}') && !a[1].contains("b.test"));
        assert_eq!(generated_blocks(conf, "b.test").len(), 1);
        assert!(generated_blocks(conf, "c.test").is_empty());
    }

    #[test]
    fn presets_are_added_once_and_unknown_ones_are_refused() {
        let a = SetArgs { site: "s".into(), directives: None, presets: Some(vec!["gzip".into(), "upload-100m".into()]) };
        let t = compose(&a, "").unwrap();
        assert!(t.contains("gzip on;") && t.contains("client_max_body_size 100m;"));
        assert_eq!(compose(&a, &t).unwrap(), t, "adding the same presets again changes nothing");
        assert!(compose(&SetArgs { presets: Some(vec!["magic".into()]), ..a.clone() }, "").is_err());
        assert!(compose(&SetArgs { directives: None, presets: None, ..a }, "").is_err());
        for (_, _, body) in PRESETS {
            validate(body).unwrap();
        }
    }
}
