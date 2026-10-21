//! The `devd.yml` project manifest.
//!
//! A manifest is executable configuration, so parsing and validation are a
//! security boundary rather than a convenience. The rules are deliberately
//! narrow:
//!
//! * `command` is a **list of arguments**, never a shell string. Nothing here is
//!   ever passed through `cmd.exe` or `sh`, so a manifest cannot smuggle a
//!   pipeline, a redirect or a second command into a process.
//! * `working_dir` must be relative and must resolve inside the project root.
//!   `..`, absolute paths and symlink escapes are refused.
//! * Only the variables declared by the manifest (plus a small, explicit
//!   baseline) are visible to the process. The daemon's own environment - which
//!   may hold API keys and credentials - is not inherited wholesale.
//! * Health checks may only address loopback, because a project process has no
//!   business probing a remote host from a manifest.
//! * The manifest is never executed merely because a directory contains one.
//!   Starting requires an approval recorded against the semantic hash of the
//!   manifest a human reviewed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use devd_core::error::{DevdError, Result};
use devd_core::util;

/// Manifest file name.
pub const MANIFEST_FILE: &str = "devd.yml";

/// Largest manifest accepted, in bytes.
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// Largest number of processes in one manifest.
pub const MAX_PROCESSES: usize = 32;

/// The version this binary understands.
pub const MANIFEST_VERSION: u32 = 1;

/// A parsed, resolved manifest.
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    /// Absolute project root.
    pub root: String,
    /// Declared project name.
    pub project: String,
    /// Semantic hash of the declaration, used for approval.
    pub hash: String,
    /// Processes, keyed by name.
    pub processes: BTreeMap<String, ProcessSpec>,
    /// When the file was read.
    pub loaded_at: String,
}

/// One resolved process declaration.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessSpec {
    /// Name from the manifest.
    pub name: String,
    /// Executable followed by its arguments.
    pub command: Vec<String>,
    /// Absolute working directory, verified inside the project root.
    pub working_dir: String,
    /// Environment visible to this process: baseline merged with manifest.
    pub env: BTreeMap<String, String>,
    /// Manifest environment only, for display and redaction.
    pub declared_env: BTreeMap<String, String>,
    /// Health check, when declared.
    pub health: Option<Health>,
    /// Restart policy.
    pub restart: String,
    /// Dependencies that must be ready first.
    pub depends_on: Vec<String>,
    /// How long to wait for readiness on start.
    pub startup_timeout_secs: u64,
}

/// A health check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    /// HTTP URL to poll, when URL-based.
    pub url: Option<String>,
    /// TCP port on loopback to connect to, when port-based.
    pub port: Option<u16>,
    /// Total time allowed for the first successful check.
    pub timeout_secs: u64,
    /// Delay between checks.
    pub interval_ms: u64,
}

// -------------------------------------------------------------- file shapes

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    version: u32,
    project: ProjectFile,
    #[serde(default)]
    processes: BTreeMap<String, ProcessFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectFile {
    name: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessFile {
    command: Vec<String>,
    #[serde(default)]
    working_dir: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    health: Option<HealthFile>,
    #[serde(default)]
    restart: Option<RestartFile>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    startup_timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthFile {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum RestartFile {
    Never,
    OnFailure,
    Always,
}

impl RestartFile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::OnFailure => "on-failure",
            Self::Always => "always",
        }
    }
}

// ------------------------------------------------------------------ loading

/// Whether a directory contains a manifest.
pub fn exists(root: &Path) -> bool {
    root.join(MANIFEST_FILE).is_file()
}

