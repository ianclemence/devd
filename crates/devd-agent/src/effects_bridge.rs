//! The small part of the effect queue the agent runtime owns.
//!
//! The queue itself lives in `devd-core`, and the maintenance pass lives in
//! `devd-ops`, which depends on neither the agent runtime nor a model. A review,
//! though, needs a reviewer model, so its row is queued here and drained by
//! [`crate::guardian`] in the process that owns the agents.
//!
//! The split exists so that a devd with no agents configured still has a queue
//! that retries its notifications, and a devd with agents has one that retries
//! everything.

use serde_json::Value;

use devd_core::capability::Ctx;
use devd_core::Result;

/// Queue a background review. `args` is what `review.start` was called with, so
/// the review that runs is the review that was asked for.
pub fn queue_review(ctx: &Ctx, id: &str, args: &Value, project: &str) -> Result<()> {
    ctx.store
        .enqueue_effect("review.start", id, &serde_json::json!({ "id": id, "args": args, "project": project }))
}

/// Where a person can read what the queue is doing.
pub fn health(ctx: &Ctx) -> Vec<Value> {
    ctx.store
        .effect_health()
        .unwrap_or_default()
        .into_iter()
        .map(|h| {
            serde_json::json!({
                "kind": h.kind,
                "pending": h.pending,
                "failing": h.failing,
                "dropped": h.dropped,
                "last_error": h.last_error,
                "state": if h.failing > 0 { "error" } else if h.pending > 0 { "working" } else { "ok" },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queued_review_keeps_the_exact_arguments_it_was_asked_with() {
        let store = std::sync::Arc::new(devd_core::Store::open_in_memory().unwrap());
        let dir = std::env::temp_dir().join(format!("devd-qr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = Ctx {
            store: store.clone(),
            paths: std::sync::Arc::new(devd_core::Paths::rooted(&dir)),
            settings: std::sync::Arc::new(devd_core::Settings::default()),
            events: devd_core::EventBus::new(),
            actor: devd_core::Actor::human(devd_core::Surface::Cli),
            run: None,
            dry_run: false,
        };
        queue_review(&ctx, "r1", &serde_json::json!({ "project": dir.display().to_string(), "base": "main" }), "proj").unwrap();
        let now = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
        let due = store.due_effects(&now, 10).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, "review.start");
        assert_eq!(due[0].payload["args"]["base"], "main");
    }
}
