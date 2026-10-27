//! Durable, single-use approval records.
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
//! Approval is a two-step negotiation with a token:
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
//! # Why it is durable
//!
//! The record lives in `state.db`, so an approval survives a restart, is visible
//! to every interface, and cannot be lost by a process that exits while a person
//! is reading the plan. Only the SHA-256 of the token is stored: a token is a
//! bearer secret, and writing it to disk would turn a database read into the
//! ability to approve. A row can be consumed exactly once, expires after
//! [`devd_core::APPROVAL_LIFETIME_MINUTES`], and is refused after a decline.

use std::sync::Arc;

use devd_core::capability::Ctx;
use devd_core::store_runs::{token_hash, NewApproval};
use devd_core::{ApprovalRecord, DevdError, Result, Store};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// How long an issued approval stays valid, in milliseconds, for the UI.
pub const LIFETIME_MS: u64 = (devd_core::APPROVAL_LIFETIME_MINUTES as u64) * 60 * 1000;

/// A pending invocation awaiting a human decision.
#[derive(Debug, Clone, Serialize)]
pub struct Record {
    /// Opaque single-use handle. Never derived from the invocation.
    pub token: String,
    /// Stored row id (a hash of the token), used to link the decision.
    pub id: String,
    /// The run the approval belongs to, when it has one.
    pub run_id: Option<String>,
    /// Capability that would run.
    pub capability: String,
    /// The server's own copy of the arguments.
    ///
    /// Serialised into responses only as part of a redacted plan; the raw value
    /// is what gets replayed, so it must never be echoed back to a client that
    /// could then tamper with it.
    #[serde(skip)]
    pub args: Value,
    /// SHA-256 of the canonical argument encoding, for the audit trail.
    pub args_hash: String,
    /// Who asked, e.g. `human:http`.
    pub actor: String,
    /// Tier, so the UI can style the decision without a second request.
    pub tier: String,
    /// When the token stops being valid, as a count of milliseconds from now.
    pub expires_in_ms: u64,
    /// When the token stops being valid, absolute.
    pub expires_at: String,
}

/// The durable set of approvals awaiting a decision.
pub struct ApprovalStore {
    store: Arc<Store>,
}

impl ApprovalStore {
    /// A store backed by the authoritative state database.
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// Record an invocation and return the token that approves it.
    pub fn issue(&self, ctx: &Ctx, capability: &str, args: &Value, tier: &str) -> Result<Record> {
        // Sweep first: an expired row must never occupy a slot or look answerable.
        let _ = self.store.expire_approvals();
        let pending = self.store.list_approvals(true, 1000)?.len();
        if pending >= 64 {
            return Err(DevdError::conflict(
                "too many operations are already awaiting a decision",
            ));
        }
        let run_id = ctx.run.as_ref().map(|r| r.run_id.clone());
        let (token, record) = self.store.create_approval(NewApproval {
            run_id: run_id.as_deref(),
            task_id: None,
            tool_call_id: None,
            capability,
            args,
            args_hash: &args_hash(args),
            plan: None,
            plan_hash: None,
            scope: "single",
            tier,
            risk: if matches!(tier, "destructive" | "admin") {
                "destructive"
            } else {
                "normal"
            },
            actor: &ctx.actor.label(),
            requested_by: &ctx.actor.label(),
        })?;
        Ok(to_record(token, record))
    }

    /// Consume a token, returning the invocation it stands for.
    ///
    /// The atomic `state='pending'` update is the single-use guarantee: two
    /// concurrent requests presenting the same token cannot both succeed.
    pub fn take(&self, token: &str) -> Result<Record> {
        let record = self.store.consume_approval(token)?;
        Ok(to_record(token.to_string(), record))
    }

    /// Decline the invocation a token stands for. It can never be used after.
    pub fn decline(&self, token: &str, by: &str, reason: Option<&str>) -> Result<Record> {
        let id = token_hash(token);
        let record = self.store.decline_approval(&id, by, reason)?;
        Ok(to_record(token.to_string(), record))
    }

    /// Discard a token without acting on it.
    pub fn discard(&self, token: &str) -> Result<bool> {
        let id = token_hash(token);
        Ok(self
            .store
            .decline_approval(
                &id,
                "system:discarded",
                Some("the caller discarded the request"),
            )
            .is_ok())
    }

