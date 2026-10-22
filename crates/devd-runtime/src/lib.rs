//! Managed runtimes: PHP, Node and Go.
//!
//! The promise this crate implements is the one Laragon makes with a folder full
//! of pre-bundled binaries, but without bundling them: **one command installs a
//! runtime, and it works**.
//!
//! ```text
//! devd runtime install php@8.3
//! devd runtime install node@lts
//! devd runtime install go@latest
//! ```
//!
//! Each install resolves the request against the vendor's own catalogue,
//! downloads the official build over HTTPS, checks the vendor's published
//! SHA-256, unpacks it under `runtimes/<kind>/<version>/`, performs the
//! post-install steps that make the build usable (for PHP, generating
//! `php.ini`), runs the result to prove it starts, and records it in the state
//! store.
//!
//! Nothing is bundled with `devd` and nothing is redistributed by it: every
//! byte comes from the vendor, on first use, under the vendor's own licence.
//! That is what keeps `devd` MIT-clean while still installing MySQL, which is
//! GPL.

pub mod caps;
pub mod catalogue;
pub mod php;
pub mod system;

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use devd_core::capability::Ctx;
use devd_core::config::Settings;
use devd_core::doctor::Check;
use devd_core::download::{self, Artifact, ProgressFn};
use devd_core::paths::Paths;
use devd_core::state::{Runtime, Store};
use devd_core::util;
use devd_core::{archive, DevdError, Result};

pub use catalogue::Available;

/// Runtime kinds `devd` can install.
pub const KINDS: &[&str] = &["php", "node", "go"];

/// Whether a kind is one of them.
pub fn known_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// Path of the executable inside an unpacked build, relative to its root.
///
/// The archives all wrap their content in one version-stamped directory, which
/// [`archive::extract`] strips, so these are the paths that remain.
pub fn executable_relative(kind: &str) -> Result<PathBuf> {
    let path = match (kind, cfg!(windows)) {
        ("php", true) => "php.exe",
        ("php", false) => "bin/php",
        ("node", true) => "node.exe",
        ("node", false) => "bin/node",
        ("go", true) => "bin/go.exe",
        ("go", false) => "bin/go",
        other => {
            return Err(DevdError::unsupported(format!(
                "`{}` is not a runtime devd can install (try one of: {})",
                other.0,
                KINDS.join(", ")
            )))
        }
    };
    Ok(PathBuf::from(path))
}

/// How long to allow a runtime to report its own version.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

/// An installed runtime, as the surfaces see it.
#[derive(Debug, Clone, Serialize)]
pub struct InstalledRow {
    /// Store row.
    pub runtime: Runtime,
    /// Whether the install directory is still on disk.
    pub present: bool,
    /// Whether the executable is where it should be.
    pub executable: String,
    /// Whether this is the runtime new sites use by default.
    pub primary: bool,
    /// Whether devd installed this runtime itself.
    ///
    /// A runtime found on the machine - one you installed with another tool -
    /// is real and usable, but devd does not own it: it cannot remove it,
    /// and it must never point devd's own services at a directory devd does
    /// not manage.
    pub managed: bool,
}

/// What an install produced.
#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    /// The recorded runtime.
    pub runtime: Runtime,
    /// Absolute path to the executable.
    pub executable: String,
    /// Version the interpreter reported when it was run.
    pub verified_version: Option<String>,
    /// Extensions or modules that were enabled.
    pub enabled: Vec<String>,
    /// Extensions switched back off because the build could not load them.
    pub disabled: Vec<String>,
    /// Warnings worth showing the user.
    pub warnings: Vec<String>,
    /// Bytes downloaded.
    pub bytes: u64,
    /// Whether the download came from the local cache.
    pub from_cache: bool,
}

/// Which builds are available upstream for a kind.
///
/// Returns the builds and any notes that explain an empty or partial list, so a
/// surface can say *why* nothing was found rather than showing a blank table.
pub fn available(kind: &str, thread_safe: bool) -> Result<(Vec<Available>, Vec<String>)> {
    let mut notes = Vec::new();
    let list = match kind {
        "php" => {
            if !cfg!(windows) {
                notes.push(
                    "PHP publishes binary builds for Windows only; on this platform devd uses \
                     the system PHP (install it with your package manager)"
                        .to_string(),
                );
            }
            catalogue::php(thread_safe)?
        }
        "node" => catalogue::node()?,
        "go" => catalogue::go()?,
        other => {
            return Err(DevdError::unsupported(format!(
                "`{other}` is not a runtime devd can install (try one of: {})",
                KINDS.join(", ")
            )))
        }
    };
    Ok((list, notes))
}

