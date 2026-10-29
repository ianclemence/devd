//! Installing a database server on Windows, with a checksum devd pinned itself.
//!
//! EnterpriseDB (PostgreSQL) publishes its Windows binaries with no checksum at
//! all, and Oracle publishes MySQL's only as an MD5 on a web page. Most things
//! devd downloads are checked against a strong digest the publisher states; these
//! cannot be. What devd can do honestly is pin the SHA-256 of a build it has
//! downloaded and inspected, in this source file, so every later download is
//! compared against a fixed value that went through review rather than against
//! whatever the server returns today. For MySQL the pinned file also matched the
//! MD5 Oracle publishes.
//!
//! That is a weaker guarantee than a digest the vendor signs, and the install
//! result says so. A tampered mirror, a truncated transfer or a substituted file
//! are all refused. It also means devd installs exactly one version of each per
//! release of devd: the one pinned here.
//!
//! Linux and macOS take these from the system package manager and never come
//! through this file.

use std::path::Path;

use devd_core::capability::Ctx;
use devd_core::download::{Artifact, ProgressFn};
use devd_core::{download, DevdError, Result};

use crate::InstalledService;

/// One pinned build.
pub struct Pin {
    /// The service it installs.
    pub service: &'static str,
    /// What to call it.
    pub label: &'static str,
    /// The publisher's own version string.
    pub version: &'static str,
    /// The dotted version people ask for (`17.6`, `8.4.13`).
    pub short: &'static str,
    /// Where it is downloaded from.
    pub url: &'static str,
    /// SHA-256 devd computed over the download and pinned.
    pub sha256: &'static str,
    /// Size of the download, from the server's `Content-Length`.
    pub bytes: u64,
    /// The archive's top folder, with a trailing slash.
    pub prefix: &'static str,
    /// Folders under the top folder that are not needed to run the server and its tools.
    pub skip_dirs: &'static [&'static str],
    /// File endings that are never kept (debug symbols).
    pub skip_suffixes: &'static [&'static str],
    /// Who publishes it, for the install message.
    pub publisher: &'static str,
}

/// PostgreSQL 17.6 for Windows x64.
pub const POSTGRES: Pin = Pin {
    service: "postgres",
    label: "PostgreSQL",
    version: "17.6-1",
    short: "17.6",
    url: "https://get.enterprisedb.com/postgresql/postgresql-17.6-1-windows-x64-binaries.zip",
    sha256: "d378882abd001a186735acd6f6ba716bca6ccd192e800412d4fd15ed25376b3e",
    bytes: 329_891_687,
    prefix: "pgsql/",
    // The graphical administrator and the installer helper alone are most of the archive.
    skip_dirs: &["pgAdmin 4/", "StackBuilder/", "doc/", "symbols/", "include/"],
    skip_suffixes: &[],
    publisher: "EnterpriseDB",
};

/// MySQL 8.4 (the long-term-support series) for Windows x64.
pub const MYSQL: Pin = Pin {
    service: "mysql",
    label: "MySQL",
    version: "8.4.13",
    short: "8.4.13",
    url: "https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.13-winx64.zip",
    sha256: "e9cd98b1d23f122987b1afca26ad942efec5891481103d48ae3cde237fa3ea54",
    bytes: 281_154_912,
    prefix: "mysql-8.4.13-winx64/",
    skip_dirs: &["docs/", "include/", "lib/debug/"],
    skip_suffixes: &[".pdb", ".lib"],
    publisher: "Oracle",
};

/// The pin for a service, if devd installs it this way.
pub fn pin_for(service: &str) -> Option<&'static Pin> {
    [&POSTGRES, &MYSQL].into_iter().find(|p| p.service == service)
}

impl Pin {
    /// Whether a requested version is the one pinned: nothing or `latest`, the
    /// publisher's string, the short version, or a leading part of it (`17`, `8.4`).
    pub fn accepts(&self, spec: &str) -> bool {
        let want = spec.trim().trim_start_matches('v');
        want.is_empty()
            || want == "latest"
            || want == self.version
            || want == self.short
            || self.short.starts_with(&format!("{want}."))
    }

    /// Whether an archive entry is part of what devd keeps.
    pub fn wanted(&self, entry: &str) -> bool {
        let name = entry.replace('\\', "/");
        let Some(rest) = name.strip_prefix(self.prefix) else {
            return false;
        };
        !self.skip_dirs.iter().any(|d| rest.starts_with(d))
            && !self.skip_suffixes.iter().any(|s| rest.ends_with(s))
    }
}

/// Unpack the wanted entries of `archive` into `dest`, which is created.
///
/// Entries are placed with `enclosed_name`, which refuses any path that would
/// land outside `dest`.
pub fn unpack(pin: &Pin, archive: &Path, dest: &Path) -> Result<usize> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| DevdError::internal(format!("the archive is unreadable: {e}")))?;
    std::fs::create_dir_all(dest)?;
    let mut written = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| DevdError::internal(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if !pin.wanted(&rel.to_string_lossy()) {
            continue;
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut target = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut target)?;
        written += 1;
    }
    Ok(written)
}

