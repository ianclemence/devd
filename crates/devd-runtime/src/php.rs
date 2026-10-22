//! PHP-specific install steps.
//!
//! A PHP zip is not usable when it is unpacked. It ships `php.ini-development`
//! and nothing else, so out of the box there is no `php.ini`, no extension
//! directory wiring and none of the extensions a Laravel or WordPress project
//! assumes. This module turns the upstream build into a working development
//! interpreter, and then *proves* it works by running it.
//!
//! Four rules keep that honest, and each one exists because the naive version
//! fails on the real PHP distribution:
//!
//! * **Only extensions the build ships are considered.** `devd` reads the
//!   build's own `ext/` directory rather than trusting a list.
//! * **Only a curated set is enabled.** The template comments out forty
//!   extensions, several of which (`pdo_firebird`, `pdo_oci`, `pgsql`, `snmp`)
//!   load a client library that is not present, so enabling everything shipped
//!   produces a `php.ini` that fails on every request.
//! * **Extensions already compiled in are skipped.** `php -n -m` is run first,
//!   so a module the build provides statically is never also loaded from a DLL
//!   - which is what "Module mysqli is already loaded" means.
//! * **Whatever still fails to load is switched back off.** After generating
//!   the file the interpreter is run, the extensions that reported a startup
//!   error are commented out, and it is run again. The install reports both what
//!   was enabled and what had to be disabled, instead of leaving a file full of
//!   warnings for the user to discover.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use devd_core::util;
use devd_core::{DevdError, Result};

/// Extensions enabled when the build ships them.
///
/// This is the intersection of "what a PHP web project needs" and "what loads
/// without an external client library" on the Windows builds. Extensions that
/// require a third-party client - Firebird, Oracle, PostgreSQL, SNMP, IMAP - are
/// deliberately absent: they cannot work on a machine that has never had those
/// clients installed, and a `php.ini` that logs a startup error on every request
/// is worse than one missing an extension nobody asked for.
pub const PREFERRED_EXTENSIONS: &[&str] = &[
    "bz2",
    "curl",
    "exif",
    "fileinfo",
    "ftp",
    "gd",
    "gettext",
    "gmp",
    "intl",
    "ldap",
    "mbstring",
    "mysqli",
    "odbc",
    "openssl",
    "pdo_mysql",
    "pdo_odbc",
    "pdo_sqlite",
    "soap",
    "sockets",
    "sodium",
    "sqlite3",
    "xsl",
    "zip",
];

/// Zend extensions enabled when the build ships them. `opcache` is not an
/// ordinary extension: loading it with `extension=` instead of `zend_extension=`
/// is a startup error.
pub const PREFERRED_ZEND_EXTENSIONS: &[&str] = &["opcache"];

/// What running the interpreter reported.
#[derive(Debug, Clone, Serialize)]
pub struct PhpInfo {
    /// Version as the interpreter reports it, e.g. `8.3.35`.
    pub version: String,
    /// Modules the interpreter loaded.
    pub modules: Vec<String>,
    /// Startup problems the interpreter reported.
    pub warnings: Vec<String>,
}

/// Result of generating and then testing a `php.ini`.
#[derive(Debug, Clone, Serialize)]
pub struct PhpSetup {
    /// Extensions enabled.
    pub enabled: Vec<String>,
    /// Extensions switched back off because the build could not load them.
    pub disabled: Vec<String>,
    /// What the interpreter reports once the file is settled.
    pub info: PhpInfo,
}

/// How long to allow `php -v` / `php -m` to answer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Path to the interpreter in an install directory.
pub fn executable(install: &Path) -> PathBuf {
    install.join(if cfg!(windows) { "php.exe" } else { "bin/php" })
}

/// Generate `php.ini`, run the interpreter, and switch off anything that will
/// not load. This is the whole post-install step for PHP.
pub fn configure_and_verify(install: &Path, logs: &Path, version: &str) -> Result<PhpSetup> {
    let mut enabled = configure_ini(install, logs, version)?;
    let mut info = verify(install)?;
    let mut disabled = Vec::new();

    // Two rounds is enough: the first removes everything that failed, the second
    // confirms the result. A third would only be needed if disabling one
    // extension broke another, which PHP does not do.
    for _ in 0..2 {
        let failed = failed_modules(&info);
        if failed.is_empty() {
            break;
        }
        disable_extensions(&install.join("php.ini"), &failed)?;
        disabled.extend(failed.iter().cloned());
        info = verify(install)?;
    }

    enabled.retain(|name| !disabled.contains(name));

    Ok(PhpSetup {
        enabled,
        disabled,
        info,
    })
}

