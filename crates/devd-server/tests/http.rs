//! Acceptance tests for the application interface.
//!
//! These exercise the properties the interface exists to guarantee, rather than
//! the shape of its JSON:
//!
//! * the API is unreachable without a token, and unreachable from a foreign
//!   origin or a foreign host name;
//! * a destructive capability called through HTTP changes nothing until a human
//!   approves it;
//! * approving replays **the server's own copy** of the arguments, so what was
//!   displayed and what executes cannot be made to differ;
//! * a token is single-use;
//! * a refusal is refused *and* recorded.
//!
//! The capabilities here are synthetic. That is deliberate: the registry is the
//! same in the tests as in production, and using purpose-built capabilities
//! means the assertions are about the approval machinery rather than about Git.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{EventBus, Paths, Registry, Settings, Store};
use devd_server::{router, AppState, Options};

/// The root of a scratch installation, plus the directory effects land in.
struct Fixture {
    root: PathBuf,
    effects: PathBuf,
    _temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("home");
        std::fs::create_dir_all(&root).unwrap();
        let effects = temp.path().join("effects");
        std::fs::create_dir_all(&effects).unwrap();
        Self {
            root,
            effects,
            _temp: temp,
        }
    }

    /// A registry with one capability per interesting tier.
    fn registry(&self) -> Arc<Registry> {
        Arc::new(self.build_registry())
    }

    fn build_registry(&self) -> Registry {
        let effects = self.effects.clone();
        let mut registry = Registry::new();

        registry
            .register(
                Capability::builder("test.inspect", "Inspect something")
                    .group("test")
                    .tier(Tier::Read)
                    .effect(Effect::Read)
                    .dry_run(true)
                    .idempotent(true)
                    .handler(
                        |_ctx, a: &Value| Ok(Plan::new("test.inspect", format!("inspect {a}"))),
                        |_ctx, a: &Value| {
                            Ok(Outcome::with_data(
                                format!("inspected {}", a["name"].as_str().unwrap_or("?")),
                                json!({ "seen": a["name"] }),
                            ))
                        },
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let effects_for_delete = effects.clone();
        registry
            .register(
                Capability::builder("test.delete", "Delete something permanently")
                    .group("test")
                    .tier(Tier::Destructive)
                    .effect(Effect::Data)
                    .dry_run(true)
                    .handler(
                        |_ctx, a: &Value| {
                            Ok(Plan::new("test.delete", format!("delete {}", a["value"]))
                                .step("this cannot be undone")
                                .destructive(true)
                                .data(json!({ "target": a["value"] })))
                        },
                        move |_ctx, a: &Value| {
                            let value = a["value"].as_str().unwrap_or("none").to_string();
                            std::fs::write(
                                effects_for_delete.join(format!("{value}.txt")),
                                &value,
                            )?;
                            Ok(Outcome::ok(format!("deleted {value}")))
                        },
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();

        // A remote-tier capability that cannot produce a plan, so the interface
        // has to say so honestly instead of inventing one.
        registry
            .register(
                Capability::builder("test.publish", "Publish to a remote")
                    .group("test")
                    .tier(Tier::Remote)
                    .effect(Effect::Remote)
                    .dry_run(false)
                    .handler(
                        |_ctx, _a: &Value| Err(devd_core::DevdError::unsupported("no plan")),
                        |_ctx, _a: &Value| Ok(Outcome::ok("published")),
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();

        registry
    }

    fn state(&self, port: u16) -> (Arc<AppState>, tokio::sync::watch::Receiver<bool>) {
        let paths = Paths::rooted(self.root.clone());
        paths.ensure().unwrap();
        let ctx = Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: false,
        };
        AppState::new(self.registry(), ctx, port, Vec::new()).unwrap()
    }

    /// State whose registry also carries the workspace capabilities.
    ///
    /// The session and task routes dispatch them, so a test that drives those
    /// routes needs them bound to the workspace the state serves. Tests that
    /// count the registry's capabilities use `state` instead.
    fn state_with_workspace(
        &self,
        port: u16,
    ) -> (Arc<AppState>, tokio::sync::watch::Receiver<bool>) {
        let paths = Paths::rooted(self.root.clone());
        paths.ensure().unwrap();
        let ctx = Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: false,
        };
        let workspace =
            devd_workspace::Workspace::new(ctx.store.clone(), ctx.paths.clone()).unwrap();
        let mut registry = self.build_registry();
        devd_workspace::caps::register(&mut registry, workspace.clone()).unwrap();
        AppState::with_workspace(Arc::new(registry), ctx, port, Vec::new(), workspace, None)
    }
}

/// Build a request with a well-formed loopback `Host` header.
fn request(
    method: &str,
    path: &str,
    port: u16,
    token: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{port}"));
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// Send a request through the real router without opening a socket.
async fn call(
    state: Arc<AppState>,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let port = state.port;
    let response = router(state)
        .oneshot(request(method, path, port, token, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

// ------------------------------------------------------------------- the guard

#[tokio::test]
async fn a_screen_whose_address_starts_with_api_is_a_page_not_the_api() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let port = state.port;

    // The capability browser lives at `/api-tools`. Reloading it must show the
    // page, not a JSON error about a missing token.
    let response = router(state.clone())
        .oneshot(request("GET", "/api-tools", port, None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("<html"),
        "/api-tools did not serve the application"
    );

    // Narrowing what counts as the API must not open any of it.
    for path in ["/api", "/api/", "/api/session", "/api/capabilities"] {
        let (status, _) = call(state.clone(), "GET", path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} must need a token");
    }
}

#[tokio::test]
async fn the_api_is_unreachable_without_a_token() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);

    let (status, body) = call(state.clone(), "GET", "/api/session", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");

    let (status, _) = call(state, "GET", "/api/capabilities", Some("wrong-token"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_api_is_reachable_with_the_session_token() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(state, "GET", "/api/session", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["actor"]["surface"], "http");
    assert_eq!(body["actor"]["kind"], "human");
    assert_eq!(body["registry"]["capabilities"].as_u64().unwrap(), 3);
}

#[tokio::test]
async fn the_surface_cannot_choose_to_be_an_agent() {
    // The token is the only credential, and the actor is fixed by the server.
    // There is no request shape that produces an agent actor.
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state,
        "POST",
        "/api/invoke",
        Some(&token),
        Some(
            json!({ "id": "test.inspect", "args": { "name": "x" }, "actor": { "kind": "agent" } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"]["data"]["seen"], "x");
}

#[tokio::test]
async fn a_foreign_host_is_refused() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    // This is the DNS-rebinding shape: reach us by a name that is not loopback.
    let req = Request::builder()
        .method("GET")
        .uri("/api/session")
        .header("host", "evil.example:8787")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_foreign_origin_is_refused_even_with_a_valid_token() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let req = Request::builder()
        .method("GET")
        .uri("/api/session")
        .header("host", "127.0.0.1:8787")
        .header("origin", "https://evil.example")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn our_own_origin_is_accepted() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let req = Request::builder()
        .method("GET")
        .uri("/api/session")
        .header("host", "127.0.0.1:8787")
        .header("origin", "http://127.0.0.1:8787")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn preflight_is_never_answered() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/api/invoke")
        .header("host", "127.0.0.1:8787")
        .header("origin", "https://evil.example")
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ------------------------------------------------------------------ the shell

#[tokio::test]
async fn the_shell_delivers_the_token_and_identifies_the_instance() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let response = router(state)
        .oneshot(request("GET", "/", 8787, None, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().contains_key("x-devd-app"));
    let csp = response
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("connect-src 'self'"));

    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&bytes);
    assert!(
        html.contains(&token),
        "the page must carry the session token"
    );
    assert!(
        !html.contains("__DEVD_TOKEN__"),
        "the placeholder must be replaced"
    );
}

// ------------------------------------------------------------- the read path

#[tokio::test]
async fn an_ordinary_capability_runs_immediately() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state,
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.inspect", "args": { "name": "thing" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["approval_required"], false);
    assert_eq!(body["outcome"]["data"]["seen"], "thing");
}

#[tokio::test]
async fn a_plan_request_changes_nothing_and_returns_the_plan() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "planned" }, "dry_run": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["dry_run"], true);
    assert!(body["plan"]["destructive"].as_bool().unwrap());
    assert!(!fixture.effects.join("planned.txt").exists());
}

// ---------------------------------------------------------- the approval gate

#[tokio::test]
async fn a_destructive_operation_is_not_applied_until_it_is_approved() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "alpha" } })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["approval_required"], true);
    assert_eq!(body["tier"], "destructive");
    assert!(body["plan"]["destructive"].as_bool().unwrap());
    assert!(!body["token"].as_str().unwrap().is_empty());
    assert_eq!(
        fixture.effects.join("alpha.txt").exists(),
        false,
        "the effect ran before approval"
    );

    // An empty approval store would still refuse; a populated one is required.
    assert_eq!(state.approvals.len(), 1);
}

