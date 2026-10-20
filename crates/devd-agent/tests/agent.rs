//! The built-in agent against scripted model servers. Nothing here reaches a
//! real provider.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::Registry;

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

/// A tiny HTTP server that answers each request from a script.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn mock(script: impl Fn(usize, &Seen) -> (u16, String) + Send + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let (head_end, len) = loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break (buf.len(), 0);
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_string();
                    let len = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end.saturating_sub(4).min(buf.len())]).to_string();
            let mut lines = head.lines();
            let path = lines.next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
            let headers = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
            let body = serde_json::from_slice(&buf[head_end.min(buf.len())..]).unwrap_or(Value::Null);
            let req = Seen { path, headers, body };
            let (status, text) = script(n, &req);
            s2.lock().unwrap().push(req);
            let resp = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    Mock { base, seen }
}

#[allow(dead_code)]
struct Env {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    reg: Arc<Registry>,
    hub: Arc<devd_agent::Hub>,
    human: Ctx,
    agent: Ctx,
}

fn ctx_for(root: &std::path::Path, store: Arc<Store>, events: EventBus, actor: Actor) -> Ctx {
    let paths = Paths::rooted(root.join("home"));
    paths.ensure().unwrap();
    let mut settings = Settings::default();
    settings.www_root = Some(root.join("www"));
    Ctx { store, paths: Arc::new(paths), settings: Arc::new(settings), events, actor, dry_run: false }
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let store = Store::open_in_memory().unwrap();
    store.migrate().unwrap();
    let store = Arc::new(store);
    let events = EventBus::new();
    let hub = devd_agent::Hub::new(ctx_for(&root, store.clone(), events.clone(), Actor::system()));
    let mut reg = Registry::new();
    devd_stacks::caps::register(&mut reg).unwrap();
    devd_agent::caps::register(&mut reg, hub.clone()).unwrap();
    let reg = Arc::new(reg);
    hub.attach(reg.clone());
    std::fs::create_dir_all(root.join("www").join("proj")).unwrap();
    Env {
        human: ctx_for(&root, store.clone(), events.clone(), Actor::human(Surface::Http)),
        agent: ctx_for(&root, store, events, Actor::agent("claude-code")),
        _dir: dir,
        root,
        reg,
        hub,
    }
}

fn call(e: &Env, ctx: &Ctx, id: &str, args: Value, approval: Approval) -> devd_core::Result<Value> {
    let r = e.reg.dispatch(ctx, &InvokeRequest { id: id.into(), args, dry_run: false }, approval)?;
    Ok(r.outcome.map(|o| o.data).unwrap_or(Value::Null))
}

fn add_provider(e: &Env, kind: &str, base: &str) {
    call(e, &e.human, "provider.save", json!({ "id": "test", "kind": kind, "label": "Test", "base_url": base, "api_key": "sk-test-0123456789abcdef", "default_model": "model-1" }), Approval::None).unwrap();
}

