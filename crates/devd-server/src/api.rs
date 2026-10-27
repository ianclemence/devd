//! HTTP handlers.
//!
//! Every handler is a thin adapter: parse the transport, call the registry, and
//! render the result. Where a handler appears to make a decision, it is a
//! *transport* decision - which errors become which status codes, and how an
//! approval is negotiated - and never a permission decision. No handler may
//! decide that an operation is allowed; only [`devd_core::Registry::dispatch`]
//! does that, and every path here goes through it.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Approval, Ctx, InvokeRequest};
use devd_core::{
    evaluate, DevdError, DispatchOptions, InvokeResult, RunContext, RunMode, RunSpec, RunState,
    Store, Tier,
};

use crate::{error_response, refuse, AppState};

/// Largest page of capabilities a client may request at once.
const MAX_PAGE: usize = 500;

/// Longest audit page a client may request at once.
const MAX_AUDIT: u32 = 1000;

/// `GET /api/session` - who this server is and how it is configured.
pub async fn session(State(state): State<Arc<AppState>>) -> Response {
    let groups: Vec<Value> = state
        .registry
        .groups()
        .into_iter()
        .map(|(name, caps)| json!({ "name": name, "count": caps.len() }))
        .collect();

    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "actor": {
            "kind": state.ctx.actor.kind,
            "surface": state.ctx.actor.surface.as_str(),
            "name": state.ctx.actor.name,
        },
        "port": state.port,
        "uptime_ms": state.started.elapsed().as_millis() as u64,
        "pending_approvals": state.approvals.len(),
        "native_picker": state.picker.is_some(),
        "paths": {
            "root": state.ctx.paths.root().display().to_string(),
            "state_db": state.ctx.paths.state_db().display().to_string(),
            "www": state.ctx.paths.www().display().to_string(),
            "logs": state.ctx.paths.logs().display().to_string(),
            "runtimes": state.ctx.paths.runtimes().display().to_string(),
            "services": state.ctx.paths.services().display().to_string(),
        },
        "settings": state.ctx.settings.as_ref(),
        "registry": {
            "capabilities": state.registry.len(),
            "groups": groups,
        },
    }))
    .into_response()
}

/// Query for the capability list.
#[derive(Debug, Deserialize)]
pub struct CapQuery {
    /// Free-text match against id, group and summary.
    #[serde(default)]
    q: Option<String>,
    /// Restrict to one group.
    #[serde(default)]
    group: Option<String>,
    /// Page size.
    #[serde(default)]
    limit: Option<usize>,
    /// Page offset.
    #[serde(default)]
    offset: Option<usize>,
}

/// Shape of one row in the capability list.
///
/// Deliberately not the full description: the palette renders hundreds of rows
/// and only needs enough to rank and label them. `args_schema` and the prose
/// description come from `GET /api/capabilities/:id` for the one row the user
/// actually opens.
fn cap_row(cap: &devd_core::Capability) -> Value {
    json!({
        "id": cap.id,
        "summary": cap.summary,
        "group": cap.group,
        "tier": cap.tier.as_str(),
        "effect": cap.effect.as_str(),
        "agent_safe": cap.agent_safe,
        "human_only": cap.human_only,
        "requires_approval": cap.tier.requires_approval(),
        "supports_dry_run": cap.supports_dry_run,
        "idempotent": cap.idempotent,
    })
}

/// `GET /api/capabilities` - ranked search over the registry.
pub async fn capabilities(
    State(state): State<Arc<AppState>>,
    Query(q): Query<CapQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_PAGE);
    let offset = q.offset.unwrap_or(0);
    let query = q.q.unwrap_or_default();

    // The registry's search is the ranking; asking it for everything lets this
    // endpoint report a true total, which a paging UI needs, without a second
    // traversal of the registry.
    let all = state
        .registry
        .search(&query, q.group.as_deref(), usize::MAX, 0);
    let total = all.len();
    let items: Vec<Value> = all
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|c| cap_row(&c))
        .collect();

    Json(json!({ "total": total, "offset": offset, "limit": limit, "items": items }))
        .into_response()
}

