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
        "undo": cap.undo,
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
        let dispatched = tokio::task::spawn_blocking(move || st.registry.dispatch(&st.ctx, &req, Approval::None)).await;
        let dispatched = match dispatched {
            Ok(r) => r,
            Err(e) => return error_response(&DevdError::internal(format!("the operation panicked: {e}"))),
        };
        return match dispatched {
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

    let st = state.clone();
    let dispatched = match tokio::task::spawn_blocking(move || st.registry.dispatch(&st.ctx, &req, Approval::Granted)).await {
        Ok(r) => r,
        Err(e) => return error_response(&DevdError::internal(format!("the operation panicked: {e}"))),
    };
    match dispatched {
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
//
// Polling output, sending keystrokes and resizing are deliberately not
// capability calls: at 80ms a poll would generate an audit row per frame, which
// is noise rather than accountability. They are still behind the same bearer
// token, host allow-list and origin check as every other endpoint. Every other
// mutating operation - stop, rename, dismiss, cancel - is dispatched through
// the registry, so its tier is enforced and the attempt is audited.

/// `GET /api/sessions/harnesses` - the coding-agent CLIs found on this machine.
pub async fn session_harnesses() -> Response {
    Json(devd_workspace::session::harnesses()).into_response()
}

#[derive(Deserialize)]
pub struct TerminalStart {
    pub kind: String,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub label: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub command: Option<Vec<String>>,
}

/// Start a session through `session.start`, so the start is audited and the
/// tier is enforced like any other launch. Returns the session view as JSON.
fn start_session(state: &Arc<AppState>, req: TerminalStart) -> Result<Value, String> {
    let cwd = req
        .cwd
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .ok_or_else(|| "a working directory is required".to_string())?;
    let request = InvokeRequest {
        id: "session.start".to_string(),
        args: json!({
            "harness": req.kind,
            "cwd": cwd,
            "project": req.project,
            "label": req.label,
            "cols": req.cols,
            "rows": req.rows,
            "command": req.command,
        }),
        dry_run: false,
    };
    state
        .registry
        .dispatch(&state.ctx, &request, Approval::None)
        .map_err(|e| e.to_string())?
        .outcome
        .map(|outcome| outcome.data)
        .ok_or_else(|| "session.start returned no session".to_string())
}

/// `POST /api/sessions` - start a session and return its durable record.
pub async fn sessions_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TerminalStart>,
) -> Response {
    let task = state.clone();
    match tokio::task::spawn_blocking(move || start_session(&task, req)).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(message)) => refuse(StatusCode::BAD_REQUEST, "terminal", &message),
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("the terminal task failed: {e}"),
        ),
    }
}

#[derive(Deserialize)]
pub struct SessionQuery {
    project: Option<String>,
}