/// Download, verify, unpack and record a pinned service.
pub fn install(
    ctx: &Ctx,
    pin: &Pin,
    spec: &str,
    port: Option<u16>,
    force: bool,
    progress: Option<&ProgressFn<'_>>,
) -> Result<InstalledService> {
    let name = pin.service;
    if !pin.accepts(spec) {
        return Err(DevdError::unsupported(format!(
            "this devd installs {} {} (the build whose checksum it has pinned); {spec} is not available",
            pin.label, pin.short
        )));
    }

    if let Ok(Some(existing)) = ctx.store.get_service(name) {
        if crate::running_pid(ctx, &existing).is_some() {
            return Err(DevdError::conflict(format!("{} is running; stop it first with `devd service stop {name}`", pin.label)));
        }
        if !force && crate::program_of(&existing).map(|p| p.is_file()).unwrap_or(false) {
            return Err(DevdError::conflict(format!("{} is already installed; pass --force to install it again", pin.label)));
        }
    }

    let artifact = Artifact::new(pin.url)
        .filename(format!("{name}-{}-windows-x64.zip", pin.version))
        .sha256(pin.sha256)
        .bytes(pin.bytes);
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), progress)?;

    // Unpack beside the final place and move it in, so a failure leaves nothing
    // half-installed where the service will look.
    let dest = crate::install_dir(ctx, name);
    let staging = dest.with_file_name(format!("{name}.installing"));
    let _ = std::fs::remove_dir_all(&staging);
    let count = unpack(pin, &fetched.path, &staging)?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&staging, &dest)?;

    let mut installed = crate::adopt(ctx, name, &dest, port)?;
    installed.digest = Some(fetched.sha256.clone());
    installed.bytes = fetched.bytes;
    installed.from_cache = fetched.from_cache;
    installed.warnings = vec![
        format!(
            "{} {} was downloaded from {} and checked against the SHA-256 devd pinned for it ({}). {} publishes no strong checksum, so this guards against a changed or damaged download, not against the publisher itself.",
            pin.label,
            pin.version,
            pin.publisher,
            &pin.sha256[..16],
            pin.publisher
        ),
        format!("{count} files were unpacked; documentation, headers and debug symbols were left out"),
        format!("devd's data for it is in {}", crate::engines::data_dir(ctx, name).display()),
    ];
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn only_the_server_and_its_tools_are_kept() {
        assert!(POSTGRES.wanted("pgsql/bin/postgres.exe"));
        assert!(POSTGRES.wanted("pgsql/lib/libpq.dll"));
        assert!(POSTGRES.wanted("pgsql/share/postgres.bki"));
        assert!(!POSTGRES.wanted("pgsql/pgAdmin 4/runtime/pgAdmin4.exe"));
        assert!(!POSTGRES.wanted("pgsql/StackBuilder/bin/stackbuilder.exe"));
        assert!(!POSTGRES.wanted("pgsql/doc/postgresql/html/index.html"));
        assert!(!POSTGRES.wanted("pgsql/include/libpq-fe.h"));
        assert!(!POSTGRES.wanted("elsewhere/bin/postgres.exe"));
        assert!(POSTGRES.wanted("pgsql\\bin\\psql.exe"), "a backslashed name is the same name");

        assert!(MYSQL.wanted("mysql-8.4.13-winx64/bin/mysqld.exe"));
        assert!(MYSQL.wanted("mysql-8.4.13-winx64/share/english/errmsg.sys"));
        assert!(MYSQL.wanted("mysql-8.4.13-winx64/lib/plugin/auth_socket.dll"));
        assert!(!MYSQL.wanted("mysql-8.4.13-winx64/bin/mysqld.pdb"));
        assert!(!MYSQL.wanted("mysql-8.4.13-winx64/docs/ChangeLog"));
        assert!(!MYSQL.wanted("mysql-8.4.13-winx64/include/mysql.h"));
        assert!(!MYSQL.wanted("other/bin/mysqld.exe"));
    }

    #[test]
    fn unpacking_keeps_the_wanted_files_and_refuses_a_path_that_climbs_out() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pg.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("pgsql/bin/postgres.exe", "server"),
                ("pgsql/pgAdmin 4/big.exe", "no"),
                ("pgsql/share/a.sample", "sample"),
                ("../escape.txt", "escaped"),
                ("pgsql/../../escape2.txt", "escaped"),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let dest = dir.path().join("out");
        let n = unpack(&POSTGRES, &archive, &dest).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read_to_string(dest.join("pgsql/bin/postgres.exe")).unwrap(), "server");
        assert!(!dest.join("pgsql/pgAdmin 4").exists());
        assert!(!dir.path().join("escape.txt").exists());
        assert!(!dir.path().join("escape2.txt").exists());
    }

    #[test]
    fn each_pin_is_a_well_formed_digest_and_only_the_pinned_version_is_accepted() {
        for pin in [&POSTGRES, &MYSQL] {
            assert_eq!(pin.sha256.len(), 64, "{}", pin.service);
            assert!(pin.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", pin.service);
            assert!(pin.url.starts_with("https://"));
            assert!(pin.prefix.ends_with('/'));
            assert!(pin.accepts("") && pin.accepts("latest") && pin.accepts(pin.short) && pin.accepts(pin.version));
            assert!(!pin.accepts("1.0.0"));
        }
        assert!(POSTGRES.accepts("17"));
        assert!(POSTGRES.accepts("v17.6"));
        assert!(!POSTGRES.accepts("16"));
        assert!(MYSQL.accepts("8.4.13"));
        assert!(!MYSQL.accepts("9.0.0"));
        assert!(pin_for("postgres").is_some() && pin_for("mysql").is_some() && pin_for("mariadb").is_none());
    }
}