fn wait_for(e: &Env, tid: &str, states: &[&str]) -> Value {
    let t0 = Instant::now();
    loop {
        let v = call(e, &e.human, "chat.get", json!({ "thread": tid }), Approval::None).unwrap();
        let state = v["thread"]["state"].as_str().unwrap_or("").to_string();
        if states.contains(&state.as_str()) && !v["running"].as_bool().unwrap_or(true) {
            return v;
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "timed out in state {state}: {v}");
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn anthropic_tool_use(id: &str, name: &str, input: Value) -> String {
    json!({ "id": "msg", "type": "message", "role": "assistant", "stop_reason": "tool_use", "usage": { "input_tokens": 100, "output_tokens": 20 },
        "content": [{ "type": "text", "text": "Working on it." }, { "type": "tool_use", "id": id, "name": name, "input": input }] }).to_string()
}

fn anthropic_text(t: &str) -> String {
    json!({ "id": "msg", "type": "message", "role": "assistant", "stop_reason": "end_turn", "usage": { "input_tokens": 150, "output_tokens": 10 }, "content": [{ "type": "text", "text": t }] }).to_string()
}

#[test]
fn provider_keys_are_stored_privately_and_never_returned() {
    let e = env();
    add_provider(&e, "anthropic", "https://api.anthropic.com");
    let list = call(&e, &e.human, "provider.list", json!({}), Approval::None).unwrap();
    let text = list.to_string();
    assert!(!text.contains("0123456789abcdef") && !text.contains("sk-test"), "{text}");
    assert_eq!(list[0]["has_key"], true);
    assert_eq!(list[0]["key_hint"], "••••cdef");
    assert_eq!(list[0]["is_default"], true);
    // The audit trail redacts the key too.
    let audit = e.human.store.list_audit(50).unwrap();
    assert!(audit.iter().all(|a| !a.args.contains("0123456789abcdef")), "{audit:?}");
    // Agents cannot read or change credentials.
    assert_eq!(call(&e, &e.agent, "provider.save", json!({ "id": "x", "preset": "anthropic", "api_key": "sk-evil-0123456789" }), Approval::None).unwrap_err().code(), "permission_denied");
    assert!(call(&e, &e.agent, "provider.list", json!({}), Approval::None).is_ok());
    // A key must not travel over plain http to another host.
    let err = call(&e, &e.human, "provider.save", json!({ "id": "bad", "kind": "openai-compatible", "base_url": "http://api.example.com/v1" }), Approval::None).unwrap_err();
    assert_eq!(err.code(), "invalid_input");
    // Removal needs approval and deletes the key file.
    assert_eq!(call(&e, &e.human, "provider.remove", json!({ "id": "test" }), Approval::None).unwrap_err().code(), "approval_required");
    call(&e, &e.human, "provider.remove", json!({ "id": "test" }), Approval::Granted).unwrap();
    assert!(!e.root.join("home/secrets/providers/test.key").exists());
}

#[test]
fn testing_a_provider_sends_the_documented_headers_and_reports_real_errors() {
    let e = env();
    let m = mock(|n, _| if n == 0 { (200, json!({ "data": [{ "id": "claude-a", "display_name": "A" }, { "id": "claude-b", "display_name": "B" }] }).to_string()) } else { (401, json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }).to_string()) });
    add_provider(&e, "anthropic", &m.base);
    let ok = call(&e, &e.human, "provider.test", json!({ "id": "test" }), Approval::None).unwrap();
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["models"], 2);
    let seen = m.seen.lock().unwrap()[0].clone();
    assert!(seen.path.starts_with("/v1/models"));
    assert!(seen.headers.iter().any(|(k, v)| k == "x-api-key" && v == "sk-test-0123456789abcdef"));
    assert!(seen.headers.iter().any(|(k, v)| k == "anthropic-version" && v == "2023-06-01"));
    let bad = call(&e, &e.human, "provider.test", json!({ "id": "test" }), Approval::None).unwrap();
    assert_eq!(bad["ok"], false);
    assert!(bad["message"].as_str().unwrap().contains("rejected the API key"), "{bad}");
    assert!(!bad.to_string().contains("0123456789abcdef"));
}

#[test]
fn models_are_fetched_from_the_provider_and_cached() {
    let e = env();
    let m = mock(|_, _| (200, json!({ "data": [{ "id": "claude-x", "display_name": "X", "max_input_tokens": 200000 }] }).to_string()));
    add_provider(&e, "anthropic", &m.base);
    call(&e, &e.human, "provider.models", json!({ "id": "test" }), Approval::None).unwrap();
    let cat = call(&e, &e.agent, "models.catalogue", json!({ "source": "provider:test" }), Approval::None).unwrap();
    assert_eq!(cat[0]["model_id"], "claude-x");
    assert_eq!(cat[0]["context_window"], 200000);
}

