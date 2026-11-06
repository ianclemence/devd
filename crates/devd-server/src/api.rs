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
    DevdError, DispatchOptions, InvokeResult, RunContext, RunMode, RunState,
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
/// Read a dotted path out of a JSON body: `repository.full_name`.
fn hook_pick<'a>(body: &'a Value, path: &str) -> Option<&'a Value> {
    let mut at = body;
    for part in path.split('.') {
        at = at.get(part)?;
    }
    Some(at)
}

/// Whether a hook's stored filter accepts this body and event.
fn hook_matches(filter: &Value, body: &Value, event: &str) -> bool {
    let Some(obj) = filter.as_object() else { return true };
    if let Some(want) = obj.get("event").and_then(Value::as_str) {
        if want != event {
            return false;
        }
    }
    if let Some(actions) = obj.get("actions").and_then(Value::as_array) {
        let actual = body.get("action").and_then(Value::as_str).unwrap_or("");
        if !actions.iter().any(|a| a.as_str() == Some(actual)) {
            return false;
        }
    }
    if let Some(repo) = obj.get("repo").and_then(Value::as_str) {
        if hook_pick(body, "repository.full_name").and_then(Value::as_str) != Some(repo) {
            return false;
        }
    }
    // A branch matches a pull request's target or source branch, or a push's ref.
    if let Some(branch) = obj.get("branch").and_then(Value::as_str) {
        let pushed = hook_pick(body, "ref")
            .and_then(Value::as_str)
            .map(|r| r.trim_start_matches("refs/heads/"));
        let hit = [
            hook_pick(body, "pull_request.base.ref").and_then(Value::as_str),
            hook_pick(body, "pull_request.head.ref").and_then(Value::as_str),
            pushed,
        ]
        .into_iter()
        .flatten()
        .any(|b| b == branch);
        if !hit {
            return false;
        }
    }
    if let Some(equals) = obj.get("equals").and_then(Value::as_object) {
        for (path, want) in equals {
            if hook_pick(body, path) != Some(want) {
                return false;
            }
        }
    }
    true
}

/// Replace `{{dotted.path}}` in a stored action string with a body value.
fn hook_fill(template: &str, body: &Value, event: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        let value = if key == "event" {
            Some(Value::String(event.to_string()))
        } else {
            hook_pick(body, key).cloned()
        };
        match value {
            Some(Value::String(s)) => out.push_str(&s),
            Some(Value::Null) | None => out.push_str("(none)"),
            Some(other) => out.push_str(&other.to_string()),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// `POST /hooks/task/:token` - an incoming event from a service that cannot hold
/// a bearer token. The token names a stored rule, so this is a caller with no
/// person behind it and no new permission path: the action goes through the
/// ordinary registry.
pub async fn task_hook(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    // GitHub names the event in a header ("pull_request", "push"); other senders put it
    // in the body. Either way it is matched against the hook's stored event.
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .map(|e| format!("github.{e}"))
        .or_else(|| body.get("event").and_then(Value::as_str).map(str::to_string))
        .or_else(|| body.get("_event").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let st = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let rule = st
            .ctx
            .store
            .automation_by_token(&token)
            .ok()
            .flatten();
        let Some(rule) = rule else {
            return (
                StatusCode::NOT_FOUND,
                json!({ "ok": false, "error": "no hook has that token" }),
            );
        };
        if !rule.enabled {
            return (
                StatusCode::FORBIDDEN,
                json!({ "ok": false, "error": "that hook is switched off" }),
            );
        }
        let event = if event.is_empty() {
            // GitHub sends its event in a header; the body still names the
            // action. The stored event is the contract, so use it.
            rule.event.clone()
        } else {
            event
        };
        if event != rule.event {
            return (
                StatusCode::ACCEPTED,
                json!({ "ok": true, "ignored": "event does not match this hook" }),
            );
        }
        if !hook_matches(&rule.filter, &body, &event) {
            return (
                StatusCode::ACCEPTED,
                json!({ "ok": true, "ignored": "the filter did not match" }),
            );
        }
        let kind = rule.action["type"].as_str().unwrap_or("");
        let (capability, args) = match kind {
            "team" => {
                let goal = hook_fill(rule.action["goal"].as_str().unwrap_or(""), &body, &event);
                let mut args = json!({ "goal": goal });
                if let Some(p) = rule.action["project"].as_str().filter(|p| !p.trim().is_empty()) {
                    args["project"] = json!(p);
                }
                if let Some(list) = rule.action["done_when"].as_array() {
                    args["done_when"] = json!(list.iter().filter_map(Value::as_str).map(|s| hook_fill(s, &body, &event)).collect::<Vec<_>>());
                }
                ("team.start", args)
            }
            "chat" => {
                let message = hook_fill(rule.action["message"].as_str().unwrap_or(""), &body, &event);
                let mut args = json!({ "message": message });
                if let Some(t) = rule.action["thread"].as_str() {
                    args["thread"] = json!(t);
                }
                ("chat.send", args)
            }
            "inbox" => {
                let subject = hook_fill(rule.action["subject"].as_str().unwrap_or("Incoming event"), &body, &event);
                let text = hook_fill(rule.action["body"].as_str().unwrap_or(""), &body, &event);
                let mut args = json!({ "subject": subject, "body": text });
                if let Some(p) = rule.action["project"].as_str().filter(|p| !p.trim().is_empty()) {
                    args["project"] = json!(p);
                }
                ("inbox.post", args)
            }
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    json!({ "ok": false, "error": "the hook action is not understood" }),
                )
            }
        };
        let mut ctx: Ctx = st.ctx.clone();
        ctx.actor = devd_core::capability::Actor::system();
        ctx.run = None;
        let req = InvokeRequest { id: capability.into(), args, dry_run: false };
        let dispatch = st
            .registry
            .dispatch_with(&ctx, &req, Approval::None, &DispatchOptions::default());
        match dispatch {
            Ok(result) => {
                let _ = st.ctx.store.mark_automation(&rule.name, None);
                let data = result.outcome.as_ref().map(|o| o.data.clone()).unwrap_or(Value::Null);
                (
                    StatusCode::ACCEPTED,
                    json!({ "ok": true, "hook": rule.name, "action": kind, "result": data }),
                )
            }
            Err(e) => {
                let why = e.to_string();
                let _ = st.ctx.store.mark_automation(&rule.name, Some(&why));
                (
                    StatusCode::BAD_GATEWAY,
                    json!({ "ok": false, "hook": rule.name, "error": why }),
                )
            }
        }
    })
    .await;
    match result {
        Ok((status, body)) => (status, axum::Json(body)).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({ "ok": false, "error": "the hook could not run" })),
        )
            .into_response(),
    }
}

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

