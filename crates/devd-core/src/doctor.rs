//! Environment self-check.
//!
//! `devd doctor` on the command line and the **System** panel in the
//! application show the same list, from the same code, because a check that
//! exists on only one surface is a check that will disagree with the other.
//!
//! Only checks that `devd-core` can perform on its own live here. Anything that
//! needs a component the core does not depend on - most obviously the Git
//! backend, which lives in `devd-git` - is appended by the caller through
//! [`Check::ok`]/[`Check::fail`], so no surface has to invent its own list.

use serde::Serialize;

use crate::config::Settings;
use crate::paths::Paths;
use crate::state::Store;

/// How serious a check result is.
///
/// Three levels rather than a boolean because "port 80 is already in use" is
/// neither success nor failure: it is true, it matters, and `devd` is designed
/// to say so without pretending the installation is broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Checked and correct.
    Ok,
    /// Worth knowing, not a problem.
    Info,
    /// Broken; `devd` will not work as intended until this is fixed.
    Fail,
}

/// One result from the environment check.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// Short stable identifier, e.g. `state` or `port.http`.
    pub name: String,
    /// How serious the result is.
    pub level: Level,
    /// Human-readable explanation, including the values observed.
    pub detail: String,
}

impl Check {
    /// A check that passed.
    pub fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level: Level::Ok,
            detail: detail.into(),
        }
    }

    /// A check that passed but is worth reporting.
    pub fn info(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level: Level::Info,
            detail: detail.into(),
        }
    }

    /// A check that failed.
    pub fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            level: Level::Fail,
            detail: detail.into(),
        }
    }

    /// Whether this result is a failure.
    pub fn is_fail(&self) -> bool {
        self.level == Level::Fail
    }
}

/// Every check the core can perform without knowing about optional components.
///
/// The checks run in dependency order: the layout must exist before the state
/// database can be opened, and the state database must open before anything can
/// read the tables inside it. A failing early check does not stop later ones, so
/// one report shows every problem instead of only the first.
pub fn core_checks(paths: &Paths, settings: &Settings) -> Vec<Check> {
    core_checks_for(paths, settings, None)
}

/// [`core_checks`], for a process that already has its state database open. The state check then
/// asks that database, the one the process is really using, instead of opening the file at the path
/// a second time. `None` looks at the file, which is what the command line does.
pub fn core_checks_for(paths: &Paths, settings: &Settings, store: Option<&Store>) -> Vec<Check> {
    let mut checks = Vec::new();

    // A check is a question, not an installation: it must not create the home
    // folder. `DEVD_HOME` may point at a folder nobody has made yet (a stale
    // variable, a typo, a path on a drive that is not there), and making it
    // here would leave a second, empty devd home behind.
    let home_exists = paths.root().exists();
    checks.push(Check::ok("home", paths.root().display().to_string()));
    checks.extend(home_conflict(paths.root(), user_home_setting().as_deref(), &default_home(), &|d| d.join("state.db").is_file()));

    if home_exists {
        // Look, do not repair: a check that creates what is missing hides that it was missing.
        let missing = paths.missing();
        if missing.is_empty() {
            checks.push(Check::ok("layout", "all directories exist"));
        } else {
            let names: Vec<String> = missing.iter().filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
            checks.push(Check::info("layout", format!("missing: {}. devd creates them when it starts; doctor creates nothing", names.join(", "))));
        }
    } else {
        checks.push(Check::info(
            "layout",
            "this folder does not exist yet; devd creates it the first time something needs it",
        ));
    }

    match settings.validate() {
        Ok(()) => checks.push(Check::ok(
            "settings",
            format!(
                "tld=.{}, http={}, https={}, web_server={:?}",
                settings.tld, settings.http_port, settings.https_port, settings.web_server
            ),
        )),
        Err(e) => checks.push(Check::fail("settings", e.to_string())),
    }

    checks.push(if let Some(store) = store {
        state_of(store, &paths.state_db())
    } else if home_exists {
        state_check(paths)
    } else {
        Check::info("state", "no state database yet")
    });
    checks.extend(port_checks(settings));
    checks.push(www_root_check(paths, settings));

    checks
}

/// The home devd would use with no `DEVD_HOME` at all.
fn default_home() -> Option<std::path::PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("devd"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|p| std::path::PathBuf::from(p).join("devd"))
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share/devd")))
    }
}

/// The `DEVD_HOME` the person has set for their account, as a new terminal or the window would see
/// it. On Windows that is the user's environment in the registry, which can differ from the value
/// in a process that was started earlier and kept an old copy.
fn user_home_setting() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let out = crate::util::run(
        std::path::Path::new("reg.exe"),
        &["query".into(), "HKCU\\Environment".into(), "/v".into(), "DEVD_HOME".into()],
        None,
        &[],
        Some(std::time::Duration::from_secs(10)),
    )
    .ok()?;
    if out.code != 0 {
        return None;
    }
    parse_reg_value(&out.stdout)
}