#[test]
fn the_agent_loop_calls_tools_and_finishes() {
    let e = env();
    let target = e.root.join("www/proj/hello.txt").display().to_string();
    let t2 = target.clone();
    let m = mock(move |n, _| match n {
        0 => (200, anthropic_tool_use("toolu_1", "devd_invoke", json!({ "id": "fs.write", "args": { "path": t2, "content": "hi from the agent\n" } }))),
        _ => (200, anthropic_text("I created hello.txt.")),
    });
    add_provider(&e, "anthropic", &m.base);
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "make a hello file", "project": e.root.join("www/proj").display().to_string() }), Approval::None).unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let v = wait_for(&e, &tid, &["idle", "error"]);
    assert_eq!(v["thread"]["state"], "idle", "{v}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi from the agent\n");
    let kinds: Vec<&str> = v["events"].as_array().unwrap().iter().map(|x| x["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["user", "assistant", "tool_result", "assistant"]);
    assert_eq!(v["thread"]["input_tokens"], 250);

    let seen = m.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    let first = &seen[0].body;
    assert_eq!(seen[0].path, "/v1/messages");
    assert!(first["system"][0]["text"].as_str().unwrap().contains("built-in engineer"));
    assert_eq!(first["tools"].as_array().unwrap().len(), 5, "progressive disclosure: five tools, not hundreds");
    let second = &seen[1].body;
    assert_eq!(second["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(second["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
    // The agent acted as an agent: the audit trail says so.
    let audit = e.human.store.list_audit(100).unwrap();
    assert!(audit.iter().any(|a| a.capability == "fs.write" && a.actor == "agent:devd-agent" && a.ok));
}

#[test]
fn dangerous_operations_wait_for_a_person_and_the_agent_resumes_after() {
    let e = env();
    let victim = e.root.join("www/proj/old.txt");
    std::fs::write(&victim, "x").unwrap();
    let v2 = victim.display().to_string();
    let m = mock(move |n, req| match n {
        0 => (200, anthropic_tool_use("toolu_d", "devd_invoke", json!({ "id": "fs.delete", "args": { "path": v2 } }))),
        _ => {
            let last = req.body["messages"].as_array().unwrap().last().unwrap().clone();
            let content = last["content"][0]["content"].as_str().unwrap_or("").to_string();
            (200, anthropic_text(&format!("saw: {}", if content.contains("approved_by_a_person") { "approved" } else if content.contains("declined") { "declined" } else { "other" })))
        }
    });
    add_provider(&e, "anthropic", &m.base);
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "clean up old.txt", "project": e.root.join("www/proj").display().to_string() }), Approval::None).unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let v = wait_for(&e, &tid, &["waiting_approval", "error"]);
    assert_eq!(v["thread"]["state"], "waiting_approval", "{v}");
    assert!(victim.exists(), "nothing runs before a person decides");
    let pending = v["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["capability"], "fs.delete");
    let rid = pending[0]["id"].as_str().unwrap().to_string();

    // The conversation cannot move on, and an agent cannot approve its own request.
    assert_eq!(call(&e, &e.human, "chat.send", json!({ "message": "hurry", "thread": tid }), Approval::None).unwrap_err().code(), "conflict");
    assert_eq!(call(&e, &e.agent, "approval.decide", json!({ "id": rid, "approve": true }), Approval::Granted).unwrap_err().code(), "permission_denied");
    assert!(victim.exists());

    call(&e, &e.human, "approval.decide", json!({ "id": rid, "approve": true }), Approval::None).unwrap();
    assert!(!victim.exists(), "approved: it ran");
    let v = wait_for(&e, &tid, &["idle", "error"]);
    let last = v["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["data"]["content"][0]["text"], "saw: approved");
    let req = call(&e, &e.human, "approval.get", json!({ "id": rid }), Approval::None).unwrap();
    assert_eq!(req["state"], "approved");
    assert_eq!(req["decided_by"], "human:http");
}

#[test]
fn declining_tells_the_agent_not_to_retry() {
    let e = env();
    let victim = e.root.join("www/proj/keep.txt");
    std::fs::write(&victim, "x").unwrap();
    let v2 = victim.display().to_string();
    let m = mock(move |n, req| match n {
        0 => (200, anthropic_tool_use("toolu_d", "devd_invoke", json!({ "id": "fs.delete", "args": { "path": v2 } }))),
        _ => {
            let content = req.body["messages"].as_array().unwrap().last().unwrap()["content"][0]["content"].as_str().unwrap_or("").to_string();
            (200, anthropic_text(if content.contains("declined") { "ok, leaving it" } else { "??" }))
        }
    });
    add_provider(&e, "anthropic", &m.base);
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "delete keep.txt", "project": e.root.join("www/proj").display().to_string() }), Approval::None).unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let v = wait_for(&e, &tid, &["waiting_approval"]);
    let rid = v["pending"][0]["id"].as_str().unwrap().to_string();
    call(&e, &e.human, "approval.decide", json!({ "id": rid, "approve": false }), Approval::None).unwrap();
    let v = wait_for(&e, &tid, &["idle"]);
    assert!(victim.exists());
    assert_eq!(v["events"].as_array().unwrap().last().unwrap()["data"]["content"][0]["text"], "ok, leaving it");
}

