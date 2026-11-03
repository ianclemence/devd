//! The project manifest: what a project needs, written down once.
//!
//! devd already reads the ecosystem's own files to work out a project's runtimes
//! * `.nvmrc`, `.node-version`, `.python-version`, `.ruby-version`, `global.json`,
//!   `go.mod`, `Cargo.toml`, `pom.xml`, `composer.json`, `Gemfile` - and it should
//!   keep doing that, because those files are the truth for a Node or Go project
//!   and a person already has them.
//!
//! What those files cannot say is everything else: which *services* the project
//! needs (a Postgres, a queue, a mail catcher), which ports, what to run on start
//! and how to know it came up, and what an agent may do inside it. Today that is
//! rediscovered by hand, or guessed by the assistant every time.
//!
//! So this reads one file, `devd.toml`, and only for the things no standard file
//! describes. Runtimes stay where they are; if `devd.toml` names a runtime it is
//! a *pin* that wins over detection, and the reason it wins is written in the file.
//!
//! ```toml
//! # The parts of the environment no other file can describe.
//! [project]
//! name = "acme"
//!
//! [run]
//! # How to start the app, and how devd knows it is up.
//! start   = "npm run dev"
//! health  = "http://127.0.0.1:{port}/up"
//! port    = 3000
//!
//! [services]
//! postgres = "16"
//! redis    = "7"
//! mailpit  = true
//!
//! [env]
//! # Names and templates only. Never values that are secret: see `check`.
//! APP_ENV     = "local"
//! DATABASE_URL = "postgres://localhost:{{postgres.port}}/acme"
//!
//! [agents]
//! # What an agent may do here, in addition to its role's own scope.
//! allow = ["project.run", "fs.write"]
//! ```
//!
//! The rule that keeps this from becoming a second source of truth: **a value in
//! `devd.toml` is a declaration, and devd checks it against what it finds.** If
//! the file says Postgres 16 and an unsupported version is installed, the file is
//! not silently overruled and the version is not silently changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::Result;

/// The file's name. One name, no fallbacks: two ways to spell it is two ways to
/// get it wrong.
pub const MANIFEST: &str = "devd.toml";

/// A project's declaration of the environment it needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// What the project is called.
    #[serde(default)]
    pub project: ProjectSection,
    /// How to run it.
    #[serde(default)]
    pub run: RunSection,
    /// Services it needs, by name, with an optional version.
    #[serde(default)]
    pub services: BTreeMap<String, ServiceRequest>,
    /// Environment variables: names and templates, never secrets.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// What an agent may do inside this project.
    #[serde(default)]
    pub agents: AgentsSection,
    /// Runtimes, when the ecosystem file is not enough or is missing.
    #[serde(default)]
    pub runtime: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    /// A display name. Defaults to the folder name.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSection {
    /// The command that starts the application.
    #[serde(default)]
    pub start: Option<String>,
    /// A URL devd polls, with `{port}` substituted, to decide it is up.
    #[serde(default)]
    pub health: Option<String>,
    /// The port it listens on, when it is not given by the framework.
    #[serde(default)]
    pub port: Option<u16>,
    /// The command that stops it, when a signal is not enough.
    #[serde(default)]
    pub stop: Option<String>,
}