/// Download, verify, unpack and record a runtime.
pub fn install(
    ctx: &Ctx,
    kind: &str,
    spec: &str,
    thread_safe: bool,
    force: bool,
    progress: Option<&ProgressFn<'_>>,
) -> Result<Installed> {
    if !known_kind(kind) {
        return Err(DevdError::unsupported(format!(
            "`{kind}` is not a runtime devd can install (try one of: {})",
            KINDS.join(", ")
        )));
    }

    // php.net publishes binaries for Windows only; elsewhere the PHP the
    // system's package manager installed is adopted.
    if kind == "php" && devd_core::platform::Os::current().uses_system_packages() {
        return system::install_php(ctx, spec, force, &devd_core::platform::SystemHost);
    }

    let (list, notes) = available(kind, thread_safe)?;
    if list.is_empty() {
        return Err(DevdError::unsupported(if notes.is_empty() {
            format!("no {kind} builds are published for this platform")
        } else {
            notes.join("; ")
        }));
    }
    let chosen = catalogue::resolve(&list, spec, Some(if thread_safe { "ts" } else { "nts" }))
        .or_else(|e| {
            // Node and Go publish a single build per version, so an explicit
            // channel never matches them; retry without one before failing.
            if kind == "php" {
                Err(e)
            } else {
                catalogue::resolve(&list, spec, None)
            }
        })?;

    let dest = ctx.paths.runtime_version(kind, &chosen.version);
    if dest.exists() && !force {
        return Err(DevdError::conflict(format!(
            "{kind} {} is already installed at {}; pass force to reinstall it",
            chosen.version,
            dest.display()
        )));
    }

    let mut artifact = Artifact::new(chosen.url.clone());
    artifact.sha256 = chosen.sha256.clone();
    artifact.bytes = chosen.bytes;
    // Node publishes its digests in a per-release file rather than its index.
    if artifact.sha256.is_none() {
        if let Ok(Some(digest)) = catalogue::node_digest(&chosen.version, &artifact.filename) {
            artifact.sha256 = Some(digest);
        }
    }

    if let Some(report) = progress {
        report(&download::Progress {
            received: 0,
            total: chosen.bytes,
        });
    }
    let fetched = download::fetch(&artifact, &ctx.paths.cache(), progress)?;

    let mut warnings = Vec::new();
    if artifact.sha256.is_none() {
        warnings.push(format!(
            "upstream publishes no digest for {}, so the download could only be checked for \
             length ({} bytes); it came over HTTPS from {}",
            artifact.filename, fetched.bytes, chosen.url
        ));
    }

    archive::extract(&fetched.path, &dest, force)?;

    let mut enabled = Vec::new();
    let mut disabled = Vec::new();
    let mut warnings = Vec::new();
    let verified_version = match kind {
        "php" => {
            let setup = php::configure_and_verify(&dest, &ctx.paths.logs(), &chosen.version)?;
            enabled = setup.enabled;
            disabled = setup.disabled;
            warnings.extend(setup.info.warnings);
            Some(setup.info.version)
        }
        _ => {
            let exe = dest.join(executable_relative(kind)?);
            if !exe.is_file() {
                return Err(DevdError::invalid(format!(
                    "the {kind} build unpacked without {}; the archive layout was not what \
                     devd expected",
                    exe.display()
                )));
            }
            let out = util::run(
                &exe,
                &[version_flag(kind)],
                Some(&dest),
                &[],
                Some(VERIFY_TIMEOUT),
            )?;
            if !out.ok() {
                return Err(DevdError::Command {
                    program: exe.display().to_string(),
                    code: out.code,
                    stderr: util::truncate(out.stderr.trim(), 400),
                });
            }
            first_version(&out.stdout)
        }
    };

    let runtime = Runtime {
        id: runtime_id(kind, &chosen.version),
        kind: kind.to_string(),
        version: chosen.version.clone(),
        path: dest.display().to_string(),
        digest: Some(fetched.sha256.clone()),
        pinned: false,
        installed_at: util::now(),
    };
    ctx.store.upsert_runtime(&runtime)?;

    // A first runtime becomes the default for new sites. This is the behaviour a
    // Laragon user expects: install PHP and sites use it, without a second step.
    if kind == "php" && persisted_settings(ctx).primary_php.is_none() {
        set_primary_php(ctx, &chosen.version)?;
    }

    ctx.events.emit_with(
        format!("runtime.{kind}.installed"),
        format!("installed {kind} {}", chosen.version),
        json!({
            "kind": kind,
            "version": chosen.version,
            "path": runtime.path,
            "sha256": fetched.sha256,
        }),
    );

    Ok(Installed {
        executable: dest.join(executable_relative(kind)?).display().to_string(),
        runtime,
        verified_version,
        enabled,
        disabled,
        warnings,
        bytes: fetched.bytes,
        from_cache: fetched.from_cache,
    })
}

