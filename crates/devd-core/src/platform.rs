//! The operating system devd is running on, and what it knows about software
//! the system's package manager installed.
//!
//! On Windows devd downloads each program from its publisher, checks the
//! publisher's digest and unpacks it. Linux and macOS have no such publisher for
//! nginx, MariaDB or PHP: the distribution (or Homebrew) builds and signs them.
//! devd therefore does not download a second copy. It finds the program the
//! package manager installed, asks the package manager who owns the file and
//! whether it is still what was installed, and records the answer. That is the
//! same contract as the Windows path - *a program devd cannot account for is a
//! program it does not run* - with the package manager standing where the
//! publisher's digest stands.
//!
//! Everything that asks the machine a question goes through [`Commands`], so the
//! parsing and the decisions are tested on canned output rather than on whatever
//! the test machine happens to have installed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::util;
use crate::{DevdError, Result};

/// The operating systems devd targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    /// Windows 10 and 11.
    Windows,
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
}

impl Os {
    /// The system this binary was built for.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }

    /// Whether programs are installed by a package manager rather than
    /// downloaded from their publisher.
    pub fn uses_system_packages(self) -> bool {
        self != Os::Windows
    }

    /// The default HTTP and HTTPS ports for local sites.
    ///
    /// Windows lets an ordinary user listen on 80 and 443. Linux does not (the
    /// first unprivileged port is 1024 unless the administrator moved it), and
    /// macOS allows it only for some kinds of listening address, depending on
    /// the release. A web server that cannot bind its port is not a default
    /// anyone wants, so everywhere but Windows starts on 8080 and 8443; the
    /// ports are a setting, and `devd setup` moves them when one is taken.
    pub fn default_ports(self) -> (u16, u16) {
        match self {
            Os::Windows => (80, 443),
            Os::Linux | Os::MacOs => (8080, 8443),
        }
    }
}

/// Directories where packages put server programs that are not on a user's
/// `PATH`: `nginx` lives in `/usr/sbin`, which Debian leaves out of a normal
/// user's `PATH`.
pub fn system_bin_dirs() -> Vec<PathBuf> {
    [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
        "/opt/homebrew/sbin",
        "/opt/homebrew/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// Find the first of `names` on `PATH` or in the system directories.
pub fn find_binary(names: &[&str]) -> Option<PathBuf> {
    find_binary_in(names, &system_bin_dirs())
}

/// [`find_binary`] with the extra directories given, for tests.
pub fn find_binary_in(names: &[&str], dirs: &[PathBuf]) -> Option<PathBuf> {
    for name in names {
        if let Some(found) = util::which(name) {
            return Some(found);
        }
        for dir in dirs {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Point `link` at `target`, replacing a link that is already there.
///
/// Only a link is replaced. A regular file or a directory at `link` means
/// something other than devd put it there, and removing it would be deleting
/// somebody's work to make a layout tidy.
#[cfg(unix)]
pub fn link(target: &Path, link: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(link)?,
        Ok(_) => {
            return Err(DevdError::conflict(format!(
                "{} exists and is not a link devd made; it was left alone",
                link.display()
            )))
        }
        Err(_) => {}
    }
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

/// Links are a Unix idea here: Windows installs real copies.
#[cfg(not(unix))]
pub fn link(_target: &Path, _link: &Path) -> Result<()> {
    Err(DevdError::unsupported(
        "linking a system program into devd's layout is only done on Linux and macOS",
    ))
}

// ------------------------------------------------------------------ packages

/// A package manager that can say where a file came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    /// Debian, Ubuntu and derivatives.
    Dpkg,
    /// Fedora, RHEL and derivatives.
    Rpm,
    /// Arch and derivatives.
    Pacman,
    /// Homebrew, on macOS or Linux.
    Homebrew,
}

impl PackageManager {
    /// The name people use for it.
    pub fn label(self) -> &'static str {
        match self {
            PackageManager::Dpkg => "dpkg",
            PackageManager::Rpm => "rpm",
            PackageManager::Pacman => "pacman",
            PackageManager::Homebrew => "Homebrew",
        }
    }
}

/// What the package manager said about the integrity of the installed files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum Verification {
    /// The package manager compared the file with what the package installed and
    /// it matched.
    Verified,
    /// The file differs from what the package installed. devd will not run it.
    Failed(String),
    /// The package manager has no way to re-check an installed file; the reason
    /// is given.
    Unchecked(String),
}

/// Where an installed program came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Origin {
    /// The package manager that owns the file.
    pub manager: PackageManager,
    /// The package.
    pub package: String,
    /// The package version, as the package manager states it.
    pub version: String,
    /// Whether the installed file still matches the package.
    pub verification: Verification,
}

/// Something that can run a program and say what it printed.
///
/// `None` means the program does not exist on this machine, which is different
/// from the program failing: it decides which package manager is in use.
pub trait Commands {
    /// Run `program` with `args`; the exit code and the text it printed.
    fn run(&self, program: &str, args: &[&str]) -> Option<(i32, String)>;
}

/// The machine, as far as adopting an installed program needs to see it.
pub trait Host: Commands {
    /// The first of `names` that is installed.
    fn find(&self, names: &[&str]) -> Option<PathBuf>;
    /// What `exe args` prints, standard output and standard error together.
    /// nginx, for one, prints its version on standard error.
    fn banner(&self, exe: &Path, args: &[&str]) -> Result<String>;
}

/// [`Host`] backed by the real machine.
pub struct SystemHost;

impl Commands for SystemHost {
    fn run(&self, program: &str, args: &[&str]) -> Option<(i32, String)> {
        let exe = util::which(program).or_else(|| find_binary(&[program]))?;
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = util::run(&exe, &args, None, &[], Some(Duration::from_secs(60))).ok()?;
        Some((out.code, format!("{}{}", out.stdout, out.stderr)))
    }
}

impl Host for SystemHost {
    fn find(&self, names: &[&str]) -> Option<PathBuf> {
        find_binary(names)
    }

    fn banner(&self, exe: &Path, args: &[&str]) -> Result<String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = util::run(exe, &args, None, &[], Some(Duration::from_secs(30)))?;
        Ok(format!("{}\n{}", out.stdout, out.stderr))
    }
}