/// Load and validate the manifest in `root`.
pub fn load(root: &Path) -> Result<Manifest> {
    let root = root
        .canonicalize()
        .map_err(|e| DevdError::invalid(format!("project directory is not usable: {e}")))?;
    let root = util::strip_verbatim(&root);
    if !root.is_dir() {
        return Err(DevdError::invalid("the project path is not a directory"));
    }
    let path = root.join(MANIFEST_FILE);
    // The message is a sentence, not an operating-system complaint: "the
    // system cannot find the file specified" tells a person nothing they can
    // act on.
    let metadata = std::fs::metadata(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DevdError::not_found(format!(
            "there is no {MANIFEST_FILE} in {}",
            root.display()
        )),
        std::io::ErrorKind::PermissionDenied => DevdError::denied(format!(
            "{MANIFEST_FILE} in {} cannot be read",
            root.display()
        )),
        _ => DevdError::not_found(format!(
            "{MANIFEST_FILE} in {} cannot be read: {e}",
            root.display()
        )),
    })?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(DevdError::invalid(format!(
            "{MANIFEST_FILE} is larger than {} KiB",
            MAX_MANIFEST_BYTES / 1024
        )));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| DevdError::invalid(format!("cannot read {MANIFEST_FILE}: {e}")))?;
    parse(&root, &text)
}

/// Parse a manifest for an explicit project root.
pub fn parse(root: &Path, text: &str) -> Result<Manifest> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root = util::strip_verbatim(&root);
    let file: ManifestFile = serde_yaml::from_str(text)
        .map_err(|e| DevdError::invalid(format!("invalid {MANIFEST_FILE}: {e}")))?;

    if file.version != MANIFEST_VERSION {
        return Err(DevdError::unsupported(format!(
            "{MANIFEST_FILE} version {} is not supported; this devd understands version {}",
            file.version, MANIFEST_VERSION
        )));
    }
    validate_display_name(&file.project.name)?;
    if file.processes.is_empty() {
        return Err(DevdError::invalid(
            "a manifest must declare at least one process",
        ));
    }
    if file.processes.len() > MAX_PROCESSES {
        return Err(DevdError::invalid(format!(
            "a manifest may declare at most {MAX_PROCESSES} processes"
        )));
    }
    validate_env(&file.project.env, "project.env")?;

    let mut processes = BTreeMap::new();
    for (name, spec) in &file.processes {
        util::validate_name(name)
            .map_err(|e| DevdError::invalid(format!("process name `{name}` is not usable: {e}")))?;
        if spec.command.is_empty() {
            return Err(DevdError::invalid(format!(
                "process `{name}` has an empty command"
            )));
        }
        let mut total = 0usize;
        for arg in &spec.command {
            if arg.len() > 4096 {
                return Err(DevdError::invalid(format!(
                    "process `{name}` has an argument longer than 4096 bytes"
                )));
            }
            if arg
                .chars()
                .any(|c| c == '\0' || (c.is_control() && c != '\t'))
            {
                return Err(DevdError::invalid(format!(
                    "process `{name}` has a control character in its command"
                )));
            }
            total += arg.len();
        }
        if total > 32 * 1024 {
            return Err(DevdError::invalid(format!(
                "process `{name}` command is too long"
            )));
        }
        if spec.command[0].trim().is_empty() {
            return Err(DevdError::invalid(format!(
                "process `{name}` has an empty executable name"
            )));
        }

        validate_env(&spec.env, &format!("processes.{name}.env"))?;
        let working_dir = resolve_working_dir(&root, name, spec.working_dir.as_deref())?;
        let health = spec
            .health
            .as_ref()
            .map(|h| validate_health(name, h))
            .transpose()?;
        let startup_timeout_secs = spec.startup_timeout_secs.unwrap_or(30);
        if !(1..=600).contains(&startup_timeout_secs) {
            return Err(DevdError::invalid(format!(
                "process `{name}` startup_timeout_secs must be between 1 and 600"
            )));
        }
        for dep in &spec.depends_on {
            if dep == name {
                return Err(DevdError::invalid(format!(
                    "process `{name}` cannot depend on itself"
                )));
            }
            if !file.processes.contains_key(dep) {
                return Err(DevdError::invalid(format!(
                    "process `{name}` depends on `{dep}`, which is not declared"
                )));
            }
        }

        let mut env = baseline_env();
        for (k, v) in &file.project.env {
            env.insert(k.clone(), v.clone());
        }
        for (k, v) in &spec.env {
            env.insert(k.clone(), v.clone());
        }

        processes.insert(
            name.clone(),
            ProcessSpec {
                name: name.clone(),
                command: spec.command.clone(),
                working_dir: working_dir.to_string_lossy().to_string(),
                env,
                declared_env: spec.env.clone(),
                health,
                restart: spec
                    .restart
                    .unwrap_or(RestartFile::OnFailure)
                    .as_str()
                    .to_string(),
                depends_on: spec.depends_on.clone(),
                startup_timeout_secs,
            },
        );
    }

    detect_cycles(&processes)?;

    let canonical = serde_json::to_vec(&serde_json::json!({
        "version": file.version,
        "project": { "name": file.project.name, "env": file.project.env },
        "processes": file.processes.iter().map(|(name, spec)| {
            (name, serde_json::json!({
                "command": spec.command,
                "working_dir": spec.working_dir,
                "env": spec.env,
                "health": spec.health.as_ref().map(|h| serde_json::json!({
                    "url": h.url, "port": h.port,
                    "timeout_secs": h.timeout_secs, "interval_ms": h.interval_ms,
                })),
                "restart": spec.restart.map(|r| r.as_str()),
                "depends_on": spec.depends_on,
                "startup_timeout_secs": spec.startup_timeout_secs,
            }))
        }).collect::<BTreeMap<_, _>>(),
    }))
    .map_err(|e| DevdError::internal(e.to_string()))?;
    let hash = util::sha256_hex(&canonical);

    Ok(Manifest {
        root: root.to_string_lossy().to_string(),
        project: file.project.name,
        hash,
        processes,
        loaded_at: util::now(),
    })
}

