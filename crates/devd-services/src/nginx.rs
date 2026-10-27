//! nginx configuration, rendered from the state store.
//!
//! This is the inversion that makes `devd` different from a folder of hand-edited
//! `vhosts/*.conf` files: **the database is the truth and `nginx.conf` is output**.
//! Creating a site inserts a row, re-renders this file and reloads nginx, so the
//! configuration cannot drift from what `devd` believes, and "the site exists in
//! the store but not in the file" is not a state that can occur.
//!
//! Three details make the generated file correct on Windows rather than nearly
//! correct:
//!
//! * Every path is absolute and written with forward slashes. nginx on Windows
//!   treats a backslash as an escape character inside a configuration value.
//! * The prefix passed on the command line ends with a separator, which is what
//!   nginx requires for `-p`.
//! * `mime.types` and `fastcgi_params` are included from the *installed* nginx
//!   rather than from the layout, because they belong to the nginx build and
//!   change between releases.
//!
//! The only user-supplied text in the file is a hostname and a document root,
//! both validated when the site is created, and both are quoted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use devd_core::capability::Ctx;
use devd_core::state::Site;
use devd_core::util;
use devd_core::{DevdError, Result};

/// Everything the renderer needs that is not a site.
struct Render {
    /// nginx's prefix, with the trailing separator nginx requires.
    prefix: String,
    /// devd's log directory, in slash form.
    logs: String,
    /// devd's certificate directory, in slash form.
    certs: String,
    /// Port the plain-HTTP listener uses.
    http: u16,
    /// Port the TLS listener uses.
    https: u16,
    /// The PHP FastCGI listeners, as series and port.
    ///
    /// One per installed PHP version, so a site pinned to a version is served
    /// by that version and a site that is not pinned is served by the primary.
    php: Vec<(String, u16)>,
    /// How many FastCGI processes serve each series.
    workers: u8,
    /// Each site's redirects, by site name.
    redirects: std::collections::BTreeMap<String, Vec<crate::redirects::Redirect>>,
    /// Sites served by the pool that runs the other PHP limits profile.
    alt_sites: std::collections::BTreeSet<String>,
    /// Each site's own directives, by site name.
    extras: std::collections::BTreeMap<String, String>,
    /// The series the primary PHP version belongs to.
    primary: Option<String>,
}

/// Where nginx is installed under the layout.
pub fn directory(ctx: &Ctx) -> PathBuf {
    ctx.paths.services().join("nginx")
}

/// The generated configuration file.
pub fn config_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.etc().join("nginx").join("nginx.conf")
}