/// One program of an installation: the name it takes in devd's layout, and the
/// names the system might know it by.
pub struct Want {
    /// Path inside devd's install directory, e.g. `bin/mariadbd`.
    pub link: &'static str,
    /// Names to look for, most specific first.
    pub names: &'static [&'static str],
}

/// A program found on the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    /// Path inside devd's install directory.
    pub link: String,
    /// Where the system keeps it.
    pub target: PathBuf,
}

/// An installation devd can adopt.
#[derive(Debug, Clone, Serialize)]
pub struct Adoption {
    /// Every program, in the order asked for; the first is the one whose
    /// version and origin describe the installation.
    pub found: Vec<Found>,
    /// What the first program printed when asked for its version.
    pub banner: String,
    /// Which package owns the first program, when one does.
    pub origin: Option<Origin>,
    /// The directory that holds `bin/` and `share/` for the first program.
    pub prefix: PathBuf,
    /// Things to tell the person.
    pub warnings: Vec<String>,
}

/// Find every program in `wants`, ask the first one for its version, and decide
/// whether the package manager still vouches for it.
///
/// `what` names the thing for messages (`nginx`); `install_hint` says how to get
/// it. Nothing on disk is changed: linking the programs into devd's layout is a
/// separate, explicit step, so an adoption can be planned and shown first.
pub fn adopt(
    host: &dyn Host,
    what: &str,
    install_hint: &str,
    wants: &[Want],
    version_args: &[&str],
) -> Result<Adoption> {
    let mut found = Vec::new();
    for want in wants {
        match host.find(want.names) {
            Some(target) => found.push(Found {
                link: want.link.to_string(),
                target,
            }),
            None => {
                return Err(DevdError::not_found(format!(
                    "{what} is not installed on this system (looked for {}). {install_hint}",
                    want.names.join(", ")
                )))
            }
        }
    }
    let first = found
        .first()
        .ok_or_else(|| DevdError::internal(format!("nothing to adopt for {what}")))?
        .target
        .clone();
    let banner = host.banner(&first, version_args)?;
    let origin = origin_of(&first, host);
    let warnings = require_intact(&first, origin.as_ref())?;
    let real = std::fs::canonicalize(&first).unwrap_or_else(|_| first.clone());
    let real = util::strip_verbatim(&real);
    let prefix = real
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/usr"));
    Ok(Adoption {
        found,
        banner,
        origin,
        prefix,
        warnings,
    })
}

/// Link every adopted program into `dir`, the layout the rest of devd expects.
pub fn link_into(dir: &Path, found: &[Found]) -> Result<()> {
    for f in found {
        link(&f.target, &dir.join(&f.link))?;
    }
    Ok(())
}