/// Generate `php.ini` from the build's own development template.
///
/// Returns the extensions that were enabled.
pub fn configure_ini(install: &Path, logs: &Path, version: &str) -> Result<Vec<String>> {
    let template_path = ["php.ini-development", "php.ini-production"]
        .iter()
        .map(|name| install.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            DevdError::invalid(format!(
                "{} does not contain php.ini-development; it is not a PHP distribution",
                install.display()
            ))
        })?;

    let template = std::fs::read_to_string(&template_path)?;
    let available = shipped_extensions(install);
    let builtin = builtin_modules(install);

    let (mut text, enabled) = render_ini(&template, &available, &builtin);
    text.push_str(&development_defaults(logs, version));
    util::atomic_write(&install.join("php.ini"), &text)?;
    Ok(enabled)
}

/// The pure part of `php.ini` generation: no filesystem, no processes.
///
/// Separated so that the rules can be tested directly - including the cases the
/// real distributions produce, such as one extension being listed twice and
/// another being compiled in.
pub fn render_ini(
    template: &str,
    available: &BTreeSet<String>,
    builtin: &BTreeSet<String>,
) -> (String, Vec<String>) {
    let mut out = String::with_capacity(template.len() + 1024);
    let mut enabled: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in template.lines() {
        let trimmed = line.trim_start();

        // The extension directory has to be set before any `extension=` line is
        // read, which is why this is edited in place rather than appended.
        if trimmed.starts_with(";extension_dir") {
            out.push_str("extension_dir = \"ext\"\n");
            continue;
        }

        if let Some((zend, name)) = commented_extension(trimmed) {
            let wanted = if zend {
                PREFERRED_ZEND_EXTENSIONS.contains(&name.as_str())
            } else {
                PREFERRED_EXTENSIONS.contains(&name.as_str())
            };
            let loadable =
                available.contains(&name) && !builtin.contains(&name) && !seen.contains(&name);
            if wanted && loadable {
                seen.insert(name.clone());
                out.push_str(&format!(
                    "{}={name}\n",
                    if zend { "zend_extension" } else { "extension" }
                ));
                enabled.push(name);
                continue;
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    (out, enabled)
}

/// Extension names whose DLL this build actually ships.
fn shipped_extensions(install: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(install.join("ext")) else {
        return set;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        // `php_curl.dll` -> `curl`; `php_pdo_mysql.dll` -> `pdo_mysql`.
        if let Some(stem) = name
            .strip_prefix("php_")
            .and_then(|n| n.strip_suffix(".dll"))
        {
            set.insert(stem.to_string());
        }
    }
    set
}

/// Modules the build provides without any `php.ini` at all.
///
/// Best effort: on a machine missing the runtime PHP needs, this returns nothing
/// and the generated file simply includes those extensions, which the follow-up
/// verification then corrects.
fn builtin_modules(install: &Path) -> BTreeSet<String> {
    let exe = executable(install);
    if !exe.is_file() {
        return BTreeSet::new();
    }
    let Ok(out) = util::run(
        &exe,
        &["-n".to_string(), "-m".to_string()],
        Some(install),
        &[],
        Some(PROBE_TIMEOUT),
    ) else {
        return BTreeSet::new();
    };
    parse_module_list(&out.stdout)
        .into_iter()
        .map(|m| m.to_lowercase())
        .collect()
}

/// `;extension=curl`, `;zend_extension=opcache` and the older
/// `;extension=php_curl.dll` all resolve to `(is_zend, name)`.
fn commented_extension(line: &str) -> Option<(bool, String)> {
    let rest = line.strip_prefix(';')?.trim_start();
    let (zend, value) = if let Some(v) = rest.strip_prefix("zend_extension=") {
        (true, v)
    } else {
        let v = rest.strip_prefix("extension=")?;
        (false, v)
    };
    // Several entries carry a trailing explanation: `;extension=exif ; Must be
    // after mbstring as it depends on it`.
    let value = value.split(';').next().unwrap_or("").trim();
    if value.is_empty() || value.starts_with('"') {
        return None;
    }
    let base = value.strip_prefix("php_").unwrap_or(value);
    let name = base.strip_suffix(".dll").unwrap_or(base).trim();
    if name.is_empty() || name.contains(std::path::MAIN_SEPARATOR) || name.contains('/') {
        return None;
    }
    Some((zend, name.to_lowercase()))
}

/// The devd development defaults, appended to the generated `php.ini`.
fn development_defaults(logs: &Path, version: &str) -> String {
    let error_log = logs.join(format!("php-{version}.log"));
    format!(
        "\n; ----------------------------------------------------------------- devd\n\
         ; Written by `devd runtime install`. Local development values: errors are\n\
         ; visible, uploads and memory limits are generous, and output is logged to\n\
         ; the devd log directory so the application can show it.\n\
         error_reporting = E_ALL\n\
         display_errors = On\n\
         display_startup_errors = On\n\
         log_errors = On\n\
         error_log = \"{error_log}\"\n\
         memory_limit = 512M\n\
         upload_max_filesize = 64M\n\
         post_max_size = 64M\n\
         max_execution_time = 120\n\
         max_input_time = 120\n\
         date.timezone = UTC\n\
         ; nginx hands over the script path, so the fix-up has to be on.\n\
         cgi.fix_pathinfo = 1\n\
         ; Realpath cache tuned for a project tree that is being edited constantly.\n\
         realpath_cache_size = 4096k\n\
         realpath_cache_ttl = 120\n\
         opcache.enable = 1\n\
         opcache.enable_cli = 0\n\
         opcache.memory_consumption = 128\n\
         opcache.max_accelerated_files = 10000\n\
         opcache.validate_timestamps = 1\n\
         opcache.revalidate_freq = 0\n",
        error_log = error_log.display()
    )
}

/// Run the interpreter and report what it says about itself.
pub fn verify(install: &Path) -> Result<PhpInfo> {
    let exe = executable(install);
    if !exe.is_file() {
        return Err(DevdError::invalid(format!(
            "{} does not contain a PHP interpreter",
            install.display()
        )));
    }

    let version_out = util::run(
        &exe,
        &["-v".to_string()],
        Some(install),
        &[],
        Some(PROBE_TIMEOUT),
    )?;

    // With `display_errors = On`, PHP writes startup warnings to standard
    // output, so the version has to be found rather than assumed to be first.
    let version = version_token(&version_out.stdout).ok_or_else(|| {
        DevdError::invalid(format!(
            "could not read the PHP version from `{}`; it printed: {}",
            exe.display(),
            util::truncate(version_out.stdout.trim(), 200)
        ))
    })?;

    let mut warnings = startup_problems(&version_out.stdout);
    warnings.extend(startup_problems(&version_out.stderr));
    warnings.dedup();

    let module_out = util::run(
        &exe,
        &["-m".to_string()],
        Some(install),
        &[],
        Some(PROBE_TIMEOUT),
    )?;
    for line in startup_problems(&module_out.stdout)
        .into_iter()
        .chain(startup_problems(&module_out.stderr))
    {
        if !warnings.contains(&line) {
            warnings.push(line);
        }
    }

    Ok(PhpInfo {
        version,
        modules: parse_module_list(&module_out.stdout),
        warnings,
    })
}

/// The first token that looks like a three-part version.
fn version_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.'))
        .find(|token| {
            let parts: Vec<&str> = token.split('.').collect();
            parts.len() == 3
                && parts.iter().all(|p| {
                    !p.is_empty()
                        && p.chars().next().is_some_and(|c| c.is_ascii_digit())
                        && p.chars().all(|c| c.is_ascii_alphanumeric())
                })
        })
        .map(str::to_string)
}

/// The module names in `php -m` output.
fn parse_module_list(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .map(str::to_string)
        .collect()
}

/// Lines that report a startup problem, kept verbatim for the install report.
fn startup_problems(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("Warning:")
                || l.contains("PHP Startup")
                || l.contains("Unable to load dynamic library")
                || l.starts_with("PHP Warning:")
        })
        .map(str::to_string)
        .collect()
}

