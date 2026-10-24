//! End-to-end tests for devd as an MCP **client**: a real server process, the real protocol, and the
//! real dispatch rules.

use std::sync::Arc;

use serde_json::json;

use devd_core::capability::{Actor, Approval, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::state::Store;
use devd_core::{DevdError, InvokeRequest, McpServer, McpTrust, Paths, Registry};

fn setup(tag: &str) -> (Ctx, Paths) {
    let dir = std::env::temp_dir().join(format!("devd-mcp-client-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(&dir);
    paths.ensure().unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths.clone()),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        dry_run: false,
    };
    (ctx, paths)
}

fn fake(name: &str, trust: McpTrust) -> McpServer {
    McpServer {
        id: format!("id-{name}"),
        name: name.to_string(),
        transport: "stdio".into(),
        command: Some(env!("CARGO_BIN_EXE_fake-mcp").to_string()),
        args: vec![],
        url: None,
        cwd: None,
        enabled: true,
        trust,
        timeout_secs: 30,
        tools: vec![],
        last_error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

#[test]
fn a_real_server_is_discovered_and_its_tools_are_called() {
    let (ctx, paths) = setup("discover");
    let server = fake("fake", McpTrust::Ask);
    ctx.store.mcp_put(&server).unwrap();

    let tools = devd_mcp::client::pool().test(&paths, &server).unwrap();
    assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["echo", "fail"]);

    let out = devd_mcp::client::pool()
        .call(&paths, &server, "echo", &json!({ "text": "hello from the server" }))
        .unwrap();
    assert!(!out.is_error);
    assert_eq!(out.content, "hello from the server");

    let failed = devd_mcp::client::pool().call(&paths, &server, "fail", &json!({})).unwrap();
    assert!(failed.is_error, "the server's isError is preserved");
    assert!(failed.content.contains("it did not work"));

    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn an_agent_waits_for_a_person_and_a_trusted_server_does_not() {
    let (ctx, _paths) = setup("approval");
    let mut reg = Registry::new();
    devd_mcp::caps::register(&mut reg).unwrap();

    // `ask` (the default): an agent's call never reaches the handler.
    ctx.store.mcp_put(&fake("ask", McpTrust::Ask)).unwrap();
    let agent = Ctx { actor: Actor::agent("assistant"), ..ctx.clone() };
    let call = InvokeRequest { id: "mcp.call".into(), args: json!({ "server": "ask", "tool": "echo", "args": { "text": "x" } }), dry_run: false };
    let err = reg.dispatch(&agent, &call, Approval::None).unwrap_err();
    assert!(matches!(err, DevdError::ApprovalRequired { .. }), "{err:?}");
    assert!(err.to_string().contains("approval"), "{err}");

    // The trusted path refuses an agent for a server that is not `allow`.
    let trusted = InvokeRequest { id: "mcp.call_trusted".into(), args: call.args.clone(), dry_run: false };
    let err = reg.dispatch(&agent, &trusted, Approval::None).unwrap_err();
    assert!(matches!(err, DevdError::Denied(_)), "{err:?}");

    // A person is not asked; they chose the server.
    let human = InvokeRequest { id: "mcp.call".into(), args: call.args.clone(), dry_run: false };
    let result = reg.dispatch(&ctx, &human, Approval::None).unwrap();
    let outcome = result.outcome.unwrap();
    assert_eq!(outcome.data["content"], "x");

    // A server a person marked `allow` is reachable without asking...
    ctx.store.mcp_put(&fake("allowed", McpTrust::Allow)).unwrap();
    let allowed = InvokeRequest {
        id: "mcp.call_trusted".into(),
        args: json!({ "server": "allowed", "tool": "echo", "args": { "text": "no question" } }),
        dry_run: false,
    };
    let result = reg.dispatch(&agent, &allowed, Approval::None).unwrap();
    assert_eq!(result.outcome.unwrap().data["content"], "no question");

    // ...and `deny` keeps an agent away from it entirely.
    ctx.store.mcp_put(&fake("denied", McpTrust::Deny)).unwrap();
    let denied = InvokeRequest { id: "mcp.call_trusted".into(), args: json!({ "server": "denied", "tool": "echo", "args": { "text": "x" } }), dry_run: false };
    let err = reg.dispatch(&agent, &denied, Approval::None).unwrap_err();
    assert!(matches!(err, DevdError::Denied(_)), "{err:?}");

    // A person's approval is not a way around `deny`: the approved call is still the agent's.
    let approved = InvokeRequest { id: "mcp.call".into(), args: denied.args.clone(), dry_run: false };
    let err = reg.dispatch(&agent, &approved, Approval::Granted).unwrap_err();
    assert!(matches!(err, DevdError::Denied(_)), "approval must not bypass deny: {err:?}");

    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn the_tools_offered_to_the_model_are_prefixed_and_come_from_the_servers() {
    let (ctx, _paths) = setup("external");
    ctx.store.mcp_put(&fake("files", McpTrust::Ask)).unwrap();
    let (tools, failures) = devd_mcp::client::external_tools(&ctx);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(tools.len(), 2, "the fake server publishes two tools");
    assert!(tools.iter().any(|t| t.exposed == "mcp__files__echo"));
    assert!(tools.iter().all(|t| t.server.name == "files"));

    // A denied server is not offered at all.
    ctx.store.mcp_put(&fake("hidden", McpTrust::Deny)).unwrap();
    let (tools, _) = devd_mcp::client::external_tools(&ctx);
    assert!(tools.iter().all(|t| t.server.name != "hidden"));

    let _ = std::fs::remove_dir_all(ctx.paths.root());
}