fn version_flag(kind: &str) -> String {
    match kind {
        "go" => "version".to_string(),
        _ => "--version".to_string(),
    }
}

/// The first token that looks like a version, from a `--version` style output.
///
/// Handles the three shapes the toolchains use: `v22.14.0` (Node), `go1.24.1`
/// (Go, whose first token is the word `version`) and `8.3.35` (PHP's banner,
/// where the version follows the word `PHP`).
fn first_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|token| {
            let token = token.trim_start_matches('v');
            token.strip_prefix("go").unwrap_or(token)
        })
        .find(|token| {
            token.chars().next().is_some_and(|c| c.is_ascii_digit()) && token.contains('.')
        })
        .map(str::to_string)
}

/// Settings as they are on disk.
///
/// The `Ctx` carries a snapshot taken at start-up and is never mutated in place,
/// so a surface that writes a setting and then reads it back - the application
/// window, or a test - has to look at the file. Falling back to the snapshot
/// keeps a read working when the file cannot be read at all.
fn persisted_settings(ctx: &Ctx) -> Settings {
    Settings::load(&ctx.paths.settings_file()).unwrap_or_else(|_| (*ctx.settings).clone())
}

/// Every runtime `devd` has recorded, with a live check of the disk.
pub fn list(ctx: &Ctx) -> Result<Vec<InstalledRow>> {
    let settings = persisted_settings(ctx);
    let rows = ctx.store.list_runtimes()?;
    let mut out = Vec::new();
    for runtime in rows {
        let root = PathBuf::from(&runtime.path);
        let exe = executable_relative(&runtime.kind)
            .map(|rel| root.join(rel))
            .unwrap_or_else(|_| root.clone());
        let primary = runtime.kind == "php"
            && settings.primary_php.as_deref() == Some(runtime.version.as_str());
        out.push(InstalledRow {
            present: root.is_dir(),
            runtime,
            executable: exe.display().to_string(),
            primary,
            managed: true,
        });
    }
    // A runtime that is installed on this machine but that devd did not
    // install is still a runtime. Saying "not installed" because the row is
    // missing from devd's own registry is a lie the person can see through.
    for found in discover() {
        if out.iter().any(|r| r.runtime.kind == found.runtime.kind && r.runtime.version == found.runtime.version) {
            continue;
        }
        out.push(found);
    }
    out.sort_by(|a, b| {
        a.runtime
            .kind
            .cmp(&b.runtime.kind)
            .then_with(|| b.runtime.version.cmp(&a.runtime.version))
    });
    Ok(out)
}

/// The runtimes this machine already has, found by asking the machine.
///
/// Only the interpreters are asked for, and only once each. Probing every
/// tool in the catalogue takes minutes and has no place on a screen that
/// answers in a moment.
fn discover() -> Vec<InstalledRow> {
    let mut out = Vec::new();
    // Each interpreter is asked in its own language: Go says `go version` and
    // has no `--version` at all.
    for (kind, exe, args, pattern) in [
        ("php", "php", &["--version"][..], r"PHP (\d+\.\d+\.\d+)"),
        ("node", "node", &["--version"][..], r"v(\d+\.\d+\.\d+)"),
        ("go", "go", &["version"][..], r"go(\d+\.\d+(?:\.\d+)?)"),
    ] {
        let Some(exe) = devd_core::util::which(exe) else { continue };
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let Ok(result) = devd_core::util::run(
            &exe,
            &args,
            None,
            &[],
            Some(std::time::Duration::from_secs(10)),
        ) else {
            continue;
        };
        let text = format!("{}\n{}", result.stdout, result.stderr);
        let Some(version) = util::extract_version(&text, Some(pattern)) else {
            continue;
        };
        out.push(InstalledRow {
            runtime: Runtime {
                id: format!("{kind}-{version}-system"),
                kind: kind.into(),
                version: version.clone(),
                path: exe.display().to_string(),
                digest: None,
                pinned: false,
                installed_at: String::new(),
            },
            present: true,
            executable: exe.display().to_string(),
            // The primary is whatever devd installed and marked as such. A
            // runtime found on the machine is a fallback, never the primary.
            primary: false,
            managed: false,
        });
    }
    out
}

