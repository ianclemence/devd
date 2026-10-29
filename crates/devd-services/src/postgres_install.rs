//! Installing PostgreSQL on Windows, with a checksum devd pinned itself.
//!
//! EnterpriseDB publishes the Windows binaries as a plain zip and publishes no
//! checksum for it. Most things devd downloads are checked against a digest the
//! publisher states; this one cannot be. What devd can do honestly is pin the
//! digest of a build it has downloaded and inspected, in this source file, so
//! that every later download is compared against a fixed value that went through
//! review rather than against whatever the server returns today.
//!
//! That is a weaker guarantee than a digest the vendor signs, and it is stated as
//! such in the install result. It does mean a tampered mirror, a truncated
//! transfer or a substituted file are all refused. It also means devd installs
//! exactly one PostgreSQL version per release of devd: the one pinned here.
//!
//! Linux and macOS take PostgreSQL from the system package manager, as they take
//! nginx and MariaDB, and never come through this file.

use std::path::Path;

use devd_core::capability::Ctx;
use devd_core::download::{Artifact, ProgressFn};
use devd_core::{download, util, DevdError, Result};

use crate::InstalledService;

/// The pinned build.
pub struct Pinned {
    /// Version, as the publisher numbers it (`17.6-1`).
    pub version: &'static str,
    /// Where it is downloaded from.
    pub url: &'static str,
    /// SHA-256 devd computed over the download and pinned.
    pub sha256: &'static str,
    /// Size of the download, from the server's `Content-Length`.
    pub bytes: u64,
}

/// PostgreSQL 17.6 for Windows x64.
pub const PINNED: Pinned = Pinned {
    version: "17.6-1",
    url: "https://get.enterprisedb.com/postgresql/postgresql-17.6-1-windows-x64-binaries.zip",
    sha256: "d378882abd001a186735acd6f6ba716bca6ccd192e800412d4fd15ed25376b3e",
    bytes: 329_891_687,
};

/// Directories in the archive that are not needed to run the server and its
/// tools. The graphical administrator and the installer helper alone are most of
/// the archive's size.
const SKIP: &[&str] = &["pgsql/pgAdmin 4/", "pgsql/StackBuilder/", "pgsql/doc/", "pgsql/symbols/", "pgsql/include/"];

/// Whether an archive entry is part of what devd keeps.
pub fn wanted(entry: &str) -> bool {
    let name = entry.replace('\\', "/");
    name.starts_with("pgsql/") && !SKIP.iter().any(|s| name.starts_with(s))
}

/// Unpack the wanted entries of `archive` into `dest`, which is created.
///
/// Entries are placed with `enclosed_name`, which refuses any path that would
/// land outside `dest`.
pub fn unpack(archive: &Path, dest: &Path) -> Result<usize> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| DevdError::internal(format!("the archive is unreadable: {e}")))?;
    std::fs::create_dir_all(dest)?;
    let mut written = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| DevdError::internal(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if !wanted(&rel.to_string_lossy()) {
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

/// Download, verify, unpack and record PostgreSQL.
pub fn install(
    ctx: &Ctx,
    spec: &str,
    port: Option<u16>,
    force: bool,
    progress: Option<&ProgressFn<'_>>,
) -> Result<InstalledService> {
    let wanted_version = spec.trim().trim_start_matches('v');
    let major_minor = PINNED.version.split('-').next().unwrap_or(PINNED.version);
    if !(wanted_version.is_empty()
        || wanted_version == "latest"
        || wanted_version == PINNED.version
        || wanted_version == major_minor
        || PINNED.version.starts_with(&format!("{wanted_version}.")))
    {
        return Err(DevdError::unsupported(format!(
            "this devd installs PostgreSQL {major_minor} (the build whose checksum it has pinned); {spec} is not available"
        )));
    }

    if let Ok(Some(existing)) = ctx.store.get_service("postgres") {
        if crate::running_pid(ctx, &existing).is_some() {
            return Err(DevdError::conflict("PostgreSQL is running; stop it first with `devd service stop postgres`"));
        }
        if !force && crate::program_of(&existing).map(|p| p.is_file()).unwrap_or(false) {
            return Err(DevdError::conflict("PostgreSQL is already installed; pass --force to install it again"));
        }
    }

    let artifact = Artifact::new(PINNED.url)
        .filename(format!("postgresql-{}-windows-x64-binaries.zip", PINNED.version))
        .sha256(PINNED.sha256)
        .bytes(PINNED.bytes);
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), progress)?;

    // Unpack beside the final place and move it in, so a failure leaves nothing
    // half-installed where the service will look.
    let dest = crate::install_dir(ctx, "postgres");
    let staging = dest.with_file_name("postgres.installing");
    let _ = std::fs::remove_dir_all(&staging);
    let count = unpack(&fetched.path, &staging)?;
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&staging, &dest)?;

    let mut installed = crate::adopt(ctx, "postgres", &dest, port)?;
    installed.digest = Some(fetched.sha256.clone());
    installed.bytes = fetched.bytes;
    installed.from_cache = fetched.from_cache;
    installed.warnings = vec![
        format!(
            "PostgreSQL {} was downloaded from EnterpriseDB and checked against the SHA-256 devd pinned for it ({}). EnterpriseDB publishes no checksum, so this guards against a changed or damaged download, not against the publisher itself.",
            PINNED.version,
            &PINNED.sha256[..16]
        ),
        format!("{count} files were unpacked; the graphical administrator and documentation were left out"),
        format!("devd's data for it is in {}", crate::engines::data_dir(ctx, "postgres").display()),
    ];
    let _ = util::now();
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn only_the_server_and_its_tools_are_kept() {
        assert!(wanted("pgsql/bin/postgres.exe"));
        assert!(wanted("pgsql/lib/libpq.dll"));
        assert!(wanted("pgsql/share/postgres.bki"));
        assert!(!wanted("pgsql/pgAdmin 4/runtime/pgAdmin4.exe"));
        assert!(!wanted("pgsql/StackBuilder/bin/stackbuilder.exe"));
        assert!(!wanted("pgsql/doc/postgresql/html/index.html"));
        assert!(!wanted("pgsql/include/libpq-fe.h"));
        assert!(!wanted("elsewhere/bin/postgres.exe"));
        assert!(wanted("pgsql\\bin\\psql.exe"), "a backslashed name is the same name");
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
        let n = unpack(&archive, &dest).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read_to_string(dest.join("pgsql/bin/postgres.exe")).unwrap(), "server");
        assert!(!dest.join("pgsql/pgAdmin 4").exists());
        assert!(!dir.path().join("escape.txt").exists());
        assert!(!dir.path().join("escape2.txt").exists());
    }

    #[test]
    fn the_pin_is_a_well_formed_digest_and_only_the_pinned_version_is_offered() {
        assert_eq!(PINNED.sha256.len(), 64);
        assert!(PINNED.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(PINNED.url.starts_with("https://"));
    }
}
