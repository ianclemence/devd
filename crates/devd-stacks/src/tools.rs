//! The tool catalogue: compilers, runtimes, package managers, databases,
//! mobile tooling and utilities - how to detect them, which versions are
//! acceptable, how to install and remove them, and how to run those that are
//! services.
//!
//! Detection never trusts a package-manager exit code: a tool is "installed"
//! when its executable runs and reports a version, and a service is "running"
//! when its port accepts connections and its health probe passes.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use devd_core::{util, DevdError, Result};

use crate::model::DocLink;

/// One way of installing or removing a tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Recipe {
    /// `devd-runtime`, `apt`, `dnf`, `pacman`, `brew`, `brew-cask`, `winget`, `npm`, `gem`, `manual`.
    pub manager: String,
    /// Package names (or the single winget id).
    #[serde(default)]
    pub packages: Vec<String>,
    /// Extra explanation, or the manual instructions for `manual`.
    #[serde(default)]
    pub note: String,
    /// Link for `manual` recipes.
    #[serde(default)]
    pub url: Option<String>,
}

/// How a tool that runs as a service is controlled and checked.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceDef {
    /// Candidate service unit names, first that exists wins.
    pub units: Vec<String>,
    /// Default TCP port.
    pub port: u16,
    /// Health probe, run after the port accepts connections.
    #[serde(default)]
    pub health: Vec<String>,
    /// Start command for hosts without a service manager (containers).
    #[serde(default)]
    pub direct_start: Vec<String>,
    /// Stop command matching `direct_start`.
    #[serde(default)]
    pub direct_stop: Vec<String>,
    /// Where data lives; never removed by devd.
    #[serde(default)]
    pub data_dir: String,
    /// Where to look for logs.
    #[serde(default)]
    pub logs: String,
    /// Security note about defaults.
    #[serde(default)]
    pub security: String,
}

/// A tool in the catalogue.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolDef {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `runtime`, `package-manager`, `database`, `vcs`, `mobile`, `container`, `build`, `utility`.
    pub category: String,
    /// What it is for.
    pub purpose: String,
    /// Executable names, first found wins.
    #[serde(default)]
    pub exes: Vec<String>,
    /// Arguments that print the version.
    #[serde(default)]
    pub version_args: Vec<String>,
    /// Regex whose first group is the version.
    #[serde(default)]
    pub version_regex: Option<String>,
    /// Text the output must contain for this program to be this tool.
    ///
    /// Several tools share an executable name. MariaDB's client prints
    /// "MariaDB"; MySQL's does not; Windows' `convert.exe` is a disk
    /// converter and says nothing about images. Without this, the wrong
    /// program answers and devd reports a tool that is not there.
    #[serde(default)]
    pub version_require: Option<String>,
    /// Text the output must not contain.
    ///
    /// MySQL's client is also MariaDB's client, and says "MariaDB" when it is
    /// MariaDB. MySQL must not match that.
    #[serde(default)]
    pub version_reject: Option<String>,
    /// Oldest version devd considers supported.
    #[serde(default)]
    pub supported_min: Option<String>,
    /// Official documentation.
    #[serde(default)]
    pub docs: Vec<DocLink>,
    /// Tools that must be present first.
    #[serde(default)]
    pub depends: Vec<String>,
    /// Install recipes, in order of preference.
    #[serde(default)]
    pub install: Vec<Recipe>,
    /// Removal recipes.
    #[serde(default)]
    pub remove: Vec<Recipe>,
    /// Where the software comes from and why that is trusted.
    #[serde(default)]
    pub trust: String,
    /// Upgrade and rollback considerations.
    #[serde(default)]
    pub upgrade: String,
    /// Service control, when the tool runs as one.
    #[serde(default)]
    pub service: Option<ServiceDef>,
    /// Installed with the project, not the machine (e.g. `npx expo`).
    #[serde(default)]
    pub per_project: bool,
    /// Environment variables the tool relies on.
    #[serde(default)]
    pub env: Vec<String>,
}

