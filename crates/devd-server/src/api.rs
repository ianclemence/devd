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

use devd_core::capability::{Approval, InvokeRequest};
use devd_core::DevdError;

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
        "undo": cap.undo,
    })
}

/// `GET /api/capabilities` - ranked search over the registry.
pub async fn capabilities(State(state): State<Arc<AppState>>, Query(q): Query<CapQuery>) -> Response {
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

    Json(json!({ "total": total, "offset": offset, "limit": limit, "items": items })).into_response()
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
        return match state.registry.dispatch(&state.ctx, &req, Approval::None) {
            Ok(result) => Json(json!({
                "capability": result.capability,
                "dry_run": result.dry_run,
                "plan": result.plan,
                "outcome": result.outcome,
                "approval_required": false,
            }))
            .into_response(),
            Err(e) => error_response(&e),
        };
    }

    // The plan is computed here, before a token exists, so that what the human
    // is shown is produced by the capability itself rather than described by the
    // UI. A capability that cannot plan simply has none to show.
    let plan = plan_for(&state, &body.id, &body.args);

    match state
        .approvals
        .issue(&state.ctx, &body.id, &body.args, cap.tier.as_str())
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

    match state.registry.dispatch(&state.ctx, &req, Approval::Granted) {
        Ok(result) => Json(json!({
            "capability": result.capability,
            "approved_by": record.actor,
            "args_hash": record.args_hash,
            "outcome": result.outcome,
        }))
        .into_response(),
        Err(e) => error_response(&e),
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

    let record = match state.approvals.take(&body.token) {
        Ok(record) => record,
        Err(e) => return error_response(&e),
    };

    let req = InvokeRequest {
        id: record.capability.clone(),
        args: record.args.clone(),
        dry_run: false,
    };
    let _ = state.registry.dispatch(&state.ctx, &req, Approval::Denied);

    Json(json!({
        "declined": true,
        "capability": record.capability,
        "args_hash": record.args_hash,
    }))
    .into_response()
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

/// Re-exported so tests can assert on the status a `devd` error maps to.
pub fn status_for(err: &DevdError) -> StatusCode {
    match err {
        DevdError::NotFound(_) => StatusCode::NOT_FOUND,
        DevdError::Invalid(_) | DevdError::Unsupported(_) => StatusCode::BAD_REQUEST,
        DevdError::Conflict(_) => StatusCode::CONFLICT,
        DevdError::Denied(_) | DevdError::ApprovalRequired { .. } => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
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

// ---------------------------------------------------------------- terminals

/// `GET /api/terminal/harnesses` - the coding-agent CLIs found on this machine.
pub async fn terminal_harnesses() -> Response {
    Json(crate::terminal::harnesses()).into_response()
}

#[derive(Deserialize)]
pub struct TerminalStart {
    pub kind: String,
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

/// `POST /api/terminal/start` - run a harness in a real PTY.
pub async fn terminal_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalStart>,
) -> Response {
    let cols = req.cols.unwrap_or(100);
    let rows = req.rows.unwrap_or(30);
    let kind = req.kind;
    let cwd = req.cwd;
    let task = state.clone();
    match tokio::task::spawn_blocking(move || {
        task.terminals.start(&kind, cwd.as_deref(), cols, rows)
    })
    .await
    {
        Ok(Ok(id)) => Json(json!({ "id": id })).into_response(),
        Ok(Err(message)) => refuse(StatusCode::BAD_REQUEST, "terminal", &message),
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("the terminal task failed: {e}"),
        ),
    }
}

#[derive(Deserialize)]
pub struct TerminalOutput {
    pub id: String,
    pub cursor: Option<u64>,
}

/// `POST /api/terminal/output` - bytes since the caller's cursor, base64.
pub async fn terminal_output(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalOutput>,
) -> Response {
    match state.terminals.output(&req.id, req.cursor.unwrap_or(0)) {
        Ok(value) => Json(value).into_response(),
        Err(message) => refuse(StatusCode::NOT_FOUND, "not_found", &message),
    }
}

#[derive(Deserialize)]
pub struct TerminalInput {
    pub id: String,
    pub data: String,
}

/// `POST /api/terminal/input` - keystrokes or pasted text.
pub async fn terminal_input(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalInput>,
) -> Response {
    match state.terminals.write(&req.id, &req.data) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(message) => refuse(StatusCode::NOT_FOUND, "not_found", &message),
    }
}

#[derive(Deserialize)]
pub struct TerminalResize {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `POST /api/terminal/resize` - keep curses programs drawing correctly.
pub async fn terminal_resize(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalResize>,
) -> Response {
    match state.terminals.resize(&req.id, req.cols, req.rows) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(message) => refuse(StatusCode::NOT_FOUND, "not_found", &message),
    }
}

#[derive(Deserialize)]
pub struct TerminalStop {
    pub id: String,
}

/// `POST /api/terminal/stop` - kill the harness and forget the terminal.
pub async fn terminal_stop(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalStop>,
) -> Response {
    match state.terminals.stop(&req.id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(message) => refuse(StatusCode::NOT_FOUND, "not_found", &message),
    }
}