/// A topological start order, dependencies first.
pub fn start_order(manifest: &Manifest) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut temporary = BTreeSet::new();
    let mut permanent = BTreeSet::new();
    for name in manifest.processes.keys() {
        visit(name, manifest, &mut temporary, &mut permanent, &mut out)?;
    }
    Ok(out)
}

fn visit(
    name: &str,
    manifest: &Manifest,
    temporary: &mut BTreeSet<String>,
    permanent: &mut BTreeSet<String>,
    out: &mut Vec<String>,
) -> Result<()> {
    if permanent.contains(name) {
        return Ok(());
    }
    if !temporary.insert(name.to_string()) {
        return Err(DevdError::invalid(format!(
            "the process dependencies contain a cycle at `{name}`"
        )));
    }
    if let Some(spec) = manifest.processes.get(name) {
        for dep in &spec.depends_on {
            visit(dep, manifest, temporary, permanent, out)?;
        }
    }
    temporary.remove(name);
    permanent.insert(name.to_string());
    out.push(name.to_string());
    Ok(())
}

fn detect_cycles(processes: &BTreeMap<String, ProcessSpec>) -> Result<()> {
    fn walk(
        name: &str,
        processes: &BTreeMap<String, ProcessSpec>,
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.to_string()) {
            return Err(DevdError::invalid(format!(
                "the process dependencies contain a cycle at `{name}`"
            )));
        }
        if let Some(spec) = processes.get(name) {
            for dep in &spec.depends_on {
                if processes.contains_key(dep) {
                    walk(dep, processes, visiting, done)?;
                }
            }
        }
        visiting.remove(name);
        done.insert(name.to_string());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for name in processes.keys() {
        walk(name, processes, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}

fn validate_display_name(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(DevdError::invalid(
            "project.name must be between 1 and 80 characters",
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(DevdError::invalid(
            "project.name may not contain control characters",
        ));
    }
    Ok(())
}

fn validate_env(env: &BTreeMap<String, String>, where_: &str) -> Result<()> {
    if env.len() > 64 {
        return Err(DevdError::invalid(format!(
            "{where_} declares more than 64 variables"
        )));
    }
    for (key, value) in env {
        let mut chars = key.chars();
        let Some(first) = chars.next() else {
            return Err(DevdError::invalid(format!("{where_} has an empty key")));
        };
        if !(first.is_ascii_alphabetic() || first == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(DevdError::invalid(format!(
                "{where_} key `{key}` is not a valid environment variable name"
            )));
        }
        if key.to_ascii_uppercase().starts_with("DEVD_") {
            return Err(DevdError::invalid(format!(
                "{where_} may not set the reserved key `{key}`"
            )));
        }
        if value.contains('\0') {
            return Err(DevdError::invalid(format!(
                "{where_} value for `{key}` contains a NUL byte"
            )));
        }
        if value.len() > 8192 {
            return Err(DevdError::invalid(format!(
                "{where_} value for `{key}` is longer than 8192 bytes"
            )));
        }
    }
    Ok(())
}

fn resolve_working_dir(root: &Path, process: &str, rel: Option<&str>) -> Result<PathBuf> {
    let rel = rel.unwrap_or(".");
    let candidate = Path::new(rel);
    if candidate.is_absolute() {
        return Err(DevdError::invalid(format!(
            "process `{process}` working_dir must be relative to the project"
        )));
    }
    for component in candidate.components() {
        match component {
            Component::ParentDir => {
                return Err(DevdError::invalid(format!(
                    "process `{process}` working_dir may not contain `..`"
                )))
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(DevdError::invalid(format!(
                    "process `{process}` working_dir must stay inside the project"
                )))
            }
            _ => {}
        }
    }
    let joined = root.join(candidate);
    let canonical = joined.canonicalize().map_err(|e| {
        DevdError::invalid(format!(
            "process `{process}` working_dir `{rel}` does not exist: {e}"
        ))
    })?;
    let canonical = util::strip_verbatim(&canonical);
    if !canonical.starts_with(root) {
        return Err(DevdError::invalid(format!(
            "process `{process}` working_dir escapes the project directory"
        )));
    }
    if !canonical.is_dir() {
        return Err(DevdError::invalid(format!(
            "process `{process}` working_dir is not a directory"
        )));
    }
    Ok(canonical)
}

fn validate_health(process: &str, h: &HealthFile) -> Result<Health> {
    if h.url.is_some() == h.port.is_some() {
        return Err(DevdError::invalid(format!(
            "process `{process}` health must declare exactly one of `url` or `port`"
        )));
    }
    if let Some(url) = &h.url {
        let parsed = url::Url::parse(url).map_err(|e| {
            DevdError::invalid(format!("process `{process}` health url is not valid: {e}"))
        })?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(DevdError::invalid(format!(
                "process `{process}` health url must be http or https"
            )));
        }
        let host = parsed.host_str().unwrap_or_default();
        let loopback = host == "localhost"
            || host == "::1"
            || host
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false);
        if !loopback {
            return Err(DevdError::invalid(format!(
                "process `{process}` health url must address loopback"
            )));
        }
    }
    if let Some(port) = h.port {
        if port == 0 {
            return Err(DevdError::invalid(format!(
                "process `{process}` health port must not be zero"
            )));
        }
    }
    let timeout_secs = h.timeout_secs.unwrap_or(30);
    if !(1..=600).contains(&timeout_secs) {
        return Err(DevdError::invalid(format!(
            "process `{process}` health.timeout_secs must be between 1 and 600"
        )));
    }
    let interval_ms = h.interval_ms.unwrap_or(500).clamp(50, 30_000);
    Ok(Health {
        url: h.url.clone(),
        port: h.port,
        timeout_secs,
        interval_ms,
    })
}

/// The small, explicit environment every project process receives.
///
/// Deliberately not `std::env::vars()`: the devd process may hold provider keys,
/// tokens and other credentials that a project manifest did not ask for and
/// should not see. These are the variables a normal shell needs to find tools
/// and temp directories.
fn baseline_env() -> BTreeMap<String, String> {
    const KEYS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "NUMBER_OF_PROCESSORS",
        "LANG",
        "LC_ALL",
        "TERM",
        "SHELL",
    ];
    let mut out = BTreeMap::new();
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            out.insert((*key).to_string(), value);
        }
    }
    out
}

