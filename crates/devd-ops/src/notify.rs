//! Where to tell people when something finishes or fails.
//!
//! A target is the inbox (always available), or a Slack, Discord or generic webhook URL. Each
//! names the events it wants: `deploy.finished`, `deploy.failed`, `cron.failed`. Sending never
//! fails the thing that is being reported: a webhook that is down is noted in the log and the
//! deploy still counts as done.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::{util, DevdError, Result};

/// The kind under which targets are stored.
pub const KIND: &str = "notify";

/// Events a target can ask for.
pub const EVENTS: &[&str] = &["deploy.finished", "deploy.failed", "cron.failed", "backup.failed", "health.down", "health.up"];

/// The channels a target can use.
pub const CHANNELS: &[&str] = &["inbox", "slack", "discord", "webhook"];

/// A target, as the capabilities show it. The URL is shown with its secret path hidden.
#[derive(Debug, Clone, Serialize)]
pub struct Target {
    /// Id.
    pub id: String,
    /// `inbox`, `slack`, `discord` or `webhook`.
    pub channel: String,
    /// Where it posts, shortened to the host.
    pub destination: String,
    /// The events it wants.
    pub events: Vec<String>,
    /// Only for this site, when set.
    pub site: Option<String>,
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").to_string()
}

fn target_of(item: &devd_core::store_ops::OpsItem) -> Target {
    let channel = item.data["channel"].as_str().unwrap_or("").to_string();
    Target {
        id: item.id.clone(),
        destination: if channel == "inbox" { "the devd inbox".to_string() } else { host_of(item.data["url"].as_str().unwrap_or("")) },
        channel,
        events: item.data["events"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        site: item.data["site"].as_str().map(str::to_string),
    }
}

/// The targets.
pub fn list(ctx: &Ctx) -> Result<Vec<Target>> {
    Ok(ctx.store.ops_list(KIND, None)?.iter().map(target_of).collect())
}

/// Validate a webhook URL: https, a host, no credentials, and not a loopback or private address
/// unless it is plainly a local development hook.
pub fn valid_url(url: &str) -> Result<()> {
    let rest = url.strip_prefix("https://").ok_or_else(|| DevdError::invalid("a webhook URL must start with https://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || host.contains('@') || url.len() > 600 || url.contains(char::is_whitespace) {
        return Err(DevdError::invalid("that is not a usable webhook URL (an https address with a host, no credentials, no spaces)"));
    }
    let h = host.split(':').next().unwrap_or("").to_ascii_lowercase();
    if h == "localhost" || h.starts_with("127.") || h.starts_with("10.") || h.starts_with("192.168.") || h.starts_with("169.254.") || h == "[::1]" {
        return Err(DevdError::invalid("a webhook must point at a public host, not this machine or a private network"));
    }
    Ok(())
}

/// Add a target.
pub fn add(ctx: &Ctx, channel: &str, url: Option<&str>, events: &[String], site: Option<&str>) -> Result<Target> {
    if !CHANNELS.contains(&channel) {
        return Err(DevdError::invalid(format!("`{channel}` is not a channel; use {}", CHANNELS.join(", "))));
    }
    if channel != "inbox" {
        valid_url(url.ok_or_else(|| DevdError::invalid(format!("{channel} needs a url")))?)?;
    }
    for e in events {
        if !EVENTS.contains(&e.as_str()) {
            return Err(DevdError::invalid(format!("`{e}` is not an event; use {}", EVENTS.join(", "))));
        }
    }
    if let Some(s) = site {
        crate::site_named(ctx, s)?;
    }
    let events: Vec<String> = if events.is_empty() { EVENTS.iter().map(|e| e.to_string()).collect() } else { events.to_vec() };
    let id = util::new_id()[..8].to_string();
    ctx.store.ops_put(KIND, &id, site.unwrap_or(""), &json!({ "channel": channel, "url": url, "events": events, "site": site }))?;
    Ok(target_of(&ctx.store.ops_get(KIND, &id)?.ok_or_else(|| DevdError::internal("the target was not saved"))?))
}

/// Remove a target.
pub fn remove(ctx: &Ctx, id: &str) -> Result<()> {
    if !ctx.store.ops_delete(KIND, id)? {
        return Err(DevdError::not_found(format!("no notification target `{id}`")));
    }
    Ok(())
}

fn post(url: &str, body: &Value) -> std::result::Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout(Duration::from_secs(15)).build();
    agent
        .post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map(|_| ())
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("answered {code}"),
            other => other.to_string(),
        })
}

/// Deliver a message to one target. Returns what happened, for `notify.test`.
pub fn deliver(ctx: &Ctx, item: &devd_core::store_ops::OpsItem, event: &str, site: &str, message: &str, data: &Value) -> std::result::Result<(), String> {
    let channel = item.data["channel"].as_str().unwrap_or("");
    let url = item.data["url"].as_str().unwrap_or("");
    match channel {
        "inbox" => ctx
            .store
            .post_inbox("devd", None, &format!("{event}: {site}"), message, false)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "slack" => post(url, &json!({ "text": message })),
        "discord" => post(url, &json!({ "content": message })),
        "webhook" => post(url, &json!({ "event": event, "site": site, "message": message, "data": data, "at": util::now() })),
        other => Err(format!("unknown channel {other}")),
    }
}

/// Tell every target that wants this event. Never fails; a delivery that does not work is logged.
pub fn send(ctx: &Ctx, event: &str, site: &str, message: &str, data: Value) {
    let Ok(items) = ctx.store.ops_list(KIND, None) else { return };
    for item in items {
        let wants = item.data["events"].as_array().is_some_and(|a| a.iter().any(|e| e.as_str() == Some(event)));
        let scoped = item.data["site"].as_str().is_none_or(|s| s == site);
        if !wants || !scoped {
            continue;
        }
        if let Err(e) = deliver(ctx, &item, event, site, message, &data) {
            let line = format!("[{}] notify {} ({}) failed: {e}\n", util::now(), item.id, item.data["channel"].as_str().unwrap_or(""));
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(crate::log_dir(ctx).join("notify.log"))
                .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        }
    }
}

/// Send a test message to one target and report the result.
pub fn test(ctx: &Ctx, id: &str) -> Result<()> {
    let item = ctx.store.ops_get(KIND, id)?.ok_or_else(|| DevdError::not_found(format!("no notification target `{id}`")))?;
    deliver(ctx, &item, "test", "devd", "devd test message: this target works.", &json!({})).map_err(DevdError::unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_https_addresses_are_accepted() {
        assert!(valid_url("https://hooks.slack.com/services/T/B/x").is_ok());
        for bad in ["http://hooks.slack.com/x", "https://localhost/x", "https://127.0.0.1:8080/x", "https://192.168.1.5/x", "https://user:pw@example.com/x", "https:///x", "https://a b.com/"] {
            assert!(valid_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_destination_never_shows_the_secret_part_of_the_url() {
        assert_eq!(host_of("https://hooks.slack.com/services/T000/B000/SECRET"), "hooks.slack.com");
    }
}
