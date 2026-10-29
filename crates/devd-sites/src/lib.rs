//! Sites: a hostname, a directory, a certificate and a web server block.
//!
//! Creating a site is one operation because the four things it produces are not
//! useful separately. It:
//!
//! 1. creates the project directory (and a working `index.php` if the directory
//!    is empty, so the site answers immediately rather than 403-ing),
//! 2. issues a certificate for the hostname from the local authority,
//! 3. records the site in the state store, and
//! 4. re-renders the web server configuration and reloads it if it is running.
//!
//! It deliberately does **not** edit the hosts file. That is `hosts.sync`, an
//! `admin`-tier operation, because it changes how the whole machine resolves
//! names; keeping it separate means creating a site is a `write`-tier action
//! that does not need an elevated approval.
//!
//! The document root handed to the web server comes from
//! [`Site::document_root`](devd_core::state::Site::document_root), which prefers
//! a `public/` subdirectory when one exists. That is what makes Laravel, Symfony
//! and a plain `index.php` all work without a per-framework setting.

pub mod caps;
pub mod dns;
pub mod hosts;
pub mod htaccess;
pub mod park;
pub mod verify;

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::doctor::Check;
use devd_core::paths::Paths;
use devd_core::state::{Site, Store};
use devd_core::util;
use devd_core::{DevdError, Result};

/// How a site's TLS is provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    /// Served over plain HTTP only.
    Off,
    /// A certificate from devd's own authority.
    Internal,
}

impl Tls {
    /// The value stored in the site row.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Internal => "internal",
        }
    }

    /// Parse from stored text, defaulting to the safe answer for anything else.
    ///
    /// Before TLS is requested the site is plain HTTP; that is the state that
    /// behaves correctly without a certificate, so it is the fallback.
    pub fn parse(value: &str) -> Self {
        match value {
            "internal" | "trusted" => Self::Internal,
            _ => Self::Off,
        }
    }
}

/// What `site.create` produced.
#[derive(Debug, Clone, Serialize)]
pub struct Created {
    /// The recorded site.
    pub site: Site,
    /// Project directory.
    pub directory: String,
    /// Document root the web server was given.
    pub document_root: String,
    /// The URL the site is served at.
    pub url: String,
    /// Certificate details, when TLS was requested.
    pub certificate: Option<devd_services::tls::Issued>,
    /// Whether the web server was reloaded.
    pub reloaded: bool,
    /// Database created alongside, when one was asked for.
    pub database: Option<String>,
    /// Anything the caller should know.
    pub warnings: Vec<String>,
}

/// The directory projects are created in: `www_root` when it is set, otherwise
/// the layout's own `www`.
pub fn www_root(ctx: &Ctx) -> PathBuf {
    ctx.settings.effective_www_root(&ctx.paths.www())
}

/// The default project directory for a site.
pub fn default_directory(ctx: &Ctx, name: &str) -> PathBuf {
    www_root(ctx).join(name)
}

/// Whether a hostname resolves without an entry in the hosts file.
///
/// `localhost` and its subdomains are mapped to loopback by the operating
/// system and by browsers, so a site published there needs no elevation. This
/// is why the first-run site uses it.
pub fn resolves_without_hosts(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// The URL a site is served at.
pub fn url_for(ctx: &Ctx, site: &Site) -> String {
    let tls = Tls::parse(&site.tls_mode);
    match tls {
        Tls::Off => format!("http://{}:{}", site.host, ctx.settings.http_port),
        Tls::Internal => format!("https://{}:{}", site.host, ctx.settings.https_port),
    }
}

/// The path a site's certificate chain is written to.
pub fn certificate_path(ctx: &Ctx, host: &str) -> PathBuf {
    devd_services::tls::cert_path_for_host(ctx, host)
}

/// The path a site's private key is written to.
pub fn key_path(ctx: &Ctx, host: &str) -> PathBuf {
    devd_services::tls::key_path_for_host(ctx, host)
}

/// Turn what a person typed into the address nginx will proxy to.
///
/// `3000`, `localhost:5173`, `http://127.0.0.1:8000` and `https://localhost:8443`
/// are all accepted. The result is always `scheme://host:port` with nothing
/// else on it.
///
/// Only a program on this machine can be a site's upstream. A proxy site hands
/// every request that reaches the hostname to the address, so an address that
/// pointed elsewhere would make devd a relay for whatever answered there; a
/// program on another machine is a deliberate decision for a configuration
/// file, not a field. The text ends up inside an nginx configuration, so
/// anything that is not part of a plain address is refused rather than quoted.
pub fn normalize_upstream(raw: &str) -> Result<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(DevdError::invalid(
            "an upstream is the address of a program that is running, such as 3000 or \
             http://127.0.0.1:5173",
        ));
    }
    if text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "\"'`;{}$\\@?#|<>*".contains(c))
    {
        return Err(DevdError::invalid(format!(
            "`{text}` is not a plain address; use a port, or scheme://host:port"
        )));
    }

    // A bare port means "on this machine".
    if text.chars().all(|c| c.is_ascii_digit()) {
        let port = parse_port(text)?;
        return Ok(format!("http://127.0.0.1:{port}"));
    }

    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("http".to_string(), text),
    };
    if scheme != "http" && scheme != "https" {
        return Err(DevdError::invalid(format!(
            "an upstream is spoken to over http or https, not {scheme}"
        )));
    }
    let rest = rest.trim_end_matches('/');
    if rest.contains('/') {
        return Err(DevdError::invalid(
            "an upstream is an address without a path: every request is forwarded, so put \
             the path in the browser",
        ));
    }

    let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
        let (host, after) = inner
            .split_once(']')
            .ok_or_else(|| DevdError::invalid("an IPv6 address needs a closing `]`"))?;
        let port = match after.strip_prefix(':') {
            Some(p) => parse_port(p)?,
            None if after.is_empty() => default_port(&scheme),
            None => return Err(DevdError::invalid(format!("`{rest}` is not an address"))),
        };
        (format!("[{host}]"), port)
    } else {
        match rest.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), parse_port(port)?),
            None => (rest.to_string(), default_port(&scheme)),
        }
    };

    let loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")
        || host.ends_with(".localhost")
        || host.starts_with("127.")
            && host.split('.').count() == 4
            && host.split('.').all(|p| p.parse::<u8>().is_ok());
    if !loopback {
        return Err(DevdError::invalid(format!(
            "{host} is not on this machine. A proxy site forwards to a program on \
             127.0.0.1 or localhost"
        )));
    }
    Ok(format!("{scheme}://{host}:{port}"))
}

fn default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

fn parse_port(text: &str) -> Result<u16> {
    match text.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(DevdError::invalid(format!(
            "`{text}` is not a port between 1 and 65535"
        ))),
    }
}

/// The port of a normalised upstream.
fn upstream_port(upstream: &str) -> Option<u16> {
    upstream.rsplit(':').next()?.parse().ok()
}

/// Whether something is accepting connections at a site's upstream.
///
/// `None` for a site that is not a proxy. A dev server that is not running is
/// the ordinary state of a proxy site, so this is information to show, not an
/// error.
pub fn upstream_listening(site: &Site) -> Option<bool> {
    let port = upstream_port(site.upstream.as_deref()?)?;
    Some(devd_services::local_tcp_open(port))
}