/// A package manager for a language ecosystem.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PackageManager {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `node`, `php`, `python`, `ruby`, `go`, `rust`, `java`, `dotnet`.
    pub ecosystem: String,
    /// Catalogue tool that provides it.
    pub tool: String,
    /// Run a script/task: `<run> NAME`.
    #[serde(default)]
    pub run: Vec<String>,
    /// Install all dependencies.
    #[serde(default)]
    pub install: Vec<String>,
    /// Add a dependency: `<add> PKG`.
    #[serde(default)]
    pub add: Vec<String>,
    /// Run a locally installed binary.
    #[serde(default)]
    pub exec: Vec<String>,
    /// Run a package without installing it (scaffolders).
    #[serde(default)]
    pub dlx: Vec<String>,
    /// Lockfiles that identify it.
    #[serde(default)]
    pub lockfiles: Vec<String>,
    /// Other files that identify it.
    #[serde(default)]
    pub markers: Vec<String>,
    /// Link.
    #[serde(default)]
    pub docs: Option<String>,
}

/// The whole catalogue file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Catalogue {
    /// Tools.
    pub tools: Vec<ToolDef>,
    /// Package managers.
    pub package_managers: Vec<PackageManager>,
}

/// Parse a dotted version into numeric components; non-numeric tails are ignored.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim_start_matches(['v', 'V', '='])
        .split(['.', '-', '+', '_'])
        .map_while(|p| {
            let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse().ok()
            }
        })
        .collect()
}

/// Compare two versions numerically, padding with zeros.
pub fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (parse_version(a), parse_version(b));
    for i in 0..x.len().max(y.len()) {
        let (p, q) = (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0));
        if p != q {
            return p.cmp(&q);
        }
    }
    std::cmp::Ordering::Equal
}

/// Whether `version` satisfies an inclusive minimum and a prefix-inclusive maximum.
pub fn satisfies(version: &str, min: Option<&str>, max: Option<&str>) -> bool {
    if let Some(m) = min {
        if cmp_version(version, m) == std::cmp::Ordering::Less {
            return false;
        }
    }
    if let Some(m) = max {
        let mp = parse_version(m);
        let vp = parse_version(version);
        for (i, want) in mp.iter().enumerate() {
            let have = vp.get(i).copied().unwrap_or(0);
            if have != *want {
                return have < *want;
            }
        }
    }
    true
}

/// What a probe found.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolStatus {
    /// Tool id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Category.
    pub category: String,
    /// Whether an executable was found and ran.
    pub installed: bool,
    /// Path to the executable.
    pub path: Option<String>,
    /// Reported version.
    pub version: Option<String>,
    /// `path` or `devd-runtime`.
    pub source: Option<String>,
    /// Why it is not usable, when it is not.
    pub problem: Option<String>,
    /// Below the oldest supported version.
    pub outdated: bool,
}

/// Search directories first, then `PATH`.
///
/// The Windows extensions come before the bare name, for the same reason as
/// in `util::which`: an installer that leaves a shell script beside its
/// Windows shim would otherwise be found and could not be started.
pub fn which_in(program: &str, dirs: &[PathBuf]) -> Option<(PathBuf, bool)> {
    for d in dirs {
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                let p = d.join(format!("{program}.{ext}"));
                if p.is_file() {
                    return Some((p, true));
                }
            }
        }
        let p = d.join(program);
        if p.is_file() {
            return Some((p, true));
        }
    }
    util::which(program).map(|p| (p, false))
}

/// Extract a version from command output.
pub fn extract_version(text: &str, regex: Option<&str>) -> Option<String> {
    util::extract_version(text, regex)
}

