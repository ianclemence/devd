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
    let mut checks = Vec::new();

    // A check is a question, not an installation: it must not create the home
    // folder. `DEVD_HOME` may point at a folder nobody has made yet (a stale
    // variable, a typo, a path on a drive that is not there), and making it
    // here would leave a second, empty devd home behind.
    let home_exists = paths.root().exists();
    checks.push(Check::ok("home", paths.root().display().to_string()));

    if home_exists {
        match paths.ensure() {
            Ok(()) => checks.push(Check::ok("layout", "all directories exist")),
            Err(e) => checks.push(Check::fail("layout", e.to_string())),
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

    checks.push(if home_exists {
        state_check(paths)
    } else {
        Check::info("state", "no state database yet")
    });
    checks.extend(port_checks(settings));
    checks.push(www_root_check(paths, settings));

    checks
}

fn state_check(paths: &Paths) -> Check {
    match Store::open(&paths.state_db()) {
        Ok(store) => match (store.integrity_check(), store.schema_version()) {
            (Ok(integrity), Ok(version)) => {
                if integrity == "ok" {
                    Check::ok(
                        "state",
                        format!(
                            "{} at schema v{version}, integrity ok",
                            paths.state_db().display()
                        ),
                    )
                } else {
                    Check::fail("state", format!("the database is damaged; close devd and run `devd repair-state` ({})", integrity.lines().next().unwrap_or("")))
                }
            }
            (Ok(_), Err(e)) => Check::fail("state", format!("schema version unreadable: {e}")),
            (Err(e), _) => Check::fail("state", format!("integrity check failed: {e}")),
        },
        Err(e) => Check::fail(
            "state",
            format!("cannot open {}: {e}", paths.state_db().display()),
        ),
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
    fn levels_serialise_lowercase() {
        let json = serde_json::to_string(&Level::Fail).unwrap();
        assert_eq!(json, "\"fail\"");
    }
}