/// Arguments accepted by [`create`].
#[derive(Debug, Clone)]
pub struct CreateRequest {
    /// Project name. Validated as a name and used as the directory name.
    pub name: String,
    /// Fully qualified hostname. Defaults to `<name>.<tld>`.
    pub host: Option<String>,
    /// Project directory. Defaults to `<www>/<name>`.
    pub root: Option<String>,
    /// Whether to issue a certificate.
    pub tls: bool,
    /// Whether to create a database named after the site.
    pub db: bool,
    /// A running program to forward every request to, instead of serving a
    /// directory: `3000`, `localhost:5173`, `http://127.0.0.1:8000`.
    pub upstream: Option<String>,
    /// What the directory serves, when the caller knows. `None` leaves it to be
    /// read from the directory, which is what almost every caller wants.
    pub serving: Option<devd_core::serving::Content>,
}

/// Create a site: directory, certificate, record, configuration.
pub fn create(ctx: &Ctx, request: &CreateRequest) -> Result<Created> {
    create_with(ctx, request, true, true)
}

/// [`create`], with control over the two things a caller creating many sites
/// does not want repeated: writing a starter page into a directory that has
/// nothing to serve, and re-rendering and reloading the web server.
pub(crate) fn create_with(
    ctx: &Ctx,
    request: &CreateRequest,
    starter_page: bool,
    reload: bool,
) -> Result<Created> {
    util::validate_name(&request.name)?;
    let host = request
        .host
        .clone()
        .unwrap_or_else(|| format!("{}.{}", request.name, ctx.settings.tld));
    util::validate_hostname(&host)?;
    let upstream = request
        .upstream
        .as_deref()
        .map(normalize_upstream)
        .transpose()?;

    for existing in ctx.store.list_sites()? {
        if existing.name == request.name {
            return Err(DevdError::conflict(format!(
                "a site named `{}` already exists; `devd site list` shows it",
                request.name
            )));
        }
        if existing.host.eq_ignore_ascii_case(&host) {
            return Err(DevdError::conflict(format!(
                "the hostname {host} is already served by the site `{}`",
                existing.name
            )));
        }
    }

    let directory = request
        .root
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| default_directory(ctx, &request.name));
    // A proxy site serves a program, not a directory, so it creates neither the
    // directory nor a starter page.
    let starter = if upstream.is_none() {
        std::fs::create_dir_all(&directory)?;
        if starter_page {
            write_starter_page(&directory, &request.name)?
        } else {
            None
        }
    } else {
        None
    };

    let mut warnings = Vec::new();
    if let Some(page) = &starter {
        warnings.push(format!("created a starter page at {page}"));
    }

    let tls = if request.tls { Tls::Internal } else { Tls::Off };
    let mut certificate = None;
    if tls == Tls::Internal {
        let issued = devd_services::tls::issue(ctx, &host)?;
        certificate = Some(issued);
    }

    let site = Site {
        id: util::new_id(),
        name: request.name.clone(),
        host: host.clone(),
        root: directory.display().to_string(),
        web_server: ctx.settings.web_server.as_str().to_string(),
        // A new site follows the default PHP. Writing today's default into the
        // record froze it: after the default moved, sites created earlier kept the
        // old version and later ones followed the new one, for no reason a person
        // could see. A version is pinned only by `site.set_php`.
        php: None,
        serving: request.serving,
        tls_mode: tls.as_str().to_string(),
        db_name: None,
        upstream: upstream.clone(),
        created_at: util::now(),
    };
    if upstream.is_some() && upstream_listening(&site) == Some(false) {
        warnings.push(format!(
            "nothing is listening at {} yet; start the program and {host} will reach it",
            upstream.as_deref().unwrap_or_default()
        ));
    }
    ctx.store.upsert_site(&site)?;

    // A database is created only when a server is actually there to create it
    // on; otherwise the site is still created and the reason is reported.
    let mut database = None;
    let mut site = site;
    if request.db {
        match devd_services::mariadb_service(ctx) {
            Some((port, running)) if running => {
                devd_services::mariadb::create_database(ctx, port, &request.name)?;
                database = Some(request.name.clone());
                site.db_name = Some(request.name.clone());
                ctx.store.upsert_site(&site)?;
            }
            Some(_) => warnings.push(
                "no database was created because MariaDB is not running; start it with \
                 `devd service start mariadb` and run `devd db create <name>`"
                    .to_string(),
            ),
            None => warnings.push(
                "no database was created because no database server is installed; \
                 `devd service install mariadb` installs one"
                    .to_string(),
            ),
        }
    }

    let reloaded = if reload {
        reconcile(ctx, &mut warnings)?
    } else {
        false
    };

    // The hosts file is deliberately not written here; say so rather than leaving
    // the user with a name that does not resolve and no explanation. `localhost`
    // and its subdomains are exempt: they resolve without a hosts entry, which is
    // why the first-run site uses one.
    let managed = hosts::current_entries()?;
    if !resolves_without_hosts(&host) && !managed.iter().any(|e| e.host.eq_ignore_ascii_case(&host))
    {
        warnings.push(format!(
            "{host} does not resolve on this machine yet; `devd hosts sync` adds it to {}",
            hosts::path().display()
        ));
    }

    ctx.events.emit_with(
        "site.created",
        format!("created {} at {}", site.name, site.host),
        json!({ "site": site.name, "host": site.host, "directory": site.root }),
    );

    Ok(Created {
        url: url_for(ctx, &site),
        directory: directory.display().to_string(),
        document_root: site.document_root(),
        site,
        certificate,
        reloaded,
        database,
        warnings,
    })
}

/// Render the web server configuration and reload it when it is running.
///
/// Returns whether a reload happened. A reload is a convenience: everything has
/// already been written, and the state store is the truth, so a web server that
/// is not running does not make the operation a failure.
pub(crate) fn reconcile(ctx: &Ctx, warnings: &mut Vec<String>) -> Result<bool> {
    let server = devd_services::web::service_name(ctx);
    match devd_services::web::render(ctx) {
        Ok(_) => {}
        Err(e) if e.code() == "not_found" => {
            warnings.push(format!(
                "the web server is not installed, so nothing serves this site yet; \
                 `devd service install {server}` installs it"
            ));
            return Ok(false);
        }
        Err(e) => return Err(e),
    }

    // The site is already recorded and its configuration written. A web server
    // devd has not installed as a service is not running, which is not a reason
    // to fail the operation that has just succeeded.
    let running = match devd_services::status(ctx, server) {
        Ok(status) => status.running,
        Err(e) if e.code() == "not_found" => false,
        Err(e) => return Err(e),
    };
    if !running {
        return Ok(false);
    }
    devd_services::web::reload(ctx)?;
    Ok(true)
}