impl ToolDef {
    /// Detect the tool.
    pub fn probe(&self, extra_dirs: &[PathBuf]) -> ToolStatus {
        let mut st = ToolStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            category: self.category.clone(),
            installed: false,
            path: None,
            version: None,
            source: None,
            problem: None,
            outdated: false,
        };
        if self.per_project {
            st.problem = Some("installed per project, not on the machine".into());
            return st;
        }
        let mut rejected: Vec<String> = Vec::new();
        for exe in &self.exes {
            let Some((path, managed)) = which_in(exe, extra_dirs) else {
                continue;
            };
            let args = if self.version_args.is_empty() {
                vec!["--version".to_string()]
            } else {
                self.version_args.clone()
            };
            match util::run(&path, &args, None, &[], Some(Duration::from_secs(15))) {
                Ok(out) => {
                    let text = format!("{}\n{}", out.stdout, out.stderr);
                    // The program that answered has to be this tool and not
                    // another one that happens to share its name.
                    if let Some(need) = &self.version_require {
                        if !text.contains(need) {
                            rejected.push(format!("`{exe}` is a different program"));
                            continue;
                        }
                    }
                    if let Some(no) = &self.version_reject {
                        if text.contains(no) {
                            rejected.push(format!("`{exe}` is {no}"));
                            continue;
                        }
                    }
                    let version = extract_version(&text, self.version_regex.as_deref());
                    // Some tools print their version to a non-zero exit; a
                    // version we could parse is proof enough that it runs.
                    let installed = out.ok() || version.is_some();
                    if installed {
                        st.path = Some(path.display().to_string());
                        st.source = Some(if managed { "devd-runtime" } else { "path" }.to_string());
                        st.version = version;
                        st.installed = true;
                        if let (Some(v), Some(min)) = (&st.version, &self.supported_min) {
                            st.outdated = cmp_version(v, min) == std::cmp::Ordering::Less;
                        }
                        st.problem = None;
                        return st;
                    }
                    // This candidate is the wrong program, or a broken one.
                    // Try the next name before saying anything about it.
                    st.problem = Some(format!(
                        "`{exe}` was found but failed to run: {}",
                        util::truncate(out.stderr.trim(), 200)
                    ));
                }
                Err(e) => {
                    st.problem = Some(format!("`{exe}` was found but did not run: {e}"));
                }
            }
        }
        if st.problem.is_none() {
            st.problem = Some(if rejected.is_empty() {
                format!("`{}` was not found on PATH", self.exes.join("` or `"))
            } else {
                format!(
                    "`{}` was not found on PATH; {}",
                    self.exes.join("` or `"),
                    rejected.join("; ")
                )
            });
        }
        st
    }
}

/// The machine devd is running on.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Host {
    /// `linux`, `macos` or `windows`.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Package managers found.
    pub managers: Vec<String>,
    /// Running as root.
    pub root: bool,
    /// `sudo` can run without a password prompt.
    pub sudo: bool,
    /// systemd manages services.
    pub systemd: bool,
}

impl Host {
    /// Inspect the current machine.
    pub fn detect() -> Self {
        let managers: Vec<String> = [
            ("apt-get", "apt"),
            ("dnf", "dnf"),
            ("pacman", "pacman"),
            ("brew", "brew"),
            ("winget", "winget"),
            ("npm", "npm"),
            ("gem", "gem"),
        ]
        .iter()
        .filter(|(exe, _)| util::which(exe).is_some())
        .map(|(_, id)| id.to_string())
        .collect();
        let root = is_root();
        let sudo = !root
            && util::which("sudo")
                .map(|s| {
                    util::run(&s, &["-n".into(), "true".into()], None, &[], Some(Duration::from_secs(5)))
                        .map(|o| o.ok())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
        Self {
            os: std::env::consts::OS.replace("macos", "macos"),
            arch: std::env::consts::ARCH.to_string(),
            managers,
            root,
            sudo,
            systemd: Path::new("/run/systemd/system").exists(),
        }
    }

    /// Whether a package manager is usable here.
    pub fn has(&self, manager: &str) -> bool {
        match manager {
            "brew-cask" => self.managers.iter().any(|m| m == "brew"),
            "devd-runtime" | "manual" => true,
            m => self.managers.iter().any(|x| x == m),
        }
    }
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(2).map(|u| u == "0"))
            })
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// A reviewable installation or removal plan.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolPlan {
    /// Tool id.
    pub tool: String,
    /// `install` or `remove`.
    pub action: String,
    /// The recipe chosen.
    pub manager: Option<String>,
    /// Commands in order, each an argv.
    pub commands: Vec<Vec<String>>,
    /// Display form of the commands.
    pub display: Vec<String>,
    /// Needs administrator rights.
    pub privileged: bool,
    /// Tools that must be installed first and are not.
    pub missing_dependencies: Vec<String>,
    /// Already installed at this version.
    pub already_installed: Option<String>,
    /// Reasons it cannot be done automatically here.
    pub blockers: Vec<String>,
    /// Manual instructions when automation is not possible.
    pub manual: Option<String>,
    /// Where the software comes from.
    pub trust: String,
    /// Upgrade and rollback notes.
    pub upgrade: String,
    /// Data that stays behind on removal.
    pub data_note: Option<String>,
}

