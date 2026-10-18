//! A process-wide event bus.
//!
//! Every mutation publishes an event. The TUI, the GUI, and the HTTP event
//! stream subscribe to the same bus, which is what lets all interfaces render
//! live state without polling the database.

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::util;

/// A single observable occurrence.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// RFC 3339 timestamp.
    pub at: String,
    /// Machine-readable kind, e.g. `service.started`, `site.created`.
    pub kind: String,
    /// Human-readable summary.
    pub message: String,
    /// Optional structured payload.
    pub data: Value,
}

/// Fan-out event bus. Cloning is cheap; all clones share one channel.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Create a bus with a 1024-event backlog.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Self { tx }
    }

    /// Publish an event with no payload.
    pub fn emit(&self, kind: impl Into<String>, message: impl Into<String>) {
        self.emit_with(kind, message, Value::Null);
    }

    /// Publish an event with a structured payload.
    ///
    /// Send failures are ignored: a bus with no subscribers is normal.
    pub fn emit_with(&self, kind: impl Into<String>, message: impl Into<String>, data: Value) {
        let event = Event {
            at: util::now(),
            kind: kind.into(),
            message: message.into(),
            data,
        };
        let _ = self.tx.send(event);
    }

    /// Subscribe to subsequent events.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Number of currently attached subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_reach_subscribers() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit("test.event", "hello");
        let got = rx.recv().await.unwrap();
        assert_eq!(got.kind, "test.event");
        assert_eq!(got.message, "hello");
    }

    #[test]
    fn emitting_without_subscribers_is_fine() {
        let bus = EventBus::new();
        bus.emit("nobody.listening", "still fine");
        assert_eq!(bus.subscriber_count(), 0);
    }
}