/// Write a starter page into a directory that is new: one with nothing in it.
///
/// A directory that already holds files is somebody's project, whatever it is. Writing an
/// `index.php` into a Go module or a Node app would change what it is (it would become a PHP
/// site) and put a file the person did not write into their repository. Such a directory is
/// left as it is; if it has nothing to serve, the site says so and `site.set_serving` is how
/// to override that.
fn write_starter_page(directory: &Path, name: &str) -> Result<Option<String>> {
    let has_content = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| !e.file_name().to_string_lossy().starts_with('.'))
        })
        .unwrap_or(false);
    if has_content {
        return Ok(None);
    }
    let php = directory.join("index.php");
    let page = format!(
        "<?php\n\
         // Written by devd when this project was created. Replace it with your application.\n\
         header('Content-Type: text/html; charset=utf-8');\n\
         ?>\n\
         <!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
           <meta charset=\"utf-8\">\n\
           <title>{name} - served by devd</title>\n\
           <style>\n\
             body {{ font: 16px/1.6 system-ui, sans-serif; margin: 4rem auto; max-width: 40rem; color: #111; }}\n\
             code {{ background: #f4f4f5; padding: .15em .35em; border-radius: 4px; }}\n\
             h1 {{ font-size: 1.6rem; }}\n\
           </style>\n\
         </head>\n\
         <body>\n\
           <h1>{name} is served by devd</h1>\n\
           <p>Host <code><?= htmlspecialchars($_SERVER['HTTP_HOST'] ?? '') ?></code> is answering\n\
              through <?= htmlspecialchars(strtok($_SERVER['SERVER_SOFTWARE'] ?? 'the web server', '/')) ?>, and PHP is running as FastCGI.</p>\n\
           <p>PHP <code><?= PHP_VERSION ?></code> on <?= PHP_OS_FAMILY ?>\n\
              (<?= htmlspecialchars(php_sapi_name()) ?>).</p>\n\
           <p>Document root <code><?= htmlspecialchars(__DIR__) ?></code>.</p>\n\
         </body>\n\
         </html>\n"
    );
    util::atomic_write(&php, &page)?;
    Ok(Some(php.display().to_string()))
}

