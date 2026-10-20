//! Mapping from the `devd runtime ...` command surface to capabilities.
//!
//! Nothing here performs work. Each arm shapes typed arguments and dispatches a
//! capability, so installing PHP from the command line goes through the same
//! permission, approval and audit path as installing it from the application or
//! asking an agent to install it over MCP.

use serde_json::json;

use devd_core::{DevdError, Result};

use crate::app::App;
use crate::RuntimeCommand;

/// Split `php@8.3`, `node@lts` or a bare `php` into `(kind, spec)`.
pub fn split_spec(text: &str) -> Result<(String, String)> {
    let (kind, version) = match text.split_once('@') {
        Some((kind, version)) => (kind, version),
        None => (text, "latest"),
    };
    let kind = kind.trim().to_lowercase();
    if kind.is_empty() {
        return Err(DevdError::invalid(
            "a runtime is required, e.g. `php@8.3`, `node@lts` or `go@latest`",
        ));
    }
    let version = version.trim();
    if version.is_empty() {
        return Err(DevdError::invalid(format!(
            "`{text}` has an empty version; write `{kind}@latest` or `{kind}@8.3`"
        )));
    }
    Ok((kind, version.to_string()))
}

/// Run a `devd runtime` subcommand.
pub fn run(app: &App, cmd: RuntimeCommand) -> Result<()> {
    match cmd {
        RuntimeCommand::List => app.invoke("runtime.list", json!({})),
        RuntimeCommand::Available { kind, thread_safe } => app.invoke(
            "runtime.available",
            json!({ "kind": kind.to_lowercase(), "thread_safe": thread_safe }),
        ),
        RuntimeCommand::Install {
            spec,
            thread_safe,
            force,
        } => {
            let (kind, version) = split_spec(&spec)?;
            app.invoke(
                "runtime.install",
                json!({
                    "kind": kind,
                    "version": version,
                    "thread_safe": thread_safe,
                    "force": force,
                }),
            )
        }
        RuntimeCommand::Verify { spec } => {
            let (kind, version) = split_spec(&spec)?;
            app.invoke(
                "runtime.verify",
                json!({ "kind": kind, "version": version }),
            )
        }
        RuntimeCommand::Remove { spec } => {
            let (kind, version) = split_spec(&spec)?;
            app.invoke(
                "runtime.remove",
                json!({ "kind": kind, "version": version }),
            )
        }
        RuntimeCommand::Use { version } => app.invoke("runtime.use", json!({ "version": version })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_are_split_with_a_sensible_default() {
        assert_eq!(split_spec("php@8.3").unwrap(), ("php".into(), "8.3".into()));
        assert_eq!(split_spec("PHP@8.3").unwrap(), ("php".into(), "8.3".into()));
        assert_eq!(split_spec("go").unwrap(), ("go".into(), "latest".into()));
        assert_eq!(
            split_spec("node@lts").unwrap(),
            ("node".into(), "lts".into())
        );
    }

    #[test]
    fn a_missing_version_is_refused_rather_than_guessed() {
        let err = split_spec("php@").unwrap_err();
        assert_eq!(err.code(), "invalid_input");
        assert!(err.to_string().contains("php@latest"), "{err}");
        assert_eq!(split_spec("@8.3").unwrap_err().code(), "invalid_input");
    }
}