/// Most of a diff the run sheet is sent. The rest stays in the repository.
const CHANGES_LIMIT: usize = 120_000;

/// `GET /api/runs/:id/changes` - what the run left in its project, and the
/// environment it was working in.
///
/// Both come from read capabilities, through the registry like everything else:
/// the working-tree diff of the project's repository, and the project's
/// environment manifest (runtimes, services, ports, sites, with a fingerprint).
/// Neither is a claim the run made; they are the project as it is now, which is
/// what a person deciding whether to trust the run needs next to its evidence.
/// Every part can be absent - a project that is not a git repository has no
/// diff - and says why instead of failing the sheet.
pub async fn run_changes(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let st = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let run = st
            .ctx
            .store
            .run_by_id(&id)?
            .ok_or_else(|| devd_core::DevdError::not_found(format!("no run `{id}`")))?;
        let Some(project) = run.project.clone() else {
            return Ok(json!({ "project": null, "note": "this run is not tied to a project" }));
        };
        let call = |capability: &str, args: Value| -> Option<devd_core::Result<Value>> {
            let req = InvokeRequest {
                id: capability.into(),
                args,
                dry_run: false,
            };
            st.registry.get(capability)?;
            Some(
                st.registry
                    .dispatch(&st.ctx, &req, Approval::None)
                    .map(|r| r.outcome.map(|o| o.data).unwrap_or(Value::Null)),
            )
        };

        let mut out = json!({ "project": project });
        match call("git.diff", json!({ "repo": project, "context": 3 })) {
            Some(Ok(data)) => {
                let patch = data["patch"].as_str().unwrap_or("");
                let truncated = patch.len() > CHANGES_LIMIT;
                let mut end = patch.len().min(CHANGES_LIMIT);
                while !patch.is_char_boundary(end) {
                    end -= 1;
                }
                out["diff"] = json!({ "patch": &patch[..end], "bytes": patch.len(), "truncated": truncated });
            }
            Some(Err(e)) => out["diff_error"] = json!(e.to_string()),
            None => out["diff_error"] = json!("git is not available in this build"),
        }
        match call("environment.manifest", json!({ "path": project })) {
            Some(Ok(data)) => out["environment"] = data,
            Some(Err(e)) => out["environment_error"] = json!(e.to_string()),
            None => {}
        }
        // The observed changes are evidence; surface them beside the diff so the
        // two can be read together.
        let observed: Vec<Value> = st
            .ctx
            .store
            .list_evidence(&id, 1000)?
            .into_iter()
            .filter(|e| e.capability == devd_core::runs::OBSERVED_CAPABILITY)
            .map(|e| json!({ "summary": e.summary, "detail": e.detail, "at": e.created_at }))
            .collect();
        out["observed"] = json!(observed);
        Ok::<Value, devd_core::DevdError>(out)
    })
    .await;
    match result {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(e)) => error_response(&e),
        Err(_) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "reading the run's changes failed",
        ),
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
        .start_standalone_run(RunMode::Api, goal, &actor)
        .ok()?;
    let mut ctx = state.ctx.clone();
    ctx.run = Some(RunContext::new(run.id.clone()).with_store(state.ctx.store.clone()));
    Some((ctx, run.id))
}