/// Run an installed runtime and report what it says.
pub fn verify(ctx: &Ctx, kind: &str, version: &str) -> Result<serde_json::Value> {
    let root = ctx.paths.runtime_version(kind, version);
    if !root.is_dir() {
        return Err(DevdError::not_found(format!(
            "{kind} {version} is not installed"
        )));
    }
    if kind == "php" {
        let info = php::verify(&root)?;
        return Ok(json!({
            "kind": kind,
            "version": info.version,
            "modules": info.modules,
            "warnings": info.warnings,
        }));
    }
    let exe = root.join(executable_relative(kind)?);
    let out = util::run(
        &exe,
        &[version_flag(kind)],
        Some(&root),
        &[],
        Some(VERIFY_TIMEOUT),
    )?;
    if !out.ok() {
        return Err(DevdError::Command {
            program: exe.display().to_string(),
            code: out.code,
            stderr: util::truncate(out.stderr.trim(), 400),
        });
    }
    Ok(json!({
        "kind": kind,
        "version": first_version(&out.stdout).unwrap_or_else(|| version.to_string()),
        "output": util::truncate(out.stdout.trim(), 300),
    }))
}

/// Delete an installed runtime.
pub fn remove(ctx: &Ctx, kind: &str, version: &str) -> Result<serde_json::Value> {
    // A service that is wired to this runtime would be left pointing at a
    // directory that no longer exists. Refusing is better than a broken nginx.
    for service in ctx.store.list_services()? {
        if let Some(referenced) = &service.runtime_id {
            if referenced == &runtime_id(kind, version) {
                return Err(DevdError::conflict(format!(
                    "`{}` is configured to use {kind} {version}; stop and remove it first",
                    service.name
                )));
            }
        }
    }

    let root = ctx.paths.runtime_version(kind, version);
    let existed = root.is_dir();
    if existed {
        std::fs::remove_dir_all(&root)?;
    }
    let removed_row = ctx.store.delete_runtime(kind, version)?;

    if !existed && !removed_row {
        return Err(DevdError::not_found(format!(
            "{kind} {version} is not installed"
        )));
    }

    // Reassignment after removal: a site pinned to a version that is gone would
    // otherwise serve nothing, with no explanation.
    let mut reassigned = Vec::new();
    if kind == "php" && ctx.settings.primary_php.as_deref() == Some(version) {
        let fallback = ctx
            .store
            .runtimes_of_kind("php")?
            .into_iter()
            .map(|r| r.version)
            .max_by_key(|v| catalogue::version_key(v));
        match fallback {
            Some(v) => {
                set_primary_php(ctx, &v)?;
                reassigned.push(v);
            }
            None => {
                let mut settings = (*ctx.settings).clone();
                settings.primary_php = None;
                settings.save(&ctx.paths.settings_file())?;
            }
        }
    }

    ctx.events.emit(
        format!("runtime.{kind}.removed"),
        format!("removed {kind} {version}"),
    );

    Ok(json!({
        "kind": kind,
        "version": version,
        "path": root.display().to_string(),
        "directory_removed": existed,
        "primary_reassigned_to": reassigned.first(),
    }))
}

/// Make a runtime the default for new sites.
pub fn set_primary_php(ctx: &Ctx, version: &str) -> Result<()> {
    if !ctx.paths.runtime_version("php", version).is_dir() {
        return Err(DevdError::not_found(format!(
            "php {version} is not installed"
        )));
    }
    let mut settings = (*ctx.settings).clone();
    settings.primary_php = Some(version.to_string());
    settings.validate()?;
    settings.save(&ctx.paths.settings_file())
}

/// Idempotent identifier for a runtime.
pub fn runtime_id(kind: &str, version: &str) -> String {
    format!("{kind}-{version}")
}