/// Extension names an interpreter reported it could not load.
pub fn failed_modules(info: &PhpInfo) -> Vec<String> {
    let mut out = Vec::new();
    for warning in &info.warnings {
        if !warning.contains("Unable to load dynamic library") {
            continue;
        }
        let Some(start) = warning.find('\'') else {
            continue;
        };
        let rest = &warning[start + 1..];
        let Some(end) = rest.find('\'') else { continue };
        let quoted = &rest[..end];
        let name = quoted
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(quoted)
            .trim()
            .to_lowercase();
        let name = name.strip_prefix("php_").unwrap_or(&name);
        let name = name.strip_suffix(".dll").unwrap_or(name).to_string();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Comment out the `extension=` lines naming these modules.
///
/// A generated file that we already know contains a broken directive is a
/// liability, so the correction is written back rather than only reported.
pub fn disable_extensions(ini: &Path, names: &[String]) -> Result<usize> {
    let text = std::fs::read_to_string(ini)?;
    let mut changed = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        let matches = names.iter().any(|name| {
            trimmed == format!("extension={name}") || trimmed == format!("zend_extension={name}")
        });
        if matches {
            out.push_str(&format!(";{line}  ; disabled by devd: it would not load\n"));
            changed += 1;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if changed > 0 {
        util::atomic_write(ini, &out)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devd-php-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The template as the real distributions write it, including the two
    /// traps: a duplicate entry and an entry with a trailing explanation.
    const TEMPLATE: &str = "; comment\n\
        ;extension_dir = \"ext\"\n\
        ;extension=mysqli\n\
        ;extension=curl\n\
        ;extension=gd\n\
        ;extension=exif      ; Must be after mbstring as it depends on it\n\
        ;extension=pdo_firebird\n\
        ;extension=php_pdo_mysql.dll\n\
        ;extension=mysqli\n\
        ;zend_extension=opcache\n\
        ;extension=snmp\n\
        memory_limit = 128M\n\
        \n\
        [opcache]\n\
        ;opcache.enable=1\n";

    fn render(available: &[&str], builtin: &[&str]) -> (String, Vec<String>) {
        render_ini(TEMPLATE, &set(available), &set(builtin))
    }

    /// A miniature PHP build: the template plus the extension DLLs.
    fn fake_build(dir: &Path, dlls: &[&str]) {
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        for dll in dlls {
            std::fs::write(dir.join(format!("ext/php_{dll}.dll")), b"dll").unwrap();
        }
        std::fs::write(dir.join("php.ini-development"), TEMPLATE).unwrap();
    }

    #[test]
    fn enables_only_curated_extensions_the_build_ships() {
        let (ini, enabled) = render(
            &[
                "mysqli",
                "curl",
                "gd",
                "exif",
                "pdo_firebird",
                "pdo_mysql",
                "opcache",
            ],
            &[],
        );
        assert_eq!(
            enabled,
            vec![
                "mysqli".to_string(),
                "curl".to_string(),
                "gd".to_string(),
                "exif".to_string(),
                "pdo_mysql".to_string(),
                "opcache".to_string(),
            ]
        );
        assert!(ini.contains("extension_dir = \"ext\""));
        // Shipped, but needs a Firebird client that is not installed.
        assert!(ini.contains(";extension=pdo_firebird"), "{ini}");
        // Not shipped at all.
        assert!(ini.contains(";extension=snmp"), "{ini}");
        // opcache is not an ordinary extension.
        assert!(ini.contains("\nzend_extension=opcache\n"), "{ini}");
        assert!(!ini.contains("\nextension=opcache"));
        // The template's own settings survive.
        assert!(ini.contains("memory_limit = 128M"));
        assert!(ini.contains("[opcache]"));
    }

    #[test]
    fn an_extension_listed_twice_is_enabled_once() {
        let (ini, enabled) = render(&["mysqli"], &[]);
        assert_eq!(enabled, vec!["mysqli".to_string()]);
        assert_eq!(ini.matches("\nextension=mysqli\n").count(), 1, "{ini}");
    }

    #[test]
    fn extensions_compiled_into_the_build_are_not_loaded_again() {
        // "Module mysqli is already loaded" is the failure this prevents.
        let (ini, enabled) = render(&["mysqli", "curl"], &["mysqli"]);
        assert_eq!(enabled, vec!["curl".to_string()]);
        assert!(!ini.contains("\nextension=mysqli\n"), "{ini}");
        assert!(ini.contains(";extension=mysqli"), "{ini}");
    }

    #[test]
    fn a_trailing_explanation_does_not_hide_an_extension() {
        let (_, enabled) = render(&["exif"], &[]);
        assert_eq!(enabled, vec!["exif".to_string()]);
    }

    #[test]
    fn appends_development_defaults_that_win() {
        let dir = scratch("defaults");
        fake_build(&dir, &["curl"]);
        configure_ini(&dir, &dir, "8.3.35").unwrap();
        let ini = std::fs::read_to_string(dir.join("php.ini")).unwrap();
        let bound = ini.find("memory_limit = 512M").expect("a bound is set");
        assert!(
            bound > ini.find("memory_limit = 128M").unwrap(),
            "the appended value must win"
        );
        assert!(ini.contains("cgi.fix_pathinfo = 1"));
        assert!(ini.contains("php-8.3.35.log"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_without_a_template_is_rejected() {
        let dir = scratch("notphp");
        std::fs::write(dir.join("readme.txt"), b"hello").unwrap();
        let err = configure_ini(&dir, &dir, "8.3.0").unwrap_err();
        assert!(err.to_string().contains("not a PHP distribution"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn commented_extension_forms_are_understood() {
        assert_eq!(
            commented_extension(";extension=curl"),
            Some((false, "curl".into()))
        );
        assert_eq!(
            commented_extension(";extension=php_pdo_mysql.dll"),
            Some((false, "pdo_mysql".into()))
        );
        assert_eq!(
            commented_extension(";zend_extension=opcache"),
            Some((true, "opcache".into()))
        );
        assert_eq!(
            commented_extension(";   extension=exif      ; Must be after mbstring"),
            Some((false, "exif".into()))
        );
        assert_eq!(
            commented_extension("extension=curl"),
            None,
            "already active"
        );
        assert_eq!(commented_extension("; extension = gd"), None);
        assert_eq!(commented_extension(";extension="), None);
        assert_eq!(commented_extension(";extension=\"dir/x\""), None);
    }

    #[test]
    fn a_version_is_found_among_startup_warnings() {
        let noisy = "Warning: PHP Startup: Unable to load dynamic library 'pdo_firebird' \
                     (tried: ext\\pdo_firebird ...)\n\nPHP 8.3.35 (cli) (built: Sep 22 2026)(NTS)\n";
        assert_eq!(version_token(noisy).as_deref(), Some("8.3.35"));
        assert_eq!(version_token("no version here"), None);
    }

    #[test]
    fn failed_extensions_are_named_from_the_warning() {
        let info = PhpInfo {
            version: "8.3.35".to_string(),
            modules: vec![],
            warnings: vec![
                "Warning: PHP Startup: Unable to load dynamic library 'pdo_firebird' (tried: ext\\pdo_firebird (The specified module could not be found), ext\\php_pdo_firebird.dll (The specified module could not be found))".to_string(),
                "Warning: PHP Startup: Unable to load dynamic library 'C:\\php\\ext\\php_gd.dll' (tried: ...)".to_string(),
                "some unrelated warning".to_string(),
            ],
        };
        assert_eq!(
            failed_modules(&info),
            vec!["pdo_firebird".to_string(), "gd".to_string()]
        );
    }

    #[test]
    fn disabling_rewrites_the_file_and_leaves_others_alone() {
        let dir = scratch("disable");
        let ini = dir.join("php.ini");
        std::fs::write(
            &ini,
            "extension_dir = \"ext\"\nextension=curl\nextension=pdo_firebird\nextension=gd\n",
        )
        .unwrap();
        let changed = disable_extensions(&ini, &["pdo_firebird".to_string()]).unwrap();
        assert_eq!(changed, 1);
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("\nextension=curl\n"));
        assert!(text.contains("\nextension=gd\n"));
        assert!(text.contains(";extension=pdo_firebird"), "{text}");
        assert!(text.contains("disabled by devd"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabling_an_absent_extension_changes_nothing() {
        let dir = scratch("disable-none");
        let ini = dir.join("php.ini");
        std::fs::write(&ini, "extension=curl\n").unwrap();
        assert_eq!(disable_extensions(&ini, &["gd".to_string()]).unwrap(), 0);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "extension=curl\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_reports_a_clear_error_when_the_binary_is_absent() {
        let dir = scratch("absent");
        let err = verify(&dir).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not contain a PHP interpreter"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