    /// Record what happened after a consumed approval ran.
    pub fn finish(&self, id: &str, result: Option<&Value>, error: Option<&str>) {
        let _ = self.store.finish_approval(id, result, error);
    }

    /// How many approvals are awaiting a decision.
    pub fn len(&self) -> usize {
        let _ = self.store.expire_approvals();
        self.store
            .list_approvals(true, 1000)
            .map(|r| r.len())
            .unwrap_or(0)
    }

    /// Whether no approvals are awaiting a decision.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every expired record.
    pub fn sweep(&self) {
        let _ = self.store.expire_approvals();
    }

    /// The pending approvals, with no raw arguments, for a UI or another surface.
    pub fn pending(&self) -> Result<Vec<Value>> {
        let _ = self.store.expire_approvals();
        Ok(self
            .store
            .list_approvals(true, 500)?
            .iter()
            .map(|r| r.public_json())
            .collect())
    }
}

fn to_record(token: String, record: ApprovalRecord) -> Record {
    Record {
        token,
        id: record.id,
        run_id: record.run_id,
        capability: record.capability,
        args: record.args,
        args_hash: record.args_hash,
        actor: record.actor,
        tier: record.tier,
        expires_in_ms: LIFETIME_MS,
        expires_at: record.expires_at,
    }
}

/// 256 bits from the operating system CSPRNG, hex encoded.
///
/// Used for the server's bearer token as well as approval handles.
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
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        Ctx {
            store: Arc::new(store),
            paths: Arc::new(devd_core::Paths::rooted("C:/tmp/devd-approval-test")),
            settings: Arc::new(devd_core::Settings::default()),
            events: devd_core::EventBus::new(),
            actor: Actor::human(Surface::Http),
            run: None,
            dry_run: false,
        }
    }

    fn store_for(c: &Ctx) -> ApprovalStore {
        ApprovalStore::new(c.store.clone())
    }

    #[test]
    fn a_token_is_single_use_and_survives_a_new_store_handle() {
        let c = ctx();
        let s = store_for(&c);
        let issued = s
            .issue(
                &c,
                "git.repo.delete",
                &json!({ "repo": "C:/x" }),
                "destructive",
            )
            .unwrap();
        // A second handle over the same database is what a restart looks like.
        let again = store_for(&c);
        let taken = again.take(&issued.token).unwrap();
        assert_eq!(taken.capability, "git.repo.delete");
        assert!(again.take(&issued.token).is_err(), "a token was reusable");
    }

    #[test]
    fn a_declined_token_can_never_be_taken() {
        let c = ctx();
        let s = store_for(&c);
        let issued = s.issue(&c, "db.reset", &json!({}), "destructive").unwrap();
        s.decline(&issued.token, "human:gui", Some("not now"))
            .unwrap();
        assert!(s.take(&issued.token).is_err());
    }

    #[test]
    fn an_expired_token_fails_closed() {
        let c = ctx();
        let s = store_for(&c);
        let issued = s.issue(&c, "db.reset", &json!({}), "destructive").unwrap();
        c.store
            .reschedule_approval(&issued.id, "2000-01-01T00:00:00Z")
            .unwrap();
        assert!(s.take(&issued.token).is_err());
    }

    #[test]
    fn the_raw_token_is_never_stored() {
        let c = ctx();
        let s = store_for(&c);
        let issued = s.issue(&c, "git.push", &json!({}), "remote").unwrap();
        let stored = c
            .store
            .get_approval(&issued.id)
            .unwrap()
            .expect("the approval row");
        assert_eq!(stored.id, issued.id);
        assert_ne!(stored.id, issued.token);
    }

    #[test]
    fn pending_records_are_visible_without_their_arguments() {
        let c = ctx();
        let s = store_for(&c);
        s.issue(
            &c,
            "git.repo.delete",
            &json!({ "repo": "C:/secret" }),
            "destructive",
        )
        .unwrap();
        let pending = s.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert!(
            !pending[0].to_string().contains("C:/secret"),
            "raw arguments leaked: {}",
            pending[0]
        );
    }
}