fn recipe_commands(manager: &str, action: &str, r: &Recipe) -> Vec<Vec<String>> {
    let pk = r.packages.clone();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let with = |mut base: Vec<String>, extra: &[String]| {
        base.extend(extra.iter().cloned());
        base
    };
    match (manager, action) {
        ("apt", "install") => vec![s(&["apt-get", "update"]), with(s(&["apt-get", "install", "-y"]), &pk)],
        ("apt", "remove") => vec![with(s(&["apt-get", "remove", "-y"]), &pk)],
        ("dnf", "install") => vec![with(s(&["dnf", "install", "-y"]), &pk)],
        ("dnf", "remove") => vec![with(s(&["dnf", "remove", "-y"]), &pk)],
        ("pacman", "install") => vec![with(s(&["pacman", "-S", "--noconfirm", "--needed"]), &pk)],
        ("pacman", "remove") => vec![with(s(&["pacman", "-R", "--noconfirm"]), &pk)],
        ("brew", "install") => vec![with(s(&["brew", "install"]), &pk)],
        ("brew", "remove") => vec![with(s(&["brew", "uninstall"]), &pk)],
        ("brew-cask", "install") => vec![with(s(&["brew", "install", "--cask"]), &pk)],
        ("brew-cask", "remove") => vec![with(s(&["brew", "uninstall", "--cask"]), &pk)],
        ("winget", "install") => pk
            .iter()
            .map(|id| s(&["winget", "install", "--id", id, "-e", "--accept-package-agreements", "--accept-source-agreements"]))
            .collect(),
        ("winget", "remove") => pk.iter().map(|id| s(&["winget", "uninstall", "--id", id, "-e"])).collect(),
        ("npm", "install") => vec![with(s(&["npm", "install", "-g"]), &pk)],
        ("npm", "remove") => vec![with(s(&["npm", "uninstall", "-g"]), &pk)],
        ("gem", "install") => vec![with(s(&["gem", "install"]), &pk)],
        ("gem", "remove") => vec![with(s(&["gem", "uninstall"]), &pk)],
        _ => Vec::new(),
    }
}

fn needs_root(manager: &str) -> bool {
    matches!(manager, "apt" | "dnf" | "pacman")
}