/// `GET /api/sessions` - every session, with live state folded in.
pub async fn sessions_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SessionQuery>,
) -> Response {
    match state.workspace.sessions().list(q.project.as_deref()) {
        Ok(rows) => Json(json!({ "items": rows })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `GET /api/sessions/:id`.
pub async fn session_get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.workspace.sessions().get(&id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct CursorBody {
    cursor: Option<u64>,
}

/// `POST /api/sessions/:id/attach` - session plus replay and cursor.
pub async fn session_attach(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    match state.workspace.sessions().attach(&id) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `POST /api/sessions/:id/output` - bytes since the cursor, base64.
pub async fn session_output(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let cursor = parse::<CursorBody>(&body)
        .and_then(|b| b.cursor)
        .unwrap_or(0);
    match state.workspace.sessions().output(&id, cursor) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct DataBody {
    pub data: String,
}

/// `POST /api/sessions/:id/input`.
pub async fn session_input(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DataBody>,
) -> Response {
    match state.workspace.sessions().write(&id, &body.data) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct SizeBody {
    pub cols: u16,
    pub rows: u16,
}

/// `POST /api/sessions/:id/resize`.
pub async fn session_resize(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<SizeBody>,
) -> Response {
    match state.workspace.sessions().resize(&id, body.cols, body.rows) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct RenameBody {
    pub label: String,
}

/// Dispatch a capability for one of the dedicated routes and answer with the
/// data the old handler returned, so a client written against the route keeps
/// working while the call is checked and audited like every other.
///
/// The operation runs with `Approval::None`: a capability that needs approval
/// refuses here with `approval_required`, and the client must use
/// `/api/invoke`, which negotiates the approval and then applies it.
async fn dispatch_route(state: Arc<AppState>, id: &'static str, args: Value) -> Response {
    let req = InvokeRequest {
        id: id.to_string(),
        args,
        dry_run: false,
    };
    let st = state.clone();
    match tokio::task::spawn_blocking(move || st.registry.dispatch(&st.ctx, &req, Approval::None))
        .await
    {
        Ok(Ok(result)) => match result.outcome {
            Some(outcome) if !outcome.data.is_null() => Json(outcome.data).into_response(),
            _ => Json(json!({ "ok": true })).into_response(),
        },
        Ok(Err(e)) => error_response(&e),
        Err(e) => error_response(&DevdError::internal(format!("the operation panicked: {e}"))),
    }
}

/// `POST /api/sessions/:id/rename`.
pub async fn session_rename(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RenameBody>,
) -> Response {
    dispatch_route(
        state,
        "session.rename",
        json!({ "id": id, "label": body.label }),
    )
    .await
}

/// `POST /api/sessions/:id/stop`.
pub async fn session_stop(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    dispatch_route(state, "session.stop", json!({ "id": id })).await
}

/// `DELETE /api/sessions/:id` - forget a finished session.
///
/// `session.dismiss` is destructive, so this route refuses with
/// `approval_required`; the window dismisses through `/api/invoke`, which shows
/// the plan and asks.
pub async fn session_dismiss(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    dispatch_route(state, "session.dismiss", json!({ "id": id })).await
}

// ------------------------------------------------------------------- tasks

#[derive(Deserialize)]
pub struct TaskQuery {
    parent_session_id: Option<String>,
    project: Option<String>,
}

/// `GET /api/tasks`.
pub async fn tasks_list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TaskQuery>,
) -> Response {
    match state
        .workspace
        .orchestrator()
        .list(q.parent_session_id.as_deref(), q.project.as_deref())
    {
        Ok(rows) => Json(json!({ "items": rows })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `GET /api/tasks/:id`.
pub async fn task_get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.workspace.store().get_agent_task(&id) {
        Ok(Some(task)) => Json(task).into_response(),
        Ok(None) => error_response(&DevdError::not_found(format!("no task `{id}`"))),
        Err(e) => error_response(&e),
    }
}

#[derive(Deserialize)]
pub struct WaitBody {
    timeout_secs: Option<u64>,
}

/// `POST /api/tasks/:id/wait` - wait asynchronously.
pub async fn task_wait(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let timeout = parse::<WaitBody>(&body).and_then(|b| b.timeout_secs);
    let workspace = state.workspace.clone();
    match tokio::task::spawn_blocking(move || workspace.orchestrator().wait(&id, timeout)).await {
        Ok(Ok(outcome)) => Json(json!({
            "task": outcome.task,
            "timed_out": outcome.timed_out,
        }))
        .into_response(),
        Ok(Err(e)) => error_response(&e),
        Err(e) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            &format!("the wait task failed: {e}"),
        ),
    }
}

/// `POST /api/tasks/:id/cancel`.
pub async fn task_cancel(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    dispatch_route(state, "agent.cancel", json!({ "task_id": id })).await
}

/// `GET /api/tasks/:id/result`.
pub async fn task_result(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.workspace.orchestrator().result(&id) {
        Ok(value) => Json(value).into_response(),
        Err(e) => error_response(&e),
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
