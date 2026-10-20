//! Single-use approval records.
//!
//! # Why this exists
//!
//! The obvious way to let a web UI approve a destructive operation is a flag on
//! the request:
//!
//! ```text
//! POST /api/invoke { "id": "git.repo.delete", "args": {…}, "approve": true }
//! ```
//!
//! That is wrong, and it is worth being precise about why. It makes approval
//! *an assertion by the caller*. Any client that can reach the endpoint can
//! assert it, so the only thing standing between a destructive capability and
//! execution is that the caller chose to send `false` the first time. The
//! capability model exists to prevent exactly that: approval is supposed to be
//! a fact produced by a human decision, not a boolean in a payload.
//!
//! # What happens instead
//!
//! Approval is a two-step negotiation with a token that is meaningful only to
//! this process:
//!
//! ```text
//! POST /api/invoke  { id, args }
//!   → { approval_required: true, plan: {…}, token: "…" }
//!
//! POST /api/approve { token: "…" }
//!   → { outcome: {…} }
//! ```
//!
//! Issuing a token stores **the server's own copy of the invocation** - the
//! capability id, the arguments, and who asked for it. Approving replays that
//! stored copy. The client never re-sends the arguments, so the operation that
//! runs cannot differ from the plan the human was shown, even if the client is
//! buggy, stale, or hostile.
//!
//! A token is 256 bits from the operating system's CSPRNG, valid for
//! [`LIFETIME`], and **removed from the map the moment it is used**, so replay
//! is not a thing that can happen even once. Nothing about a token is written
//! to disk, and nothing about it is returned by any read endpoint.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// How long an issued approval stays valid.
///
/// Long enough to read a plan carefully, short enough that a token left in a
/// forgotten browser tab stops being a live weapon.
pub const LIFETIME: Duration = Duration::from_secs(120);

/// How many approvals may be outstanding at once.
///
/// Reaching this means something is issuing approvals and never deciding them,
/// which is a bug or an attempt to fill memory; either way the right answer is
/// to refuse rather than to evict a record a human may be looking at.
pub const MAX_PENDING: usize = 64;

/// A pending invocation awaiting a human decision.
#[derive(Debug, Clone, Serialize)]
pub struct Record {
    /// Opaque single-use handle. Never derived from the invocation.
    pub token: String,
    /// Capability that would run.
    pub capability: String,
    /// The server's own copy of the arguments.
    ///
    /// Serialised into responses only as part of a redacted plan; the raw value
    /// is what gets replayed, so it must never be echoed back to a client that
    /// could then tamper with it. Keeping it here, rather than accepting it back
    /// from the client at approval time, is the whole point.
    #[serde(skip)]
    pub args: Value,
    /// SHA-256 of the canonical argument encoding, for the audit trail.
    pub args_hash: String,
    /// Who asked, e.g. `human:http`.
    pub actor: String,
    /// Tier, so the UI can style the decision without a second request.
    pub tier: &'static str,
    /// When the token was issued.
    #[serde(skip)]
    pub issued: Instant,
    /// When the token stops being valid, as a count of milliseconds from now.
    pub expires_in_ms: u64,
}

impl Record {
    /// Whether this record has expired.
    pub fn is_expired(&self) -> bool {
        self.issued.elapsed() > LIFETIME
    }
}

/// The set of approvals awaiting a decision.
///
/// A `Mutex` rather than a channel: the critical sections are a hash insert and
/// a hash remove, and holding a lock across the *dispatch* is exactly what we
/// must not do, because dispatch can take seconds and the UI must stay able to
/// render the decision it is waiting on.
#[derive(Default)]
pub struct ApprovalStore {
    pending: Mutex<HashMap<String, Record>>,
}

impl ApprovalStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an invocation and return the token that approves it.
    pub fn issue(
        &self,
        ctx: &Ctx,
        capability: &str,
        args: &Value,
        tier: &'static str,
    ) -> Result<Record> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| DevdError::internal("approval store lock poisoned"))?;

        // Drop anything already past its life before judging capacity, so a
        // long-running window is not eventually unable to ask for approval.
        pending.retain(|_, r| !r.is_expired());
        if pending.len() >= MAX_PENDING {
            return Err(DevdError::conflict(
                "too many operations are already awaiting a decision",
            ));
        }

        let record = Record {
            token: new_token(),
            capability: capability.to_string(),
            args: args.clone(),
            args_hash: args_hash(args),
            actor: ctx.actor.label(),
            tier,
            issued: Instant::now(),
            expires_in_ms: LIFETIME.as_millis() as u64,
        };
        pending.insert(record.token.clone(), record.clone());
        Ok(record)
    }

    /// Consume a token, returning the invocation it stands for.
    ///
    /// Removal happens before the validity checks and inside the same lock, so
    /// two concurrent requests presenting the same token cannot both succeed.
    pub fn take(&self, token: &str) -> Result<Record> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| DevdError::internal("approval store lock poisoned"))?;

        let Some(record) = pending.remove(token) else {
            return Err(DevdError::denied(
                "this approval is unknown, already used, or has expired",
            ));
        };
        if record.is_expired() {
            return Err(DevdError::denied(format!(
                "approval for `{}` expired before it was answered",
                record.capability
            )));
        }
        Ok(record)
    }

    /// Discard a token without acting on it.
    pub fn discard(&self, token: &str) -> Result<bool> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| DevdError::internal("approval store lock poisoned"))?;
        Ok(pending.remove(token).is_some())
    }

    /// How many approvals are awaiting a decision.
    pub fn len(&self) -> usize {
        self.pending.lock().map(|p| p.len()).unwrap_or(0)
    }

    /// Whether no approvals are awaiting a decision.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every expired record.
    pub fn sweep(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|_, r| !r.is_expired());
        }
    }
}