/// `    DEVD_HOME    REG_SZ    D:\devd` as `D:\devd`.
pub fn parse_reg_value(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with("DEVD_HOME"))?;
    let after_name = line["DEVD_HOME".len()..].trim_start();
    let value = after_name.strip_prefix("REG_EXPAND_SZ").or_else(|| after_name.strip_prefix("REG_SZ"))?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Say so when more than one devd home could be meant, and why that matters.
///
/// A process, a new terminal and the window can each resolve `DEVD_HOME` differently (a variable
/// that outlived the folder it named, one set after a program started). Each then opens its own
/// state database, or makes an empty one, and what a command changes is not what the window shows.
/// This only reports: it never picks one, moves anything or deletes anything.
///
/// * `in_use` is the home this process resolved.
/// * `user_setting` is the account's `DEVD_HOME`, when it has one.
/// * `default` is the home devd would use with no setting.
/// * `has_state` says whether a folder holds a state database.
pub fn home_conflict(
    in_use: &std::path::Path,
    user_setting: Option<&str>,
    default: &Option<std::path::PathBuf>,
    has_state: &dyn Fn(&std::path::Path) -> bool,
) -> Vec<Check> {
    let mut out = Vec::new();
    if let Some(setting) = user_setting {
        let setting = std::path::Path::new(setting);
        if !crate::util::same_folder(in_use, setting) {
            let both = has_state(in_use) && has_state(setting);
            let detail = format!(
                "this process uses {} but your account's DEVD_HOME is {}{}. A new terminal and the window use the account's, so commands run here act on a different devd than they do. Start a new terminal, or restart the program that was started before the variable changed.",
                in_use.display(),
                setting.display(),
                if both { ", and both folders hold devd data" } else { "" }
            );
            // Never a failure: `DEVD_HOME` is a documented override for portable installs and tests, so a
            // second home with data can be deliberate, and each process works correctly with its own.
            // It is said plainly, because the harm is not knowing.
            out.push(Check::info("home.conflict", detail));
        }
    }
    if let Some(default) = default {
        let reported = out.iter().any(|c| c.name == "home.conflict");
        if !reported && !crate::util::same_folder(in_use, default) && has_state(default) {
            out.push(Check::info(
                "home.other",
                format!("another devd home with data exists at {} and is not in use (this process uses {}); nothing was moved or removed", default.display(), in_use.display()),
            ));
        }
    }
    out
}

/// Whether the state database can be asked about its tables as this devd knows them: it exists and
/// is at this build's schema. A database from before the newest migration is not broken, it is
/// waiting for devd to start; one from a newer devd may have shapes this one does not know.
pub fn state_is_current(paths: &Paths) -> bool {
    Store::open_read_only(&paths.state_db()).and_then(|s| s.schema_version()).is_ok_and(|v| v == crate::state::LATEST_SCHEMA)
}

fn state_check(paths: &Paths) -> Check {
    // Doctor only looks. Opening the database the usual way would create it, switch its journal
    // mode and migrate it, so the check would change the thing it is checking.
    match Store::open_read_only(&paths.state_db()) {
        Ok(store) => state_of(&store, &paths.state_db()),
        Err(e) => Check::fail("state", format!("cannot open {}: {e}", paths.state_db().display())),
    }
}

/// What an open state database says about itself.
fn state_of(store: &Store, file: &std::path::Path) -> Check {
    let (integrity, version) = match (store.integrity_check(), store.schema_version()) {
        (Ok(integrity), Ok(version)) => (integrity, version),
        (Ok(_), Err(e)) => return Check::fail("state", format!("schema version unreadable: {e}")),
        (Err(e), _) => return Check::fail("state", format!("integrity check failed: {e}")),
    };
    if integrity != "ok" {
        return Check::fail(
            "state",
            format!("the database is damaged; close devd and run `devd repair-state` ({})", integrity.lines().next().unwrap_or("")),
        );
    }
    let latest = crate::state::LATEST_SCHEMA;
    if version < latest {
        Check::info(
            "state",
            format!(
                "{} is at schema v{version}; this devd will bring it to v{latest} the first time it starts. Doctor changed nothing, and does not read the tables a migration adds until then",
                file.display()
            ),
        )
    } else if version > latest {
        Check::info(
            "state",
            format!(
                "{} was written by a newer devd (schema v{version}; this one knows v{latest}). It is read as far as this one understands and is not changed",
                file.display()
            ),
        )
    } else {
        Check::ok("state", format!("{} at schema v{version}, integrity ok", file.display()))
    }
}

fn port_checks(settings: &Settings) -> Vec<Check> {
    let mut checks = Vec::new();
    // One listener table for both ports: enumerating it twice doubles the cost
    // of every environment check for no extra information.
    let listeners = match crate::ports::listeners() {
        Ok(list) => Some(list),
        Err(e) => {
            checks.push(Check::fail(
                "port.http",
                format!("cannot inspect listening ports: {e}"),
            ));
            None
        }
    };

    for (label, port) in [("http", settings.http_port), ("https", settings.https_port)] {
        let name = format!("port.{label}");
        let Some(list) = &listeners else { continue };
        match list.iter().find(|l| l.port == port) {
            None => checks.push(Check::ok(name, format!("{port} is free"))),
            Some(l) => {
                let owner = match (&l.program, l.pid) {
                    (Some(p), Some(pid)) => format!("{p} (pid {pid})"),
                    (Some(p), None) => p.clone(),
                    (None, Some(pid)) => format!("pid {pid}"),
                    (None, None) => "another process".to_string(),
                };
                checks.push(Check::info(
                    name,
                    format!("{port} is held by {owner} on {}", l.address),
                ));
            }
        }
    }
    checks
}

