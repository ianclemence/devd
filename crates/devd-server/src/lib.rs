//! The `devd` application: a loopback HTTP API and the window that talks to it.
//!
//! # One more projection, not one more implementation
//!
//! This crate owns **no business logic**. Every action the application can take
//! is a capability in the shared [`Registry`], dispatched through
//! [`Registry::dispatch`], which is where tiers, approvals and auditing are
//! enforced. A button in the UI and a line in the terminal reach the same
//! function with the same permission checks, so the two cannot drift apart.
//!
//! The only decisions made here are transport decisions: who may reach the
//! socket, how an approval is negotiated over it, and how a `devd` error becomes
//! an HTTP response.
//!
//! # Threat model
//!
//! A local HTTP server is reachable by more than the person who started it:
//!
//! * **Other processes on the machine.** Anything running as this user can
//!   connect to `127.0.0.1`. On a single-user development machine that is not an
//!   escalation - the same process could read the state database directly - but
//!   it does mean the API needs authentication, not merely a private port.
//! * **Web pages the user has open.** A browser will happily issue a request to
//!   `http://127.0.0.1:<port>/` from any page the user visits. This is the
//!   interesting case, because the page is *hostile* and the user is *not*.
//! * **DNS rebinding.** A page can arrange for its own hostname to resolve to
//!   `127.0.0.1` and then call `http://its-own-name:<port>/`, which reaches us
//!   with the page's origin but our socket.
//!
//! Four defences, applied to every request before any handler runs:
//!
//! 1. A **bearer token**, 256 bits from the CSPRNG, generated at startup and
//!    injected into the served HTML. A foreign page cannot read our HTML, so it
//!    cannot learn the token. Nothing is written to disk.
//! 2. A **loopback bind**. There is no option that serves this to the network.
//! 3. A **host allow-list** - only `127.0.0.1:<port>`, `localhost:<port>` and
//!    `[::1]:<port>` are accepted, which is what defuses DNS rebinding.
//! 4. An **origin check and no CORS headers**. A request carrying a foreign
//!    `Origin` is refused outright, and `OPTIONS` is never answered, so a
//!    cross-origin caller cannot even complete a preflight.
//!
//! A request without a valid token gets `401` and changes nothing.

pub mod api;
pub mod approval;
pub mod events;
pub mod picker;
pub mod ui;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use devd_workspace::Workspace;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::watch;

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::doctor::{self, Check};
use devd_core::{DevdError, Registry, Result};

pub use approval::ApprovalStore;

/// Port the application tries first.
pub const DEFAULT_PORT: u16 = 8787;

/// Number of consecutive ports to try when the default is taken.
pub const PORT_SCAN: u16 = 10;

/// Response header that identifies a `devd` server to `devd` itself.
///
/// `devd ui` uses this to decide whether the port it wanted is held by another
/// `devd`, in which case it simply opens the existing window's URL, or by some
/// unrelated program, in which case it must not.
pub const INSTANCE_HEADER: &str = "x-devd-app";

/// A function producing checks that only some callers can perform.
pub type ExtraChecks = Arc<dyn Fn() -> Vec<Check> + Send + Sync>;

/// How the server should run.
pub struct Options {
    /// Port to bind on loopback. `0` asks the operating system to choose one.
    pub port: u16,
    /// Additional checks folded into `GET /api/doctor`.
    pub extra_checks: Vec<ExtraChecks>,
    /// A runtime whose PTYs and project processes this server should adopt.
    ///
    /// The caller that owns the registry also owns the runtime the registry's
    /// capabilities were built against. Passing it here is what keeps a
    /// capability call and a UI button talking to the same terminals.
    pub workspace: Option<Arc<Workspace>>,
    /// An external shutdown signal.
    ///
    /// The desktop shell owns the process that serves the window, so it needs a
    /// way to ask for a graceful exit (stopping PTYs and project processes)
    /// without sending a request to itself.
    pub shutdown: Option<watch::Receiver<bool>>,
    /// A native folder picker, when the shell can show one.
    ///
    /// The desktop window supplies it so that choosing a project folder is a
    /// real operating-system dialog. A `devd ui` in a browser has no such
    /// dialog; there the web application uses its own folder browser instead.
    pub picker: Option<picker::Picker>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            extra_checks: Vec::new(),
            workspace: None,
            shutdown: None,
            picker: None,
        }
    }
}