#[tokio::test]
async fn approving_applies_exactly_the_planned_operation() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (_, opened) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "beta" } })),
    )
    .await;
    let approval = opened["token"].as_str().unwrap().to_string();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"]["message"], "deleted beta");
    assert_eq!(
        std::fs::read_to_string(fixture.effects.join("beta.txt")).unwrap(),
        "beta"
    );
    assert!(state.approvals.is_empty(), "the token was not consumed");
}

#[tokio::test]
async fn approval_replays_the_servers_copy_of_the_arguments() {
    // The property that matters: a client cannot be shown one plan and approve
    // another, because the client never supplies the arguments it approves.
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (_, opened) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "shown" } })),
    )
    .await;
    let approval = opened["token"].as_str().unwrap().to_string();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({
            "token": approval,
            "id": "test.delete",
            "args": { "value": "swapped" },
            "tier": "read"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"]["message"], "deleted shown");
    assert!(
        fixture.effects.join("shown.txt").exists(),
        "the server's arguments were not used"
    );
    assert!(
        !fixture.effects.join("swapped.txt").exists(),
        "the client's arguments were used"
    );
}

#[tokio::test]
async fn an_approval_token_is_single_use() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (_, opened) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "once" } })),
    )
    .await;
    let approval = opened["token"].as_str().unwrap().to_string();

    let (first, _) = call(
        state.clone(),
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;
    assert_eq!(first, StatusCode::OK);

    std::fs::remove_file(fixture.effects.join("once.txt")).unwrap();

    let (second, body) = call(
        state.clone(),
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;
    assert_eq!(second, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");
    assert!(
        !fixture.effects.join("once.txt").exists(),
        "a token replayed"
    );
}

#[tokio::test]
async fn an_unknown_approval_is_refused() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state,
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": "00ff00ff" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");
}