/// `postgres = "16"`, `redis = true`, or a table with a version and options.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServiceRequest {
    /// Just the version.
    Version(String),
    /// Just on or off.
    On(bool),
    /// Version and anything else the service needs.
    Detailed {
        #[serde(default)]
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsSection {
    /// Capability names an agent may use here that its role does not already grant.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Capability names an agent may never use here, even if its role would allow it.
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Where a manifest was found, and what it said.
#[derive(Debug)]
pub struct Found {
    /// The file.
    pub path: PathBuf,
    /// What it declared.
    pub manifest: Manifest,
}

/// Read a project's manifest.
///
/// `Ok(None)` means there is no manifest, which is the common and perfectly fine
/// case: a project with `.nvmrc` and a `package.json` needs nothing else. An
/// error means the file exists and is wrong, and saying so is the point - a
/// manifest that is silently ignored is worse than no manifest.
pub fn read(project: &Path) -> Result<Option<Found>> {
    let path = project.join(MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| devd_core::DevdError::invalid(format!("{} could not be read: {e}", path.display())))?;
    let manifest: Manifest = toml::from_str(&text).map_err(|e| {
        devd_core::DevdError::invalid(format!("{} is not valid: {e}", path.display()))
    })?;
    check(&path, &manifest)?;
    Ok(Some(Found { path, manifest }))
}

/// The rules a manifest cannot break.
///
/// Two of them, both about not turning a project file into a security problem:
/// a secret written into a file meant to be committed, and a start command that
/// would be run through a shell on someone's machine without them seeing it.
fn check(path: &Path, m: &Manifest) -> Result<()> {
    // 1. No secrets in a file that belongs in version control.
    for (name, value) in &m.env {
        let upper = name.to_uppercase();
        let looks_secret = ["PASSWORD", "SECRET", "TOKEN", "PRIVATE_KEY", "API_KEY", "CREDENTIAL", "ACCESS_KEY"]
            .iter()
            .any(|s| upper.contains(s));
        let has_value = !value.trim().is_empty() && !value.contains("{{") && value != "changeme" && value != "null";
        if looks_secret && has_value {
            return Err(devd_core::DevdError::invalid(format!(
                "{} sets {name} to a literal value. This file is committed, so it must never carry a secret: give the name and let the value come from the machine's own environment, or from an uncommitted .env.",
                path.display()
            )));
        }
    }
    // 2. A health check is a URL, so it cannot ask devd to run anything.
    if let Some(health) = &m.run.health {
        if !health.starts_with("http://") && !health.starts_with("https://") {
            return Err(devd_core::DevdError::invalid(format!(
                "{}: run.health must be an http(s) URL, not a command ({health})",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Everything devd can say about a project's declared environment.
///
/// Used by the assistant, the CLI and the screens alike, so they cannot disagree.
pub fn describe(project: &Path) -> Result<Value> {
    let found = read(project)?;
    let name = project.file_name().and_then(|s| s.to_str()).unwrap_or("project");
    let Some(f) = found else {
        return Ok(json!({
            "project": name,
            "path": project.display().to_string(),
            "manifest": null,
            "declared": false,
            "note": "No devd.toml. Runtimes come from the project's own files; services and ports are inferred.",
        }));
    };
    let services: Vec<Value> = f
        .manifest
        .services
        .iter()
        .map(|(name, req)| match req {
            ServiceRequest::Version(v) => json!({ "name": name, "version": v, "enabled": true }),
            ServiceRequest::On(on) => json!({ "name": name, "version": null, "enabled": *on }),
            ServiceRequest::Detailed { version } => json!({ "name": name, "version": version, "enabled": true }),
        })
        .collect();
    Ok(json!({
        "project": f.manifest.project.name.as_deref().unwrap_or(name),
        "path": project.display().to_string(),
        "manifest": f.path.display().to_string(),
        "declared": true,
        "run": {
            "start": f.manifest.run.start,
            "health": f.manifest.run.health,
            "port": f.manifest.run.port,
            "stop": f.manifest.run.stop,
        },
        "services": services,
        "env": f.manifest.env.keys().cloned().collect::<Vec<_>>(),
        "agents": { "allow": f.manifest.agents.allow, "deny": f.manifest.agents.deny },
        "runtime": f.manifest.runtime,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn project(dir: &Path, body: &str) {
        let mut f = std::fs::File::create(dir.join(MANIFEST)).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_project_without_a_manifest_is_not_an_error() {
        let d = tmp();
        assert!(read(d.path()).unwrap().is_none());
        let v = describe(d.path()).unwrap();
        assert_eq!(v["declared"], false);
        assert!(v["note"].as_str().unwrap().contains("No devd.toml"));
    }

    #[test]
    fn a_manifest_declares_services_a_port_and_a_start_command() {
        let d = tmp();
        project(
            d.path(),
            r#"
[project]
name = "acme"

[run]
start  = "npm run dev"
health = "http://127.0.0.1:{port}/up"
port   = 3000

[services]
postgres = "16"
redis    = "7"
mailpit  = true

[env]
APP_ENV = "local"
"#,
        );
        let f = read(d.path()).unwrap().unwrap();
        assert_eq!(f.manifest.project.name.as_deref(), Some("acme"));
        assert_eq!(f.manifest.run.port, Some(3000));
        assert_eq!(f.manifest.services.len(), 3);

        let v = describe(d.path()).unwrap();
        assert_eq!(v["declared"], true);
        let names: Vec<&str> = v["services"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"postgres") && names.contains(&"mailpit"));
        // The env lists names, never values.
        assert_eq!(v["env"], json!(["APP_ENV"]));
    }

    #[test]
    fn a_secret_written_into_a_committed_file_is_refused_by_name() {
        let d = tmp();
        project(d.path(), "[env]\nDB_PASSWORD = \"hunter2\"\n");
        let err = read(d.path()).unwrap_err().to_string();
        assert!(err.contains("DB_PASSWORD"), "{err}");
        assert!(err.contains("committed"), "{err}");
    }

    #[test]
    fn a_secret_given_as_a_template_is_allowed() {
        // The name is declared; the value comes from the machine.
        let d = tmp();
        project(d.path(), "[env]\nDATABASE_URL = \"postgres://localhost:{{postgres.port}}/acme\"\nAPI_KEY = \"\"\n");
        assert!(read(d.path()).unwrap().is_some(), "a template is the point of the file");
    }

    #[test]
    fn a_health_check_must_be_a_url_and_not_a_command() {
        let d = tmp();
        project(d.path(), "[run]\nhealth = \"curl -f localhost\"\n");
        let err = read(d.path()).unwrap_err().to_string();
        assert!(err.contains("http(s) URL"), "{err}");
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        // A typo that is silently dropped is a person believing their project is
        // configured when it is not.
        let d = tmp();
        project(d.path(), "[run]\nstrat = \"npm run dev\"\n");
        let err = read(d.path()).unwrap_err().to_string();
        assert!(err.contains("not valid"), "{err}");
    }

    #[test]
    fn the_agents_section_scopes_what_may_be_done_here() {
        let d = tmp();
        project(
            d.path(),
            "[agents]\nallow = [\"project.run\"]\ndeny = [\"service.remove\"]\n",
        );
        let v = describe(d.path()).unwrap();
        assert_eq!(v["agents"]["allow"], json!(["project.run"]));
        assert_eq!(v["agents"]["deny"], json!(["service.remove"]));
    }
}