/// Decide how a run ended, from the evidence the registry recorded.
fn settle(store: &Store, run_id: Option<&str>, result: &devd_core::Result<InvokeResult>) {
    let Some(run_id) = run_id else { return };
    match result {
        Ok(_) => store.conclude_run(run_id, None),
        Err(e) => store.conclude_run(run_id, Some(&e.to_string())),
    }
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
    #[test]
    fn a_hook_filter_matches_the_branch_a_pull_request_targets_or_a_push_updates() {
        use serde_json::json;
        let pr = json!({ "action": "opened", "repository": { "full_name": "acme/shop" },
            "pull_request": { "html_url": "https://github.com/acme/shop/pull/7", "base": { "ref": "main" }, "head": { "ref": "fix-cart" } } });
        let filter = json!({ "actions": ["opened", "synchronize"], "repo": "acme/shop", "branch": "main" });
        assert!(super::hook_matches(&filter, &pr, "github.pull_request"));
        assert!(!super::hook_matches(&json!({ "actions": ["closed"] }), &pr, "github.pull_request"));
        assert!(!super::hook_matches(&json!({ "repo": "acme/other" }), &pr, "github.pull_request"));
        let push = json!({ "ref": "refs/heads/main", "repository": { "full_name": "acme/shop" } });
        assert!(super::hook_matches(&json!({ "branch": "main" }), &push, "github.push"));
        assert!(!super::hook_matches(&json!({ "branch": "dev" }), &push, "github.push"));
        assert_eq!(
            super::hook_fill("Review {{pull_request.html_url}} ({{event}})", &pr, "github.pull_request"),
            "Review https://github.com/acme/shop/pull/7 (github.pull_request)"
        );
    }

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

// ------------------------------------------------------------------ previews

/// `POST /api/preview` - check an address before the window puts it in a frame.
///
/// The window renders a preview in an `<iframe>`, and a frame is a place a page
/// can run script. The rule for what may go in one lives in
/// `devd_core::preview` so there is one copy of it; this handler only carries the
/// answer to the window. An address that is not on this machine is refused here,
/// not in the page's JavaScript, because the page is not the thing being trusted.
pub async fn preview(Json(body): Json<OpenUrlBody>) -> Response {
    match devd_core::preview::previewable(&body.url) {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(why) => refuse(StatusCode::BAD_REQUEST, "invalid_url", &why),
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
    let mut extension = std::path::Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    // What the file is is decided by its first bytes, not by its name. A program is not
    // something to hand to an assistant, and a "picture" that is not one is refused
    // rather than stored under a name that says otherwise.
    if is_program(&body) {
        return refuse(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "upload_program",
            "program files cannot be attached. Attach the source, or a description of what it does",
        );
    }
    let picture = picture_type(&body);
    if let Some((_, right)) = picture {
        extension = right.to_string();
    } else if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return refuse(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "upload_not_a_picture",
            "this file is named like a picture but is not one",
        );
    }
    let stored = if extension.is_empty() { upload_id() } else { format!("{}.{}", upload_id(), extension) };

    let dir = state.ctx.paths.root().join("uploads");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "upload_failed",
            &format!("the upload directory could not be created: {e}"),
        );
    }
    prune_uploads(&dir, UPLOAD_KEEP_DAYS);
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
        "id": stored,
        "mime": picture.map(|(mime, _)| mime),
    }))
    .into_response()
}

/// How long an upload is kept. Files a conversation still names keep working for this long;
/// after it the file is gone and the conversation still reads fine, because it only ever
/// held the path.
const UPLOAD_KEEP_DAYS: u64 = 30;

/// Remove uploads older than `days`. Called when a new one arrives, so a folder nobody adds
/// to is not touched and there is no timer to run.
fn prune_uploads(dir: &std::path::Path, days: u64) {
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(days * 86_400)) else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).is_some_and(|t| t < cutoff);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A program (Windows, Linux or macOS) by its first bytes.
fn is_program(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ")
        || bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
        || bytes.starts_with(&[0xCA, 0xFE, 0xBA, 0xBE])
}

/// A raster picture by its first bytes: its type and the extension that belongs to it.
fn picture_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// `GET /api/upload/:id` - read back a file the page uploaded.
///
/// A picture comes back as a picture, so a thumbnail survives a reload and a conversation
/// can show what it was sent. Anything else is a download, never something the browser
/// renders, and nothing is ever sniffed into another type. Only a name `upload` itself made
/// is accepted, so this cannot reach anything else on the disk.
pub async fn upload_get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let plain = !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !id.starts_with('.');
    if !plain {
        return refuse(StatusCode::BAD_REQUEST, "bad_upload_name", "that is not an upload name");
    }
    let path = state.ctx.paths.root().join("uploads").join(&id);
    let Ok(bytes) = std::fs::read(&path) else {
        return refuse(StatusCode::NOT_FOUND, "upload_gone", "that file is no longer stored (uploads are kept for 30 days)");
    };
    let (kind, disposition) = match picture_type(&bytes) {
        Some((mime, _)) => (mime, "inline"),
        None => ("application/octet-stream", "attachment"),
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, kind.to_string()),
            (axum::http::header::CONTENT_DISPOSITION, disposition.to_string()),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (axum::http::header::CACHE_CONTROL, "private, max-age=3600".to_string()),
        ],
        bytes,
    )
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
