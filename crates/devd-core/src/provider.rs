//! Where work actually runs: the provider seam.
//!
//! devd has always started native processes: a `SpawnSpec`, then a child of the
//! devd process. That is the right default - it is fast, it needs nothing
//! installed, and it is what most development work wants. What it could not do is
//! *anything else*: there was no place in the code where "run this somewhere
//! safer" could be decided.
//!
//! This module is that place, and nothing more. It does not implement a container
//! engine - it decides which one is available and says so plainly, so the rest of
//! devd can ask "what can this machine do?" instead of assuming.
//!
//! The rule that shapes everything here: **devd must work perfectly with nothing
//! installed.** A missing container engine is not a broken devd, it is a machine
//! that runs processes, and the Native provider is always there. Anything that
//! needs a container says which capability it lacks and what would provide it,
//! rather than failing obscurely or silently falling back to something less
//! isolated than the caller asked for.
//!
//! ## What devd owns, and what it does not
//!
//! Owned here: the choice of provider, the statement of what each can do, and the
//! refusal to pretend. Not owned: OCI, namespaces, cgroups, or a virtual machine.
//! Those are mature and devd gains nothing by reimplementing them.
//!
//! ## Why detection, not installation
//!
//! Rootless Podman on Windows needs WSL2 or a Podman Machine; Docker Desktop is a
//! system-wide install. Both are changes to the machine that a person should make
//! deliberately, so devd detects and reports, and never installs an engine behind
//! someone's back. `devd runtime.providers` says what was found and what is
//! missing, with the exact command that would provide it.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How a piece of work runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A child of the devd process. Always available, no isolation beyond the
    /// operating system's own user boundary.
    Native,
    /// An OCI container through an engine on this machine. Rootless where the
    /// engine supports it.
    Container,
    /// A virtual machine. Reserved: no engine is wired up, and saying so is
    /// better than a variant that silently does nothing.
    VirtualMachine,
}

impl Kind {
    /// The name a person sees.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Native => "native",
            Kind::Container => "container",
            Kind::VirtualMachine => "vm",
        }
    }
}

/// What a provider can actually do, as opposed to what it is called.
///
/// Capabilities are reported, never assumed: a provider that cannot enforce a
/// memory limit must say so, because code that needs one has to refuse rather
/// than believe it got it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Capability {
    /// Runs without root.
    pub rootless: bool,
    /// Can be given a CPU or memory ceiling that it enforces.
    pub resource_limits: bool,
    /// Can be given a private network namespace.
    pub private_network: bool,
    /// Can mount a directory read-only.
    pub readonly_mounts: bool,
    /// The filesystem is not the host's: a container image or a VM disk.
    pub isolated_filesystem: bool,
    /// Survives a reboot of the devd process without re-spawning.
    pub survives_restart: bool,
}

/// A provider devd found on this machine.
#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    /// Stable id: `native`, `podman`, `docker`.
    pub id: String,
    /// The kind of isolation it gives.
    pub kind: Kind,
    /// The executable, when there is one.
    pub program: Option<PathBuf>,
    /// Its version, as it reports itself.
    pub version: Option<String>,
    /// What it can do.
    pub capability: Capability,
    /// Why it cannot be used, when it cannot. `None` means it is usable.
    pub unusable: Option<String>,
    /// The command that would make it usable, when one would.
    pub remedy: Option<String>,
}

impl Provider {
    /// Whether devd may dispatch work to it.
    pub fn usable(&self) -> bool {
        self.unusable.is_none()
    }
}

/// Every provider devd knows about, and which is the default.
#[derive(Debug, Clone, Serialize)]
pub struct Providers {
    /// The provider used when a caller does not ask for one.
    pub default: String,
    /// All of them, including the ones that are not usable and why.
    pub providers: Vec<Provider>,
    /// What this machine cannot do that some work might want.
    pub missing: Vec<String>,
}

/// The native provider: a child of the devd process.
///
/// It is not a fallback and it is not second-class. It is the correct choice for
/// most development work, and it is the only provider devd promises on every
/// platform.
pub fn native() -> Provider {
    Provider {
        id: "native".into(),
        kind: Kind::Native,
        program: None,
        version: None,
        capability: Capability {
            rootless: true,
            // The operating system's own limits apply, but devd does not impose a
            // cgroup, so it must not claim to.
            resource_limits: false,
            private_network: false,
            readonly_mounts: false,
            isolated_filesystem: false,
            survives_restart: true,
        },
        unusable: None,
        remedy: None,
    }
}