/// Everything a request handler needs.
pub struct AppState {
    /// The capability registry, shared with every other surface.
    pub registry: Arc<Registry>,
    /// Invocation context. The actor is **always** overwritten by [`AppState::new`].
    pub ctx: Ctx,
    /// Bearer token required on `/api/*`.
    pub token: String,
    /// The port actually bound, used by the host allow-list.
    pub port: u16,
    /// Approvals awaiting a human decision.
    pub approvals: ApprovalStore,
    /// Checks contributed by the caller.
    pub extra_checks: Vec<ExtraChecks>,
    /// When this process started serving, for the uptime readout.
    pub started: Instant,
    /// The process-owning runtime: agent PTYs and project processes.
    pub workspace: Arc<Workspace>,
    /// A native folder picker, when the shell can show one.
    pub picker: Option<picker::Picker>,
    /// Signals graceful shutdown when the user quits the application.
    shutdown: watch::Sender<bool>,
}

impl AppState {
    /// Assemble state for a server on `port`.
    ///
    /// The actor is set here rather than accepted from the caller, because the
    /// actor decides what the permission checks are allowed to conclude. A
    /// caller that could choose its own actor could choose `human:gui` and
    /// approve its own destructive operations, which would make every check in
    /// the registry decorative. The HTTP surface is a human at the application
    /// window, and that is the only thing this constructor will build.
    pub fn new(
        registry: Arc<Registry>,
        ctx: Ctx,
        port: u16,
        extra_checks: Vec<ExtraChecks>,
    ) -> Result<(Arc<Self>, watch::Receiver<bool>)> {
        let workspace = Workspace::new(ctx.store.clone(), ctx.paths.clone())?;
        Ok(Self::with_workspace(
            registry,
            ctx,
            port,
            extra_checks,
            workspace,
            None,
        ))
    }

    /// Assemble state around an existing runtime.
    pub fn with_workspace(
        registry: Arc<Registry>,
        mut ctx: Ctx,
        port: u16,
        extra_checks: Vec<ExtraChecks>,
        workspace: Arc<Workspace>,
        picker: Option<picker::Picker>,
    ) -> (Arc<Self>, watch::Receiver<bool>) {
        ctx.actor = Actor::human(Surface::Http);
        ctx.dry_run = false;

        let (shutdown, receiver) = watch::channel(false);
        let approvals = ApprovalStore::new(ctx.store.clone());
        let state = Arc::new(Self {
            registry,
            ctx,
            token: approval::new_token(),
            port,
            approvals,
            extra_checks,
            started: Instant::now(),
            workspace,
            picker,
            shutdown,
        });
        (state, receiver)
    }

    /// Begin shutting the server down. Idempotent.
    pub fn request_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    /// Every check the environment can produce, core first.
    pub fn checks(&self) -> Vec<Check> {
        let mut checks =
            doctor::core_checks_for(&self.ctx.paths, &self.ctx.settings, Some(&self.ctx.store));
        for extra in &self.extra_checks {
            checks.extend(extra());
        }
        checks
    }
}