/// Find out which package installed `path`, and whether it is still intact.
///
/// `path` is resolved through symbolic links first: `/usr/sbin/nginx` may be a
/// link into an alternatives directory, and the package owns the real file.
/// Returns `None` when no package manager claims the file, which is how a
/// program built by hand shows up.
pub fn origin_of(path: &Path, commands: &dyn Commands) -> Option<Origin> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let real = util::strip_verbatim(&real);
    let file = real.to_string_lossy().into_owned();

    if let Some(origin) = homebrew_origin(&file) {
        return Some(origin);
    }
    dpkg_origin(&file, commands)
        .or_else(|| rpm_origin(&file, commands))
        .or_else(|| pacman_origin(&file, commands))
}

/// Refuse a program the package manager says was changed after it was installed;
/// say so in a warning when the package manager cannot check, or when no package
/// owns the file.
pub fn require_intact(path: &Path, origin: Option<&Origin>) -> Result<Vec<String>> {
    match origin {
        None => Ok(vec![format!(
            "{} does not belong to any package, so there is nothing to check it against; it \
             was not built or signed by a distribution",
            path.display()
        )]),
        Some(o) => match &o.verification {
            Verification::Verified => Ok(Vec::new()),
            Verification::Unchecked(reason) => Ok(vec![format!(
                "{} {} ({}) cannot be re-checked: {reason}",
                o.package,
                o.version,
                o.manager.label()
            )]),
            Verification::Failed(detail) => Err(DevdError::invalid(format!(
                "{} no longer matches the {} package {} {} that installed it ({detail}); \
                 devd will not run a program that differs from its package. Reinstall the \
                 package and try again.",
                path.display(),
                o.manager.label(),
                o.package,
                o.version
            ))),
        },
    }
}

fn dpkg_origin(file: &str, commands: &dyn Commands) -> Option<Origin> {
    let (code, text) = commands.run("dpkg", &["-S", file])?;
    if code != 0 {
        return None;
    }
    let package = parse_dpkg_owner(&text, file)?;
    let version = commands
        .run("dpkg-query", &["-W", "-f=${Version}", &package])
        .filter(|(code, _)| *code == 0)
        .map(|(_, text)| text.trim().to_string())
        .unwrap_or_default();
    let verification = match commands.run("dpkg", &["--verify", &package]) {
        Some((_, text)) => match failed_lines(&text, file) {
            Some(line) => Verification::Failed(line),
            None => Verification::Verified,
        },
        None => Verification::Unchecked("`dpkg --verify` is not available".to_string()),
    };
    Some(Origin {
        manager: PackageManager::Dpkg,
        package,
        version,
        verification,
    })
}

/// `dpkg -S /usr/sbin/nginx` prints `nginx-core: /usr/sbin/nginx`, or
/// `nginx, nginx-core: /usr/sbin/nginx` when two packages share a file.
pub fn parse_dpkg_owner(text: &str, file: &str) -> Option<String> {
    for line in text.lines() {
        let Some((owners, path)) = line.split_once(": ") else {
            continue;
        };
        if path.trim() != file {
            continue;
        }
        return owners
            .split(',')
            .map(|p| p.trim().split(':').next().unwrap_or("").to_string())
            .find(|p| !p.is_empty());
    }
    None
}

fn rpm_origin(file: &str, commands: &dyn Commands) -> Option<Origin> {
    let (code, text) = commands.run(
        "rpm",
        &["-qf", "--qf", "%{NAME}\t%{VERSION}-%{RELEASE}\n", file],
    )?;
    if code != 0 {
        return None;
    }
    let (package, version) = parse_rpm_owner(&text)?;
    let verification = match commands.run("rpm", &["-V", &package]) {
        Some((_, text)) => match failed_lines(&text, file) {
            Some(line) => Verification::Failed(line),
            None => Verification::Verified,
        },
        None => Verification::Unchecked("`rpm -V` is not available".to_string()),
    };
    Some(Origin {
        manager: PackageManager::Rpm,
        package,
        version,
        verification,
    })
}

/// The first `NAME<TAB>VERSION-RELEASE` line of `rpm -qf --qf` output.
pub fn parse_rpm_owner(text: &str) -> Option<(String, String)> {
    let line = text.lines().find(|l| l.contains('\t'))?;
    let (name, version) = line.split_once('\t')?;
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), version.trim().to_string()))
}