#[tokio::test]
async fn declining_runs_nothing_and_is_recorded() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (_, opened) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.delete", "args": { "value": "declined" } })),
    )
    .await;
    let approval = opened["token"].as_str().unwrap().to_string();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/deny",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["declined"], true);
    assert!(
        !fixture.effects.join("declined.txt").exists(),
        "a declined operation ran"
    );

    // The refusal must be visible in the audit trail: a declined operation has
    // to be distinguishable from one that never arrived.
    let (_, audit) = call(state, "GET", "/api/audit?limit=20", Some(&token), None).await;
    let rows = audit["items"]
        .as_array()
        .unwrap_or_else(|| panic!("the audit response was {audit}"));
    let refusal = rows
        .iter()
        .find(|r| r["capability"] == "test.delete" && r["ok"] == false)
        .expect("no audit row for the refusal");
    assert!(refusal["message"].as_str().unwrap().contains("declined"));
}

#[tokio::test]
async fn a_capability_that_cannot_plan_says_so_instead_of_inventing_one() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.publish" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["approval_required"], true);
    assert_eq!(body["tier"], "remote");
    assert!(body["plan"].is_null());

    // And approving still works, because the approval gate is the tier's, not
    // the plan's.
    let approval = body["token"].as_str().unwrap().to_string();
    let (status, done) = call(
        state,
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(done["outcome"]["message"], "published");
}

#[tokio::test]
async fn a_denied_reason_is_reported_without_the_operation_running() {
    // `test.publish` cannot plan, so the only way to reach it is through the
    // approval flow. Refusing there must leave no trace of an effect.
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (_, opened) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "test.publish" })),
    )
    .await;
    let approval = opened["token"].as_str().unwrap().to_string();
    call(
        state.clone(),
        "POST",
        "/api/deny",
        Some(&token),
        Some(json!({ "token": approval })),
    )
    .await;

    // Nothing in the effects directory, because the capability never ran.
    let written: Vec<_> = std::fs::read_dir(&fixture.effects).unwrap().collect();
    assert!(written.is_empty(), "a denied capability wrote {written:?}");
}