fn www_root_check(paths: &Paths, settings: &Settings) -> Check {
    let root = settings.effective_www_root(&paths.www());
    if !root.exists() {
        return Check::info("www_root", format!("{} does not exist yet", root.display()));
    }
    let probe = root.join(".devd-write-probe");
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Check::ok("www_root", format!("{} is writable", root.display()))
        }
        Err(e) => Check::fail(
            "www_root",
            format!("{} is not writable: {e}", root.display()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("devd-doctor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths::rooted(dir)
    }

    #[test]
    fn reports_home_layout_settings_and_state() {
        let paths = scratch("core");
        let checks = core_checks(&paths, &Settings::default());
        let names: Vec<String> = checks.iter().map(|c| c.name.clone()).collect();
        for expected in [
            "home",
            "layout",
            "settings",
            "state",
            "www_root",
            "port.http",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing `{expected}` in {names:?}"
            );
        }
        assert!(!checks.iter().any(|c| c.is_fail()), "{checks:#?}");
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[test]
    fn broken_settings_are_reported_not_panicked() {
        let paths = scratch("bad-settings");
        let mut settings = Settings::default();
        settings.tld = "local".to_string();
        let checks = core_checks(&paths, &settings);
        let settings_check = checks.iter().find(|c| c.name == "settings").unwrap();
        assert!(settings_check.is_fail());
        // A later check still runs, so one report lists every problem.
        assert!(checks.iter().any(|c| c.name == "state"));
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[test]
    fn a_held_port_is_informational() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = scratch("port");
        let mut settings = Settings::default();
        settings.http_port = port;
        let checks = core_checks(&paths, &settings);
        let http = checks.iter().find(|c| c.name == "port.http").unwrap();
        assert_eq!(http.level, Level::Info, "{http:?}");
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[test]
    fn a_check_does_not_create_a_home_that_is_not_there() {
        let paths = scratch("absent");
        assert!(!paths.root().exists());
        let checks = core_checks(&paths, &Settings::default());
        assert!(
            !paths.root().exists(),
            "doctor created {}",
            paths.root().display()
        );
        assert!(!checks.iter().any(|c| c.is_fail()), "{checks:#?}");
        assert_eq!(checks.iter().find(|c| c.name == "layout").unwrap().level, Level::Info);
        assert_eq!(checks.iter().find(|c| c.name == "state").unwrap().level, Level::Info);
    }

    #[test]
    fn a_registry_line_is_read_back_as_the_value() {
        let text = "\r\nHKEY_CURRENT_USER\\Environment\r\n    DEVD_HOME    REG_SZ    D:\\devd\r\n\r\n";
        assert_eq!(parse_reg_value(text).as_deref(), Some("D:\\devd"));
        assert_eq!(parse_reg_value("    OTHER    REG_SZ    x"), None);
    }

    #[test]
    fn a_process_that_disagrees_with_the_account_is_told_so_and_nothing_else_happens() {
        let none = |_: &std::path::Path| false;
        let used = std::path::Path::new("D:\\devd-data");
        let found = home_conflict(used, Some("D:\\devd"), &None, &none);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].level, Level::Info, "one home with data is not a failure: {found:?}");
        assert!(found[0].detail.contains("D:\\devd-data") && found[0].detail.contains("D:\\devd"), "{:?}", found[0]);

        // The same home spelled differently is the same home.
        assert!(home_conflict(std::path::Path::new("d:/devd/"), Some("D:\\devd"), &None, &none).is_empty());
    }

    #[test]
    fn two_homes_that_both_hold_data_are_said_plainly_but_are_not_a_failure() {
        let both = |_: &std::path::Path| true;
        let found = home_conflict(std::path::Path::new("D:\\devd-data"), Some("D:\\devd"), &None, &both);
        assert_eq!(found[0].level, Level::Info, "a deliberate portable home must not turn doctor red: {found:?}");
        assert!(found[0].detail.contains("both folders hold devd data"));
    }

    #[test]
    fn another_home_with_data_is_reported_but_never_touched() {
        let default = Some(std::path::PathBuf::from("C:\\Users\\x\\AppData\\Local\\devd"));
        let only_default = |p: &std::path::Path| p.to_string_lossy().contains("Local");
        let found = home_conflict(std::path::Path::new("D:\\devd"), None, &default, &only_default);
        assert_eq!(found.len(), 1);
        assert!(found[0].detail.contains("nothing was moved or removed"));
        assert!(home_conflict(std::path::Path::new("D:\\devd"), None, &default, &|_| false).is_empty());
    }

    #[test]
    fn levels_serialise_lowercase() {
        let json = serde_json::to_string(&Level::Fail).unwrap();
        assert_eq!(json, "\"fail\"");
    }
}