impl ToolDef {
    /// Build an install or removal plan for this host.
    pub fn plan(&self, action: &str, host: &Host, status: &ToolStatus, all: &BTreeMap<String, ToolStatus>, managed_kinds: &[&str]) -> ToolPlan {
        let recipes = if action == "install" { &self.install } else { &self.remove };
        let mut plan = ToolPlan {
            tool: self.id.clone(),
            action: action.to_string(),
            manager: None,
            commands: Vec::new(),
            display: Vec::new(),
            privileged: false,
            missing_dependencies: Vec::new(),
            already_installed: None,
            blockers: Vec::new(),
            manual: None,
            trust: self.trust.clone(),
            upgrade: self.upgrade.clone(),
            data_note: self
                .service
                .as_ref()
                .filter(|_| action == "remove")
                .map(|s| format!("Data in {} is not removed by devd.", s.data_dir)),
        };
        if action == "install" {
            if status.installed {
                plan.already_installed = status.version.clone().or(Some("unknown version".into()));
            }
            for d in &self.depends {
                if !all.get(d).map(|s| s.installed).unwrap_or(false) {
                    plan.missing_dependencies.push(d.clone());
                }
            }
        } else if !status.installed {
            plan.blockers.push(format!("{} is not installed", self.name));
        }
        if self.per_project {
            plan.blockers.push(format!(
                "{} is installed per project (see its stack's package manager), not on the machine",
                self.name
            ));
            return plan;
        }
        let usable = recipes.iter().find(|r| {
            host.has(&r.manager)
                && r.manager != "manual"
                && (r.manager != "devd-runtime" || {
                    let kind = r.packages.first().map(|s| s.as_str()).unwrap_or("");
                    managed_kinds.contains(&kind) && (kind != "php" || host.os == "windows")
                })
        });
        if let Some(r) = usable {
            plan.manager = Some(r.manager.clone());
            plan.privileged = needs_root(&r.manager);
            if r.manager == "devd-runtime" {
                plan.display.push(format!(
                    "devd runtime install {}@latest  (verified download into the devd directory; no administrator rights)",
                    r.packages.first().cloned().unwrap_or_default()
                ));
            } else {
                for c in recipe_commands(&r.manager, action, r) {
                    let mut argv = c;
                    if plan.privileged && !host.root {
                        argv.insert(0, "sudo".into());
                        argv.insert(1, "-n".into());
                    }
                    plan.display.push(crate::template::display(&argv));
                    plan.commands.push(argv);
                }
            }
            if plan.privileged && !host.root && !host.sudo {
                plan.blockers.push(
                    "administrator rights are required and devd has neither root nor passwordless sudo; run the commands above yourself"
                        .into(),
                );
            }
        } else if let Some(m) = recipes.iter().find(|r| r.manager == "manual") {
            plan.manual = Some(match &m.url {
                Some(u) => format!("{} ({u})", m.note),
                None => m.note.clone(),
            });
            plan.blockers.push(format!(
                "devd cannot automate this on {} with the package managers found ({}); follow the manual steps",
                host.os,
                if host.managers.is_empty() { "none".to_string() } else { host.managers.join(", ") }
            ));
        } else {
            plan.blockers.push(format!(
                "no {action} recipe matches this machine (package managers found: {})",
                if host.managers.is_empty() { "none".to_string() } else { host.managers.join(", ") }
            ));
        }
        plan
    }
}

/// What a service check found.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceState {
    /// Tool id.
    pub tool: String,
    /// The tool is installed.
    pub installed: bool,
    /// Unit used to control it.
    pub unit: Option<String>,
    /// Port checked.
    pub port: u16,
    /// Something accepts connections on the port.
    pub listening: bool,
    /// The health probe passed (or there is none and the port is open).
    pub healthy: bool,
    /// Another program holds the port.
    pub port_conflict: Option<String>,
    /// How the host can control it.
    pub control: String,
    /// Explanation of the state.
    pub detail: String,
    /// Where the data and logs are.
    pub data_dir: String,
    /// Where the logs are.
    pub logs: String,
    /// Security note.
    pub security: String,
}