/// Point a site at a running program, or turn it back into a directory site.
///
/// `Some(address)` makes every request to the hostname go to that program and
/// drops the PHP pin, since nothing runs PHP for it. `None` serves the site's
/// project directory again, which needs that directory to exist: the site is
/// not left pointing at nothing.
pub fn set_upstream(ctx: &Ctx, name: &str, upstream: Option<&str>) -> Result<serde_json::Value> {
    let mut site = ctx
        .store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`")))?;

    match upstream.map(str::trim).filter(|u| !u.is_empty()) {
        Some(address) => {
            site.upstream = Some(normalize_upstream(address)?);
            site.php = None;
        }
        None => {
            if site.upstream.is_none() {
                return Err(DevdError::invalid(format!(
                    "`{name}` already serves its project directory"
                )));
            }
            if !Path::new(&site.root).is_dir() {
                return Err(DevdError::invalid(format!(
                    "`{name}` has no project directory at {} to serve; create it, or give \
                     the site another upstream",
                    site.root
                )));
            }
            site.upstream = None;
        }
    }
    ctx.store.upsert_site(&site)?;

    let mut warnings = Vec::new();
    if upstream_listening(&site) == Some(false) {
        warnings.push(format!(
            "nothing is listening at {} yet; start the program and {} will reach it",
            site.upstream.as_deref().unwrap_or_default(),
            site.host
        ));
    }
    let reloaded = reconcile(ctx, &mut warnings)?;
    ctx.events.emit_with(
        "site.upstream_changed",
        format!(
            "{} now serves {}",
            site.name,
            site.upstream.as_deref().unwrap_or("its directory")
        ),
        json!({ "site": site.name, "upstream": site.upstream }),
    );
    Ok(json!({
        "site": site,
        "url": url_for(ctx, &site),
        "reloaded": reloaded,
        "warnings": warnings,
    }))
}

/// Remove a site.
///
/// The files are kept unless they were explicitly asked for: deleting a project
/// because a hostname was renamed is not a mistake this should be able to make.
pub fn remove(ctx: &Ctx, name: &str, delete_files: bool) -> Result<serde_json::Value> {
    let site = ctx.store.get_site(name)?.ok_or_else(|| {
        DevdError::not_found(format!(
            "no site named `{name}`; `devd site list` shows them"
        ))
    })?;

    let mut warnings = Vec::new();

    // The certificate belongs to the hostname and stops being meaningful with
    // it, so it goes. It is regenerated on the next create.
    for path in [certificate_path(ctx, &site.host), key_path(ctx, &site.host)] {
        if path.is_file() {
            std::fs::remove_file(&path)?;
        }
    }

    ctx.store.delete_site(name)?;

    let mut removed_files = false;
    let mut bytes = 0u64;
    if delete_files {
        let root = PathBuf::from(&site.root);
        // The project directory is removed only when it is inside the
        // configured projects directory. A site pointed at an arbitrary path -
        // a repository somewhere else - must never take that directory with it.
        let base = www_root(ctx);
        let inside_www = root.starts_with(&base);
        if !inside_www {
            return Err(DevdError::denied(format!(
                "the project directory {} is outside {}, so it will not be deleted; \
                 remove it yourself if that is what you want",
                root.display(),
                base.display()
            )));
        }
        if root.is_dir() {
            bytes = util::directory_size(&root);
            std::fs::remove_dir_all(&root)?;
            removed_files = true;
        }
    } else {
        warnings.push(format!("the project directory {} was kept", site.root));
    }

    if site.db_name.is_some() {
        warnings.push(format!(
            "the database `{}` was kept; drop it with `devd db drop {}`",
            site.db_name.clone().unwrap_or_default(),
            site.db_name.clone().unwrap_or_default()
        ));
    }

    let managed = hosts::current_entries()?;
    if managed.iter().any(|e| e.host == site.host) {
        warnings.push(format!(
            "{} is still in {}; `devd hosts sync` removes it",
            site.host,
            hosts::path().display()
        ));
    }

    let reloaded = reconcile(ctx, &mut warnings)?;

    ctx.events.emit_with(
        "site.removed",
        format!("removed {}", site.name),
        json!({ "site": site.name, "host": site.host }),
    );

    Ok(json!({
        "site": site.name,
        "host": site.host,
        "hosts_entry": site.host,
        "files_removed": removed_files,
        "bytes_removed": bytes,
        "reloaded": reloaded,
        "warnings": warnings,
    }))
}

/// One site as the surfaces show it.
#[derive(Debug, Clone, Serialize)]
pub struct SiteRow {
    /// The stored site.
    pub site: Site,
    /// Project directory, with a live check that it exists.
    pub present: bool,
    /// The URL the site is served at.
    pub url: String,
    /// Document root the web server reads.
    pub document_root: String,
    /// Whether the hostname is in the hosts file.
    pub in_hosts: bool,
    /// Whether a certificate exists for the hostname.
    pub certificate: bool,
    /// Whether the web server is running at all.
    pub server_running: bool,
    /// The PHP versions that are installed, so a surface can offer them.
    pub php_installed: Vec<String>,
    /// The project this site serves (its folder, or the folder above a `public/`).
    pub project: String,
    /// Whether devd knows that folder as a project: imported, created, or an indexed repository.
    pub project_known: bool,
    /// What the site is served as: a proxy, PHP, static files, or nothing (a directory with
    /// no web entry point, which answers with an explanation instead of exposing itself).
    pub serving: devd_core::serving::Serving,
    /// Where that came from: the upstream, a person's choice, or what the files are.
    pub serving_source: devd_core::serving::Source,
    /// What the directory contains. `None` for a proxy site, which has none.
    pub detection: Option<devd_core::serving::Detection>,
    /// That, as a sentence a person can read: what the directory is and why it is or is not served.
    pub detected: Option<String>,
    /// The PHP version the site is served by. `None` unless the site serves PHP: a version
    /// means nothing for a Go module, a static site or a proxy.
    pub php_effective: Option<String>,
    /// How that version came about: `pinned` (a person chose it) or `inherited` (it follows
    /// the default). `None` unless the site serves PHP.
    pub php_source: Option<&'static str>,
    /// A PHP version recorded for the site that nothing uses, because the site does not
    /// serve PHP. Shown so it can be removed, not presented as the site's runtime.
    pub php_unused_pin: Option<String>,
    /// For a proxy site, whether anything is accepting connections at its
    /// upstream right now. `None` for a site that serves a directory.
    pub upstream_listening: Option<bool>,
    /// How many `php_value`-style directives in the site's `.htaccess` Apache cannot
    /// read (always 0 under nginx).
    pub htaccess_php: usize,
    /// The public address, while a share link is open.
    pub shared: Option<String>,
}

/// Every site, with a live check of what is actually on disk.
pub fn list(ctx: &Ctx) -> Result<Vec<SiteRow>> {
    let managed = hosts::current_entries().unwrap_or_default();
    let server_running = devd_services::status(ctx, devd_services::web::service_name(ctx))
        .map(|s| s.running)
        .unwrap_or(false);
    let php_installed = devd_services::installed_php_series(ctx).unwrap_or_default();
    let primary = devd_services::primary_php_series(ctx);
    let shares = devd_services::share::list(ctx).unwrap_or_default();
    let known_projects: Vec<PathBuf> = ctx
        .store
        .list_stack_projects()
        .unwrap_or_default()
        .into_iter()
        .map(|p| PathBuf::from(p.path))
        .chain(
            ctx.store
                .list_repos()
                .unwrap_or_default()
                .into_iter()
                .map(|r| PathBuf::from(r.path)),
        )
        .collect();

    Ok(ctx
        .store
        .list_sites()?
        .into_iter()
        .map(|site| {
            // The version a site runs on: its own, or the default. A site
            // pinned to a version that is not installed falls back to the
            // default, because that is what it is actually served by.
            let pinned = site
                .php
                .as_deref()
                .map(devd_services::php_series)
                .filter(|s| !s.is_empty());
            let effective = site.effective_serving();
            let uses_php = effective.serving.uses_php();
            let php_effective = if !uses_php {
                None
            } else {
                match &pinned {
                    Some(series) if php_installed.iter().any(|s| s == series) => {
                        Some(series.clone())
                    }
                    // The pinned version is gone, so the default takes over - unless
                    // there is no default either, in which case the pin is what the
                    // site says it wants.
                    Some(series) => primary.clone().or_else(|| Some(series.clone())),
                    None => primary.clone(),
                }
            };
            let php_source = uses_php.then_some(if pinned.is_some() {
                "pinned"
            } else {
                "inherited"
            });
            let php_unused_pin = if uses_php { None } else { site.php.clone() };
            let detected = effective.detection.as_ref().map(|d| d.summary());
            let project_dir = site.project_dir();
            let project_known = known_projects
                .iter()
                .any(|p| util::same_folder(p, &project_dir));
            SiteRow {
                project: project_dir.display().to_string(),
                project_known,
                serving: effective.serving,
                serving_source: effective.source,
                detected,
                detection: effective.detection,
                php_source,
                php_unused_pin,
                // A proxy site has no project directory to be missing.
                present: site.upstream.is_some() || Path::new(&site.root).is_dir(),
                url: url_for(ctx, &site),
                document_root: site.document_root(),
                in_hosts: managed.iter().any(|e| e.host == site.host),
                certificate: devd_services::tls::leaf_valid(ctx, &site.host),
                server_running,
                php_installed: php_installed.clone(),
                php_effective,
                upstream_listening: upstream_listening(&site),
                htaccess_php: htaccess_php(ctx, &site),
                shared: shares
                    .iter()
                    .find(|s| s.site == site.name)
                    .map(|s| s.url.clone()),
                site,
            }
        })
        .collect())
}

/// Directives in a site's `.htaccess` that Apache, which runs PHP as FastCGI, rejects.
fn htaccess_php(ctx: &Ctx, site: &Site) -> usize {
    if site.upstream.is_some() || ctx.settings.web_server.as_str() != "apache" {
        return 0;
    }
    std::fs::read_to_string(Path::new(&site.document_root()).join(".htaccess"))
        .map(|t| htaccess::scan(&t).len())
        .unwrap_or(0)
}

/// Pin a site to a PHP version, or unpin it.
///
/// A site that is pinned is served by that version's listener however the
/// default changes. A site that is not pinned follows the default. The web
/// server configuration is re-rendered, because that is what decides which
/// listener a request reaches.
pub fn set_php(ctx: &Ctx, name: &str, version: Option<&str>) -> Result<serde_json::Value> {
    let mut site = ctx
        .store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`")))?;

    // A version that is not installed cannot be served, so it is refused
    // rather than recorded and discovered to be wrong later.
    if let Some(version) = version {
        let series = devd_services::php_series(version);
        let installed = devd_services::installed_php_series(ctx)?;
        if !installed.iter().any(|s| s == &series) {
            return Err(DevdError::invalid(format!(
                "PHP {series} is not installed; the installed versions are {}",
                installed.join(", ")
            )));
        }
        site.php = Some(series);
    } else {
        site.php = None;
    }
    ctx.store.upsert_site(&site)?;

    // The listener for the pinned version has to be running, or every request
    // to the site is a 502. The row for it is made here, because the version
    // may have been installed since the last time the stack was started.
    let mut started = Vec::new();
    if let Some(series) = &site.php {
        let _ = devd_services::ensure_php_services(ctx);
        let listener = devd_services::php_service_name(series);
        let running = ctx
            .store
            .get_service(&listener)?
            .map(|s| devd_services::running_pid(ctx, &s).is_some())
            .unwrap_or(false);
        if !running {
            if let Ok(status) = devd_services::start(ctx, &listener) {
                started.push(status.name);
            }
        }
    }

    // The configuration is what decides which upstream a request reaches, so
    // it is re-rendered whenever the pin changes.
    let mut warnings = Vec::new();
    if let Err(e) = devd_services::web::render(ctx) {
        warnings.push(format!(
            "the web server configuration could not be re-rendered: {e}"
        ));
    }

    Ok(json!({
        "site": site,
        "php": site.php,
        "started": started,
        "warnings": warnings,
    }))
}

