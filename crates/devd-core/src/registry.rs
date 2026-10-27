//! The capability registry and its dispatcher.
//!
//! [`Registry::dispatch`] is the **only** path from any interface to a side
//! effect. It resolves the capability, enforces the permission tier, refuses
//! human-only operations to agents, requires an approval that the caller had to
//! obtain from a person, writes an audit row for every attempt, and publishes an
//! event on success.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::capability::{Approval, Capability, Ctx, InvokeRequest, InvokeResult};
use crate::error::{DevdError, Result};
use crate::state::AuditRecord;

/// Every capability `devd` knows about.
#[derive(Default)]
pub struct Registry {
    caps: BTreeMap<String, Arc<Capability>>,
}

/// What an agent may still do on its own in production mode: write notes,
/// report to its team, and read documentation.
const PRODUCTION_FREE: &[&str] = &[
    "team.report",
    "team.plan_submit",
    "inbox.post",
    "scratch.",
    "todo.",
    "docs.fetch",
    "search.",
    "approval.get",
];

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            caps: BTreeMap::new(),
        }
    }

    /// Add a capability. Duplicate identifiers are rejected, which keeps the
    /// CLI, GUI, HTTP and MCP surfaces from silently shadowing each other.
    pub fn register(&mut self, cap: Capability) -> Result<()> {
        if self.caps.contains_key(&cap.id) {
            return Err(DevdError::internal(format!(
                "capability `{}` is already registered",
                cap.id
            )));
        }
        self.caps.insert(cap.id.clone(), Arc::new(cap));
        Ok(())
    }

    /// Add many capabilities.
    pub fn register_many(&mut self, caps: Vec<Capability>) -> Result<()> {
        for cap in caps {
            self.register(cap)?;
        }
        Ok(())
    }

    /// Look up a capability.
    pub fn get(&self, id: &str) -> Option<Arc<Capability>> {
        self.caps.get(id).cloned()
    }

    /// All capabilities, ordered by identifier.
    pub fn list(&self) -> Vec<Arc<Capability>> {
        self.caps.values().cloned().collect()
    }

    /// Capabilities grouped by their discovery group.
    pub fn groups(&self) -> BTreeMap<String, Vec<Arc<Capability>>> {
        let mut out: BTreeMap<String, Vec<Arc<Capability>>> = BTreeMap::new();
        for cap in self.caps.values() {
            out.entry(cap.group.clone()).or_default().push(cap.clone());
        }
        out
    }

    /// Number of registered capabilities.
    pub fn len(&self) -> usize {
        self.caps.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    /// Ranked search used by the MCP meta-tools and the GUI command palette.
    ///
    /// Ranking: exact id, then id prefix, then substring in id, then substring
    /// in the summary. Ties are broken alphabetically for determinism.
    pub fn search(
        &self,
        query: &str,
        group: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Vec<Arc<Capability>> {
        let q = query.trim().to_lowercase();
        let mut scored: Vec<(u8, Arc<Capability>)> = self
            .caps
            .values()
            .filter(|c| group.map(|g| g == c.group).unwrap_or(true))
            .filter_map(|c| {
                if q.is_empty() {
                    return Some((10, c.clone()));
                }
                let id = c.id.to_lowercase();
                let summary = c.summary.to_lowercase();
                let score = if id == q {
                    0
                } else if id.starts_with(&q) {
                    1
                } else if id.contains(&q) {
                    2
                } else if c.group.to_lowercase().contains(&q) {
                    3
                } else if summary.contains(&q) {
                    4
                } else {
                    return None;
                };
                Some((score, c.clone()))
            })
            .collect();

        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, c)| c)
            .collect()
    }

    /// A JSON description of one capability, used by the HTTP API and MCP.
    pub fn describe_json(&self, id: &str) -> Result<Value> {
        let cap = self
            .get(id)
            .ok_or_else(|| DevdError::not_found(format!("no capability `{id}`")))?;
        Ok(serde_json::json!({
            "id": cap.id,
            "summary": cap.summary,
            "description": cap.description,
            "group": cap.group,
            "tier": cap.tier.as_str(),
            "effect": cap.effect.as_str(),
            "agent_safe": cap.agent_safe,
            "human_only": cap.human_only,
            "agent_needs_approval": cap.agent_needs_approval,
            "supports_dry_run": cap.supports_dry_run,
            "idempotent": cap.idempotent,
            "requires_approval": cap.tier.requires_approval(),
            "args_schema": cap.args_schema,
        }))
    }

    /// Resolve, authorise, plan or apply - the single gateway to side effects.
    pub fn dispatch(
        &self,
        ctx: &Ctx,
        req: &InvokeRequest,
        approval: Approval,
    ) -> Result<InvokeResult> {
        self.dispatch_with(ctx, req, approval, &DispatchOptions::default())
    }

    /// Dispatch with the run-scoped metadata a durable caller carries.
    ///
    /// `idempotency_key` makes a retried call return the original result instead
    /// of repeating a side effect. `approval_id` links the record to the durable
    /// approval that authorised it, and `call_id` keeps the provider's tool-use
    /// id so a conversation can be reconstructed exactly.
    pub fn dispatch_with(
        &self,
        ctx: &Ctx,
        req: &InvokeRequest,
        approval: Approval,
        opts: &DispatchOptions<'_>,
    ) -> Result<InvokeResult> {
        let cap = self
            .get(&req.id)
            .ok_or_else(|| DevdError::not_found(format!("no capability `{}`", req.id)))?;

        // A completed call for this exact key is the answer, not a second side
        // effect. Only successful calls are reused: a failure may be retried.
        if let Some(key) = opts.idempotency_key {
            if let Some(previous) = ctx.store.completed_call_by_key(key)? {
                if let Some(result) = previous
                    .result
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<InvokeResult>(v.clone()).ok())
                {
                    return Ok(result);
                }
            }
        }

        let started = std::time::Instant::now();
        let started_at = crate::util::now();
        let result = self.dispatch_inner(ctx, req, approval, &cap);
        self.record(
            ctx,
            req,
            &cap,
            &result,
            opts,
            started.elapsed().as_millis() as i64,
            &started_at,
        );
        result
    }

    fn dispatch_inner(
        &self,
        ctx: &Ctx,
        req: &InvokeRequest,
        approval: Approval,
        cap: &Arc<Capability>,
    ) -> Result<InvokeResult> {
        // 1. A human explicitly declined. Never proceed.
        if approval == Approval::Denied {
            return Err(DevdError::denied(format!(
                "`{}` was declined by the operator",
                cap.id
            )));
        }

        // 2. Human-only operations can never be performed by an agent, and this
        //    check cannot be satisfied by supplying an approval.
        if cap.human_only && ctx.actor.is_agent() {
            return Err(DevdError::denied(format!(
                "`{}` is a human-only operation and cannot be performed by an agent",
                cap.id
            )));
        }
        if !cap.agent_safe && ctx.actor.is_agent() {
            return Err(DevdError::denied(format!(
                "`{}` is not available to agents in this configuration",
                cap.id
            )));
        }

        // 2b. In production mode an agent changes nothing without a person's
        //     approval, however low the capability's tier. Notes, plans and
        //     reads stay free so the agent can still work out what to ask for.
        if ctx.actor.is_agent()
            && !req.dry_run
            && approval != Approval::Granted
            && cap.tier != crate::capability::Tier::Read
            && !PRODUCTION_FREE
                .iter()
                .any(|p| cap.id == *p || (p.ends_with('.') && cap.id.starts_with(p)))
            && ctx.store.environment_mode() == "production"
        {
            return Err(DevdError::ApprovalRequired {
                capability: cap.id.clone(),
                reason: format!("devd is in production mode: `{}` changes things, and an agent needs a person's approval for every change here", cap.id),
            });
        }

        // 3. Sensitive tiers require an approval that a human granted.
        if cap.tier.requires_approval() && !req.dry_run && approval != Approval::Granted {
            return Err(DevdError::ApprovalRequired {
                capability: cap.id.clone(),
                reason: format!(
                    "`{}` is a {} operation and requires explicit human approval",
                    cap.id,
                    cap.tier.as_str()
                ),
            });
        }

        // 3b. An agent that supplies the code devd will run (a cron command, a deploy script)
        //     waits for a person to read it, whatever the tier. The plan, which carries the
        //     code, is what the person approves, and a plan itself needs no approval.
        if cap.agent_needs_approval
            && ctx.actor.is_agent()
            && !req.dry_run
            && approval != Approval::Granted
        {
            return Err(DevdError::ApprovalRequired {
                capability: cap.id.clone(),
                reason: format!(
                    "`{}` runs code the caller wrote, so an agent needs a person's approval to use it",
                    cap.id
                ),
            });
        }

        // 4. Dry runs on capabilities that cannot plan are a caller mistake.
        if req.dry_run && !cap.supports_dry_run {
            return Err(DevdError::unsupported(format!(
                "`{}` does not support dry runs",
                cap.id
            )));
        }

        // 5. The run may have been cancelled or run out of time while this call
        //    waited. Check once, centrally, so no handler can start work on a
        //    run a person already stopped.
        if let Some(run) = &ctx.run {
            run.check()?;
        }

        // 6. Arguments are checked against the declared schema at the one
        //    boundary every caller crosses - direct alias, MCP, HTTP, team or
        //    schedule - rather than by each handler as a matter of habit.
        if !cap.args_schema.is_null() {
            crate::schema::validate(&cap.args_schema, &req.args)?;
        }

        let run_ctx = Ctx {
            store: ctx.store.clone(),
            paths: ctx.paths.clone(),
            settings: ctx.settings.clone(),
            events: ctx.events.clone(),
            actor: ctx.actor.clone(),
            run: ctx.run.clone(),
            dry_run: req.dry_run,
        };

        if req.dry_run {
            let plan = cap.handler.plan(&run_ctx, &req.args)?;
            Ok(InvokeResult {
                capability: cap.id.clone(),
                dry_run: true,
                plan: Some(plan),
                outcome: None,
            })
        } else {
            let outcome = cap.handler.apply(&run_ctx, &req.args)?;
            Ok(InvokeResult {
                capability: cap.id.clone(),
                dry_run: false,
                plan: None,
                outcome: Some(outcome),
            })
        }
    }

    // The arguments are the pieces of one normalized record; grouping them into
    // a struct only moves the same fields one level down.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        ctx: &Ctx,
        req: &InvokeRequest,
        cap: &Arc<Capability>,
        result: &Result<InvokeResult>,
        opts: &DispatchOptions<'_>,
        duration_ms: i64,
        started_at: &str,
    ) {
        let (ok, message) = match result {
            Ok(_) => (true, "ok".to_string()),
            Err(e) => (false, e.to_string()),
        };
        let redacted = redact(&req.args);
        let redacted_value = redact_value(&req.args);
        let _ = ctx.store.record_audit(AuditRecord {
            actor: &ctx.actor.label(),
            surface: ctx.actor.surface.as_str(),
            capability: &cap.id,
            tier: cap.tier.as_str(),
            effect: cap.effect.as_str(),
            dry_run: req.dry_run,
            ok,
            args: &redacted,
            message: &message,
        });

        // The unified record. A dry run is a plan, not an invocation, so it is
        // audited but does not claim a side effect or produce evidence.
        if let Some(run) = &ctx.run {
            if !req.dry_run {
                let args_hash = crate::util::sha256_hex(req.args.to_string().as_bytes());
                let outcome = result.as_ref().ok().and_then(|r| r.outcome.as_ref());
                let evidence = crate::runs::evidence_for(&cap.id, &req.args, outcome);
                let error_code = result.as_ref().err().map(|e| e.code());
                let error = result.as_ref().err().map(|e| e.to_string());
                let serialized = result
                    .as_ref()
                    .ok()
                    .and_then(|r| serde_json::to_value(r).ok());
                let call = ctx.store.record_tool_call(crate::store_runs::NewToolCall {
                    run_id: Some(&run.run_id),
                    step_id: run.step_id.as_deref(),
                    call_id: opts.call_id,
                    actor: &ctx.actor.label(),
                    surface: ctx.actor.surface.as_str(),
                    capability: &cap.id,
                    args: &redacted_value,
                    args_hash: &args_hash,
                    approval_id: opts.approval_id,
                    idempotency_key: opts.idempotency_key,
                    ok,
                    error_code,
                    error: error.as_deref(),
                    exit_code: evidence.exit_code,
                    duration_ms,
                    result: serialized.as_ref(),
                    started_at,
                });
                if let Ok(call) = call {
                    let _ = ctx.store.add_evidence(crate::store_runs::NewEvidence {
                        run_id: &run.run_id,
                        step_id: run.step_id.as_deref(),
                        tool_call_id: Some(&call.id),
                        artifact_id: None,
                        kind: evidence.kind.as_str(),
                        capability: &cap.id,
                        passed: evidence.passed,
                        claim: None,
                        summary: &evidence.summary,
                        detail: evidence.detail.as_deref(),
                    });
                }
            }
        }

        if ok {
            ctx.events.emit_with(
                "capability.applied",
                format!("{} ({})", cap.id, ctx.actor.label()),
                serde_json::json!({ "capability": cap.id, "dry_run": req.dry_run, "run_id": ctx.run.as_ref().map(|r| r.run_id.clone()) }),
            );
        }
    }
}