/// 256 bits from the operating system CSPRNG, hex encoded.
pub(crate) fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Canonical hash of the arguments, for the audit row.
///
/// `serde_json::Value::Object` is backed by a `BTreeMap` in this build, so
/// serialising a value sorts its keys and the hash is stable across repeated
/// encodings of the same object. That matters because the hash is how a human
/// can later confirm that the audit row and the plan describe one thing.
pub fn args_hash(args: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(args.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::capability::{Actor, Surface};
    use serde_json::json;

    fn ctx() -> Ctx {
        Ctx {
            store: std::sync::Arc::new(devd_core::Store::open_in_memory().unwrap()),
            paths: std::sync::Arc::new(devd_core::Paths::rooted("C:/tmp/devd-approval-test")),
            settings: std::sync::Arc::new(devd_core::Settings::default()),
            events: devd_core::EventBus::new(),
            actor: Actor::human(Surface::Http),
            dry_run: false,
        }
    }

    fn args() -> Value {
        json!({ "repo": "C:/code/app", "branch": "main" })
    }

    #[test]
    fn issuing_returns_a_256_bit_token() {
        let store = ApprovalStore::new();
        let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        assert_eq!(rec.token.len(), 64);
        assert!(rec.token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_are_unique_across_issues() {
        let store = ApprovalStore::new();
        let mut seen = std::collections::HashSet::new();
        // Each token is consumed immediately, because only `MAX_PENDING` may be
        // outstanding at once - which is itself covered by another test.
        for _ in 0..200 {
            let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
            assert!(seen.insert(rec.token.clone()), "token repeated");
            store.take(&rec.token).unwrap();
        }
    }

    #[test]
    fn taking_consumes_the_token_exactly_once() {
        let store = ApprovalStore::new();
        let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        assert!(store.take(&rec.token).is_ok());
        assert!(store.take(&rec.token).is_err(), "a token was reusable");
        assert!(store.is_empty());
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let store = ApprovalStore::new();
        let err = store.take("00ff").unwrap_err();
        assert_eq!(err.code(), "permission_denied");
    }

    #[test]
    fn the_record_carries_the_servers_copy_of_the_arguments() {
        let store = ApprovalStore::new();
        let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        let taken = store.take(&rec.token).unwrap();
        assert_eq!(taken.args, args());
        assert_eq!(taken.args_hash, args_hash(&args()));
        assert_eq!(taken.actor, "human:http");
    }

    #[test]
    fn args_hash_is_key_order_independent() {
        let a = json!({ "a": 1, "b": 2 });
        let b = json!({ "b": 2, "a": 1 });
        assert_eq!(args_hash(&a), args_hash(&b));
    }

    #[test]
    fn expiry_is_enforced_on_take() {
        let store = ApprovalStore::new();
        let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        // Rewrite the record's issue time to look ancient. Reaching into the
        // map keeps the test honest about the path a real expiry takes.
        {
            let mut pending = store.pending.lock().unwrap();
            if let Some(entry) = pending.get_mut(&rec.token) {
                entry.issued = Instant::now() - (LIFETIME + Duration::from_secs(1));
            }
        }
        let err = store.take(&rec.token).unwrap_err();
        assert!(err.to_string().contains("expired"), "{err}");
    }

    #[test]
    fn sweeps_expired_records_and_frees_capacity() {
        let store = ApprovalStore::new();
        let rec = store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        {
            let mut pending = store.pending.lock().unwrap();
            if let Some(entry) = pending.get_mut(&rec.token) {
                entry.issued = Instant::now() - (LIFETIME + Duration::from_secs(1));
            }
        }
        store.sweep();
        assert!(store.is_empty());
    }

    #[test]
    fn refuses_to_hold_more_than_the_cap() {
        let store = ApprovalStore::new();
        for _ in 0..MAX_PENDING {
            store.issue(&ctx(), "git.push", &args(), "remote").unwrap();
        }
        let err = store
            .issue(&ctx(), "git.push", &args(), "remote")
            .unwrap_err();
        assert_eq!(err.code(), "conflict");
    }
}