/// Whether a key looks like a credential, for log redaction and UI masking.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "private",
        "credential",
        "passphrase",
        "auth",
    ]
    .iter()
    .any(|m| k.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    const GOOD: &str = r#"
version: 1
project:
  name: example-app
  env:
    APP_ENV: development
processes:
  web:
    command: ["node", "-e", "console.log('web')"]
    working_dir: "."
    health:
      url: "http://127.0.0.1:3000/"
  worker:
    command: ["node", "-e", "console.log('worker')"]
    depends_on: ["web"]
"#;

    #[test]
    fn a_valid_manifest_resolves_relative_to_the_project() {
        let dir = temp_root();
        let manifest = parse(dir.path(), GOOD).unwrap();
        assert_eq!(manifest.project, "example-app");
        assert_eq!(manifest.processes.len(), 2);
        let worker = &manifest.processes["worker"];
        assert_eq!(worker.depends_on, vec!["web".to_string()]);
        assert_eq!(
            worker.env.get("APP_ENV").map(String::as_str),
            Some("development")
        );
        assert!(Path::new(&worker.working_dir).is_dir());
        assert_eq!(start_order(&manifest).unwrap(), vec!["web", "worker"]);
    }

    #[test]
    fn a_string_command_is_refused() {
        let dir = temp_root();
        let text = GOOD.replace(
            r#"["node", "-e", "console.log('web')"]"#,
            r#""node -e console.log('web')""#,
        );
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(err.to_string().contains("invalid"), "{err}");
    }

    #[test]
    fn traversal_out_of_the_project_is_refused() {
        let dir = temp_root();
        let text = GOOD.replace(r#"working_dir: ".""#, r#"working_dir: "../outside""#);
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(err.to_string().contains(".."), "{err}");
    }

    #[test]
    fn absolute_working_dirs_are_refused() {
        let dir = temp_root();
        let text = GOOD.replace(r#"working_dir: ".""#, "working_dir: \"/tmp\"");
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(
            err.to_string().contains("relative") || err.to_string().contains("inside"),
            "{err}"
        );
    }

    #[test]
    fn dependency_cycles_are_refused() {
        let dir = temp_root();
        let text = GOOD
            .replace("depends_on: [\"web\"]", "depends_on: [\"web\"]")
            .replace("  web:\n", "  web:\n    depends_on: [\"worker\"]\n")
            .replace("    health:\n      url: \"http://127.0.0.1:3000/\"\n", "");
        // `web` now depends on `worker`, and `worker` on `web`.
        let text = text.replacen("working_dir: \".\"\n", "working_dir: \".\"\n", 1);
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");
    }

    #[test]
    fn remote_health_urls_are_refused() {
        let dir = temp_root();
        let text = GOOD.replace("http://127.0.0.1:3000/", "https://example.com/");
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(err.to_string().contains("loopback"), "{err}");
    }

    #[test]
    fn reserved_environment_keys_are_refused() {
        let dir = temp_root();
        let text = GOOD.replace("APP_ENV: development", "DEVD_HOME: /tmp/evil");
        let err = parse(dir.path(), &text).unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
    }

    #[test]
    fn the_hash_is_semantic_not_bytewise() {
        let dir = temp_root();
        let a = parse(dir.path(), GOOD).unwrap();
        let with_comment = format!("# a comment\n{GOOD}");
        let b = parse(dir.path(), &with_comment).unwrap();
        assert_eq!(a.hash, b.hash);
        let changed = GOOD.replace("development", "production");
        let c = parse(dir.path(), &changed).unwrap();
        assert_ne!(a.hash, c.hash);
    }

    #[test]
    fn unknown_fields_are_refused() {
        let dir = temp_root();
        let text = GOOD.replace("version: 1", "version: 1\nunexpected: true");
        assert!(parse(dir.path(), &text).is_err());
    }
}
