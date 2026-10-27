//! `GET /api/events` - what is happening, as it happens, and what happened
//! while the window was away.
//!
//! The live bus is convenient but it is not a record: a broadcast channel drops
//! the oldest messages when a slow reader falls behind, and a window that closes
//! loses everything it did not see. The authoritative stream is therefore the
//! durable `run_events` table, which has stable ids and a sequence per run. On
//! connect the stream replays from `Last-Event-ID` (or `?after=`), then tails
//! that table while also forwarding the one thing that is not durable by design:
//! the text of a reply as it is being written.
//!
//! This is the one `/api/*` route that does not require the bearer token: an
//! `EventSource` cannot send an `Authorization` header. It is still fenced by
//! the loopback `Host` and `Origin` checks every other route uses, so a page on
//! another site cannot read this conversation through the person's browser.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

/// Non-durable kinds forwarded from the live bus. Durable run events are read
/// from the database instead, so nothing here can be silently lost.
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

/// Optional filters and cursor for the event stream.
#[derive(Debug, Default, Deserialize)]
pub struct EventQuery {
    /// Global run-event id to start after. `Last-Event-ID` wins when both are set.
    #[serde(default)]
    pub after: Option<i64>,
    /// Only events for this run.
    #[serde(default)]
    pub run: Option<String>,
}

/// `GET /api/events` - a server-sent event stream of agent activity.
pub async fn events(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EventQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(reason) = same_origin(&headers, state.port) {
        return (StatusCode::FORBIDDEN, reason).into_response();
    }
    // The cursor is the durable event id, not a byte offset. A reconnecting
    // EventSource sends it automatically; a manual client can use `?after=`.
    let cursor = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .or(q.after)
        .unwrap_or(0)
        .max(0);
    let thread = headers
        .get("x-devd-thread")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let run = match q.run.clone() {
        Some(id) => Some(id),
        None => thread
            .as_deref()
            .and_then(|t| state.ctx.store.run_for_thread(t).ok().flatten())
            .map(|r| r.id),
    };

    let store = state.ctx.store.clone();
    let rx = state.ctx.events.subscribe();
    let stream = stream::unfold((cursor, rx), move |(mut cursor, mut rx)| {
        let store = store.clone();
        let run = run.clone();
        let thread = thread.clone();
        async move {
            // 1. Durable history first, from the cursor. This is what makes a
            //    reconnect able to reconstruct the timeline exactly.
            loop {
                let rows = store.events_after(cursor, 200).unwrap_or_default();
                if rows.is_empty() {
                    break;
                }
                let mut picked = None;
                for e in rows {
                    cursor = cursor.max(e.id);
                    if run.as_deref().map(|id| id != e.run_id).unwrap_or(false) {
                        continue;
                    }
                    picked = Some(e);
                    break;
                }
                if let Some(e) = picked {
                    let payload = serde_json::json!({
                        "kind": e.kind,
                        "at": e.created_at,
                        "message": e.message,
                        "data": e.data,
                        "run": e.run_id,
                        "seq": e.seq,
                        "step": e.step_id,
                    });
                    let frame = Event::default()
                        .id(e.id.to_string())
                        .event("run")
                        .data(payload.to_string());
                    return Some((Ok::<Event, Infallible>(frame), (cursor, rx)));
                }
                // Every row in this page belonged to another run; keep going
                // without spinning forever on the same cursor.
                if cursor == 0 {
                    break;
                }
            }

            // 2. Non-durable live activity. A lag is not swallowed: the next
            //    iteration replays the durable rows the client missed, and the
            //    client is told that the live text may have a gap.
            match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
                Ok(Ok(event)) => {
                    if !allowed(&event.kind) {
                        return Some((
                            Ok::<Event, Infallible>(Event::default().comment("skip")),
                            (cursor, rx),
                        ));
                    }
                    if let Some(want) = &thread {
                        if event.data["thread"]
                            .as_str()
                            .map(|t| t != want)
                            .unwrap_or(false)
                        {
                            return Some((
                                Ok::<Event, Infallible>(Event::default().comment("skip")),
                                (cursor, rx),
                            ));
                        }
                    }
                    let payload = serde_json::json!({ "kind": event.kind, "at": event.at, "data": event.data });
                    let frame = Event::default().event("live").data(payload.to_string());
                    Some((Ok::<Event, Infallible>(frame), (cursor, rx)))
                }
                Ok(Err(RecvError::Lagged(missed))) => {
                    let payload = serde_json::json!({
                        "kind": "stream.lag",
                        "at": devd_core::util::now(),
                        "data": { "missed": missed, "resume_after": cursor },
                    });
                    let frame = Event::default().event("live").data(payload.to_string());
                    Some((Ok::<Event, Infallible>(frame), (cursor, rx)))
                }
                Ok(Err(RecvError::Closed)) => None,
                // A quiet moment is how the durable poll gets a turn even when
                // no live event arrives.
                Err(_) => Some((
                    Ok::<Event, Infallible>(Event::default().comment("poll")),
                    (cursor, rx),
                )),
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

/// Refuse a request that did not address this server by a loopback name, or
/// that a browser marks as coming from another site.
///
/// The middleware applies both rules to every route. They are repeated here
/// because this is the one route without a bearer token, so a future change to
/// the router must not be able to leave it reachable from a foreign page.
fn same_origin(headers: &HeaderMap, port: u16) -> Result<(), &'static str> {
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !crate::host_is_local(host, port) {
        return Err("the event stream is only for the devd window");
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if !crate::origin_is_local(origin, port) {
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
        // No Host header at all is not an addressed request.
        assert!(same_origin(&h, 8787).is_err());
        h.insert("host", "127.0.0.1:8787".parse().unwrap());
        assert!(same_origin(&h, 8787).is_ok());
        h.insert("origin", "https://evil.example".parse().unwrap());
        assert!(same_origin(&h, 8787).is_err());
        h.insert("origin", "http://127.0.0.1:26688".parse().unwrap());
        assert!(same_origin(&h, 8787).is_err());
        h.insert("origin", "http://127.0.0.1:8787".parse().unwrap());
        assert!(same_origin(&h, 8787).is_ok());
        // A prefix that only looks like loopback is not loopback.
        h.insert("origin", "http://127.0.0.1.evil.example".parse().unwrap());
        assert!(same_origin(&h, 8787).is_err());
        h.insert("origin", "http://127.0.0.1:8787".parse().unwrap());
        h.insert("host", "evil.example:8787".parse().unwrap());
        assert!(same_origin(&h, 8787).is_err());
    }
}