#[test]
fn a_missing_provider_or_key_is_explained_not_hidden() {
    let e = env();
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "hello" }), Approval::None).unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let v = wait_for(&e, &tid, &["error"]);
    assert!(v["thread"]["error"].as_str().unwrap().contains("Providers"), "{v}");
    // A provider with no key.
    call(&e, &e.human, "provider.save", json!({ "id": "nokey", "preset": "anthropic", "default_model": "m", "key_env": "DEVD_TEST_NO_SUCH_KEY" }), Approval::None).unwrap();
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "hello" }), Approval::None).unwrap();
    let v = wait_for(&e, sent["id"].as_str().unwrap(), &["error"]);
    assert!(v["thread"]["error"].as_str().unwrap().contains("no API key"), "{v}");
}

#[test]
fn provider_errors_reach_the_conversation_with_the_real_reason() {
    let e = env();
    let m = mock(|_, _| (401, json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }).to_string()));
    add_provider(&e, "anthropic", &m.base);
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "hi" }), Approval::None).unwrap();
    let v = wait_for(&e, sent["id"].as_str().unwrap(), &["error"]);
    assert!(v["thread"]["error"].as_str().unwrap().contains("rejected the API key"));
}

#[test]
fn openai_compatible_providers_work_through_the_same_loop() {
    let e = env();
    let target = e.root.join("www/proj/o.txt").display().to_string();
    let t2 = target.clone();
    let m = mock(move |n, _| match n {
        0 => (200, json!({ "choices": [{ "finish_reason": "tool_calls", "message": { "content": null, "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "devd_invoke", "arguments": json!({ "id": "fs.write", "args": { "path": t2, "content": "x" } }).to_string() } }] } }], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }).to_string()),
        _ => (200, json!({ "choices": [{ "finish_reason": "stop", "message": { "content": "wrote it" } }], "usage": { "prompt_tokens": 20, "completion_tokens": 2 } }).to_string()),
    });
    add_provider(&e, "openai-compatible", &format!("{}/v1", m.base));
    let sent = call(&e, &e.human, "chat.send", json!({ "message": "write o.txt", "project": e.root.join("www/proj").display().to_string() }), Approval::None).unwrap();
    let v = wait_for(&e, sent["id"].as_str().unwrap(), &["idle", "error"]);
    assert_eq!(v["thread"]["state"], "idle", "{v}");
    assert!(std::path::Path::new(&target).exists());
    let seen = m.seen.lock().unwrap().clone();
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert!(seen[0].headers.iter().any(|(k, v)| k == "authorization" && v.starts_with("Bearer ")));
    assert_eq!(seen[1].body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").count(), 1);
}

#[test]
fn the_inbox_is_a_two_way_channel_between_agents_and_the_person() {
    let e = env();
    let posted = call(&e, &e.agent, "inbox.post", json!({ "subject": "Which database?", "body": "MySQL or Postgres for the blog?", "needs_reply": true }), Approval::None).unwrap();
    let id = posted["id"].as_str().unwrap().to_string();
    assert_eq!(call(&e, &e.agent, "inbox.reply", json!({ "id": id, "reply": "self" }), Approval::None).unwrap_err().code(), "permission_denied");
    call(&e, &e.human, "inbox.reply", json!({ "id": id, "reply": "Postgres" }), Approval::None).unwrap();
    let all = call(&e, &e.agent, "inbox.list", json!({ "open_only": false }), Approval::None).unwrap();
    assert_eq!(all[0]["reply"], "Postgres");
    assert_eq!(all[0]["from_actor"], "agent:claude-code");
    let health = call(&e, &e.human, "health.status", json!({}), Approval::None).unwrap();
    assert!(health["checks"].as_array().unwrap().iter().all(|c| c["name"] != "inbox.messages"), "a resolved question no longer needs attention");
}

#[test]
fn mcp_agents_get_their_approvals_queued() {
    let e = env();
    let victim = e.root.join("www/proj/q.txt");
    std::fs::write(&victim, "x").unwrap();
    let store = e.agent.store.clone();
    let queue: devd_mcp::ApprovalQueue = Arc::new(move |cap, args, plan| store.create_request("agent:claude-code", cap, args, plan, None, None).ok().map(|r| r.id));
    let server = devd_mcp::McpServer::new(e.reg.clone(), e.agent.clone(), "claude-code").with_approval_queue(queue);
    let (is_err, text) = server.call("devd_invoke", &json!({ "id": "fs.delete", "args": { "path": victim.display().to_string() } }));
    assert!(is_err);
    let body: Value = serde_json::from_str(&text).unwrap();
    let rid = body["request_id"].as_str().expect("queued").to_string();
    assert!(victim.exists());
    // Human-only operations are never queued: an approval cannot make them agent-runnable.
    let (_, t2) = server.call("devd_invoke", &json!({ "id": "provider.remove", "args": { "id": "x" } }));
    assert!(!t2.contains("request_id"), "{t2}");
    call(&e, &e.human, "approval.decide", json!({ "id": rid, "approve": true }), Approval::None).unwrap();
    assert!(!victim.exists());
    let polled = call(&e, &e.agent, "approval.get", json!({ "id": rid }), Approval::None).unwrap();
    assert_eq!(polled["state"], "approved");
}

#[test]
fn health_says_what_is_wrong_and_what_to_do() {
    let e = env();
    let h = call(&e, &e.human, "health.status", json!({}), Approval::None).unwrap();
    assert_eq!(h["level"], "attention", "{h}");
    let provider = h["checks"].as_array().unwrap().iter().find(|c| c["name"] == "ai.provider").unwrap().clone();
    assert_eq!(provider["fix"]["view"], "providers");
    assert!(provider["meaning"].as_str().unwrap().contains("provider"));
    for c in h["checks"].as_array().unwrap() {
        assert!(c["meaning"].as_str().is_some() && c["fix"].is_object(), "every check explains itself: {c}");
    }
}

/// Reads the real documentation page. Run with `cargo test -p devd-agent -- --ignored`.
#[test]
#[ignore = "needs network access to platform.claude.com"]
fn the_real_anthropic_models_page_still_parses() {
    let models = devd_agent::providers::fetch_anthropic_docs().unwrap();
    assert!(models.len() >= 4, "{models:?}");
    assert!(models.iter().all(|m| m.model_id.starts_with("claude-") && m.context_window.is_some()));
    assert!(models.iter().any(|m| m.input_price.is_some()));
}
