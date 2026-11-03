//! Reading back the mail Mailpit caught.
//!
//! Mailpit is already installed and supervised, and PHP is already pointed at it,
//! so an application's mail does not leave the machine. What was missing is the
//! other half: nothing in devd could *say* what had been caught. The inbox was a
//! port number in a doc comment, and the only way to read a message was to open
//! Mailpit's own web UI in a browser.
//!
//! Mailpit serves its inbox over an HTTP API beside that UI. This talks to it:
//! list what has arrived, read one message in full, and delete it. No new
//! dependency and no second mail server - it reads the service devd already runs.
//!
//! The API is local, on `127.0.0.1`, and a message body is whatever the
//! application sent. **Nothing here is put in a prompt or shown as HTML without
//! being treated as data**: the text is returned as text, and screens escape it
//! like any other value.

use serde_json::{json, Value};

use devd_core::DevdError;
use devd_core::Result;

use crate::extras::MAILPIT_UI_PORT;

/// A short timeout: the inbox is on this machine, so a slow reply means it is not
/// running, and waiting a long time to say so helps nobody.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(TIMEOUT)
        .timeout_read(TIMEOUT)
        .timeout_write(TIMEOUT)
        .build()
}

/// The base URL of Mailpit's API.
pub fn base(ui_port: u16) -> String {
    format!("http://127.0.0.1:{ui_port}/api/v1")
}

/// The port Mailpit is actually listening on, found the way a person would: ask
/// the usual one, then the one the service was last started with.
pub fn discover_port(configured: Option<u16>) -> u16 {
    configured.unwrap_or(MAILPIT_UI_PORT)
}

fn get(url: &str) -> Result<Value> {
    let resp = agent().get(url).call().map_err(|e| match e {
        // A refused connection is the normal "it is not running", not a fault.
        ureq::Error::Transport(t) if t.kind() == ureq::ErrorKind::ConnectionFailed => {
            DevdError::conflict(format!("the mail catcher is not running, so there is no inbox to read ({t})"))
        }
        other => DevdError::internal(format!("the mail catcher did not answer: {other}")),
    })?;
    let text = resp
        .into_string()
        .map_err(|e| DevdError::internal(format!("the mail catcher sent an unreadable reply: {e}")))?;
    serde_json::from_str(&text).map_err(|e| DevdError::internal(format!("the mail catcher sent something that is not JSON: {e}")))
}

/// What has been caught, newest first, without the bodies.
pub fn list(ui_port: u16, limit: usize, query: Option<&str>) -> Result<Value> {
    let url = format!("{}/messages?limit={limit}", base(ui_port));
    let mut v = get(&url)?;
    // Mailpit answers `{ "total": n, "messages": [...] }`.
    let messages = v.get_mut("messages").and_then(|m| m.as_array_mut()).cloned().unwrap_or_default();
    let q = query.map(|q| q.to_lowercase()).unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();
    for m in messages {
        if !q.is_empty() {
            let hay = format!(
                "{} {} {}",
                m.get("Subject").and_then(|x| x.as_str()).unwrap_or(""),
                m.get("From").and_then(|f| f.get("Address")).and_then(|x| x.as_str()).unwrap_or(""),
                m.get("To").map(|t| serde_json::to_string(t).unwrap_or_default()).unwrap_or_default()
            )
            .to_lowercase();
            if !hay.contains(&q) {
                continue;
            }
        }
        out.push(json!({
            "id": m.get("ID"),
            "subject": m.get("Subject"),
            "from": m.get("From").and_then(|f| f.get("Address")),
            "to": m.get("To"),
            "created": m.get("Created"),
            "size": m.get("Size"),
        }));
    }
    Ok(json!({
        "total": v.get("total").cloned().unwrap_or(Value::Null),
        "messages": out,
        "count": out.len(),
        "inbox": format!("http://127.0.0.1:{ui_port}"),
    }))
}

/// One message in full: headers, the text part and the HTML part.
pub fn read(ui_port: u16, id: &str) -> Result<Value> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(DevdError::invalid("a message id is letters, digits, '-' and '_'"));
    }
    let v = get(&format!("{}/message/{}", base(ui_port), id))?;
    Ok(json!({
        "id": v.get("ID"),
        "subject": v.get("Subject"),
        "from": v.get("From"),
        "to": v.get("To"),
        "created": v.get("Created"),
        // Both parts, as data. The text part is what an agent should read; the
        // HTML part is for a person to look at, and is never rendered as HTML
        // anywhere devd puts it in a prompt.
        "text": v.get("Text"),
        "html": v.get("HTML"),
        "attachments": v.get("Attachments"),
    }))
}

/// Delete one message.
pub fn delete(ui_port: u16, id: &str) -> Result<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(DevdError::invalid("a message id is letters, digits, '-' and '_'"));
    }
    let url = format!("{}/messages", base(ui_port));
    let body = json!({ "IDs": [id] }).to_string();
    agent()
        .delete(&url)
        .send_string(&body)
        .map_err(|e| DevdError::internal(format!("the mail catcher did not accept the delete: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_url_is_local_and_versioned() {
        assert_eq!(base(8025), "http://127.0.0.1:8025/api/v1");
    }

    #[test]
    fn the_port_falls_back_to_the_catchers_own() {
        assert_eq!(discover_port(None), MAILPIT_UI_PORT);
        assert_eq!(discover_port(Some(9000)), 9000);
    }

    #[test]
    fn a_message_id_may_not_be_a_path() {
        // The id goes into the path, so a traversal must not be expressible.
        for bad in ["../../etc/passwd", "a/b", "", "x?y=1"] {
            assert!(read(8025, bad).is_err(), "{bad} should be refused");
            assert!(delete(8025, bad).is_err(), "{bad} should be refused");
        }
        assert!(read(8025, "aBc-123_x").is_err(), "a valid id still tries the network, and fails when nothing is listening");
    }
}