/// `GET /api/capabilities/:id` - the full declaration, including its schema.
pub async fn capability(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.registry.describe_json(&id) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `GET /api/groups` - group names with their capability counts.
pub async fn groups(State(state): State<Arc<AppState>>) -> Response {
    let groups: Vec<Value> = state
        .registry
        .groups()
        .into_iter()
        .map(|(name, caps)| json!({ "name": name, "count": caps.len() }))
        .collect();
    Json(json!({ "groups": groups })).into_response()
}

/// Body of an invocation request.
#[derive(Debug, Deserialize)]
pub struct InvokeBody {
    /// Capability to invoke.
    pub id: String,
    /// Arguments object. Anything other than an object is refused by the
    /// capability's own parser.
    #[serde(default)]
    pub args: Value,
    /// Ask for a plan instead of an application.
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /hooks/deploy/:site/:token` - start a deploy of a site whose push-to-deploy hook is on.
///
/// Every refusal is the same `404`, so a wrong site, a wrong secret and a hook that is off cannot be told apart.
pub async fn deploy_hook(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((site, token)): axum::extract::Path<(String, String)>,
) -> Response {
    let st = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        if !devd_ops::deploy::hook_ok(&st.ctx, &site, &token) {
            return None;
        }
        // A hook is a caller with no human behind it, but it is still a caller:
        // it goes through the same `deploy.run` capability, run and evidence as
        // a person pressing Deploy, so the history reads the same either way.
        let mut ctx = st.ctx.clone();
        let mut run_id = None;
        if let Some((run_ctx, id)) = begin_run(&st, &format!("deploy {site} from its push hook")) {
            ctx = run_ctx;
            run_id = Some(id);
        }
        let req = InvokeRequest {
            id: "deploy.run".into(),
            args: json!({ "site": site, "wait": false }),
            dry_run: false,
        };
        let dispatch =
            st.registry
                .dispatch_with(&ctx, &req, Approval::None, &DispatchOptions::default());
        settle(&ctx.store, run_id.as_deref(), &dispatch);
        Some(dispatch)
    })
    .await;
    match result {
        Ok(Some(Ok(result))) => {
            let deployment = result
                .outcome
                .as_ref()
                .map(|o| o.data["deployment"].clone())
                .unwrap_or(Value::Null);
            (
                StatusCode::ACCEPTED,
                axum::Json(json!({
                    "ok": true,
                    "deployment": deployment.get("id").cloned().unwrap_or(Value::Null),
                    "state": deployment.get("state").cloned().unwrap_or(Value::Null),
                })),
            )
                .into_response()
        }
        Ok(Some(Err(e))) => error_response(&e),
        _ => refuse(StatusCode::NOT_FOUND, "not_found", "there is no such hook"),
    }
}

/// `POST /api/invoke` - run a capability, or open an approval for one.
///
/// Three outcomes, and which one happens is decided by the capability's tier,
/// not by anything the client sends:
///
/// * the tier needs no approval, or the client asked for a plan: dispatch and
///   return the plan or the outcome;
/// * the tier needs approval: return the plan **and a token**, and change
///   nothing. The token carries the server's own copy of the arguments, which
///   is what makes the second step meaningful.
pub async fn invoke(State(state): State<Arc<AppState>>, body: String) -> Response {
    let Some(mut body) = parse::<InvokeBody>(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "expected a JSON object with an `id` field",
        );
    };
    if body.args.is_null() {
        body.args = json!({});
    }

    let Some(cap) = state.registry.get(&body.id) else {
        return error_response(&DevdError::not_found(format!(
            "no capability `{}`",
            body.id
        )));
    };

    if body.dry_run || !cap.tier.requires_approval() {
        let req = InvokeRequest {
            id: body.id.clone(),
            args: body.args.clone(),
            dry_run: body.dry_run,
        };
        // Stack operations run for minutes; dispatch off the async workers so a
        // long initialisation never starves the requests that poll its progress.
        let st = state.clone();
        let mut ctx = st.ctx.clone();
        let mut run_id = None;
        // A plan is not an invocation, and a read leaves nothing to prove. Every
        // other call gets a durable run so it appears on the same timeline as an
        // agent's work and leaves the same evidence.
        if !body.dry_run && cap.tier != Tier::Read {
            if let Some((run_ctx, id)) = begin_run(&st, &format!("{} via the HTTP API", body.id)) {
                ctx = run_ctx;
                run_id = Some(id);
            }
        }
        let run_for_settle = run_id.clone();
        let dispatched = tokio::task::spawn_blocking(move || {
            let result =
                st.registry
                    .dispatch_with(&ctx, &req, Approval::None, &DispatchOptions::default());
            settle(&ctx.store, run_for_settle.as_deref(), &result);
            result
        })
        .await;
        let dispatched = match dispatched {
            Ok(r) => r,
            Err(e) => {
                return error_response(&DevdError::internal(format!("the operation panicked: {e}")))
            }
        };
        return match dispatched {
            Ok(result) => Json(json!({
                "capability": result.capability,
                "dry_run": result.dry_run,
                "plan": result.plan,
                "outcome": result.outcome,
                "approval_required": false,
                "run_id": run_id,
            }))
            .into_response(),
            Err(e) => error_response(&e),
        };
    }

    // The plan is computed here, before a token exists, so that what the human
    // is shown is produced by the capability itself rather than described by the
    // UI. A capability that cannot plan simply has none to show.
    let plan = plan_for(&state, &body.id, &body.args);

    // The pending approval and the eventual execution share one run, so the
    // decision and the work it authorised are one timeline entry rather than
    // two unrelated records.
    let mut ctx = state.ctx.clone();
    if let Some((run_ctx, id)) = begin_run(&state, &format!("{} via the HTTP API", body.id)) {
        let _ = state
            .ctx
            .store
            .transition_run(&id, RunState::AwaitingApproval, None);
        ctx = run_ctx;
    }

    match state
        .approvals
        .issue(&ctx, &body.id, &body.args, cap.tier.as_str())
    {
        Ok(record) => Json(json!({
            "capability": record.capability,
            "dry_run": false,
            "approval_required": true,
            "tier": record.tier,
            "summary": cap.summary,
            "description": cap.description,
            "plan": plan,
            "token": record.token,
            "expires_in_ms": record.expires_in_ms,
            "args_hash": record.args_hash,
            "outcome": Value::Null,
        }))
        .into_response(),
        Err(e) => error_response(&e),
    }
}

/// Body carrying an approval token.
#[derive(Debug, Deserialize)]
pub struct TokenBody {
    /// Token from the `invoke` response.
    pub token: String,
}

/// `POST /api/approve` - apply the invocation a token stands for.
///
/// The arguments come from the stored record, never from this request, so a
/// client cannot show one plan and approve another.
pub async fn approve(State(state): State<Arc<AppState>>, body: String) -> Response {
    let Some(body) = parse::<TokenBody>(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "expected a JSON object with a `token` field",
        );
    };

    let record = match state.approvals.take(&body.token) {
        Ok(record) => record,
        Err(e) => return error_response(&e),
    };

    let req = InvokeRequest {
        id: record.capability.clone(),
        args: record.args.clone(),
        dry_run: false,
    };
    let mut ctx = state.ctx.clone();
    let run_id = record.run_id.clone();
    if let Some(id) = &run_id {
        let _ = state
            .ctx
            .store
            .transition_run(id, RunState::Executing, None);
        ctx.run = Some(RunContext::new(id.clone()).with_store(state.ctx.store.clone()));
    }
    let approval_id = record.id.clone();
    let actor = record.actor.clone();
    let args_hash = record.args_hash.clone();

    let st = state.clone();
    let dispatched = match tokio::task::spawn_blocking(move || {
        let opts = DispatchOptions {
            approval_id: Some(&approval_id),
            ..DispatchOptions::default()
        };
        let result = st
            .registry
            .dispatch_with(&ctx, &req, Approval::Granted, &opts);
        settle(&ctx.store, run_id.as_deref(), &result);
        result
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return error_response(&DevdError::internal(format!("the operation panicked: {e}")))
        }
    };

    match dispatched {
        Ok(result) => {
            let stored = serde_json::to_value(&result).ok();
            state.approvals.finish(&record.id, stored.as_ref(), None);
            Json(json!({
                "capability": result.capability,
                "approved_by": actor,
                "args_hash": args_hash,
                "outcome": result.outcome,
            }))
            .into_response()
        }
        Err(e) => {
            state
                .approvals
                .finish(&record.id, None, Some(&e.to_string()));
            error_response(&e)
        }
    }
}

/// `POST /api/deny` - decline the invocation a token stands for.
///
/// The declined operation is still *dispatched*, with [`Approval::Denied`], so
/// that the refusal lands in the audit trail and in the event stream. A decline
/// that left no record would be indistinguishable from a request that never
/// arrived, which is exactly the ambiguity an audit log exists to remove.
pub async fn deny(State(state): State<Arc<AppState>>, body: String) -> Response {
    let Some(body) = parse::<TokenBody>(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "expected a JSON object with a `token` field",
        );
    };

    let record = match state.approvals.decline(
        &body.token,
        &state.ctx.actor.label(),
        Some("declined in the window"),
    ) {
        Ok(record) => record,
        Err(e) => return error_response(&e),
    };

    // A decline is a decision, not a failed call: the approval row already
    // records it durably. The dispatch below only writes the audit entry, so the
    // refusal is visible on every surface that reads the audit trail.
    let req = InvokeRequest {
        id: record.capability.clone(),
        args: record.args.clone(),
        dry_run: false,
    };
    if let Some(id) = &record.run_id {
        let _ =
            state
                .ctx
                .store
                .transition_run(id, RunState::Cancelled, Some("declined by a person"));
    }
    let _ = state.registry.dispatch(&state.ctx, &req, Approval::Denied);

    Json(json!({
        "declined": true,
        "capability": record.capability,
        "args_hash": record.args_hash,
    }))
    .into_response()
}

/// Query for the run list.
#[derive(Debug, Deserialize)]
pub struct RunsQuery {
    /// Maximum rows to return.
    #[serde(default)]
    limit: Option<i64>,
    /// Rows to skip.
    #[serde(default)]
    offset: Option<i64>,
}

/// `GET /api/runs` - the durable work list, newest first.
pub async fn runs(State(state): State<Arc<AppState>>, Query(q): Query<RunsQuery>) -> Response {
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let offset = q.offset.unwrap_or(0).max(0);
    match state.ctx.store.page_runs(limit, offset) {
        Ok(items) => {
            Json(json!({ "items": items, "limit": limit, "offset": offset })).into_response()
        }
        Err(e) => error_response(&e),
    }
}

/// `GET /api/runs/:id` - one run and everything that proves it.
pub async fn run(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.ctx.store.run_timeline(&id) {
        Ok(timeline) => Json(timeline).into_response(),
        Err(e) => error_response(&e),
    }
}

/// Query for one run's event page.
#[derive(Debug, Deserialize)]
pub struct RunEventsQuery {
    /// Last sequence number already seen.
    #[serde(default)]
    after: Option<i64>,
    /// Oldest sequence number already held; returns the page before it.
    #[serde(default)]
    before: Option<i64>,
    /// Maximum rows.
    #[serde(default)]
    limit: Option<i64>,
}

/// `GET /api/runs/:id/events` - a page of the durable timeline.
///
/// With no cursor the newest page is returned, because that is what a person
/// opening a run wants to see. `before` walks backwards through a long history
/// and `after` follows it live.
pub async fn run_events(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<RunEventsQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let events = if let Some(before) = q.before {
        state.ctx.store.list_run_events_before(&id, before, limit)
    } else if let Some(after) = q.after {
        state.ctx.store.list_run_events(&id, after, limit)
    } else {
        state.ctx.store.tail_run_events(&id, limit)
    };
    match events {
        Ok(items) => {
            let first = items.first().map(|e| e.seq).unwrap_or(0);
            let last = items.last().map(|e| e.seq).unwrap_or(0);
            Json(json!({ "items": items, "first": first, "last": last, "limit": limit }))
                .into_response()
        }
        Err(e) => error_response(&e),
    }
}

/// Body of a cancellation.
#[derive(Debug, Default, Deserialize)]
pub struct CancelBody {
    /// Why work should stop.
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /api/runs/:id/cancel` - ask the executor to stop.
///
/// A run that is actually executing is only *asked* to stop: the state changes
/// when the executor observes the request and unwinds, because claiming it
/// stopped while a process is still running would be a lie. A run waiting on a
/// person has no executor, so it can be closed immediately.
pub async fn run_cancel(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let body = parse::<CancelBody>(&body).unwrap_or_default();
    let by = state.ctx.actor.label();
    let reason = body.reason.as_deref().unwrap_or("cancelled in the window");
    let run = match state.ctx.store.request_run_cancel(&id, &by, Some(reason)) {
        Ok(run) => run,
        Err(e) => return error_response(&e),
    };
    let idle = !matches!(run.typed_state(), RunState::Executing | RunState::Verifying);
    if idle {
        let _ = state
            .ctx
            .store
            .transition_run(&id, RunState::Cancelled, Some(reason));
        for a in state
            .ctx
            .store
            .list_approvals_for_run(&id, 200)
            .unwrap_or_default()
        {
            if a.state == "pending" {
                let _ = state
                    .ctx
                    .store
                    .decline_approval(&a.id, &by, Some("the run was cancelled"));
            }
        }
        if let Some(thread) = &run.thread_id {
            let _ = state.ctx.store.set_thread_state(thread, "idle", None);
        }
    }
    match state.ctx.store.run_by_id(&id) {
        Ok(Some(run)) => Json(json!({ "run": run, "requested": true })).into_response(),
        _ => error_response(&DevdError::internal("the run disappeared")),
    }
}

/// `POST /api/runs/:id/retry` - continue a stopped run through the agent hub.
pub async fn run_retry(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    // The hub owns the conversation loop, so the retry is a capability like any
    // other, not a second implementation in the HTTP layer.
    let invoke = InvokeRequest {
        id: "run.retry".into(),
        args: json!({ "run_id": id }),
        dry_run: false,
    };
    let st = state.clone();
    let out =
        tokio::task::spawn_blocking(move || st.registry.dispatch(&st.ctx, &invoke, Approval::None))
            .await;
    match out {
        Ok(Ok(result)) => Json(json!({ "outcome": result.outcome })).into_response(),
        Ok(Err(e)) => error_response(&e),
        Err(e) => error_response(&DevdError::internal(format!("the retry panicked: {e}"))),
    }
}

/// `GET /api/approvals` - every decision waiting for a person.
pub async fn approvals(State(state): State<Arc<AppState>>) -> Response {
    match state.approvals.pending() {
        Ok(items) => Json(json!({ "items": items })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `POST /api/approvals/:id/approve` - decide by durable approval id.
///
/// The token endpoint remains for a caller that was handed a token. This one is
/// for a request the assistant queued, which never exposed one: the id names the
/// same row, and the row's single-use update is the authority either way.
pub async fn approval_approve(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    decide_approval(state, &id, true).await
}

/// `POST /api/approvals/:id/deny` - decline by durable approval id.
pub async fn approval_deny(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    decide_approval(state, &id, false).await
}

async fn decide_approval(state: Arc<AppState>, id: &str, approve: bool) -> Response {
    // A conversation request is resumed by the hub, which knows how to put the
    // result back into the model's turn. Reuse its capability rather than
    // duplicating that logic here.
    if let Ok(Some(req)) = state.ctx.store.request_by_approval(id) {
        let invoke = InvokeRequest {
            id: "approval.decide".into(),
            args: json!({ "id": req.id, "approve": approve }),
            dry_run: false,
        };
        let st = state.clone();
        let out = tokio::task::spawn_blocking(move || {
            st.registry.dispatch(&st.ctx, &invoke, Approval::None)
        })
        .await;
        return match out {
            Ok(Ok(result)) => Json(json!({ "outcome": result.outcome })).into_response(),
            Ok(Err(e)) => error_response(&e),
            Err(e) => error_response(&DevdError::internal(format!("the decision panicked: {e}"))),
        };
    }

    let record = if approve {
        match state.ctx.store.consume_approval_by_id(id) {
            Ok(r) => r,
            Err(e) => return error_response(&e),
        }
    } else {
        match state.ctx.store.decline_approval(
            id,
            &state.ctx.actor.label(),
            Some("declined in the window"),
        ) {
            Ok(r) => r,
            Err(e) => return error_response(&e),
        }
    };
    let mut ctx = state.ctx.clone();
    if let Some(run_id) = &record.run_id {
        if approve {
            let _ = state
                .ctx
                .store
                .transition_run(run_id, RunState::Executing, None);
            ctx.run = Some(RunContext::new(run_id.clone()).with_store(state.ctx.store.clone()));
        } else {
            let _ = state.ctx.store.transition_run(
                run_id,
                RunState::Cancelled,
                Some("declined by a person"),
            );
        }
    }
    let req = InvokeRequest {
        id: record.capability.clone(),
        args: record.args.clone(),
        dry_run: false,
    };
    let approval = if approve {
        Approval::Granted
    } else {
        Approval::Denied
    };
    let st = state.clone();
    let approval_id = record.id.clone();
    let out = tokio::task::spawn_blocking(move || {
        let opts = DispatchOptions {
            approval_id: Some(&approval_id),
            ..DispatchOptions::default()
        };
        let result = st.registry.dispatch_with(&ctx, &req, approval, &opts);
        if approve {
            let stored = result
                .as_ref()
                .ok()
                .and_then(|r| serde_json::to_value(r).ok());
            let error = result.as_ref().err().map(|e| e.to_string());
            let _ = st
                .ctx
                .store
                .finish_approval(&approval_id, stored.as_ref(), error.as_deref());
        }
        result
    })
    .await;
    match out {
        Ok(Ok(result)) => Json(json!({ "outcome": result.outcome })).into_response(),
        Ok(Err(e)) => error_response(&e),
        Err(e) => error_response(&DevdError::internal(format!("the decision panicked: {e}"))),
    }
}

/// Query for the audit list.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// Maximum rows to return.
    #[serde(default)]
    limit: Option<u32>,
}

/// `GET /api/audit` - the most recent capability attempts.
pub async fn audit(State(state): State<Arc<AppState>>, Query(q): Query<AuditQuery>) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_AUDIT);
    match state.ctx.store.list_audit(limit) {
        Ok(rows) => Json(json!({ "items": rows })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `GET /api/doctor` - the environment check, shared with `devd doctor`.
pub async fn doctor(State(state): State<Arc<AppState>>) -> Response {
    let checks = state.checks();
    let ok = !checks.iter().any(|c| c.is_fail());
    Json(json!({ "ok": ok, "checks": checks })).into_response()
}

/// `POST /api/shutdown` - stop the server.
///
/// Needed because closing the application window closes a browser, not a
/// process: without this the user would have to find the terminal they launched
/// `devd` from, which is the kind of papercut that makes a tool feel unfinished.
pub async fn shutdown(State(state): State<Arc<AppState>>) -> Response {
    state.request_shutdown();
    Json(json!({ "stopping": true })).into_response()
}

/// Start the durable run for one HTTP invocation.
///
/// The run starts executing immediately; the caller may move it to
/// `awaiting_approval` if the capability needs a person first. Failure to
/// persist a run must not stop the invocation: the audit trail still records the
/// call, and a missing timeline entry is better than a broken deployment hook.
fn begin_run(state: &Arc<AppState>, goal: &str) -> Option<(Ctx, String)> {
    let actor = state.ctx.actor.label();
    let run = state
        .ctx
        .store
        .start_run(RunSpec {
            goal,
            mode: RunMode::Api,
            project: None,
            actor: &actor,
            thread_id: None,
            team_run_id: None,
            acceptance: &[],
        })
        .ok()?;
    let _ = state
        .ctx
        .store
        .transition_run(&run.id, RunState::Executing, None);
    let mut ctx = state.ctx.clone();
    ctx.run = Some(RunContext::new(run.id.clone()).with_store(state.ctx.store.clone()));
    Some((ctx, run.id))
}

/// Decide how a run ended, from the evidence the registry recorded.
///
/// A failed invocation is a failed run. A successful one is only `completed`
/// when its completion policy is satisfied; otherwise it is `unverified`, which
/// is the honest word for "the call returned but nothing proved the goal".
fn settle(store: &Store, run_id: Option<&str>, result: &devd_core::Result<InvokeResult>) {
    let Some(run_id) = run_id else { return };
    let Some(run) = store.run_by_id(run_id).ok().flatten() else {
        return;
    };
    let (next, error) = match result {
        Ok(_) => {
            let evidence = store.list_evidence(run_id, 10_000).unwrap_or_default();
            (evaluate(&run.completion_policy, &evidence).state, None)
        }
        Err(e) => (RunState::Failed, Some(e.to_string())),
    };
    let _ = store.transition_run(run_id, next, error.as_deref());
}

/// Compute a plan for a capability, tolerating one that cannot plan.
fn plan_for(state: &Arc<AppState>, id: &str, args: &Value) -> Option<devd_core::Plan> {
    let req = InvokeRequest {
        id: id.to_string(),
        args: args.clone(),
        dry_run: true,
    };
    state
        .registry
        .dispatch(&state.ctx, &req, Approval::None)
        .ok()
        .and_then(|r| r.plan)
}

/// Parse a request body, treating an empty body as an empty object.
///
/// Returning `None` rather than an error keeps the "malformed body" response in
/// the same envelope as every other failure, so a client has exactly one error
/// shape to handle.
fn parse<T: for<'de> Deserialize<'de>>(raw: &str) -> Option<T> {
    let text = if raw.trim().is_empty() { "{}" } else { raw };
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_body_parses_as_an_empty_object() {
        let parsed: Option<Value> = parse("");
        assert_eq!(parsed, Some(json!({})));
    }

    #[test]
    fn a_malformed_body_is_not_an_error_type() {
        let parsed: Option<TokenBody> = parse("{not json");
        assert!(parsed.is_none());
    }

    #[test]
    fn a_token_body_round_trips() {
        let parsed: Option<TokenBody> = parse(r#"{"token":"abc"}"#);
        assert_eq!(parsed.unwrap().token, "abc");
    }
}

// ----------------------------------------------------------------- projects

#[derive(Deserialize)]
pub struct ProjectQuery {
    project: String,
}

/// `GET /api/project` - the reviewable manifest plan.
pub async fn project_plan(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectQuery>,
) -> Response {
    match state.workspace.projects().plan(&q.project) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `GET /api/project/status`.
pub async fn project_status(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ProjectQuery>,
) -> Response {
    let workspace = state.workspace.clone();
    let project = q.project;
    match tokio::task::spawn_blocking(move || workspace.projects().status(&project)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => error_response(&e),
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("the status task failed: {e}"),
        ),
    }
}

fn dispatch_project(state: &Arc<AppState>, id: &str, args: Value) -> Response {
    let req = InvokeRequest {
        id: id.to_string(),
        args,
        dry_run: false,
    };
    match state.registry.dispatch(&state.ctx, &req, Approval::None) {
        Ok(result) => Json(json!({
            "capability": result.capability,
            "outcome": result.outcome,
        }))
        .into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct ProjectBody {
    project: String,
}

/// `POST /api/project/approve`.
pub async fn project_approve(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProjectBody>,
) -> Response {
    dispatch_project(
        &state,
        "project.approve",
        json!({ "project": body.project }),
    )
}

#[derive(Deserialize)]
pub struct ProcessBody {
    project: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    all: bool,
}

/// `POST /api/project/start`.
pub async fn project_start(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProcessBody>,
) -> Response {
    dispatch_project(
        &state,
        "project.start",
        json!({ "project": body.project, "name": body.name, "all": body.all }),
    )
}

/// `POST /api/project/stop`.
pub async fn project_stop(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProcessBody>,
) -> Response {
    dispatch_project(
        &state,
        "project.stop",
        json!({ "project": body.project, "name": body.name, "all": body.all }),
    )
}

/// `POST /api/project/restart`.
pub async fn project_restart(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ProcessBody>,
) -> Response {
    dispatch_project(
        &state,
        "project.restart",
        json!({ "project": body.project, "name": body.name }),
    )
}

#[derive(Deserialize)]
pub struct LogQuery {
    project: String,
    name: String,
    lines: Option<usize>,
    query: Option<String>,
}

/// `GET /api/project/logs`.
pub async fn project_logs(
    State(state): State<Arc<AppState>>,
    Query(q): Query<LogQuery>,
) -> Response {
    match state.workspace.projects().logs(
        &q.project,
        &q.name,
        q.lines.unwrap_or(200),
        q.query.as_deref(),
    ) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
    }
}

// ------------------------------------------------------------------ system

#[derive(Deserialize)]
pub struct OpenUrlBody {
    pub url: String,
}

/// `POST /api/open-url` - hand a validated HTTP(S) URL to the operating system.
///
/// The native shell cannot rely on `window.open`: a Tauri window does not own
/// the user's browser. Handing the URL to the server keeps one implementation
/// for both surfaces, and keeps the shell's navigation rule (only devd itself
/// stays in the window). Only HTTP(S) is accepted; `file:`, `javascript:` and
/// custom schemes never reach the OS launcher.
pub async fn open_url(Json(body): Json<OpenUrlBody>) -> Response {
    if body.url.len() > 4096 {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_url",
            "the URL is too long",
        );
    }
    let parsed = match url::Url::parse(&body.url) {
        Ok(url) => url,
        Err(e) => {
            return refuse(
                StatusCode::BAD_REQUEST,
                "invalid_url",
                &format!("not a valid URL: {e}"),
            )
        }
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_url",
            "only http and https URLs can be opened",
        );
    }
    match open::that(parsed.as_str()) {
        Ok(()) => Json(json!({ "opened": true })).into_response(),
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "open_failed",
            &format!("the operating system could not open the URL: {e}"),
        ),
    }
}

// ------------------------------------------------------------------ uploads

/// The largest upload devd accepts.
///
/// A recording or a screenshot is a megabyte or two. A file that is much
/// larger is not something a person meant to attach to a message, and refusing
/// it is better than filling the disk.
const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

/// `POST /api/upload` - store a file the page wants to send.
///
/// The browser cannot hand a file to a capability through JSON, so the file
/// arrives here first and is stored under the devd home. What comes back is a
/// path, and that path is what the capabilities take.
///
/// The file name is reduced to its last component and its characters are
/// limited, so a name cannot be used to write outside the upload directory.
pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if body.is_empty() {
        return refuse(
            StatusCode::BAD_REQUEST,
            "empty_upload",
            "the file was empty",
        );
    }
    if body.len() > MAX_UPLOAD_BYTES {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload_too_large",
            &format!(
                "the file is larger than {} MiB",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
        );
    }

    let name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(safe_file_name)
        .unwrap_or_else(|| "upload".to_string());
    let extension = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let stored = format!("{}.{}", upload_id(), extension);

    let dir = state.ctx.paths.root().join("uploads");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "upload_failed",
            &format!("the upload directory could not be created: {e}"),
        );
    }
    let path = dir.join(&stored);
    if let Err(e) = std::fs::write(&path, &body) {
        return refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "upload_failed",
            &format!("the file could not be stored: {e}"),
        );
    }

    Json(json!({
        "path": path.display().to_string(),
        "name": name,
        "bytes": body.len(),
    }))
    .into_response()
}