/// Resolve the executable of an installed runtime.
///
/// With no version, PHP resolves through the settings' primary version and the
/// other kinds through the newest install, which is what a site wants when it
/// has not pinned anything.
pub fn executable(ctx: &Ctx, kind: &str, version: Option<&str>) -> Result<PathBuf> {
    let installed = ctx.store.runtimes_of_kind(kind)?;
    if installed.is_empty() {
        return Err(DevdError::not_found(format!(
            "no {kind} runtime is installed; run `devd runtime install {kind}@latest`"
        )));
    }
    let chosen = match version {
        Some(v) => installed
            .iter()
            .find(|r| r.version == v)
            .ok_or_else(|| DevdError::not_found(format!("{kind} {v} is not installed")))?,
        None => {
            let preferred = if kind == "php" {
                persisted_settings(ctx).primary_php.clone()
            } else {
                None
            };
            preferred
                .and_then(|v| installed.iter().find(|r| r.version == v))
                .or_else(|| {
                    installed
                        .iter()
                        .max_by_key(|r| catalogue::version_key(&r.version))
                })
                .ok_or_else(|| DevdError::not_found(format!("no {kind} runtime is installed")))?
        }
    };
    let root = PathBuf::from(&chosen.path);
    let exe = root.join(executable_relative(kind)?);
    if !exe.is_file() {
        return Err(DevdError::not_found(format!(
            "the {kind} {} install is missing {}",
            chosen.version,
            exe.display()
        )));
    }
    Ok(exe)
}

/// The runtime component of the environment check.
pub fn doctor_check(paths: &Paths) -> Vec<Check> {
    let Ok(store) = Store::open(&paths.state_db()) else {
        return vec![Check::fail("runtimes", "the state store could not be read")];
    };
    let Ok(rows) = store.list_runtimes() else {
        return vec![Check::fail(
            "runtimes",
            "the runtime table could not be read",
        )];
    };
    if rows.is_empty() {
        return vec![Check::info(
            "runtimes",
            "none installed yet; `devd runtime install php@8.3` adds one",
        )];
    }

    let mut checks = Vec::new();
    let mut missing = Vec::new();
    for runtime in &rows {
        let root = PathBuf::from(&runtime.path);
        let exe = executable_relative(&runtime.kind)
            .map(|rel| root.join(rel))
            .unwrap_or_else(|_| root.clone());
        if !exe.is_file() {
            missing.push(format!("{}-{}", runtime.kind, runtime.version));
        }
    }
    let summary = rows
        .iter()
        .map(|r| format!("{} {}", r.kind, r.version))
        .collect::<Vec<_>>()
        .join(", ");
    if missing.is_empty() {
        checks.push(Check::ok(
            "runtimes",
            format!("{} installed: {summary}", rows.len()),
        ));
    } else {
        checks.push(Check::fail(
            "runtimes",
            format!(
                "{} installed but {} is missing its executable; reinstall with \
                 `devd runtime install <kind>@<version>`",
                summary,
                missing.join(", ")
            ),
        ));
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kinds_are_rejected_with_the_list() {
        let err = executable_relative("ruby").unwrap_err();
        assert_eq!(err.code(), "unsupported");
        assert!(err.to_string().contains("php, node, go"), "{err}");
    }

    #[test]
    fn executable_paths_match_the_unpacked_layouts() {
        let php = executable_relative("php").unwrap();
        let go = executable_relative("go").unwrap();
        assert!(
            php.to_string_lossy().ends_with("php.exe")
                || php.to_string_lossy().ends_with("bin/php")
        );
        assert!(go
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with("bin/go"));
    }

    #[test]
    fn runtime_ids_are_stable_and_self_describing() {
        assert_eq!(runtime_id("php", "8.3.35"), "php-8.3.35");
    }

    #[test]
    fn versions_are_picked_out_of_tool_output() {
        assert_eq!(first_version("v22.14.0\n").as_deref(), Some("22.14.0"));
        assert_eq!(
            first_version("go version go1.24.1 windows/amd64").as_deref(),
            Some("1.24.1")
        );
        assert_eq!(first_version("no version here"), None);
    }

    #[test]
    fn doctor_reports_an_empty_install_as_information() {
        let dir = std::env::temp_dir().join(format!("devd-rt-doctor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let checks = doctor_check(&paths);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].level, devd_core::Level::Info);
        assert!(
            checks[0].detail.contains("devd runtime install"),
            "{checks:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doctor_fails_when_an_installed_runtime_lost_its_binary() {
        let dir = std::env::temp_dir().join(format!("devd-rt-doctor2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::rooted(&dir);
        paths.ensure().unwrap();
        let store = Store::open(&paths.state_db()).unwrap();
        store
            .upsert_runtime(&Runtime {
                id: runtime_id("php", "9.9.9"),
                kind: "php".to_string(),
                version: "9.9.9".to_string(),
                path: paths.runtime_version("php", "9.9.9").display().to_string(),
                digest: None,
                pinned: false,
                installed_at: util::now(),
            })
            .unwrap();
        drop(store);
        let checks = doctor_check(&paths);
        assert_eq!(checks[0].level, devd_core::Level::Fail);
        assert!(checks[0].detail.contains("9.9.9"), "{checks:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
