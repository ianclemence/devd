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
pub mod hosts;

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
}

/// Create a site: directory, certificate, record, configuration.
pub fn create(ctx: &Ctx, request: &CreateRequest) -> Result<Created> {
    util::validate_name(&request.name)?;
    let host = request
        .host
        .clone()
        .unwrap_or_else(|| format!("{}.{}", request.name, ctx.settings.tld));
    util::validate_hostname(&host)?;

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
    std::fs::create_dir_all(&directory)?;
    let starter = write_starter_page(&directory, &request.name)?;

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
        php: ctx
            .settings
            .primary_php
            .clone()
            .filter(|v| !v.is_empty()),
        tls_mode: tls.as_str().to_string(),
        db_name: None,
        created_at: util::now(),
    };
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

    let reloaded = reconcile(ctx, &mut warnings)?;

    // The hosts file is deliberately not written here; say so rather than leaving
    // the user with a name that does not resolve and no explanation. `localhost`
    // and its subdomains are exempt: they resolve without a hosts entry, which is
    // why the first-run site uses one.
    let managed = hosts::current_entries()?;
    if !resolves_without_hosts(&host) && !managed.iter().any(|e| e.host.eq_ignore_ascii_case(&host)) {
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
fn reconcile(ctx: &Ctx, warnings: &mut Vec<String>) -> Result<bool> {
    if ctx.settings.web_server.as_str() != "nginx" {
        return Ok(false);
    }
    match devd_services::nginx::render(ctx) {
        Ok(_) => {}
        Err(e) if e.code() == "not_found" => {
            warnings.push(
                "the web server is not installed, so nothing serves this site yet; \
                 `devd service install nginx` installs it"
                    .to_string(),
            );
            return Ok(false);
        }
        Err(e) => return Err(e),
    }

    let status = devd_services::status(ctx, "nginx")?;
    if !status.running {
        return Ok(false);
    }
    devd_services::nginx::reload(ctx)?;
    Ok(true)
}

/// Write a starter page when the project directory has nothing to serve.
fn write_starter_page(directory: &Path, name: &str) -> Result<Option<String>> {
    for candidate in ["index.php", "index.html", "index.htm"] {
        if directory.join(candidate).is_file() {
            return Ok(None);
        }
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
              through nginx, and PHP is running as FastCGI.</p>\n\
           <p>PHP <code><?= PHP_VERSION ?></code> on <?= PHP_OS_FAMILY ?>\n\
              (<?= htmlspecialchars(php_sapi_name()) ?>).</p>\n\
           <p>Document root <code><?= htmlspecialchars(__DIR__) ?></code>.</p>\n\
         </body>\n\
         </html>\n"
    );
    util::atomic_write(&php, &page)?;
    Ok(Some(php.display().to_string()))
}

/// Remove a site.
///
/// The files are kept unless they were explicitly asked for: deleting a project
/// because a hostname was renamed is not a mistake this should be able to make.
pub fn remove(ctx: &Ctx, name: &str, delete_files: bool) -> Result<serde_json::Value> {
    let site = ctx.store.get_site(name)?.ok_or_else(|| {
        DevdError::not_found(format!("no site named `{name}`; `devd site list` shows them"))
    })?;

    let mut warnings = Vec::new();

    // The certificate belongs to the hostname and stops being meaningful with
    // it, so it goes. It is regenerated on the next create.
    for path in [
        certificate_path(ctx, &site.host),
        key_path(ctx, &site.host),
    ] {
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
        warnings.push(format!(
            "the project directory {} was kept",
            site.root
        ));
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
}

/// Every site, with a live check of what is actually on disk.
pub fn list(ctx: &Ctx) -> Result<Vec<SiteRow>> {
    let managed = hosts::current_entries().unwrap_or_default();
    let server_running = devd_services::status(ctx, "nginx")
        .map(|s| s.running)
        .unwrap_or(false);

    Ok(ctx
        .store
        .list_sites()?
        .into_iter()
        .map(|site| SiteRow {
            present: Path::new(&site.root).is_dir(),
            url: url_for(ctx, &site),
            document_root: site.document_root(),
            in_hosts: managed.iter().any(|e| e.host == site.host),
            certificate: devd_services::tls::leaf_valid(ctx, &site.host),
            server_running,
            site,
        })
        .collect())
}

/// The site component of the environment check.
///
/// The useful check is the second one: a site whose directory has gone is
/// recorded, is in the hosts file, and returns 404 from the web server. Nothing
/// else in `devd` would notice, and the user would see an unexplained 404.
pub fn doctor_check(paths: &Paths) -> Vec<Check> {
    let Ok(store) = Store::open(&paths.state_db()) else {
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
        .filter(|s| !Path::new(&s.root).is_dir())
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
            format!("{} site(s), all of their directories are present", sites.len()),
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
        Ctx {            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(settings),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
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
        }
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

    #[test]
    fn a_site_without_tls_is_served_over_http_only() {
        let c = ctx("plain");
        let mut req = request("plain");
        req.tls = false;
        let created = create(&c, &req).unwrap();
        assert_eq!(created.site.tls_mode, "off");
        assert_eq!(created.url, "http://plain.test:8080");
        assert!(created.certificate.is_none(), "no certificate was requested");
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
    fn the_document_root_prefers_a_public_directory() {
        let c = ctx("public");
        let created = create(&c, &request("laravel")).unwrap();
        assert_eq!(created.document_root, created.directory, "no public/ yet");
        std::fs::create_dir_all(Path::new(&created.directory).join("public")).unwrap();
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
                created_at: util::now(),
            })
            .unwrap();
        drop(store);

        let checks = doctor_check(&paths);
        assert_eq!(checks[0].level, devd_core::Level::Fail);
        assert!(checks[0].detail.contains("no project directory"), "{checks:?}");
        assert!(
            checks.iter().any(|c| c.name == "sites.hosts"),
            "the hosts gap is reported too: {checks:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