/// A file name that cannot reach outside the upload directory.
fn safe_file_name(name: &str) -> String {
    let last = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("upload")
        .trim()
        .to_string();
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "upload".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

/// A name that will not collide with another upload.
fn upload_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{:x}", nanos, std::process::id())
}

// ------------------------------------------------------------------- picker

#[derive(Deserialize)]
pub struct PickFolderBody {
    /// Heading for the dialog.
    #[serde(default)]
    pub title: Option<String>,
    /// Directory the dialog should open in.
    #[serde(default)]
    pub start: Option<String>,
}

/// `POST /api/pick-folder` - ask the person for a folder with the operating
/// system's own dialog.
///
/// The desktop shell owns a native picker; a `devd ui` in a browser has none,
/// because a web page cannot open one. Rather than pretend, the endpoint says
/// so with `501`, and the web application falls back to the folder browser it
/// draws itself from `fs.browse`.
///
/// The path that comes back is treated as untrusted: it is a string, and
/// everything that acts on it goes through a capability that inspects it
/// first.
pub async fn pick_folder(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PickFolderBody>,
) -> Response {
    let picker = match &state.picker {
        Some(p) => p.clone(),
        None => {
            return refuse(
                StatusCode::NOT_IMPLEMENTED,
                "no_native_picker",
                "this surface has no native folder picker; use the built-in folder browser",
            )
        }
    };
    let title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Choose a folder")
        .to_string();
    if title.chars().count() > 200 {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_title",
            "the dialog title is too long",
        );
    }
    let start = body
        .start
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from);

    // The dialog blocks until the person answers, so it must not occupy a
    // worker thread that other requests need.
    let chosen = match tokio::task::spawn_blocking(move || picker.pick(&title, start)).await {
        Ok(chosen) => chosen,
        Err(e) => {
            return refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "picker_failed",
                &format!("the folder picker could not be shown: {e}"),
            )
        }
    };
    match chosen {
        Some(path) => Json(json!({ "path": path.to_string_lossy() })).into_response(),
        None => Json(json!({ "path": Value::Null })).into_response(),
    }
}