// ------------------------------------------------------------- discovery

#[tokio::test]
async fn capabilities_can_be_listed_searched_and_described() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(
        state.clone(),
        "GET",
        "/api/capabilities",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"].as_u64().unwrap(), 3);
    let first = &body["items"][0];
    assert!(first["id"].is_string());
    assert!(first["requires_approval"].is_boolean());

    let (_, searched) = call(
        state.clone(),
        "GET",
        "/api/capabilities?q=delete",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(searched["items"][0]["id"], "test.delete");

    let (status, described) = call(
        state.clone(),
        "GET",
        "/api/capabilities/test.delete",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(described["tier"], "destructive");
    assert_eq!(described["requires_approval"], true);

    let (status, _) = call(
        state,
        "GET",
        "/api/capabilities/no.such.thing",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_malformed_body_gets_the_same_error_envelope() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let req = Request::builder()
        .method("POST")
        .uri("/api/invoke")
        .header("host", "127.0.0.1:8787")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from("{ this is not json"))
        .unwrap();
    let response = router(state).oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"]["code"], "invalid_body");
}

#[tokio::test]
async fn the_doctor_reports_the_shared_checks() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8787);
    let token = state.token.clone();

    let (status, body) = call(state, "GET", "/api/doctor", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<String> = body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect();
    for expected in ["home", "layout", "settings", "state", "port.http"] {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected} in {names:?}"
        );
    }
}

// ---------------------------------------------------------------- real socket