fn pacman_origin(file: &str, commands: &dyn Commands) -> Option<Origin> {
    let (code, text) = commands.run("pacman", &["-Qo", file])?;
    if code != 0 {
        return None;
    }
    let (package, version) = parse_pacman_owner(&text)?;
    let verification = match commands.run("pacman", &["-Qkk", &package]) {
        Some((_, text)) => match pacman_failure(&text, file) {
            Some(line) => Verification::Failed(line),
            None => Verification::Verified,
        },
        None => Verification::Unchecked("`pacman -Qkk` is not available".to_string()),
    };
    Some(Origin {
        manager: PackageManager::Pacman,
        package,
        version,
        verification,
    })
}

/// `/usr/bin/nginx is owned by nginx 1.26.1-1`.
pub fn parse_pacman_owner(text: &str) -> Option<(String, String)> {
    let line = text.lines().find(|l| l.contains(" is owned by "))?;
    let rest = line.split(" is owned by ").nth(1)?;
    let mut parts = rest.split_whitespace();
    let package = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    Some((package, version))
}

/// `pacman -Qkk` reports a changed file as
/// `warning: nginx: /usr/bin/nginx (SHA256 checksum mismatch)`.
fn pacman_failure(text: &str, file: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains(file) && (l.contains("mismatch") || l.contains("Modification time")))
        .map(|l| l.trim().to_string())
}

/// Lines of `dpkg --verify` / `rpm -V` output that report `file` as changed.
///
/// Both print a nine-column flag string, a type letter and the path. A `5` in
/// the flags means the content digest differs. Changed configuration files are
/// ordinary and are not what is being asked, so only lines about `file` count.
pub fn failed_lines(text: &str, file: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if !line.ends_with(file) {
            continue;
        }
        let flags = line.split_whitespace().next().unwrap_or("");
        if flags.chars().take(9).any(|c| c == '5' || c == 'S') || flags.contains("missing") {
            return Some(line.to_string());
        }
    }
    None
}