/// The application router. Exposed so tests can drive it without a socket.
pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/session", get(api::session))
        .route("/api/capabilities", get(api::capabilities))
        .route("/api/capabilities/:id", get(api::capability))
        .route("/api/groups", get(api::groups))
        .route("/api/invoke", post(api::invoke))
        // What is happening, as it happens: the window's one live channel.
        .route("/api/events", get(events::events))
        .route("/api/approve", post(api::approve))
        .route("/api/deny", post(api::deny))
        .route("/api/runs", get(api::runs))
        .route("/api/runs/:id", get(api::run))
        .route("/api/runs/:id/events", get(api::run_events))
        .route("/api/runs/:id/cancel", post(api::run_cancel))
        .route("/api/runs/:id/retry", post(api::run_retry))
        .route("/api/approvals", get(api::approvals))
        .route("/api/approvals/:id/approve", post(api::approval_approve))
        .route("/api/approvals/:id/deny", post(api::approval_deny))
        .route("/api/audit", get(api::audit))
        .route("/api/doctor", get(api::doctor))
        .route("/api/shutdown", post(api::shutdown))
        .route("/api/project", get(api::project_plan))
        .route("/api/project/status", get(api::project_status))
        .route("/api/project/approve", post(api::project_approve))
        .route("/api/project/start", post(api::project_start))
        .route("/api/project/stop", post(api::project_stop))
        .route("/api/project/restart", post(api::project_restart))
        .route("/api/project/logs", get(api::project_logs))
        .route("/api/open-url", post(api::open_url))
        .route("/api/pick-folder", post(api::pick_folder))
        .route("/api/upload", post(api::upload))
        // The push-to-deploy hook. Not under /api: it carries its own secret in the path, so a service
        // that cannot hold the bearer token (GitHub) can start a deploy.
        .route("/hooks/deploy/:site/:token", post(api::deploy_hook))
        .with_state(state.clone());

    let pages = Router::new()
        .route("/", get(ui::index))
        .route("/app.js", get(ui::script))
        .route("/style.css", get(ui::style))
        .route("/favicon.svg", get(ui::favicon))
        .route("/brands/:file", get(ui::brand))
        // Every screen has an address of its own, so a link to it can be
        // copied and the back button means something. The shell answers for
        // any path it does not have a file for, and the interface reads the
        // path to decide what to show.
        .fallback(get(ui::index))
        .with_state(state.clone());

    api.merge(pages)
        .layer(middleware::from_fn_with_state(state, guard))
}

/// Where the server ended up.
#[derive(Debug, Clone)]
pub struct Bound {
    /// Address actually bound.
    pub addr: SocketAddr,
    /// Bearer token, for the caller's own informational use.
    pub token: String,
    /// Port actually bound.
    pub port: u16,
}

/// Serve until interrupted or until the application asks to quit.
///
/// `on_ready` runs after the socket is bound and before the first request is
/// accepted, so a caller can print the URL or open a window knowing the server
/// is already listening.
pub async fn serve_with<F>(
    registry: Arc<Registry>,
    ctx: Ctx,
    options: Options,
    on_ready: F,
) -> Result<Bound>
where
    F: FnOnce(&Bound) + Send,
{
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, options.port)).await?;
    let addr = listener.local_addr()?;
    let workspace = match options.workspace {
        Some(workspace) => workspace,
        None => Workspace::new(ctx.store.clone(), ctx.paths.clone())?,
    };
    let external_shutdown = options.shutdown.clone();
    let (state, mut shutdown) = AppState::with_workspace(
        registry,
        ctx,
        addr.port(),
        options.extra_checks,
        workspace,
        options.picker.clone(),
    );

    let bound = Bound {
        addr,
        token: state.token.clone(),
        port: addr.port(),
    };
    on_ready(&bound);

    let app = router(state);
    let mut external_shutdown = external_shutdown;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = shutdown.changed() => {}
                _ = async {
                    match external_shutdown.as_mut() {
                        Some(rx) => { let _ = rx.changed().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => {}
            }
        })
        .await
        .map_err(DevdError::Io)?;

    Ok(bound)
}

