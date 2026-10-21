//! PHP, adopted from the system's package manager.
//!
//! php.net publishes binaries for Windows only. Everywhere else the
//! distribution (or Homebrew) builds PHP, so devd adopts the one that is
//! installed: it asks the package manager who owns it and whether the file
//! still matches the package (see [`devd_core::platform`]), links `php` and
//! `php-cgi` into the layout every other part of devd expects, and records the
//! runtime. The system's PHP configuration stays the system's.

use devd_core::capability::Ctx;
use devd_core::platform::{self, Host, Want};
use devd_core::state::Runtime;
use devd_core::util;
use devd_core::{DevdError, Result};

use crate::{persisted_settings, runtime_id, set_primary_php, Installed};

/// `php` runs scripts and `php-cgi` serves the sites nginx hands over. Debian
/// splits the second into its own package, which is why it is named in the
/// hint below.
const PHP: &[Want] = &[
    Want {
        link: "bin/php",
        names: &["php"],
    },
    Want {
        link: "bin/php-cgi",
        names: &["php-cgi"],
    },
];

/// Whether an installed version satisfies what was asked for.
///
/// `latest` (or nothing) accepts whatever the system has; `8.3` accepts any
/// 8.3 patch release; `8` any 8.x; a full version must match exactly.
pub fn satisfies(version: &str, spec: &str) -> bool {
    let spec = spec.trim().trim_start_matches('v');
    if spec.is_empty() || spec == "latest" || spec == "*" {
        return true;
    }
    version == spec || version.starts_with(&format!("{spec}."))
}

/// Adopt the PHP the system has.
pub fn install_php(ctx: &Ctx, spec: &str, force: bool, host: &dyn Host) -> Result<Installed> {
    let adoption = platform::adopt(
        host,
        "PHP with php-cgi",
        "Install it with `devd toolchain install php` (it shows what it will do and asks first) \
         or with your package manager - on Debian and Ubuntu the FastCGI program is the \
         `php-cgi` package - then run this again.",
        PHP,
        &["--version"],
    )?;
    let version = util::extract_version(&adoption.banner, Some(r"PHP (\d+\.\d+\.\d+)"))
        .ok_or_else(|| {
            DevdError::invalid(format!(
                "could not read a PHP version from `{}`",
                adoption.banner.trim()
            ))
        })?;
    if !satisfies(&version, spec) {
        return Err(DevdError::not_found(format!(
            "this system has PHP {version}, which is not {spec}. A package manager installs \
             one PHP at a time; install the version you want with it, or use a runtime \
             manager that provides several."
        )));
    }

    let dest = ctx.paths.runtime_version("php", &version);
    if dest.join("bin/php").exists() && !force {
        return Err(DevdError::conflict(format!(
            "php {version} is already installed at {}; pass force to link it again",
            dest.display()
        )));
    }
    platform::link_into(&dest, &adoption.found)?;

    let runtime = Runtime {
        id: runtime_id("php", &version),
        kind: "php".to_string(),
        version: version.clone(),
        path: dest.display().to_string(),
        digest: None,
        pinned: false,
        installed_at: util::now(),
    };
    ctx.store.upsert_runtime(&runtime)?;
    if persisted_settings(ctx).primary_php.is_none() {
        set_primary_php(ctx, &version)?;
    }

    let mut warnings = adoption.warnings.clone();
    if let Some(origin) = &adoption.origin {
        warnings.push(format!(
            "using {} {} from {}; its configuration stays in the system's php.ini",
            origin.package,
            origin.version,
            origin.manager.label()
        ));
    }

    ctx.events.emit_with(
        "runtime.php.installed".to_string(),
        format!("adopted php {version} from the system"),
        serde_json::json!({
            "kind": "php",
            "version": version,
            "path": runtime.path,
            "origin": adoption.origin,
        }),
    );

    Ok(Installed {
        executable: dest.join("bin/php").display().to_string(),
        runtime,
        verified_version: Some(version),
        enabled: Vec::new(),
        disabled: Vec::new(),
        warnings,
        bytes: 0,
        from_cache: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_request_matches_by_prefix_on_dots() {
        assert!(satisfies("8.3.6", ""));
        assert!(satisfies("8.3.6", "latest"));
        assert!(satisfies("8.3.6", "8"));
        assert!(satisfies("8.3.6", "8.3"));
        assert!(satisfies("8.3.6", "8.3.6"));
        assert!(satisfies("8.3.6", "v8.3"));
        assert!(!satisfies("8.3.6", "8.4"));
        assert!(!satisfies("8.3.6", "8.3.5"));
        // 8.30 is not a patch release of 8.3.
        assert!(!satisfies("8.30.1", "8.3"));
        assert!(!satisfies("8.3.6", "7"));
    }
}