fn tcp_open(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

fn unit_exists(unit: &str, host: &Host) -> bool {
    if host.systemd {
        if let Some(sc) = util::which("systemctl") {
            let o = util::run(
                &sc,
                &["list-unit-files".into(), format!("{unit}.service"), "--no-legend".into()],
                None,
                &[],
                Some(Duration::from_secs(10)),
            );
            if let Ok(o) = o {
                return o.stdout.contains(unit);
            }
        }
    }
    Path::new(&format!("/etc/init.d/{unit}")).exists()
}

impl ToolDef {
    /// Report the state of a service tool.
    pub fn service_state(&self, host: &Host, status: &ToolStatus) -> Option<ServiceState> {
        let svc = self.service.as_ref()?;
        let unit = svc.units.iter().find(|u| unit_exists(u, host)).cloned();
        let listening = tcp_open(svc.port);
        let mut healthy = listening;
        if listening && !svc.health.is_empty() {
            if let Some((exe, _)) = which_in(&svc.health[0], &[]) {
                healthy = util::run(&exe, &svc.health[1..], None, &[], Some(Duration::from_secs(10)))
                    .map(|o| o.ok())
                    .unwrap_or(false);
            }
        }
        let control = if unit.is_some() && host.systemd {
            "systemd"
        } else if unit.is_some() {
            "service"
        } else if !svc.direct_start.is_empty() {
            "direct"
        } else {
            "none"
        };
        let conflict = if listening && status.installed {
            devd_core::ports::find_listener(svc.port)
                .ok()
                .flatten()
                .and_then(|l| l.program)
                .filter(|p| !self.exes.iter().chain(svc.units.iter()).any(|n| p.contains(n.as_str()) || n.contains(p.as_str())))
        } else {
            None
        };
        let detail = if !status.installed {
            format!("{} is not installed", self.name)
        } else if healthy {
            format!("running; accepting connections on 127.0.0.1:{}", svc.port)
        } else if listening {
            format!("port {} is open but the health check failed", svc.port)
        } else {
            format!("installed but not running (nothing listens on port {})", svc.port)
        };
        Some(ServiceState {
            tool: self.id.clone(),
            installed: status.installed,
            unit,
            port: svc.port,
            listening,
            healthy,
            port_conflict: conflict,
            control: control.to_string(),
            detail,
            data_dir: svc.data_dir.clone(),
            logs: svc.logs.clone(),
            security: svc.security.clone(),
        })
    }

    /// Argv to start/stop/restart/logs a service on this host.
    pub fn service_command(&self, action: &str, host: &Host, state: &ServiceState) -> Result<(Vec<String>, bool)> {
        let svc = self
            .service
            .as_ref()
            .ok_or_else(|| DevdError::unsupported(format!("{} is not a service", self.name)))?;
        let sudo = |mut argv: Vec<String>| {
            if !host.root {
                argv.insert(0, "-n".into());
                argv.insert(0, "sudo".into());
            }
            argv
        };
        match state.control.as_str() {
            "systemd" => {
                let unit = format!("{}.service", state.unit.clone().unwrap_or_default());
                let argv = if action == "logs" {
                    vec!["journalctl".into(), "-u".into(), unit, "-n".into(), "200".into(), "--no-pager".into()]
                } else {
                    vec!["systemctl".into(), action.into(), unit]
                };
                if action == "logs" {
                    Ok((argv, false))
                } else {
                    Ok((sudo(argv), true))
                }
            }
            "service" => {
                if action == "logs" {
                    return Err(DevdError::unsupported(format!("this host has no journal; look in {}", svc.logs)));
                }
                Ok((sudo(vec!["service".into(), state.unit.clone().unwrap_or_default(), action.into()]), true))
            }
            "direct" => match action {
                "start" => Ok((svc.direct_start.clone(), false)),
                "stop" if !svc.direct_stop.is_empty() => Ok((svc.direct_stop.clone(), false)),
                "restart" => Err(DevdError::unsupported("stop and start it separately on a host without a service manager")),
                _ => Err(DevdError::unsupported(format!("`{action}` is not available without a service manager; look in {}", svc.logs))),
            },
            _ => Err(DevdError::unsupported(format!(
                "this host has no service manager entry for {}; install it first or start it yourself",
                self.name
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(cmp_version("8.10", "8.9"), std::cmp::Ordering::Greater);
        assert_eq!(cmp_version("v22.22.0", "22.22"), std::cmp::Ordering::Equal);
        assert!(satisfies("8.3.6", Some("8.2"), None));
        assert!(!satisfies("8.1.0", Some("8.2"), None));
        assert!(satisfies("8.4.9", None, Some("8.4")));
        assert!(!satisfies("8.5.0", None, Some("8.4")));
    }

    #[test]
    fn versions_are_read_from_noisy_output() {
        let java = "Picked up JAVA_TOOL_OPTIONS: -Dhttps.proxyHost=127.0.0.1\nopenjdk version \"21.0.4\" 2024";
        assert_eq!(extract_version(java, Some(r#"version "(\d+(?:\.\d+)*)""#)).as_deref(), Some("21.0.4"));
        assert_eq!(extract_version(java, None).as_deref(), Some("21.0.4"));
        assert_eq!(extract_version("go version go1.24.1 linux/amd64", Some(r"go(\d+\.\d+(?:\.\d+)?)")).as_deref(), Some("1.24.1"));
    }
}
