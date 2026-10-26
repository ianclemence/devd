//! `GET /api/events` - what is happening, as it happens.
//!
//! The window used to learn about a working agent by polling `chat.get` on a
//! timer, so a reply appeared in a burst once the turn ended. This is a
//! server-sent event stream from the same bus every capability already
//! publishes to: a step starting, an approval waiting, a reply being written.
//!
//! This is the one `/api/*` route that does not require the bearer token: an
//! `EventSource` cannot send an `Authorization` header. It is still fenced by
//! the loopback `Host` and `Origin` checks every other route uses, so a page on
//! another site cannot read this conversation through the person's browser.
//!
//! Every frame carries only facts the app already shows; a streamed reply is
//! the same text `chat.get` later returns in full.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

/// The event kinds the window subscribes to.
///
/// Deliberately not every kind on the bus: `stack.output` carries a process's
/// raw output, and forwarding it would both flood the connection and wake the
/// conversation on every line. The screens that show stack progress already
/// poll it while they are open.
const KINDS: &[&str] = &[
    "chat.delta",
    "chat.event",
    "approval.*",
    "inbox.*",
    "team.*",
];

fn allowed(kind: &str) -> bool {
    KINDS.iter().any(|k| match k.strip_suffix('*') {
        Some(prefix) => kind.starts_with(prefix),
        None => *k == kind,
    })
}

/// `GET /api/events` - a server-sent event stream of agent activity.
pub async fn events(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(reason) = same_origin(&headers) {
        return (StatusCode::FORBIDDEN, reason).into_response();
    }
    let rx = state.ctx.events.subscribe();
    let thread = headers
        .get("x-devd-thread")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let stream = stream::unfold(rx, move |mut rx| {
        let thread = thread.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if !allowed(&event.kind) {
                            continue;
                        }
                        if let Some(want) = &thread {
                            if event.data["thread"]
                                .as_str()
                                .map(|t| t != want)
                                .unwrap_or(false)
                            {
                                continue;
                            }
                        }
                        let payload = serde_json::json!({ "kind": event.kind, "at": event.at, "data": event.data });
                        let frame = Event::default().event("devd").data(payload.to_string());
                        return Some((Ok::<Event, Infallible>(frame), rx));
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        }
    });
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("devd"),
        )
        .into_response()
}

/// Refuse a request the browser marks as coming from another site.
fn same_origin(headers: &HeaderMap) -> Result<(), &'static str> {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if !origin.starts_with("http://127.0.0.1") && !origin.starts_with("http://localhost") {
            return Err("the event stream is only for the devd window");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_kinds_the_window_shows_are_forwarded() {
        assert!(allowed("chat.delta"));
        assert!(allowed("chat.event"));
        assert!(allowed("approval.pending"));
        assert!(allowed("team.run"));
        assert!(!allowed("stack.output"));
        assert!(!allowed("service.nginx.installed"));
    }

    #[test]
    fn another_site_cannot_subscribe() {
        let mut h = HeaderMap::new();
        assert!(same_origin(&h).is_ok());
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(same_origin(&h).is_err());
        h.insert("origin", "http://127.0.0.1:26688".parse().unwrap());
        assert!(same_origin(&h).is_ok());
    }
}
