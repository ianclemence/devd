//! Typed judgments from Jev, TypeSafe's System One model.
//!
//! A chat model is the wrong tool for a question devd's own code has to branch
//! on: "does this evidence show the criterion is met?" Jev takes a state and typed
//! questions and returns a calibrated probability, not prose to parse. devd uses
//! it where the harness, not an agent, needs a verdict: checking a run's
//! definition of done, and ranking what is in the inbox.
//!
//! It is optional. Without a key nothing here runs and the callers say so; they
//! never pretend a criterion was checked. The key is stored the way provider keys
//! are (an owner-only file, or `TYPESAFE_API_KEY`), and what is sent is only the
//! state the caller builds: devd's evidence summary, never source files.

use std::time::Duration;

use devd_core::{DevdError, Paths, Result};
use serde_json::{json, Value};

use crate::secrets;

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// Where Jev is, unless this names another address (a test double or a gateway).
const ENDPOINT_ENV: &str = "DEVD_JUDGE_URL";
const MODEL: &str = "jev-latest";
const KEY_ID: &str = "jev";
const KEY_ENV: &str = "TYPESAFE_API_KEY";

/// A configured Jev client.
pub struct Judge {
    key: String,
}

/// Whether a key is available and from where, without reading it out.
pub fn source(paths: &Paths) -> &'static str {
    if std::env::var(KEY_ENV).map(|v| !v.trim().is_empty()).unwrap_or(false) {
        "env"
    } else if key_file(paths).is_file() {
        "file"
    } else {
        "none"
    }
}

fn key_file(paths: &Paths) -> std::path::PathBuf {
    paths.root().join("secrets").join("providers").join(format!("{KEY_ID}.key"))
}

/// Store the key privately.
pub fn save_key(paths: &Paths, key: &str) -> Result<()> {
    secrets::save_key(paths, KEY_ID, key)
}

/// Forget the stored key.
pub fn delete_key(paths: &Paths) {
    secrets::delete_key(paths, KEY_ID);
}

impl Judge {
    /// The client, when a key exists. The environment wins over the file.
    pub fn configured(paths: &Paths) -> Option<Judge> {
        let from_env = std::env::var(KEY_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let key = from_env.or_else(|| std::fs::read_to_string(key_file(paths)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))?;
        Some(Judge { key })
    }

    /// Ask typed questions about a state. `questions` is the API's own object.
    pub fn ask(&self, state: &str, questions: Value) -> Result<Value> {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(60)).build();
        let body = json!({ "state": state, "model": MODEL, "questions": questions });
        let endpoint = std::env::var(ENDPOINT_ENV).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| ENDPOINT.to_string());
        let response = agent
            .post(&endpoint)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string());
        match response {
            Ok(r) => {
                let text = r.into_string().map_err(|e| DevdError::internal(format!("could not read Jev's answer: {e}")))?;
                serde_json::from_str::<Value>(&text).map_err(|e| DevdError::internal(format!("Jev answered with something that is not JSON: {e}")))
            }
            Err(ureq::Error::Status(code, r)) => {
                let detail = r.into_string().unwrap_or_default();
                let what = match code {
                    401 | 403 => "Jev rejected the key".to_string(),
                    429 => "Jev is rate limiting this key; try again shortly".to_string(),
                    _ => format!("Jev answered {code}"),
                };
                Err(DevdError::internal(format!("{what}: {}", devd_core::util::truncate(detail.trim(), 300))))
            }
            Err(e) => Err(DevdError::internal(format!("could not reach Jev: {e}"))),
        }
    }

    /// For each statement, how likely it is true given the state (0 to 1).
    pub fn truths(&self, state: &str, statements: &[String]) -> Result<Vec<f64>> {
        let mut questions = serde_json::Map::new();
        for (i, s) in statements.iter().enumerate() {
            questions.insert(format!("q{i}"), json!({ "type": "noul", "instructions": s }));
        }
        let answer = self.ask(state, Value::Object(questions))?;
        statements
            .iter()
            .enumerate()
            .map(|(i, s)| {
                answer["answers"][format!("q{i}")]["noul"]
                    .as_f64()
                    .ok_or_else(|| DevdError::internal(format!("Jev gave no answer for: {s}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_key_nothing_is_configured() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        if std::env::var(KEY_ENV).is_err() {
            assert!(Judge::configured(&paths).is_none());
            assert_eq!(source(&paths), "none");
        }
    }

    #[test]
    fn a_stored_key_is_found_and_can_be_removed() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        save_key(&paths, "apikey_0123456789abcdef").unwrap();
        if std::env::var(KEY_ENV).is_err() {
            assert_eq!(source(&paths), "file");
            assert!(Judge::configured(&paths).is_some());
        }
        delete_key(&paths);
        if std::env::var(KEY_ENV).is_err() {
            assert!(Judge::configured(&paths).is_none());
        }
    }
}