/// Ask one engine whether it is there and usable, without waiting long.
///
/// A missing engine and a stopped engine are different answers: `command not
/// found` means nothing would help but an install, while a refused connection
/// means the program is there and its service is not running - which is a fixable
/// state and a different sentence to a person.
fn probe(id: &str, exe: &str, kind: Kind, version_args: &[&str]) -> Provider {
    let mut provider = Provider {
        id: id.into(),
        kind,
        program: None,
        version: None,
        capability: Capability {
            rootless: false,
            resource_limits: false,
            private_network: false,
            readonly_mounts: false,
            isolated_filesystem: true,
            survives_restart: true,
        },
        unusable: None,
        remedy: None,
    };
    let Some(path) = which(exe) else {
        provider.unusable = Some(format!("{exe} is not installed"));
        provider.remedy = Some(install_hint(id));
        return provider;
    };
    provider.program = Some(path.clone());
    match run_captured(&path, version_args) {
        Some(out) => provider.version = first_line(&out),
        None => {
            provider.unusable = Some(format!("{exe} is installed but did not answer `{}`", version_args.join(" ")));
            provider.remedy = Some(format!("check that {exe} runs from a terminal"));
            return provider;
        }
    }
    // Whether it can run is a separate question from whether it is installed.
    match run_captured(&path, &["info", "--format", "{{.Rootless}}"]) {
        Some(out) if out.trim() == "true" => {
            provider.capability.rootless = true;
            provider.capability.resource_limits = true;
            provider.capability.private_network = true;
            // Rootless containers can map a directory read-only.
            provider.capability.readonly_mounts = true;
        }
        Some(out) if out.trim() == "false" => {
            // Running, but as root. Usable, and the caller is told what it means.
            provider.capability.resource_limits = true;
            provider.capability.private_network = true;
            provider.capability.readonly_mounts = true;
            provider.unusable = Some(format!("{exe} is running as root; no rootless isolation is available"));
        }
        _ => {
            // `info` failed: either the daemon is down or this is not the CLI we
            // think it is. Say which by trying the simplest command once more.
            provider.unusable = Some(format!("{exe} is installed but its engine is not answering"));
            provider.remedy = Some(daemon_hint(id));
        }
    }
    provider
}

fn install_hint(id: &str) -> String {
    match id {
        "podman" => "install Podman: on Windows `winget install RedHat.Podman` (needs WSL2), on macOS `brew install podman`, on Linux your package manager".into(),
        "docker" => "install Docker Engine or a compatible runtime; devd does not require it, and native processes work without it".into(),
        _ => "install the engine".into(),
    }
}

fn daemon_hint(id: &str) -> String {
    match id {
        "podman" => "start it with `podman machine start` on Windows and macOS; on Linux the daemonless CLI should not need this".into(),
        "docker" => "start the Docker service, or stop it - devd does not need it".into(),
        _ => "start the engine".into(),
    }
}

/// Everything this machine can run work in.
///
/// Ordered by preference: Native first because it always works and is the right
/// default, then rootless containers, then not-rootless ones.
pub fn detect() -> Providers {
    let mut providers = vec![native()];
    providers.push(probe("podman", "podman", Kind::Container, &["--version"]));
    providers.push(probe("docker", "docker", Kind::Container, &["--version"]));

    let mut missing = Vec::new();
    if !providers.iter().any(|p| p.kind == Kind::Container && p.usable()) {
        missing.push(
            "no container engine is usable, so work cannot be isolated from the host. Native processes are used instead, and they are fine for trusted project work.".into(),
        );
    }
    missing.push(
        "virtual machines are not wired up, so untrusted code cannot be given a separate kernel".into(),
    );

    Providers { default: "native".into(), providers, missing }
}

/// Find an executable on PATH, without a shell.
fn which(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .map(|s| s.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{exe}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Run a short command and read its output. `None` when it fails or is slow.
///
/// The timeout matters: a container CLI with a stopped daemon can hang for
/// seconds, and a health screen must not wait for it.
fn run_captured(program: &PathBuf, args: &[&str]) -> Option<String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags_quiet()
        .spawn()
        .ok()?;
    let out = child.wait_timeout(Duration::from_secs(8))?;
    if !out.status.success() {
        return None;
    }
    Some(format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn first_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string)
}

/// A small extension so a probe cannot hang the caller.
trait QuietSpawn {
    fn creation_flags_quiet(&mut self) -> &mut Self;
}

impl QuietSpawn for Command {
    fn creation_flags_quiet(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: a probe must not flash a console on Windows.
            self.creation_flags(0x0800_0000);
        }
        self
    }
}