/// Whether `port` is held by another `devd` server, and which version.
///
/// Deliberately a hand-written request rather than an HTTP client: this is the
/// only outbound request `devd` makes to itself, the response is a header, and
/// adding a client stack to answer "is this mine?" would be a poor trade.
pub fn probe_instance(port: u16) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(600),
    )
    .ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));

    let mut stream = stream;
    let request = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;

    let mut raw = Vec::new();
    let _ = stream.take(16 * 1024).read_to_end(&mut raw);
    let response = String::from_utf8_lossy(&raw);

    let status_line = response.lines().next()?;
    if !status_line.contains(" 200") {
        return None;
    }
    // Only the head can hold headers; a body containing the literal header name
    // must not be mistaken for one.
    let head = response.split("\r\n\r\n").next().unwrap_or(&response);
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(INSTANCE_HEADER) {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// The guard that runs before every handler.
async fn guard(State(state): State<Arc<AppState>>, req: Request<Body>, next: Next) -> Response {
    // A deploy hook is called from outside, through a public link, so its Host is not a loopback
    // name. It is POST only, and the secret in its path is what authorises it.
    if req.uri().path().starts_with("/hooks/deploy/") {
        return if req.method() == Method::POST {
            next.run(req).await
        } else {
            refuse(
                StatusCode::METHOD_NOT_ALLOWED,
                "post_only",
                "a deploy hook accepts POST only",
            )
        };
    }

    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !host_is_local(host, state.port) {
        return refuse(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "this request did not address the server by a loopback name",
        );
    }

    // Never answer a preflight. This is checked before the origin rule so that
    // the answer to a preflight is the same whatever origin sent it: we have no
    // CORS policy to negotiate, and an unanswerable preflight is precisely what
    // stops a foreign page completing a call.
    if req.method() == Method::OPTIONS {
        return refuse(
            StatusCode::METHOD_NOT_ALLOWED,
            "no_cors",
            "this API does not accept cross-origin requests",
        );
    }

    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if !origin_is_local(origin, state.port) {
            return refuse(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "cross-origin requests are not accepted",
            );
        }
    }

    // `/api` and everything under `/api/` is the API. A screen whose address merely
    // begins with the letters (`/api-tools`) is a page, and refusing it for want of
    // a token made a reload or a shared link show a JSON error.
    //
    // The event stream is the one exception: `EventSource` cannot send an
    // Authorization header at all. It is still fenced by the two rules above -
    // a loopback Host and a loopback Origin, or no Origin at all - which is what
    // stops another site from reading it through this browser.
    let path = req.uri().path();
    if (path == "/api" || path.starts_with("/api/")) && path != "/api/events" {
        let presented = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default();
        if !constant_time_eq(presented.as_bytes(), state.token.as_bytes()) {
            return refuse(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "a valid bearer token is required",
            );
        }
    }

    next.run(req).await
}

fn host_is_local(host: &str, port: u16) -> bool {
    let host = host.trim().to_ascii_lowercase();
    host == format!("127.0.0.1:{port}")
        || host == format!("localhost:{port}")
        || host == format!("[::1]:{port}")
}

fn origin_is_local(origin: &str, port: u16) -> bool {
    let origin = origin.trim().trim_end_matches('/').to_ascii_lowercase();
    origin == format!("http://127.0.0.1:{port}")
        || origin == format!("http://localhost:{port}")
        || origin == format!("http://[::1]:{port}")
}

/// Compare two secrets without leaking where they first differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// A refusal in the same envelope as every other error.
pub fn refuse(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

/// Map a `devd` error onto an HTTP status without losing its machine code.
pub fn error_response(err: &DevdError) -> Response {
    let status = match err {
        DevdError::NotFound(_) => StatusCode::NOT_FOUND,
        DevdError::Invalid(_)
        | DevdError::Unsupported(_)
        | DevdError::Json(_)
        | DevdError::TomlDecode(_)
        | DevdError::TomlEncode(_) => StatusCode::BAD_REQUEST,
        DevdError::Conflict(_) => StatusCode::CONFLICT,
        DevdError::Denied(_) | DevdError::ApprovalRequired { .. } => StatusCode::FORBIDDEN,
        DevdError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
        DevdError::Command { .. } | DevdError::Mcp(_) => StatusCode::BAD_GATEWAY,
        DevdError::Io(_) | DevdError::Db(_) | DevdError::Internal(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    refuse(status, err.code(), &err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hosts_are_accepted_and_foreign_ones_are_not() {
        assert!(host_is_local("127.0.0.1:8787", 8787));
        assert!(host_is_local("localhost:8787", 8787));
        assert!(host_is_local("[::1]:8787", 8787));
        assert!(host_is_local("LOCALHOST:8787", 8787));
        assert!(!host_is_local("127.0.0.1:1", 8787));
        assert!(!host_is_local("evil.example:8787", 8787));
        assert!(!host_is_local("", 8787));
    }

    #[test]
    fn only_our_own_origins_are_accepted() {
        assert!(origin_is_local("http://127.0.0.1:8787", 8787));
        assert!(origin_is_local("http://localhost:8787/", 8787));
        assert!(!origin_is_local("https://evil.example", 8787));
        assert!(!origin_is_local("null", 8787));
    }

    #[test]
    fn constant_time_comparison_is_still_a_comparison() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }
}