/// Say what a directory site serves, or clear the choice so it is read from the directory again.
///
/// Only a directory site has content to choose; a proxy site serves a program, and changing
/// that is `site.set_upstream`. The web server configuration is re-rendered, because it is what
/// decides whether PHP runs for the site.
pub fn set_serving(
    ctx: &Ctx,
    name: &str,
    serving: Option<devd_core::serving::Content>,
) -> Result<serde_json::Value> {
    let mut site = ctx
        .store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`")))?;
    if site.upstream.is_some() {
        return Err(DevdError::conflict(format!(
            "`{name}` is a proxy site: it serves the program at {}, and has no directory content to choose. `site.set_upstream` changes what it points at",
            site.upstream.as_deref().unwrap_or_default()
        )));
    }
    site.serving = serving;
    ctx.store.upsert_site(&site)?;

    let mut warnings = Vec::new();
    if let Err(e) = devd_services::web::render(ctx) {
        warnings.push(format!(
            "the web server configuration could not be re-rendered: {e}"
        ));
    }
    let effective = site.effective_serving();
    if let (Some(pin), false) = (&site.php, effective.serving.uses_php()) {
        warnings.push(format!("{name} keeps a PHP {pin} pin that nothing uses now; `site.set_php` with no version clears it"));
    }
    Ok(json!({
        "site": site,
        "serving": effective.serving,
        "source": effective.source,
        "warnings": warnings,
    }))
}

/// The site component of the environment check.
///
/// The useful check is the second one: a site whose directory has gone is
/// recorded, is in the hosts file, and returns 404 from the web server. Nothing
/// else in `devd` would notice, and the user would see an unexplained 404.
pub fn doctor_check(paths: &Paths) -> Vec<Check> {
    // No database yet is an empty install, not a broken one; looking does not create it.
    if !paths.state_db().is_file() {
        return vec![Check::info(
            "sites",
            "none defined; `devd site create <name>` creates one",
        )];
    }
    let Ok(store) = Store::open_read_only(&paths.state_db()) else {
        return vec![Check::fail("sites", "the state store could not be read")];
    };
    let Ok(sites) = store.list_sites() else {
        return vec![Check::fail("sites", "the site table could not be read")];
    };
    if sites.is_empty() {
        return vec![Check::info(
            "sites",
            "none defined; `devd site create <name>` creates one",
        )];
    }

    let missing: Vec<&str> = sites
        .iter()
        // A proxy site serves a program, so it has no directory to be missing.
        .filter(|s| s.upstream.is_none() && !Path::new(&s.root).is_dir())
        .map(|s| s.name.as_str())
        .collect();
    let unmanaged: Vec<&str> = sites
        .iter()
        .filter(|s| {
            !resolves_without_hosts(&s.host)
                && !hosts::is_managed(
                    &std::fs::read_to_string(hosts::path()).unwrap_or_default(),
                    &s.host,
                )
        })
        .map(|s| s.host.as_str())
        .collect();

    let mut checks = vec![if missing.is_empty() {
        Check::ok(
            "sites",
            format!(
                "{} site(s), all of their directories are present",
                sites.len()
            ),
        )
    } else {
        Check::fail(
            "sites",
            format!(
                "{} site(s) have no project directory: {}",
                missing.len(),
                missing.join(", ")
            ),
        )
    }];

    if !unmanaged.is_empty() {
        checks.push(Check::info(
            "sites.hosts",
            format!(
                "{} hostname(s) are not in the hosts file: {}; run `devd hosts sync`",
                unmanaged.len(),
                unmanaged.join(", ")
            ),
        ));
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn ctx(tag: &str) -> Ctx {
        let dir = std::env::temp_dir().join(format!("devd-sites-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(dir);
        paths.ensure().unwrap();
        // Every test writes to a scratch hosts file, never the system one.
        std::env::set_var(
            hosts::ENV_HOSTS_FILE,
            paths.root().join("hosts-fixture").display().to_string(),
        );
        let mut settings = devd_core::config::Settings::default();
        settings.http_port = 8080;
        settings.https_port = 8443;
        Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(settings),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            run: None,
            dry_run: false,
        }
    }

    fn request(name: &str) -> CreateRequest {
        CreateRequest {
            name: name.to_string(),
            host: None,
            root: None,
            tls: true,
            db: false,
            upstream: None,
            serving: None,
        }
    }

    /// A project directory holding the given files (a name ending in `/` is a folder).
    fn project(parent: &Path, name: &str, files: &[&str]) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for f in files {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x").unwrap();
        }
        dir
    }

    #[test]
    fn parking_a_folder_makes_a_site_of_each_project_once_and_writes_nothing_into_them() {
        let c = ctx("park");
        let folder = c.paths.root().join("parked");
        project(&folder, "My Blog", &["public/index.php"]);
        project(&folder, "shop", &["index.html"]);
        std::fs::create_dir_all(folder.join(".git")).unwrap();
        std::fs::create_dir_all(folder.join("node_modules")).unwrap();
        std::fs::write(folder.join("notes.txt"), "not a project").unwrap();

        let report = park::sync(&c, std::slice::from_ref(&folder), None).unwrap();
        let mut made: Vec<String> = report
            .created
            .iter()
            .map(|(n, h)| format!("{n} {h}"))
            .collect();
        made.sort();
        assert_eq!(
            made,
            vec!["my-blog my-blog.test", "shop shop.test"],
            "{report:?}"
        );
        assert!(report.skipped.is_empty(), "{report:?}");
        // No starter page was written into a person's project.
        assert!(!folder.join("shop").join("index.php").exists());
        assert_eq!(
            std::fs::read_to_string(folder.join("My Blog").join("public").join("index.php"))
                .unwrap(),
            "x"
        );
        let site = c
            .store
            .list_sites()
            .unwrap()
            .into_iter()
            .find(|s| s.name == "my-blog")
            .unwrap();
        assert!(site.document_root().ends_with("public"));

        let again = park::sync(&c, std::slice::from_ref(&folder), None).unwrap();
        assert!(again.created.is_empty());
        assert_eq!(again.existing.len(), 2);

        std::fs::remove_dir_all(folder.join("shop")).unwrap();
        let gone = park::sync(&c, std::slice::from_ref(&folder), None).unwrap();
        assert_eq!(
            gone.gone,
            vec!["shop"],
            "a site whose folder vanished is reported, not removed"
        );
        assert_eq!(c.store.list_sites().unwrap().len(), 2);
    }

    #[test]
    fn discovery_changes_nothing_and_gives_the_same_answer_every_time() {
        let c = ctx("discover");
        let folder = c.paths.root().join("parked");
        project(&folder, "blog", &["index.php"]);
        project(&folder, "ghost", &["go.mod", "main.go"]);
        let before = c.store.list_sites().unwrap();

        let first =
            serde_json::to_value(park::discover(&c, std::slice::from_ref(&folder)).unwrap())
                .unwrap();
        let second =
            serde_json::to_value(park::discover(&c, std::slice::from_ref(&folder)).unwrap())
                .unwrap();

        assert_eq!(
            first, second,
            "repeating discovery cannot change what it says"
        );
        assert_eq!(
            c.store.list_sites().unwrap(),
            before,
            "discovery writes no site"
        );
        assert!(
            !c.paths.certs().join("blog.test.crt").exists(),
            "and issues no certificate"
        );
    }

    #[test]
    fn only_a_directory_with_a_web_entry_point_becomes_a_site() {
        let c = ctx("eligible");
        let folder = c.paths.root().join("parked");
        project(
            &folder,
            "blog",
            &["public/index.php", "artisan", "composer.json"],
        );
        project(&folder, "brochure", &["index.html"]);
        project(&folder, "ghost", &["go.mod", "main.go"]);
        project(&folder, "scout", &["go.mod"]);
        project(&folder, "web-app", &["package.json", "src/main.jsx"]);
        project(&folder, "php-library", &["composer.json", "src/Thing.php"]);
        project(&folder, "empty", &[]);

        let found = park::discover(&c, std::slice::from_ref(&folder)).unwrap();
        let eligible: Vec<&str> = found.eligible().filter_map(|c| c.name.as_deref()).collect();
        assert_eq!(eligible, vec!["blog", "brochure"], "{found:#?}");

        let report = park::sync(&c, std::slice::from_ref(&folder), None).unwrap();
        let mut made: Vec<&str> = report.created.iter().map(|(n, _)| n.as_str()).collect();
        made.sort();
        assert_eq!(made, vec!["blog", "brochure"]);
        assert_eq!(report.skipped.len(), 5, "{report:?}");
        assert!(
            report
                .skipped
                .iter()
                .all(|s| s.reason.contains("not a web project")),
            "{report:?}"
        );
        let ghost = report
            .skipped
            .iter()
            .find(|s| s.folder.ends_with("ghost"))
            .unwrap();
        assert!(
            ghost.reason.contains("go; no web entry point"),
            "it says what the directory is: {}",
            ghost.reason
        );
    }

    #[test]
    fn creating_a_site_never_pins_the_default_php_into_it() {
        let mut c = ctx("nopin");
        std::sync::Arc::get_mut(&mut c.settings)
            .unwrap()
            .primary_php = Some("8.5.12".to_string());
        let created = create(&c, &request("blog")).unwrap();
        assert_eq!(
            created.site.php, None,
            "a site follows the default until someone pins a version"
        );
        assert_eq!(c.store.get_site("blog").unwrap().unwrap().php, None);
    }

    #[test]
    fn a_go_module_is_never_shown_with_a_php_runtime() {
        let c = ctx("gorow");
        let dir = project(&c.paths.root().join("www"), "ghost", &["go.mod", "main.go"]);
        // Created by hand (a person's explicit `site.create --root`), and with an old PHP pin on it,
        // as the sites a parked folder produced once were.
        let mut site = create(
            &c,
            &CreateRequest {
                root: Some(dir.display().to_string()),
                ..request("ghost")
            },
        )
        .unwrap()
        .site;
        assert!(
            !dir.join("index.php").exists(),
            "a project with files in it is never given a starter page"
        );
        site.php = Some("8.5.12".to_string());
        c.store.upsert_site(&site).unwrap();

        let row = list(&c)
            .unwrap()
            .into_iter()
            .find(|r| r.site.name == "ghost")
            .unwrap();
        assert_eq!(row.serving, devd_core::serving::Serving::NoEntry);
        assert_eq!(row.php_effective, None, "PHP is not this site's runtime");
        assert_eq!(row.php_source, None);
        assert_eq!(
            row.php_unused_pin.as_deref(),
            Some("8.5.12"),
            "the leftover pin is shown as unused, so it can be cleared"
        );
        assert!(row
            .detection
            .unwrap()
            .language_names()
            .contains(&"go".to_string()));
    }

    #[test]
    fn a_php_site_says_whether_its_version_is_pinned_or_follows_the_default() {
        let c = ctx("phprow");
        let dir = project(&c.paths.root().join("www"), "blog", &["index.php"]);
        create(
            &c,
            &CreateRequest {
                root: Some(dir.display().to_string()),
                ..request("blog")
            },
        )
        .unwrap();
        let inherited = list(&c)
            .unwrap()
            .into_iter()
            .find(|r| r.site.name == "blog")
            .unwrap();
        assert_eq!(inherited.serving, devd_core::serving::Serving::Php);
        assert_eq!(inherited.php_source, Some("inherited"));

        let mut site = c.store.get_site("blog").unwrap().unwrap();
        site.php = Some("8.4".to_string());
        c.store.upsert_site(&site).unwrap();
        let pinned = list(&c)
            .unwrap()
            .into_iter()
            .find(|r| r.site.name == "blog")
            .unwrap();
        assert_eq!(pinned.php_source, Some("pinned"));
    }

    #[test]
    fn two_folders_with_the_same_site_name_are_a_conflict_not_two_sites() {
        let c = ctx("clash");
        let folder = c.paths.root().join("parked");
        project(&folder, "My Blog", &["index.php"]);
        project(&folder, "my-blog", &["index.php"]);
        let found = park::discover(&c, std::slice::from_ref(&folder)).unwrap();
        assert_eq!(found.eligible().count(), 1, "{found:#?}");
        let loser = found.candidates.iter().find(|c| !c.eligible).unwrap();
        assert!(
            loser.reason.as_deref().unwrap().contains("already taken"),
            "{loser:?}"
        );
        let report = park::sync(&c, std::slice::from_ref(&folder), None).unwrap();
        assert_eq!(report.created.len(), 1);
        assert_eq!(c.store.list_sites().unwrap().len(), 1);
    }

    #[test]
    fn naming_a_project_that_is_not_a_web_project_does_not_create_it_anyway() {
        let c = ctx("named");
        let folder = c.paths.root().join("parked");
        project(&folder, "blog", &["index.php"]);
        project(&folder, "ghost", &["go.mod"]);
        let only = vec!["ghost".to_string(), "missing".to_string()];
        let report = park::sync(&c, std::slice::from_ref(&folder), Some(&only)).unwrap();
        assert!(report.created.is_empty(), "{report:?}");
        assert_eq!(
            report.skipped.len(),
            2,
            "ghost has no web entry and `missing` is not there: {report:?}"
        );
        assert!(c.store.list_sites().unwrap().is_empty());
    }

    #[test]
    fn what_a_site_serves_can_be_chosen_and_cleared_and_a_proxy_has_no_such_choice() {
        let c = ctx("choose");
        let dir = project(&c.paths.root().join("www"), "docs", &["go.mod"]);
        create(
            &c,
            &CreateRequest {
                root: Some(dir.display().to_string()),
                ..request("docs")
            },
        )
        .unwrap();
        assert_eq!(
            list(&c).unwrap()[0].serving,
            devd_core::serving::Serving::NoEntry
        );

        set_serving(&c, "docs", Some(devd_core::serving::Content::Static)).unwrap();
        let row = list(&c).unwrap().into_iter().next().unwrap();
        assert_eq!(
            (row.serving, row.serving_source),
            (
                devd_core::serving::Serving::Static,
                devd_core::serving::Source::Configured
            )
        );

        let cleared = set_serving(&c, "docs", None).unwrap();
        assert_eq!(cleared["serving"], "no_entry");

        create(
            &c,
            &CreateRequest {
                upstream: Some("3000".into()),
                ..request("app")
            },
        )
        .unwrap();
        let err = set_serving(&c, "app", Some(devd_core::serving::Content::Php)).unwrap_err();
        assert!(err.to_string().contains("proxy site"), "{err}");
    }

    #[test]
    fn an_upstream_is_a_plain_address_on_this_machine() {
        for (typed, expected) in [
            ("3000", "http://127.0.0.1:3000"),
            (" 5173 ", "http://127.0.0.1:5173"),
            ("localhost:5173", "http://localhost:5173"),
            ("http://127.0.0.1:8000", "http://127.0.0.1:8000"),
            ("http://127.0.0.1:8000/", "http://127.0.0.1:8000"),
            ("HTTPS://localhost:8443", "https://localhost:8443"),
            ("[::1]:3000", "http://[::1]:3000"),
            ("localhost", "http://localhost:80"),
            ("http://app.localhost:4000", "http://app.localhost:4000"),
            ("127.0.0.2:9000", "http://127.0.0.2:9000"),
        ] {
            assert_eq!(normalize_upstream(typed).unwrap(), expected, "{typed}");
        }
    }

    #[test]
    fn an_upstream_that_is_not_plain_or_not_local_is_refused() {
        for bad in [
            "",
            "0",
            "70000",
            "ftp://127.0.0.1:21",
            "example.com:3000",
            "http://192.168.1.5:3000",
            "http://127.0.0.1:3000/api",
            "http://user@127.0.0.1:3000",
            "127.0.0.1:3000; return 200",
            "127.0.0.1:3000 foo",
            "http://127.0.0.1:3000/$x",
            "http://127.0.0.1:{x}",
            "http://[::1:3000",
            "127.0.0.1:abc",
            "127.0.0.1.evil.com:80",
            "999.1.1.1:80",
        ] {
            assert!(normalize_upstream(bad).is_err(), "`{bad}` was accepted");
        }
    }

    #[test]
    fn a_proxy_site_has_no_directory_no_php_and_a_proxy_block() {
        let c = ctx("proxy");
        let mut req = request("api");
        req.upstream = Some("5173".to_string());
        let created = create(&c, &req).unwrap();
        assert_eq!(
            created.site.upstream.as_deref(),
            Some("http://127.0.0.1:5173")
        );
        assert_eq!(created.site.php, None);
        assert!(
            !Path::new(&created.directory).exists(),
            "a proxy site must not create a project directory"
        );
        assert!(
            created.certificate.is_some(),
            "a proxy site still gets HTTPS"
        );
        let row = list(&c)
            .unwrap()
            .into_iter()
            .find(|r| r.site.name == "api")
            .unwrap();
        assert!(row.present, "a proxy site has no directory to be missing");
        // Nothing listens on that port in a test, and the row says so.
        assert!(row.upstream_listening.is_some());
    }

    #[test]
    fn creating_a_site_succeeds_when_nginx_is_present_but_not_yet_a_service() {
        // The program and its files are there, but `service install` has not
        // recorded a service. Creating the site must not fail after the site is
        // already stored.
        let c = ctx("noservice");
        let exe = devd_services::nginx::directory(&c).join(if cfg!(windows) {
            "nginx.exe"
        } else {
            "sbin/nginx"
        });
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"binary").unwrap();

        let mut req = request("api");
        req.upstream = Some("3999".to_string());
        let created = create(&c, &req).expect("the site must be created");
        assert!(!created.reloaded, "nothing is running to reload");
        assert!(c.store.get_site("api").unwrap().is_some());
        assert!(
            devd_services::nginx::config_path(&c).is_file(),
            "the configuration is still written"
        );
    }

    #[test]
    fn an_upstream_can_be_changed_and_a_directory_site_can_become_a_proxy() {
        let c = ctx("setproxy");
        create(&c, &request("blog")).unwrap();
        let result = set_upstream(&c, "blog", Some("3000")).unwrap();
        assert_eq!(result["site"]["upstream"], "http://127.0.0.1:3000");
        assert!(c.store.get_site("blog").unwrap().unwrap().php.is_none());
        // Back to a directory: the directory is still there.
        set_upstream(&c, "blog", None).unwrap();
        assert!(c
            .store
            .get_site("blog")
            .unwrap()
            .unwrap()
            .upstream
            .is_none());
        // Clearing a site that was never a proxy is a mistake worth naming.
        assert!(set_upstream(&c, "blog", None).is_err());
        assert!(set_upstream(&c, "blog", Some("example.com:80")).is_err());
        assert!(set_upstream(&c, "nope", Some("3000")).is_err());
    }

    #[test]
    fn creating_a_site_produces_a_directory_a_certificate_and_a_record() {
        let c = ctx("create");
        let created = create(&c, &request("blog")).unwrap();
        assert_eq!(created.site.host, "blog.test");
        assert_eq!(created.url, "https://blog.test:8443");
        assert!(Path::new(&created.directory).is_dir());
        assert!(
            Path::new(&created.directory).join("index.php").is_file(),
            "a starter page makes the site answer immediately"
        );
        // The certificate exists, and the chain has two certificates in it.
        let cert = created.certificate.unwrap();
        let chain = std::fs::read_to_string(&cert.cert_path).unwrap();
        assert_eq!(chain.matches("BEGIN CERTIFICATE").count(), 2);
        assert!(Path::new(&cert.key_path).is_file());
        // The record is in the store.
        assert_eq!(c.store.get_site("blog").unwrap().unwrap().host, "blog.test");
        // Creating a site must not write to the hosts file.
        assert!(
            hosts::current_entries().unwrap().is_empty(),
            "hosts is a separate, admin-tier operation"
        );
        assert!(
            created.warnings.iter().any(|w| w.contains("hosts sync")),
            "the user has to be told the name does not resolve yet: {:?}",
            created.warnings
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    /// A PHP runtime row, so that a version counts as installed.
    fn install_php(c: &Ctx, version: &str) {
        c.store
            .upsert_runtime(&devd_core::state::Runtime {
                id: format!("php-{version}"),
                kind: "php".to_string(),
                version: version.to_string(),
                path: c
                    .paths
                    .runtime_version("php", version)
                    .display()
                    .to_string(),
                digest: None,
                pinned: false,
                installed_at: devd_core::util::now(),
            })
            .unwrap();
    }

    #[test]
    fn a_site_can_be_pinned_to_a_php_version_and_unpinned_again() {
        let c = ctx("pin");
        install_php(&c, "8.5.11");
        create(&c, &request("pinned")).unwrap();

        let pinned = set_php(&c, "pinned", Some("8.5")).unwrap();
        assert_eq!(pinned["php"], json!("8.5"));
        assert_eq!(
            c.store.get_site("pinned").unwrap().unwrap().php.as_deref(),
            Some("8.5")
        );

        // The listing says which version the site runs on.
        let rows = list(&c).unwrap();
        let row = rows.iter().find(|r| r.site.name == "pinned").unwrap();
        assert_eq!(row.php_effective.as_deref(), Some("8.5"));
        assert!(row.php_installed.contains(&"8.5".to_string()));

        // Unpinning puts it back on the default.
        set_php(&c, "pinned", None).unwrap();
        assert!(c.store.get_site("pinned").unwrap().unwrap().php.is_none());
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn pinning_a_site_to_a_version_that_is_not_installed_is_refused() {
        let c = ctx("pin-missing");
        create(&c, &request("missing")).unwrap();
        let err = set_php(&c, "missing", Some("7.4")).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
        assert!(err.to_string().contains("not installed"), "{err}");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_site_without_tls_is_served_over_http_only() {
        let c = ctx("plain");
        let mut req = request("plain");
        req.tls = false;
        let created = create(&c, &req).unwrap();
        assert_eq!(created.site.tls_mode, "off");
        assert_eq!(created.url, "http://plain.test:8080");
        assert!(
            created.certificate.is_none(),
            "no certificate was requested"
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_name_that_is_already_taken_is_a_conflict() {
        let c = ctx("dupe");
        create(&c, &request("shop")).unwrap();
        let err = create(&c, &request("shop")).unwrap_err();
        assert_eq!(err.code(), "conflict");
        assert!(err.to_string().contains("already exists"), "{err}");

        // The same hostname from a different name is also a conflict.
        let mut other = request("shop2");
        other.host = Some("shop.test".to_string());
        let err = create(&c, &other).unwrap_err();
        assert_eq!(err.code(), "conflict");
        assert!(err.to_string().contains("already served"), "{err}");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_hostname_that_could_inject_configuration_is_refused() {
        let c = ctx("inject");
        let mut req = request("evil");
        req.host = Some("evil.test\n}".to_string());
        let err = create(&c, &req).unwrap_err();
        assert_eq!(err.code(), "invalid_input", "{err}");
        // Nothing was recorded and nothing was written.
        assert!(c.store.list_sites().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn an_existing_project_is_not_overwritten() {
        let c = ctx("existing");
        let dir = c.paths.www().join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.php"), "<?php echo 'mine';").unwrap();
        create(&c, &request("app")).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("index.php")).unwrap(),
            "<?php echo 'mine';",
            "the starter page must not replace an existing one"
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn localhost_needs_no_hosts_entry() {
        let c = ctx("localhost");
        let mut req = request("welcome");
        req.host = Some("localhost".to_string());
        let created = create(&c, &req).unwrap();
        assert_eq!(created.url, "https://localhost:8443");
        assert!(
            !created.warnings.iter().any(|w| w.contains("hosts sync")),
            "localhost resolves without the hosts file: {:?}",
            created.warnings
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn www_root_moves_where_projects_are_created() {
        let mut c = ctx("wwwroot");
        let custom = c.paths.root().join("code");
        let mut settings = (*c.settings).clone();
        settings.www_root = Some(custom.clone());
        c.settings = Arc::new(settings);

        let created = create(&c, &request("app")).unwrap();
        assert_eq!(PathBuf::from(&created.directory), custom.join("app"));
        assert!(custom.join("app").join("index.php").is_file());
        assert!(
            !c.paths.www().join("app").exists(),
            "the layout's own www is not used when www_root is set"
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn removing_a_site_keeps_the_files_unless_asked() {
        let c = ctx("remove");
        let created = create(&c, &request("blog")).unwrap();
        let dir = PathBuf::from(&created.directory);

        let result = remove(&c, "blog", false).unwrap();
        assert!(dir.is_dir(), "files are kept by default");
        assert_eq!(result["files_removed"], json!(false));
        assert!(c.store.get_site("blog").unwrap().is_none());
        // The certificate went with the hostname.
        assert!(!certificate_path(&c, "blog.test").is_file());

        create(&c, &request("blog")).unwrap();
        let result = remove(&c, "blog", true).unwrap();
        assert_eq!(result["files_removed"], json!(true));
        assert!(!dir.is_dir());
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_project_outside_www_is_never_deleted() {
        let c = ctx("outside");
        let elsewhere = c.paths.root().join("elsewhere").join("repo");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let mut req = request("repo");
        req.root = Some(elsewhere.display().to_string());
        create(&c, &req).unwrap();
        let err = remove(&c, "repo", true).unwrap_err();
        assert_eq!(err.code(), "permission_denied", "{err}");
        assert!(elsewhere.is_dir(), "the directory must survive the refusal");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_store_is_the_source_of_truth_for_the_listing() {
        let c = ctx("list");
        create(&c, &request("one")).unwrap();
        let mut req = request("two");
        req.tls = false;
        create(&c, &req).unwrap();

        let rows = list(&c).unwrap();
        assert_eq!(rows.len(), 2);
        let one = rows.iter().find(|r| r.site.name == "one").unwrap();
        assert!(one.present);
        assert!(one.certificate);
        assert!(!one.in_hosts, "nothing has synced hosts yet");
        assert_eq!(one.url, "https://one.test:8443");
        let two = rows.iter().find(|r| r.site.name == "two").unwrap();
        assert!(!two.certificate);
        assert_eq!(two.url, "http://two.test:8080");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_public_folder_of_assets_beside_an_index_html_is_not_the_document_root() {
        let c = ctx("vite");
        let created = create(&c, &request("viteapp")).unwrap();
        let dir = Path::new(&created.directory);
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        std::fs::create_dir_all(dir.join("public")).unwrap();
        std::fs::write(dir.join("public").join("favicon.ico"), "x").unwrap();
        let rows = list(&c).unwrap();
        assert_eq!(
            rows[0].document_root, created.directory,
            "{}",
            rows[0].document_root
        );
        std::fs::write(dir.join("public").join("index.php"), "<?php").unwrap();
        assert!(list(&c).unwrap()[0].document_root.ends_with("public"));
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_document_root_prefers_a_public_directory() {
        let c = ctx("public");
        let created = create(&c, &request("laravel")).unwrap();
        assert_eq!(created.document_root, created.directory, "no public/ yet");
        std::fs::create_dir_all(Path::new(&created.directory).join("public")).unwrap();
        std::fs::write(
            Path::new(&created.directory)
                .join("public")
                .join("index.php"),
            "<?php",
        )
        .unwrap();
        let rows = list(&c).unwrap();
        assert!(
            rows[0].document_root.ends_with("public"),
            "{}",
            rows[0].document_root
        );
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn the_doctor_flags_a_site_whose_directory_is_gone() {
        let dir = std::env::temp_dir().join(format!("devd-sites-doctor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        std::env::set_var(
            hosts::ENV_HOSTS_FILE,
            paths.root().join("hosts-fixture").display().to_string(),
        );
        let store = Store::open(&paths.state_db()).unwrap();
        store
            .upsert_site(&Site {
                id: util::new_id(),
                name: "gone".into(),
                host: "gone.test".into(),
                root: paths.www().join("gone").display().to_string(),
                web_server: "nginx".into(),
                php: None,
                tls_mode: "internal".into(),
                db_name: None,
                upstream: None,
                serving: None,
                created_at: util::now(),
            })
            .unwrap();
        drop(store);

        let checks = doctor_check(&paths);
        assert_eq!(checks[0].level, devd_core::Level::Fail);
        assert!(
            checks[0].detail.contains("no project directory"),
            "{checks:?}"
        );
        assert!(
            checks.iter().any(|c| c.name == "sites.hosts"),
            "the hosts gap is reported too: {checks:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