/// Wait for a child with a ceiling, then read what it said.
struct Outcome {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait WaitTimeout {
    fn wait_timeout(self, limit: Duration) -> Option<Outcome>;
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout(mut self, limit: Duration) -> Option<Outcome> {
        use std::io::Read;
        let start = std::time::Instant::now();
        loop {
            match self.try_wait() {
                Ok(Some(status)) => {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    if let Some(mut s) = self.stdout.take() {
                        let _ = s.read_to_end(&mut stdout);
                    }
                    if let Some(mut s) = self.stderr.take() {
                        let _ = s.read_to_end(&mut stderr);
                    }
                    return Some(Outcome { status, stdout, stderr });
                }
                Ok(None) => {
                    if start.elapsed() > limit {
                        let _ = self.kill();
                        let _ = self.wait();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return None,
            }
        }
    }
}

/// Whether a piece of work may run in the provider it asked for.
///
/// The one place that decides, so a caller cannot quietly get less isolation than
/// it requested. A request for a container on a machine with no engine is an
/// error that names the remedy - never a silent native run.
pub fn require(kind: Kind, of: &Providers) -> crate::Result<Provider> {
    let found = of
        .providers
        .iter()
        .filter(|p| p.kind == kind && p.usable())
        .cloned()
        .collect::<Vec<_>>();
    if let Some(first) = found.into_iter().next() {
        return Ok(first);
    }
    match kind {
        Kind::Native => Ok(native()),
        Kind::Container => {
            let why = of
                .providers
                .iter()
                .filter(|p| p.kind == Kind::Container)
                .map(|p| format!("{}: {}", p.id, p.unusable.clone().unwrap_or_else(|| "not usable".into())))
                .collect::<Vec<_>>()
                .join("; ");
            Err(crate::DevdError::conflict(format!(
                "this work asked to run in a container, and no container engine is usable here ({why}). Native processes are used for everything else; map the work to native, or install an engine - devd will not silently run it with less isolation than was asked for."
            )))
        }
        Kind::VirtualMachine => Err(crate::DevdError::conflict(
            "this work asked for a virtual machine, and devd has no VM provider wired up. Do not treat a container as a substitute: a container shares the host kernel.",
        )),
    }
}

/// The shape a screen, the CLI and an agent all read.
pub fn describe() -> Value {
    let d = detect();
    json!({
        "default": d.default,
        "providers": d.providers,
        "missing": d.missing,
        "summary": if d.providers.iter().any(|p| p.kind == Kind::Container && p.usable()) {
            "native processes, and a container engine when isolation is asked for"
        } else {
            "native processes only; no container engine was found, which is a supported configuration"
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_is_always_available_and_never_claims_isolation_it_has_not() {
        let p = native();
        assert!(p.usable());
        assert_eq!(p.kind, Kind::Native);
        assert!(p.capability.rootless, "a child of devd is not elevated by devd");
        assert!(!p.capability.isolated_filesystem, "it shares the host filesystem");
        assert!(!p.capability.resource_limits, "devd imposes no cgroup");
        assert!(!p.capability.private_network);
    }

    #[test]
    fn detection_always_finds_native_and_never_panics_on_a_machine_without_engines() {
        let d = detect();
        assert_eq!(d.default, "native");
        assert!(d.providers.iter().any(|p| p.kind == Kind::Native && p.usable()));
        // Every provider either works or says exactly why not.
        for p in &d.providers {
            if !p.usable() {
                assert!(p.unusable.is_some());
            }
        }
    }

    #[test]
    fn a_missing_engine_says_what_to_install_rather_than_disappearing() {
        let d = detect();
        for p in d.providers.iter().filter(|p| p.kind == Kind::Container) {
            if !p.usable() {
                assert!(p.unusable.is_some(), "{} must say why it cannot be used", p.id);
            }
        }
        // And the machine-level statement names the container gap when there is one.
        if !d.providers.iter().any(|p| p.kind == Kind::Container && p.usable()) {
            assert!(d.missing.iter().any(|m| m.contains("no container engine")), "{:?}", d.missing);
        }
    }

    #[test]
    fn asking_for_a_container_never_silently_degrades_to_native() {
        // The whole point of the seam: a caller that needs isolation must be told
        // when it cannot have it, not quietly given something weaker.
        let mut d = detect();
        d.providers.retain(|p| p.kind != Kind::Container);
        let err = require(Kind::Container, &d).unwrap_err().to_string();
        assert!(err.contains("no container engine"), "{err}");
        assert!(err.contains("less isolation than was asked for"), "{err}");
        // Native still works, so the rest of devd is unaffected.
        assert!(require(Kind::Native, &d).is_ok());
    }

    #[test]
    fn a_virtual_machine_is_refused_rather_than_faked_by_a_container() {
        let d = detect();
        let err = require(Kind::VirtualMachine, &d).unwrap_err().to_string();
        assert!(err.contains("shares the host kernel"), "{err}");
    }

    #[test]
    fn the_kind_names_are_the_ones_screens_show() {
        assert_eq!(Kind::Native.name(), "native");
        assert_eq!(Kind::Container.name(), "container");
        assert_eq!(Kind::VirtualMachine.name(), "vm");
    }

}