/// Run-scoped metadata for one dispatch.
///
/// A plain CLI call uses the default: no approval record, no idempotency key and
/// no provider tool-use id. A durable run fills these in so its timeline can be
/// replayed and a retried call cannot repeat a side effect.
#[derive(Debug, Clone, Default)]
pub struct DispatchOptions<'a> {
    /// Durable approval that authorised the call.
    pub approval_id: Option<&'a str>,
    /// Key that makes a retry return the first success.
    pub idempotency_key: Option<&'a str>,
    /// Provider tool-use id.
    pub call_id: Option<&'a str>,
}

/// Keys whose values are never written to logs, audit rows, or agent context.
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "app_key",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "authorization",
    "credential",
    "passphrase",
];

fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|m| k.contains(m))
}

/// Recursively replace secret-looking argument values with a placeholder.
pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_secret_key(k) {
                    out.insert(k.clone(), Value::String("[redacted]".to_string()));
                } else {
                    out.insert(k.clone(), redact_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

/// Serialise arguments for the audit trail with secrets removed and a hard
/// length cap so a runaway payload cannot bloat the database.
pub fn redact(value: &Value) -> String {
    let clean = redact_value(value);
    let text = clean.to_string();
    crate::util::truncate(&text, 4000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Actor, Outcome, Plan, Surface, Tier};
    use crate::config::Settings;
    use crate::events::EventBus;
    use crate::paths::Paths;
    use crate::state::Store;

    fn test_ctx() -> Ctx {
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        Ctx {
            run: None,
            store: Arc::new(store),
            paths: Arc::new(Paths::rooted("C:/tmp/devd-test")),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Cli),
            dry_run: false,
        }
    }

    fn registry_with_read_cap() -> Registry {
        let mut reg = Registry::new();
        reg.register(
            Capability::builder("demo.read", "read something")
                .dry_run(true)
                .handler(
                    |_, _| Ok(Plan::new("demo.read", "would read")),
                    |_, _| Ok(Outcome::ok("read")),
                )
                .build()
                .unwrap(),
        )
        .unwrap();
        reg
    }

    fn registry_with_destructive_cap() -> Registry {
        let mut reg = Registry::new();
        reg.register(
            Capability::builder("demo.wipe", "wipe data")
                .tier(Tier::Destructive)
                .effect(crate::capability::Effect::Data)
                .dry_run(true)
                .handler(
                    |_, _| Ok(Plan::new("demo.wipe", "would wipe").destructive(true)),
                    |_, _| Ok(Outcome::ok("wiped")),
                )
                .build()
                .unwrap(),
        )
        .unwrap();
        reg
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut reg = registry_with_read_cap();
        let dup = Capability::builder("demo.read", "again")
            .handler(|_, _| Ok(Plan::new("x", "y")), |_, _| Ok(Outcome::ok("z")))
            .build()
            .unwrap();
        assert!(reg.register(dup).is_err());
    }

    #[test]
    fn read_tier_runs_without_approval() {
        let reg = registry_with_read_cap();
        let ctx = test_ctx();
        let req = InvokeRequest {
            id: "demo.read".into(),
            args: serde_json::json!({}),
            dry_run: false,
        };
        let res = reg.dispatch(&ctx, &req, Approval::None).unwrap();
        assert!(res.outcome.unwrap().ok);
    }

    #[test]
    fn destructive_tier_requires_granted_approval() {
        let reg = registry_with_destructive_cap();
        let ctx = test_ctx();
        let req = InvokeRequest {
            id: "demo.wipe".into(),
            args: serde_json::json!({}),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::None).unwrap_err();
        assert!(err.is_approval_required(), "got {err:?}");

        // A plan is allowed without approval because it changes nothing.
        let plan_req = InvokeRequest {
            id: "demo.wipe".into(),
            args: serde_json::json!({}),
            dry_run: true,
        };
        assert!(reg.dispatch(&ctx, &plan_req, Approval::None).is_ok());

        // Explicitly granted approval permits the change.
        assert!(reg.dispatch(&ctx, &req, Approval::Granted).is_ok());
    }

    #[test]
    fn declined_operation_never_executes() {
        let reg = registry_with_destructive_cap();
        let ctx = test_ctx();
        let req = InvokeRequest {
            id: "demo.wipe".into(),
            args: serde_json::json!({}),
            dry_run: false,
        };
        let err = reg.dispatch(&ctx, &req, Approval::Denied).unwrap_err();
        assert!(matches!(err, DevdError::Denied(_)));
        // Even with a higher tier granted later, the denial is absolute.
        assert!(reg.dispatch(&ctx, &req, Approval::Denied).is_err());
    }

    #[test]
    fn agents_cannot_perform_human_only_operations() {
        let mut reg = Registry::new();
        reg.register(
            Capability::builder("tls.trust_install", "install root CA")
                .tier(Tier::Admin)
                .human_only(true)
                .dry_run(true)
                .handler(
                    |_, _| Ok(Plan::new("tls.trust_install", "would install")),
                    |_, _| Ok(Outcome::ok("installed")),
                )
                .build()
                .unwrap(),
        )
        .unwrap();

        let mut ctx = test_ctx();
        ctx.actor = Actor::agent("claude-code");
        let req = InvokeRequest {
            id: "tls.trust_install".into(),
            args: serde_json::json!({}),
            dry_run: false,
        };
        // Even a granted approval cannot let an agent through.
        let err = reg.dispatch(&ctx, &req, Approval::Granted).unwrap_err();
        assert!(matches!(err, DevdError::Denied(_)), "got {err:?}");
    }

    #[test]
    fn audit_rows_are_written_for_success_and_failure() {
        let reg = registry_with_destructive_cap();
        let ctx = test_ctx();
        let req = InvokeRequest {
            id: "demo.wipe".into(),
            args: serde_json::json!({ "password": "hunter2" }),
            dry_run: false,
        };
        let _ = reg.dispatch(&ctx, &req, Approval::Denied);
        let _ = reg.dispatch(&ctx, &req, Approval::Granted);
        let rows = ctx.store.list_audit(10).unwrap();
        assert_eq!(rows.len(), 2);
        // Secrets never reach the audit trail.
        for row in &rows {
            assert!(!row.args.contains("hunter2"), "leaked secret: {}", row.args);
            assert!(row.args.contains("[redacted]"));
        }
    }

    #[test]
    fn unknown_capability_is_not_found() {
        let reg = registry_with_read_cap();
        let ctx = test_ctx();
        let req = InvokeRequest {
            id: "nope.nope".into(),
            args: serde_json::json!({}),
            dry_run: false,
        };
        assert!(matches!(
            reg.dispatch(&ctx, &req, Approval::None),
            Err(DevdError::NotFound(_))
        ));
    }

    #[test]
    fn search_ranks_exact_id_first() {
        let mut reg = Registry::new();
        for id in ["git.status", "git.stage", "site.create", "status.check"] {
            reg.register(
                Capability::builder(id, "s")
                    .handler(|_, _| Ok(Plan::new("x", "y")), |_, _| Ok(Outcome::ok("z")))
                    .build()
                    .unwrap(),
            )
            .unwrap();
        }
        let hits = reg.search("git.status", None, 10, 0);
        assert_eq!(hits[0].id, "git.status");
        let status = reg.search("status", None, 10, 0);
        assert!(status.iter().any(|c| c.id == "status.check"));
    }

    #[test]
    fn secret_keys_are_redacted_recursively() {
        let v = serde_json::json!({
            "user": "ian",
            "nested": { "api_key": "abc123", "keep": "yes" },
            "list": [{ "password": "x" }]
        });
        let s = redact(&v);
        assert!(!s.contains("abc123"));
        assert!(s.contains("\"user\":\"ian\""));
        assert!(s.contains("yes"));
    }
}