/// The PHP FastCGI listeners, as series and port.
///
/// The installed runtimes say which versions exist; the service rows say which
/// port each listener actually uses. Both are consulted, because a row left
/// behind by an uninstalled runtime must not keep a site on a dead upstream,
/// and a row's port is the one the supervisor started.
pub fn php_listeners(ctx: &Ctx) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = Vec::new();
    for series in crate::installed_php_series(ctx).unwrap_or_default() {
        let name = crate::php_service_name(&series);
        let port = ctx
            .store
            .get_service(&name)
            .ok()
            .flatten()
            .and_then(|s| s.port)
            .unwrap_or_else(|| crate::php_port_for_series(&series));
        out.push((series, port));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The port the PHP FastCGI listener is on, from the `php-cgi` service row.
///
/// The port of the primary listener, for callers that do not care which
/// version they are talking to.
pub fn php_port(ctx: &Ctx) -> u16 {
    ctx.store
        .get_service("php-cgi")
        .ok()
        .flatten()
        .and_then(|s| s.port)
        .or_else(|| {
            let primary = crate::primary_php_series(ctx)?;
            crate::php_port_for(ctx, &primary)
        })
        .unwrap_or(9000)
}

/// Whether nginx's program is installed, so a configuration can be checked.
pub fn exe_present(ctx: &Ctx) -> bool {
    directory(ctx).join(exe_name()).is_file()
}

/// nginx's prefix argument. The trailing separator is required by nginx.
pub fn prefix(install: &Path) -> String {
    slashy(install) + "/"
}

fn slashy(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn render_context(ctx: &Ctx) -> Render {
    Render {
        prefix: prefix(&directory(ctx)),
        logs: slashy(&ctx.paths.logs()),
        certs: slashy(&ctx.paths.certs()),
        http: crate::http_port(ctx),
        https: crate::https_port(ctx),
        php: php_listeners(ctx),
        workers: crate::php_workers(ctx),
        redirects: crate::redirects::by_site(ctx),
        alt_sites: crate::alt_sites(ctx),
        extras: crate::siteconf::extras_by_site(ctx),
        primary: crate::primary_php_series(ctx),
    }
}

/// Render `nginx.conf` from the state store and write it atomically.
pub fn render(ctx: &Ctx) -> Result<PathBuf> {
    let exe = directory(ctx).join(exe_name());
    if !exe.is_file() {
        return Err(DevdError::not_found(
            "nginx is not installed; run `devd service install nginx`",
        ));
    }
    let sites = ctx.store.list_sites()?;
    let text = render_text(render_context(ctx), &sites);
    let path = config_path(ctx);
    util::atomic_write(&path, &text)?;
    Ok(path)
}

/// The configuration text, as a pure function of its inputs.
///
/// Separate from the write so it can be asserted in tests and shown in a plan
/// without touching the disk or requiring nginx to be installed.
fn render_text(render: Render, sites: &[Site]) -> String {
    let mut out = String::with_capacity(2048 + sites.len() * 1200);
    out.push_str("# Generated by devd. Do not edit.\n");
    out.push_str("# The state store is the source of truth; this file is rewritten whenever a\n");
    out.push_str("# site or a setting changes. `devd web render` rewrites it, `devd web test`\n");
    out.push_str("# checks it with `nginx -t`, and `devd web reload` makes nginx read it.\n\n");
    out.push_str("worker_processes  1;\n");
    out.push_str(&format!(
        "error_log  \"{}/nginx-error.log\" warn;\n",
        render.logs
    ));
    out.push_str(&format!(
        "pid        \"{}logs/nginx.pid\";\n\n",
        render.prefix
    ));
    out.push_str("events {\n    worker_connections  1024;\n}\n\n");
    out.push_str("http {\n");
    out.push_str(&format!(
        "    include       \"{}conf/mime.types\";\n",
        render.prefix
    ));
    out.push_str("    default_type  application/octet-stream;\n\n");
    out.push_str(&format!(
        "    access_log  \"{}/nginx-access.log\";\n\n",
        render.logs
    ));
    if !cfg!(windows) {
        // A package builds nginx with temporary directories under /var/lib,
        // which an ordinary user cannot create, and nginx refuses to start when
        // it cannot. Keep them under devd's own prefix.
        for directive in [
            "client_body_temp_path",
            "proxy_temp_path",
            "fastcgi_temp_path",
            "uwsgi_temp_path",
            "scgi_temp_path",
        ] {
            let leaf = directive.trim_end_matches("_temp_path");
            out.push_str(&format!(
                "    {directive} \"{}temp/{leaf}\";\n",
                render.prefix
            ));
        }
        out.push('\n');
    }
    out.push_str("    sendfile           on;\n");
    out.push_str("    keepalive_timeout  65;\n");
    out.push_str("    server_tokens      off;\n");
    out.push_str("    client_max_body_size 2g;\n");
    out.push_str("    fastcgi_read_timeout 36000s;\n\n");
    out.push_str(
        "    # One PHP FastCGI listener per installed version. A site that is \
         pinned to a version is sent to that one; the rest go to the primary.\n",
    );
    for (series, port) in &render.php {
        // A pool: worker 1 is on the series' port, the others 100 apart (see `php_worker_port`).
        let servers: String = (0..render.workers)
            .map(|i| {
                format!(
                    "        server 127.0.0.1:{};\n",
                    port.saturating_add(100 * u16::from(i))
                )
            })
            .collect();
        out.push_str(&format!(
            "    upstream devd_php_{} {{\n{servers}    }}\n\n",
            upstream_name(series)
        ));
        if !render.alt_sites.is_empty() {
            // The pool that runs the other limits profile, 50 ports above.
            let alt: String = (0..render.workers)
                .map(|i| {
                    format!(
                        "        server 127.0.0.1:{};\n",
                        port.saturating_add(50 + 100 * u16::from(i))
                    )
                })
                .collect();
            out.push_str(&format!(
                "    upstream devd_php_{}_alt {{\n{alt}    }}\n\n",
                upstream_name(series)
            ));
        }
    }
    if sites.iter().any(|s| s.upstream.is_some()) {
        // Only a request that asks to upgrade keeps the connection open for it.
        out.push_str(
            "    map $http_upgrade $devd_connection_upgrade {\n        default upgrade;\n        ''      close;\n    }\n\n",
        );
    }
    out.push_str(&default_server(&render));
    for site in sites {
        out.push_str(&site_servers(&render, site));
    }
    out.push_str("}\n");
    out
}

/// The name of the upstream block for a PHP series.
///
/// A series is a version like `8.5`, which is not a valid identifier, so the
/// dot becomes an underscore.
fn upstream_name(series: &str) -> String {
    if series.is_empty() {
        return "default".to_string();
    }
    series.replace('.', "_")
}

/// The upstream a site is sent to.
///
/// A site that pins a version is sent to that version's listener. A site
/// that pins nothing is sent to the primary. A site that pins a version that
/// is not installed falls back to the primary rather than to a listener that
/// does not exist, because a missing upstream is a 502 for every request.
fn site_upstream(render: &Render, site: &Site) -> String {
    let base = site_upstream_base(render, site);
    if render.alt_sites.contains(&site.name) && base != "devd_php_default" {
        format!("{base}_alt")
    } else {
        base
    }
}

fn site_upstream_base(render: &Render, site: &Site) -> String {
    let pinned = site
        .php
        .as_deref()
        .map(crate::php_series)
        .filter(|s| !s.is_empty());
    if let Some(series) = pinned {
        if render.php.iter().any(|(s, _)| s == &series) {
            return format!("devd_php_{}", upstream_name(&series));
        }
    }
    match &render.primary {
        Some(primary) if render.php.iter().any(|(s, _)| s == primary) => {
            format!("devd_php_{}", upstream_name(primary))
        }
        // No primary is known, so the first listener takes every site.
        _ => match render.php.first() {
            Some((first, _)) => format!("devd_php_{}", upstream_name(first)),
            None => "devd_php_default".to_string(),
        },
    }
}

/// A request that matches no site gets a page that says so, rather than nginx's
/// default page or another project's content.
fn default_server(render: &Render) -> String {
    format!(
        "    server {{\n        listen       {} default_server;\n        server_name  _;\n        \
         return 404 \"No devd site is configured for this host. \
         Run `devd site list` to see what is.\\n\";\n    }}\n\n",
        render.http
    )
}

/// The `location` that hands every request to a program that is already
/// running.
///
/// The host the browser asked for is passed through unchanged, because dev
/// servers build absolute URLs and check origins from it. The upgrade headers
/// are what let a hot-reload WebSocket (Vite, webpack, Next) through, and the
/// read timeout is long for the same reason: an idle socket is not an error.
fn proxy_location(upstream: &str) -> String {
    format!(
        "        location / {{\n\
         \x20           proxy_pass          {upstream};\n\
         \x20           proxy_http_version  1.1;\n\
         \x20           proxy_set_header    Host               $host;\n\
         \x20           proxy_set_header    X-Real-IP          $remote_addr;\n\
         \x20           proxy_set_header    X-Forwarded-For    $proxy_add_x_forwarded_for;\n\
         \x20           proxy_set_header    X-Forwarded-Proto  $scheme;\n\
         \x20           proxy_set_header    X-Forwarded-Host   $host;\n\
         \x20           proxy_set_header    Upgrade            $http_upgrade;\n\
         \x20           proxy_set_header    Connection         $devd_connection_upgrade;\n\
         \x20           proxy_read_timeout  3600s;\n\
         \x20       }}\n"
    )
}

/// What a directory that serves nothing answers with. The folder is not exposed, and the
/// response says what the directory is and what to do, instead of a bare 404 or a listing of
/// source files. Only characters that are safe inside an nginx string are used.
fn no_entry_location(site: &Site, detection: Option<&devd_core::serving::Detection>) -> String {
    let what = detection
        .map(|d| d.language_names())
        .filter(|l| !l.is_empty())
        .map(|l| l.join(" + "))
        .unwrap_or_else(|| "no recognised project files".to_string());
    format!(
        "        location / {{\n\
         \x20           default_type text/plain;\n\
         \x20           return 404 \"devd: {name} has no web entry point ({what}): there is no index.php or index.html to serve. \
         If it is a program that listens on a port, point the site at it with: devd site upstream {name} <port>\\n\";\n\
         \x20       }}\n",
        name = site.name,
    )
}

/// The server blocks for one site.
fn site_servers(render: &Render, site: &Site) -> String {
    let root = slashy(Path::new(&site.document_root()));
    let serving = site.effective_serving();
    let kind = serving.serving;
    let host = &site.host;
    let tls = site.tls_mode != "off";
    let mut out = String::new();

    if tls {
        // Plain HTTP is redirected rather than served twice, so a browser and a
        // `curl http://...` both end up on the same URL the site is developed
        // against.
        out.push_str(&format!(
            "    server {{\n        listen       {};\n        server_name  {host};\n        \
             return 301 https://$host:{}$request_uri;\n    }}\n\n",
            render.http, render.https
        ));
    }

    out.push_str("    server {\n");
    if tls {
        out.push_str(&format!("        listen       {} ssl;\n", render.https));
    } else {
        out.push_str(&format!("        listen       {};\n", render.http));
    }
    out.push_str(&format!("        server_name  {host};\n"));
    if tls {
        let cert = format!("{}/{}", render.certs, host);
        out.push_str(&format!("        ssl_certificate      \"{cert}.crt\";\n"));
        out.push_str(&format!("        ssl_certificate_key  \"{cert}.key\";\n"));
        out.push_str("        ssl_protocols        TLSv1.2 TLSv1.3;\n");
        out.push_str("        ssl_session_cache    shared:devd:2m;\n");
        out.push_str("        ssl_session_timeout  10m;\n");
    }
    match kind {
        devd_core::serving::Serving::Php => {
            out.push_str(&format!("        root         \"{root}\";\n"));
            out.push_str("        index        index.php index.html index.htm;\n");
        }
        devd_core::serving::Serving::Static => {
            out.push_str(&format!("        root         \"{root}\";\n"));
            out.push_str("        index        index.html index.htm;\n");
        }
        // A proxy site serves a program; a directory with no entry point serves nothing, and
        // naming its folder as the root would let the web server hand out whatever is in it.
        devd_core::serving::Serving::Proxy | devd_core::serving::Serving::NoEntry => {}
    }
    out.push_str(&format!(
        "        access_log   \"{}/nginx-{host}.access.log\";\n",
        render.logs
    ));
    out.push_str(&format!(
        "        error_log    \"{}/nginx-{host}.error.log\";\n\n",
        render.logs
    ));
    if let Some(upstream) = &site.upstream {
        out.push_str(&proxy_location(upstream));
        out.push_str("    }\n\n");
        return out;
    }
    if kind == devd_core::serving::Serving::NoEntry {
        out.push_str(&no_entry_location(site, serving.detection.as_ref()));
        out.push_str("    }\n\n");
        return out;
    }
    if let Some(text) = render.extras.get(&site.name) {
        out.push_str(&crate::siteconf::block_text(text));
    }
    if let Some(list) = render.redirects.get(&site.name) {
        out.push_str(&crate::redirects::nginx_block(list));
    }
    if kind == devd_core::serving::Serving::Static {
        out.push_str("        location / {\n            try_files $uri $uri/ =404;\n        }\n\n");
    } else {
        out.push_str("        location / {\n            try_files $uri $uri/ /index.php?$query_string;\n        }\n\n");
        out.push_str("        location ~ \\.php$ {\n");
        out.push_str("            try_files $uri =404;\n");
        out.push_str(&format!(
            "            fastcgi_pass  {};\n",
            site_upstream(render, site)
        ));
        out.push_str("            fastcgi_index index.php;\n");
        out.push_str(&format!(
            "            include       \"{}conf/fastcgi_params\";\n",
            render.prefix
        ));
        out.push_str(
            "            fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;\n",
        );
        out.push_str("        }\n\n");
    }
    out.push_str("        # Certificate challenges and well-known files stay reachable;\n");
    out.push_str("        # every other dotfile is not served.\n");
    out.push_str("        location ~ /\\.(?!well-known) {\n            deny all;\n        }\n");
    out.push_str("    }\n\n");
    out
}

pub(crate) fn exe_name() -> &'static str {
    if cfg!(windows) {
        "nginx.exe"
    } else {
        "sbin/nginx"
    }
}

/// Run nginx against the generated configuration with extra arguments.
fn invoke(ctx: &Ctx, extra: &[&str]) -> Result<String> {
    let dir = directory(ctx);
    let exe = dir.join(exe_name());
    if !exe.is_file() {
        return Err(DevdError::not_found(
            "nginx is not installed; run `devd service install nginx`",
        ));
    }
    let mut args = vec![
        "-p".to_string(),
        prefix(&dir),
        "-c".to_string(),
        slashy(&config_path(ctx)),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let out = util::run(&exe, &args, Some(&dir), &[], Some(Duration::from_secs(20)))?;
    // nginx writes everything, including "syntax is ok", to standard error.
    Ok(format!("{}\n{}", out.stdout, out.stderr).trim().to_string())
}

/// `nginx -t`: parse the generated file and report what nginx says.
pub fn test(ctx: &Ctx) -> Result<String> {
    let output = invoke(ctx, &["-t"])?;
    if !output.contains("syntax is ok") || output.contains("[emerg]") {
        return Err(DevdError::Command {
            program: "nginx -t".to_string(),
            code: 1,
            stderr: util::truncate(output.trim(), 2000),
        });
    }
    Ok(output)
}

/// `nginx -s reload`: make nginx re-read the configuration without dropping
/// connections.
pub fn reload(ctx: &Ctx) -> Result<String> {
    invoke(ctx, &["-s", "reload"])
}

/// Version of the installed nginx, from `nginx -v`.
pub fn version(install: &Path) -> Option<String> {
    let exe = install.join(exe_name());
    if !exe.is_file() {
        return None;
    }
    let out = util::run(
        &exe,
        &["-v".to_string()],
        Some(install),
        &[],
        Some(Duration::from_secs(15)),
    )
    .ok()?;
    // `nginx version: nginx/1.28.3` on stderr.
    let text = format!("{} {}", out.stdout, out.stderr);
    let rest = text.split("nginx/").nth(1)?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(name: &str, tls: &str) -> Site {
        Site {
            id: devd_core::util::new_id(),
            name: name.to_string(),
            host: format!("{name}.test"),
            root: format!("C:/devd-home/www/{name}"),
            web_server: "nginx".to_string(),
            php: Some("8.3.35".to_string()),
            tls_mode: tls.to_string(),
            db_name: None,
            upstream: None,
            serving: Some(devd_core::serving::Content::Php),
            created_at: devd_core::util::now(),
        }
    }

    /// A site pinned to a PHP series.
    fn site_on(name: &str, tls: &str, series: Option<&str>) -> Site {
        let mut s = site(name, tls);
        s.php = series.map(|v| v.to_string());
        s
    }

    fn render_for(sites: &[Site], http: u16, https: u16) -> String {
        render_text(
            Render {
                prefix: "D:/devd-home/services/nginx/".to_string(),
                logs: "D:/devd-home/logs".to_string(),
                certs: "D:/devd-home/certs".to_string(),
                http,
                https,
                php: vec![("8.3".to_string(), 9000), ("8.5".to_string(), 9085)],
                workers: 1,
                redirects: Default::default(),
                alt_sites: Default::default(),
                extras: Default::default(),
                primary: Some("8.3".to_string()),
            },
            sites,
        )
    }

    fn proxy(name: &str, tls: &str, upstream: &str) -> Site {
        let mut s = site(name, tls);
        s.php = None;
        s.upstream = Some(upstream.to_string());
        s
    }

    #[test]
    fn a_proxy_site_forwards_everything_and_serves_no_files() {
        let text = render_for(
            &[proxy("app", "internal", "http://127.0.0.1:5173")],
            8080,
            8443,
        );
        assert!(
            text.contains("proxy_pass          http://127.0.0.1:5173;"),
            "{text}"
        );
        // The browser's host reaches the program, and a hot-reload socket can upgrade.
        assert!(
            text.contains("proxy_set_header    Host               $host;"),
            "{text}"
        );
        assert!(
            text.contains("proxy_set_header    Upgrade            $http_upgrade;"),
            "{text}"
        );
        assert!(
            text.contains("map $http_upgrade $devd_connection_upgrade"),
            "{text}"
        );
        // Still HTTPS, with the certificate devd issued.
        assert!(text.contains("listen       8443 ssl;"), "{text}");
        assert!(
            text.contains("ssl_certificate      \"D:/devd-home/certs/app.test.crt\";"),
            "{text}"
        );
        // None of the directory-site machinery is emitted for it.
        let block = &text[text.find("server_name  app.test;").unwrap()..];
        assert!(!block.contains("fastcgi_pass"), "{text}");
        assert!(!block.contains("try_files"), "{text}");
        assert!(
            !block.contains("root         \"C:/devd-home/www/app\""),
            "{text}"
        );
    }

    #[test]
    fn the_upgrade_map_is_only_written_when_a_proxy_site_needs_it() {
        let plain = render_for(&[site("blog", "off")], 8080, 8443);
        assert!(!plain.contains("devd_connection_upgrade"), "{plain}");
        let mixed = render_for(
            &[
                site("blog", "off"),
                proxy("app", "off", "http://localhost:3000"),
            ],
            8080,
            8443,
        );
        assert_eq!(mixed.matches("map $http_upgrade").count(), 1, "{mixed}");
        // The directory site beside it is unchanged.
        assert!(mixed.contains("fastcgi_pass  devd_php_8_3;"), "{mixed}");
    }

    #[test]
    fn a_tls_site_redirects_and_serves_php() {
        let text = render_for(&[site("blog", "internal")], 8080, 8443);
        // The redirect, on the plain port.
        assert!(text.contains("listen       8080;"), "{text}");
        assert!(
            text.contains("return 301 https://$host:8443$request_uri;"),
            "{text}"
        );
        // The TLS listener, with the certificate devd issued.
        assert!(text.contains("listen       8443 ssl;"), "{text}");
        assert!(
            text.contains("ssl_certificate      \"D:/devd-home/certs/blog.test.crt\";"),
            "{text}"
        );
        assert!(
            text.contains("ssl_certificate_key  \"D:/devd-home/certs/blog.test.key\";"),
            "{text}"
        );
        // FastCGI over the upstream, not a hard-coded socket.
        assert!(text.contains("fastcgi_pass  devd_php_8_3;"), "{text}");
        assert!(text.contains("server 127.0.0.1:9000;"), "{text}");
        assert!(
            text.contains("fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;"),
            "{text}"
        );
        assert!(text.contains("include       \"D:/devd-home/services/nginx/conf/fastcgi_params\";"));
        // Document root, with forward slashes.
        assert!(
            text.contains("root         \"C:/devd-home/www/blog\";"),
            "{text}"
        );
        // The 404 body deliberately contains a `\n` for nginx to interpret, so
        // the assertion is about paths: no drive letter may be followed by a
        // backslash, which is how a Windows path reaches nginx when the renderer
        // forgets to convert it.
        assert!(
            !text.contains(":\\"),
            "no Windows path separator may reach nginx: {text}"
        );
    }

    #[test]
    fn php_is_served_by_a_pool_of_workers() {
        let text = render_text(
            Render {
                prefix: "D:/devd-home/services/nginx/".to_string(),
                logs: "D:/devd-home/logs".to_string(),
                certs: "D:/devd-home/certs".to_string(),
                http: 80,
                https: 443,
                php: vec![("8.4".to_string(), 9084)],
                workers: 3,
                redirects: Default::default(),
                alt_sites: Default::default(),
                extras: Default::default(),
                primary: Some("8.4".to_string()),
            },
            &[site("shop", "internal")],
        );
        let upstream = &text[text.find("upstream devd_php_8_4 {").unwrap()..];
        let upstream = &upstream[..upstream.find('}').unwrap()];
        for port in [9084, 9184, 9284] {
            assert!(
                upstream.contains(&format!("server 127.0.0.1:{port};")),
                "{upstream}"
            );
        }
    }

    #[test]
    fn a_site_with_the_other_limits_profile_has_its_own_upstream() {
        let mut shop = site("shop", "internal");
        shop.php = Some("8.4".to_string());
        let text = render_text(
            Render {
                prefix: "D:/devd-home/services/nginx/".to_string(),
                logs: "D:/devd-home/logs".to_string(),
                certs: "D:/devd-home/certs".to_string(),
                http: 80,
                https: 443,
                php: vec![("8.4".to_string(), 9084)],
                workers: 2,
                redirects: Default::default(),
                alt_sites: std::collections::BTreeSet::from(["shop".to_string()]),
                extras: Default::default(),
                primary: Some("8.4".to_string()),
            },
            &[shop, site("blog", "internal")],
        );
        let alt = &text[text
            .find("upstream devd_php_8_4_alt {")
            .expect("the other pool is declared")..];
        let alt = &alt[..alt.find('}').unwrap()];
        assert!(
            alt.contains("server 127.0.0.1:9134;") && alt.contains("server 127.0.0.1:9234;"),
            "{alt}"
        );
        assert!(
            text.contains("fastcgi_pass  devd_php_8_4_alt;"),
            "shop uses it: {text}"
        );
        assert!(
            text.contains("fastcgi_pass  devd_php_8_4;"),
            "blog stays on the main pool: {text}"
        );
    }

    #[test]
    fn a_site_without_tls_is_served_over_http_only() {
        let text = render_for(&[site("plain", "off")], 8080, 8443);
        assert!(text.contains("listen       8080;"), "{text}");
        assert!(!text.contains("return 301"), "{text}");
        assert!(!text.contains("ssl_certificate"), "{text}");
        assert!(text.contains("server_name  plain.test;"), "{text}");
    }

    #[test]
    fn every_site_gets_its_own_logs() {
        let text = render_for(&[site("a", "off"), site("b", "off")], 80, 443);
        assert!(text.contains("nginx-a.test.access.log"), "{text}");
        assert!(text.contains("nginx-b.test.access.log"), "{text}");
    }

    #[test]
    fn a_pinned_site_reaches_its_own_php_version() {
        let text = render_for(
            &[
                site_on("pinned", "off", Some("8.5")),
                site_on("free", "off", None),
            ],
            80,
            443,
        );
        // The pinned site is sent to the 8.5 listener.
        assert!(text.contains("fastcgi_pass  devd_php_8_5;"), "{text}");
        // The unpinned site follows the primary.
        assert!(text.contains("fastcgi_pass  devd_php_8_3;"), "{text}");
        // One upstream per installed version.
        assert!(text.contains("upstream devd_php_8_5 {"), "{text}");
        assert!(text.contains("upstream devd_php_8_3 {"), "{text}");
        assert!(text.contains("server 127.0.0.1:9085;"), "{text}");
    }

    #[test]
    fn a_patch_release_is_the_same_series_as_its_minor() {
        // A site that records 8.5.11 is served by the 8.5 listener, because a
        // patch release is not a different thing to a site.
        let text = render_for(&[site_on("patch", "off", Some("8.5.11"))], 80, 443);
        assert!(text.contains("fastcgi_pass  devd_php_8_5;"), "{text}");
    }

    #[test]
    fn a_site_pinned_to_a_version_that_is_not_installed_falls_back() {
        // A missing upstream is a 502 for every request, so the site is
        // served by the primary instead.
        let text = render_for(&[site_on("gone", "off", Some("7.4"))], 80, 443);
        assert!(text.contains("fastcgi_pass  devd_php_8_3;"), "{text}");
        assert!(!text.contains("devd_php_7_4"), "{text}");
    }

    #[test]
    fn unmatched_hosts_get_an_explanatory_404() {
        let text = render_for(&[], 80, 443);
        assert!(text.contains("listen       80 default_server;"), "{text}");
        assert!(text.contains("No devd site is configured"), "{text}");
    }

    #[test]
    fn the_prefix_is_quoted_and_slash_terminated() {
        assert_eq!(
            prefix(Path::new("C:\\devd-home\\services\\nginx")),
            "C:/devd-home/services/nginx/"
        );
        assert_eq!(
            prefix(Path::new("/opt/devd/services/nginx")),
            "/opt/devd/services/nginx/"
        );
    }

    #[test]
    fn a_document_root_of_public_is_used_when_it_exists() {
        // The store decides the document root; this asserts the renderer simply
        // uses what it is given, including a nested path.
        let mut s = site("laravel", "off");
        s.root = "C:/devd-home/www/laravel/public".to_string();
        let text = render_for(&[s], 80, 443);
        assert!(
            text.contains("root         \"C:/devd-home/www/laravel/public\";"),
            "{text}"
        );
    }

    /// A site whose directory is really on disk, so what it serves is read from its files.
    fn on_disk(tag: &str, name: &str, files: &[&str]) -> Site {
        let dir = std::env::temp_dir()
            .join(format!("devd-nginx-{tag}-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let mut s = site(name, "internal");
        s.php = None;
        s.serving = None;
        s.root = dir.display().to_string();
        s
    }

    fn block_of<'a>(text: &'a str, name: &str) -> &'a str {
        let start = text
            .find(&format!(
                "listen       8443 ssl;\n        server_name  {name}.test;"
            ))
            .expect("the site's TLS block");
        &text[start..]
    }

    #[test]
    fn a_site_that_serves_php_gets_php_and_one_that_serves_files_does_not() {
        let php = on_disk("php", "blog", &["index.php"]);
        let files = on_disk("files", "brochure", &["index.html"]);
        let text = render_for(&[php, files], 8080, 8443);
        assert!(block_of(&text, "blog").contains("fastcgi_pass"), "{text}");
        let brochure = block_of(&text, "brochure");
        let brochure = &brochure[..brochure.find("    }\n\n").unwrap_or(brochure.len())];
        assert!(
            !brochure.contains("fastcgi_pass"),
            "a static site has no PHP in its configuration:\n{brochure}"
        );
        assert!(
            brochure.contains("try_files $uri $uri/ =404;"),
            "{brochure}"
        );
        assert!(
            brochure.contains("index        index.html index.htm;"),
            "{brochure}"
        );
    }

    #[test]
    fn a_directory_with_no_web_entry_point_is_not_rooted_and_says_why() {
        let ghost = on_disk("noentry", "ghost", &["go.mod", "main.go"]);
        let text = render_for(&[ghost], 8080, 8443);
        let block = block_of(&text, "ghost");
        let block = &block[..block.find("    }\n\n").unwrap_or(block.len())];
        assert!(
            !block.contains("root "),
            "the folder must not be handed to the web server:\n{block}"
        );
        assert!(
            !block.contains("fastcgi_pass") && !block.contains("try_files"),
            "{block}"
        );
        assert!(
            block.contains("return 404 \"devd: ghost has no web entry point (go)"),
            "{block}"
        );
        assert!(
            block.contains("devd site upstream ghost <port>"),
            "it says what to do: {block}"
        );
    }

    #[test]
    fn a_php_pin_on_a_site_that_does_not_serve_php_reaches_no_listener() {
        let mut brochure = on_disk("pin", "brochure", &["index.html"]);
        brochure.php = Some("8.5".to_string());
        let text = render_for(&[brochure], 8080, 8443);
        let block = block_of(&text, "brochure");
        assert!(
            !block.contains("devd_php_8_5") && !block.contains("fastcgi_pass"),
            "{block}"
        );
    }

    #[test]
    fn what_a_person_chose_overrides_what_the_files_say() {
        let mut ghost = on_disk("chosen", "ghost", &["go.mod"]);
        ghost.serving = Some(devd_core::serving::Content::Static);
        let text = render_for(&[ghost], 8080, 8443);
        assert!(
            block_of(&text, "ghost").contains("try_files $uri $uri/ =404;"),
            "{text}"
        );
    }

    #[test]
    fn starting_the_web_server_replaces_a_configuration_an_older_devd_wrote() {
        use devd_core::capability::{Actor, Ctx};
        use devd_core::config::Settings;
        use devd_core::events::EventBus;
        use devd_core::paths::Paths;
        use devd_core::state::Store;
        use std::sync::Arc;

        let dir = std::env::temp_dir().join(format!("devd-start-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let ctx = Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::system(),
            run: None,
            dry_run: false,
        };
        // nginx "installed": the renderer only needs the program to exist. The
        // executable sits in `sbin/` off Windows, so its folder comes first.
        let exe = directory(&ctx).join(exe_name());
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(exe, b"").unwrap();
        // A site that serves nothing, and a configuration file the previous devd wrote for it.
        let go = dir.join("www").join("ghost");
        std::fs::create_dir_all(&go).unwrap();
        std::fs::write(go.join("go.mod"), "module ghost").unwrap();
        let mut s = site("ghost", "internal");
        s.php = Some("8.5.12".to_string());
        s.serving = None;
        s.root = go.display().to_string();
        ctx.store.upsert_site(&s).unwrap();
        std::fs::create_dir_all(config_path(&ctx).parent().unwrap()).unwrap();
        std::fs::write(
            config_path(&ctx),
            "# written by 0.11: ghost { fastcgi_pass devd_php_8_5; root ghost; }\n",
        )
        .unwrap();

        crate::refresh_web_config(&ctx, "nginx").unwrap();

        let text = std::fs::read_to_string(config_path(&ctx)).unwrap();
        assert!(!text.contains("written by 0.11"), "the stale file is gone");
        assert!(text.contains("ghost has no web entry point"), "{text}");
        assert!(
            crate::refresh_web_config(&ctx, "mariadb").is_ok(),
            "a service that has no configuration of this kind is left alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