/// A Homebrew formula's real file lives at `<prefix>/Cellar/<formula>/<version>/`.
fn homebrew_origin(file: &str) -> Option<Origin> {
    let after = file.split("/Cellar/").nth(1)?;
    let mut parts = after.split('/');
    let package = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    if package.is_empty() || version.is_empty() {
        return None;
    }
    Some(Origin {
        manager: PackageManager::Homebrew,
        package,
        version,
        verification: Verification::Unchecked(
            "Homebrew checks a bottle's SHA-256 when it installs it and keeps no record to \
             re-check afterwards"
                .to_string(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Canned command output, keyed by the command line.
    struct Canned(HashMap<String, (i32, String)>);

    impl Canned {
        fn new(entries: &[(&str, i32, &str)]) -> Self {
            Canned(
                entries
                    .iter()
                    .map(|(k, c, t)| (k.to_string(), (*c, t.to_string())))
                    .collect(),
            )
        }
    }

    impl Commands for Canned {
        fn run(&self, program: &str, args: &[&str]) -> Option<(i32, String)> {
            let key = format!("{program} {}", args.join(" "));
            self.0.get(&key).cloned()
        }
    }

    /// A machine with a fixed set of programs and banners.
    struct FakeHost {
        programs: Vec<(&'static str, PathBuf)>,
        banner: &'static str,
        commands: Canned,
    }

    impl Commands for FakeHost {
        fn run(&self, program: &str, args: &[&str]) -> Option<(i32, String)> {
            self.commands.run(program, args)
        }
    }

    impl Host for FakeHost {
        fn find(&self, names: &[&str]) -> Option<PathBuf> {
            names.iter().find_map(|n| {
                self.programs
                    .iter()
                    .find(|(name, _)| name == n)
                    .map(|(_, p)| p.clone())
            })
        }
        fn banner(&self, _exe: &Path, _args: &[&str]) -> Result<String> {
            Ok(self.banner.to_string())
        }
    }

    #[test]
    fn adoption_finds_every_program_and_names_the_one_that_is_missing() {
        let host = FakeHost {
            programs: vec![("mariadbd", PathBuf::from("/usr/sbin/mariadbd"))],
            banner: "mariadbd  Ver 11.4.2-MariaDB for debian-linux-gnu\n",
            commands: Canned::new(&[]),
        };
        let wants = [
            Want { link: "bin/mariadbd", names: &["mariadbd", "mysqld"] },
            Want { link: "bin/mysql", names: &["mariadb", "mysql"] },
        ];
        let err = adopt(&host, "MariaDB", "Install it with `devd toolchain install mariadb`.", &wants, &["--version"])
            .unwrap_err()
            .to_string();
        assert!(err.contains("MariaDB is not installed"), "{err}");
        assert!(err.contains("mariadb, mysql"), "{err}");
        assert!(err.contains("devd toolchain install mariadb"), "{err}");
    }

    #[test]
    fn adoption_reports_the_version_banner_origin_and_a_missing_origin_warning() {
        let host = FakeHost {
            programs: vec![("nginx", PathBuf::from("/usr/sbin/nginx"))],
            banner: "nginx version: nginx/1.24.0 (Ubuntu)\n",
            commands: Canned::new(&[]),
        };
        let wants = [Want { link: "sbin/nginx", names: &["nginx"] }];
        let adoption = adopt(&host, "nginx", "", &wants, &["-v"]).unwrap();
        assert!(adoption.banner.contains("nginx/1.24.0"));
        assert_eq!(adoption.found[0].link, "sbin/nginx");
        assert!(adoption.origin.is_none());
        assert_eq!(adoption.warnings.len(), 1, "an unowned file earns a warning");
    }

    #[test]
    fn adoption_refuses_a_binary_its_package_no_longer_vouches_for() {
        let host = FakeHost {
            programs: vec![("nginx", PathBuf::from("/usr/sbin/nginx"))],
            banner: "nginx version: nginx/1.24.0\n",
            commands: Canned::new(&[
                ("dpkg -S /usr/sbin/nginx", 0, "nginx-core: /usr/sbin/nginx\n"),
                ("dpkg-query -W -f=${Version} nginx-core", 0, "1.24.0-2"),
                ("dpkg --verify nginx-core", 1, "??5??????   /usr/sbin/nginx\n"),
            ]),
        };
        let wants = [Want { link: "sbin/nginx", names: &["nginx"] }];
        // A path that does not exist is kept as given, which is what the canned
        // dpkg output is keyed on.
        let err = adopt(&host, "nginx", "", &wants, &["-v"]).unwrap_err();
        assert!(err.to_string().contains("differs from its package"), "{err}");
    }

    #[test]
    fn only_windows_starts_on_the_privileged_ports() {
        assert_eq!(Os::Windows.default_ports(), (80, 443));
        assert_eq!(Os::Linux.default_ports(), (8080, 8443));
        assert_eq!(Os::MacOs.default_ports(), (8080, 8443));
    }

    #[test]
    fn only_windows_downloads_from_the_publisher() {
        assert!(!Os::Windows.uses_system_packages());
        assert!(Os::Linux.uses_system_packages());
        assert!(Os::MacOs.uses_system_packages());
    }

    #[test]
    fn dpkg_names_the_first_owner_even_when_two_packages_share_a_file() {
        assert_eq!(
            parse_dpkg_owner("nginx-core: /usr/sbin/nginx\n", "/usr/sbin/nginx").as_deref(),
            Some("nginx-core")
        );
        assert_eq!(
            parse_dpkg_owner("nginx, nginx-core: /usr/sbin/nginx\n", "/usr/sbin/nginx")
                .as_deref(),
            Some("nginx")
        );
        assert_eq!(
            parse_dpkg_owner("libssl3:amd64: /usr/lib/x86_64-linux-gnu/libssl.so.3\n", "/x"),
            None
        );
        assert_eq!(
            parse_dpkg_owner("libssl3:amd64: /usr/lib/libssl.so.3\n", "/usr/lib/libssl.so.3")
                .as_deref(),
            Some("libssl3")
        );
    }

    #[test]
    fn rpm_and_pacman_owners_are_read() {
        assert_eq!(
            parse_rpm_owner("nginx\t1.24.0-1.fc40\n"),
            Some(("nginx".into(), "1.24.0-1.fc40".into()))
        );
        assert_eq!(parse_rpm_owner("file /x is not owned by any package\n"), None);
        assert_eq!(
            parse_pacman_owner("/usr/bin/nginx is owned by nginx 1.26.1-1\n"),
            Some(("nginx".into(), "1.26.1-1".into()))
        );
        assert_eq!(
            parse_pacman_owner("error: No package owns /usr/local/bin/nginx\n"),
            None
        );
    }

    #[test]
    fn a_changed_binary_is_reported_and_a_changed_config_file_is_not() {
        let output = "??5?????? c /etc/nginx/nginx.conf\n??5??????   /usr/sbin/nginx\n";
        assert_eq!(
            failed_lines(output, "/usr/sbin/nginx").as_deref(),
            Some("??5??????   /usr/sbin/nginx")
        );
        assert_eq!(failed_lines("??5?????? c /etc/nginx/nginx.conf\n", "/usr/sbin/nginx"), None);
        assert_eq!(failed_lines("", "/usr/sbin/nginx"), None);
    }

    #[test]
    fn a_debian_package_is_verified_against_its_checksums() {
        let commands = Canned::new(&[
            ("dpkg -S /usr/sbin/nginx", 0, "nginx-core: /usr/sbin/nginx\n"),
            ("dpkg-query -W -f=${Version} nginx-core", 0, "1.24.0-2ubuntu7"),
            ("dpkg --verify nginx-core", 0, ""),
        ]);
        let origin = dpkg_origin("/usr/sbin/nginx", &commands).expect("owned");
        assert_eq!(origin.manager, PackageManager::Dpkg);
        assert_eq!(origin.package, "nginx-core");
        assert_eq!(origin.version, "1.24.0-2ubuntu7");
        assert_eq!(origin.verification, Verification::Verified);
        assert!(require_intact(Path::new("/usr/sbin/nginx"), Some(&origin))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_tampered_binary_is_refused() {
        let commands = Canned::new(&[
            ("dpkg -S /usr/sbin/nginx", 0, "nginx-core: /usr/sbin/nginx\n"),
            ("dpkg-query -W -f=${Version} nginx-core", 0, "1.24.0-2"),
            ("dpkg --verify nginx-core", 1, "??5??????   /usr/sbin/nginx\n"),
        ]);
        let origin = dpkg_origin("/usr/sbin/nginx", &commands).unwrap();
        assert!(matches!(origin.verification, Verification::Failed(_)));
        let err = require_intact(Path::new("/usr/sbin/nginx"), Some(&origin)).unwrap_err();
        assert!(err.to_string().contains("will not run a program that differs"));
    }

    #[test]
    fn a_program_no_package_owns_is_allowed_with_a_warning() {
        let warnings = require_intact(Path::new("/usr/local/sbin/nginx"), None).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("does not belong to any package"));
    }

    #[test]
    fn homebrew_is_recognised_from_the_cellar_path_and_says_it_cannot_recheck() {
        let origin = homebrew_origin("/opt/homebrew/Cellar/nginx/1.27.0/bin/nginx").unwrap();
        assert_eq!(origin.manager, PackageManager::Homebrew);
        assert_eq!(origin.package, "nginx");
        assert_eq!(origin.version, "1.27.0");
        let warnings = require_intact(Path::new("/x"), Some(&origin)).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(homebrew_origin("/usr/sbin/nginx").is_none());
    }

    #[test]
    fn a_missing_package_manager_is_not_an_owner() {
        let none = Canned::new(&[]);
        assert!(dpkg_origin("/usr/sbin/nginx", &none).is_none());
        assert!(rpm_origin("/usr/sbin/nginx", &none).is_none());
        assert!(pacman_origin("/usr/sbin/nginx", &none).is_none());
    }

    #[test]
    fn binaries_are_found_in_the_system_directories_when_path_misses_them() {
        let dir = tempfile::tempdir().unwrap();
        let sbin = dir.path().join("sbin");
        std::fs::create_dir_all(&sbin).unwrap();
        std::fs::write(sbin.join("devd-test-server"), b"x").unwrap();
        let found = find_binary_in(&["devd-test-nothing", "devd-test-server"], &[sbin.clone()]);
        assert_eq!(found, Some(sbin.join("devd-test-server")));
        assert!(find_binary_in(&["devd-test-nothing"], &[sbin]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_replaces_a_link_but_never_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let target_a = dir.path().join("a");
        let target_b = dir.path().join("b");
        std::fs::write(&target_a, b"a").unwrap();
        std::fs::write(&target_b, b"b").unwrap();
        let at = dir.path().join("layout/bin/tool");

        link(&target_a, &at).unwrap();
        link(&target_b, &at).unwrap();
        assert_eq!(std::fs::read(&at).unwrap(), b"b");

        let file = dir.path().join("layout/bin/real");
        std::fs::write(&file, b"keep").unwrap();
        assert!(link(&target_a, &file).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    }
}