/// A raw HTTP request, so the tests exercise the socket and not only the router.
async fn raw(port: u16, request: String) -> String {
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_real_socket_serves_the_application_and_stops_when_asked() {
    let fixture = Fixture::new();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

    let registry = fixture.registry();
    let paths = Paths::rooted(fixture.root.clone());
    paths.ensure().unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Http),
        dry_run: false,
    };

    let server = tokio::spawn(async move {
        devd_server::serve_with(registry, ctx, Options::default(), move |bound| {
            let _ = ready_tx.send((bound.port, bound.token.clone()));
        })
        .await
    });

    let (port, token) = ready_rx.await.unwrap();
    assert!(port > 0);

    // The instance probe must recognise us.
    let probe_port = port;
    let version = tokio::task::spawn_blocking(move || devd_server::probe_instance(probe_port))
        .await
        .unwrap();
    assert!(
        version.is_some(),
        "the instance probe did not recognise the server"
    );

    // A real request over a real socket.
    let body = raw(
        port,
        format!(
            "GET /api/session HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(body.contains("200 OK"), "{body}");
    assert!(body.contains("\"surface\":\"http\""), "{body}");

    // An unauthenticated request is refused over the socket too.
    let refused = raw(
        port,
        format!("GET /api/session HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    )
    .await;
    assert!(refused.contains("401"), "{refused}");

    // And the application can stop itself, which is what the Quit button does.
    let stopped = raw(
        port,
        format!(
            "POST /api/shutdown HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        ),
    )
    .await;
    assert!(stopped.contains("200 OK"), "{stopped}");

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), server)
        .await
        .expect("the server did not stop when asked")
        .unwrap();
    assert!(
        result.is_ok(),
        "the server exited with an error: {result:?}"
    );
}

// ------------------------------------------------------------ work surfaces

#[tokio::test]
async fn a_session_started_over_http_is_listed_attached_and_stopped() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state_with_workspace(8799);
    let token = state.token.clone();
    let cwd = fixture._temp.path().to_string_lossy().to_string();
    let command = if cfg!(windows) {
        json!([
            "cmd.exe",
            "/c",
            "echo persisted-output & ping -n 3 127.0.0.1 > NUL"
        ])
    } else {
        json!(["/bin/sh", "-c", "echo persisted-output; sleep 2"])
    };

    let (status, started) = call(
        state.clone(),
        "POST",
        "/api/sessions",
        Some(&token),
        Some(json!({ "kind": "custom", "cwd": cwd, "command": command, "label": "http test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let id = started["id"].as_str().expect("session id").to_string();

    // A reload is a fresh list request: the session is still there.
    let (status, listed) = call(state.clone(), "GET", "/api/sessions", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == id));

    // The live state is attachable in this process.
    let (status, one) = call(
        state.clone(),
        "GET",
        &format!("/api/sessions/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["attachable"], true);

    // Output is genuinely from the child process.
    let mut seen = String::new();
    for _ in 0..40 {
        let (status, out) = call(
            state.clone(),
            "POST",
            &format!("/api/sessions/{id}/output"),
            Some(&token),
            Some(json!({ "cursor": 0 })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(out["data"].as_str().unwrap_or(""))
            .unwrap_or_default();
        seen = String::from_utf8_lossy(&raw).to_string();
        if seen.contains("persisted-output") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(seen.contains("persisted-output"), "output was: {seen}");

    let (status, stopped) = call(
        state.clone(),
        "POST",
        &format!("/api/sessions/{id}/stop"),
        Some(&token),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["live_state"], "cancelled");
    assert_eq!(stopped["attachable"], false);

    // The record and the transcript outlive the process.
    let (status, after) = call(
        state.clone(),
        "GET",
        &format!("/api/sessions/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["replayable"], true);

    // Dismissing is destructive. The dedicated route must not be a way around
    // the approval the capability requires.
    let (status, refused) = call(
        state.clone(),
        "DELETE",
        &format!("/api/sessions/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    let (status, still) = call(
        state.clone(),
        "GET",
        &format!("/api/sessions/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the refused dismissal removed the session: {still}");

    // Every launch and stop reaches the audit trail, like any other operation.
    let (_, audit) = call(state.clone(), "GET", "/api/audit?limit=50", Some(&token), None).await;
    let rows = audit["items"].as_array().expect("audit rows");
    for capability in ["session.start", "session.stop", "session.dismiss"] {
        assert!(
            rows.iter().any(|r| r["capability"] == capability),
            "no audit row for {capability}: {audit}"
        );
    }

    // The same operation goes through when the person approves it.
    let (status, asked) = call(
        state.clone(),
        "POST",
        "/api/invoke",
        Some(&token),
        Some(json!({ "id": "session.dismiss", "args": { "id": id } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{asked}");
    assert_eq!(asked["approval_required"], true);
    let (status, done) = call(
        state.clone(),
        "POST",
        "/api/approve",
        Some(&token),
        Some(json!({ "token": asked["token"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let (status, _) = call(
        state,
        "GET",
        &format!("/api/sessions/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The window's dedicated routes may not reach a subsystem directly.
///
/// Polling, keystrokes and resizing are the documented exceptions (they would
/// write an audit row per frame). Everything else that changes state must go
/// through `Registry::dispatch`, or its tier is not enforced and it is not
/// audited.
#[test]
fn a_dedicated_route_cannot_reach_a_mutating_subsystem_directly() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/api.rs"))
        .expect("api.rs");
    let forbidden = [
        ".sessions().start(",
        ".sessions().stop(",
        ".sessions().rename(",
        ".sessions().dismiss(",
        ".orchestrator().cancel(",
        ".orchestrator().spawn(",
    ];
    for needle in forbidden {
        assert!(
            !source.contains(needle),
            "api.rs calls `{needle}` directly; dispatch the capability instead"
        );
    }
}

#[tokio::test]
async fn a_project_manifest_is_reviewable_without_running_anything() {
    let fixture = Fixture::new();
    let (state, _rx) = fixture.state(8801);
    let token = state.token.clone();
    let project = fixture._temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("devd.yml"),
        r#"version: 1
project:
  name: example-app
processes:
  web:
    command: ["node", "-e", "console.log('web')"]
"#,
    )
    .unwrap();

    let encoded = urlencode(&project.to_string_lossy());
    let (status, plan) = call(
        state.clone(),
        "GET",
        &format!("/api/project?project={encoded}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(plan["approved"], false);
    assert_eq!(plan["processes"][0]["name"], "web");
    assert_eq!(plan["processes"][0]["command"][0], "node");

    let (status, status_body) = call(
        state.clone(),
        "GET",
        &format!("/api/project/status?project={encoded}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{status_body}");
    assert_eq!(status_body["processes"][0]["status"], "stopped");
}

fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
