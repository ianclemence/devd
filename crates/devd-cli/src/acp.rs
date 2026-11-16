//! `devd acp`: devd as an Agent Client Protocol builder.
//!
//! The Agent Client Protocol is how an editor (or an engine like craft-anything) drives an
//! agent as a subprocess. devd can already be driven over MCP; ACP is the other direction, and
//! it is the one that makes devd the builder inside somebody else's recreation loop:
//!
//! ```text
//! craft-anything  --session/new, session/prompt-->  devd acp
//!                 <--session/update, request_permission--
//! ```
//!
//! devd does not grow a second agent to do this. A session is an ordinary devd conversation:
//! `chat.send` starts it, `chat.get` is followed for events, and every tool the model asks for
//! goes through the same capability registry, approval records and audit chain as the window and
//! the command line. The ACP client's permission answer becomes devd's approval, so a client
//! cannot grant itself a capability devd would refuse a person.
//!
//! What is implemented is the baseline an ACP client may rely on:
//!
//! * `initialize`, with honest capabilities (text prompts only, no session load);
//! * `session/new` (a working directory and an id);
//! * `session/prompt`, streaming text and tool calls as `session/update`;
//! * `session/request_permission` when devd needs an approval, answered by the client;
//! * `session/cancel`, which stops the conversation and returns the `cancelled` stop reason.
//!
//! Everything is newline-delimited JSON-RPC 2.0 on stdin/stdout.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::capability::{Approval, InvokeRequest};
use devd_core::{DevdError, Result};

use crate::app::App;

/// The ACP protocol major version this build speaks.
pub const PROTOCOL_VERSION: i64 = 1;

/// The name and version reported to a client.
fn agent_info() -> Value {
    json!({
        "name": "devd",
        "title": "devd",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// What `initialize` answers. It is deliberately modest: text prompts, no session load, no MCP
/// transports of its own. A client that needs a capability not listed here must not assume it.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "agentCapabilities": {
            "loadSession": false,
            "promptCapabilities": {
                "image": false,
                "audio": false,
                "embeddedContext": false
            },
            "mcpCapabilities": {
                "http": false,
                "sse": false
            }
        },
        "agentInfo": agent_info(),
        "authMethods": []
    })
}

/// The text of a `session/prompt`, joined from its text blocks.
///
/// Anything the agent promised not to accept (an image, an embedded resource) is refused by name
/// rather than silently dropped: a client sending one has read the capabilities wrong.
pub fn prompt_text(params: &Value) -> Result<String> {
    let blocks = params["prompt"]
        .as_array()
        .ok_or_else(|| DevdError::invalid("`prompt` is a list of content blocks"))?;
    let mut parts = Vec::new();
    for block in blocks {
        match block["type"].as_str().unwrap_or("") {
            "text" => {
                if let Some(text) = block["text"].as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                }
            }
            "resource_link" => {
                if let Some(uri) = block["uri"].as_str() {
                    parts.push(format!("The person referenced {uri}."));
                }
            }
            other => {
                return Err(DevdError::invalid(format!(
                    "this agent accepts text prompts; it did not advertise `{other}`"
                )))
            }
        }
    }
    if parts.is_empty() {
        return Err(DevdError::invalid("the prompt has no text"));
    }
    Ok(parts.join("\n\n"))
}

/// A `session/update` notification.
fn update(session: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session, "update": update }
    })
}

/// An agent message chunk.
fn message_chunk(session: &str, message_id: &str, text: &str) -> Value {
    update(
        session,
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": message_id,
            "content": { "type": "text", "text": text }
        }),
    )
}

/// A tool call, as the client's timeline draws it.
fn tool_call(session: &str, id: &str, title: &str, kind: &str, status: &str) -> Value {
    update(
        session,
        json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": title,
            "kind": kind,
            "status": status,
        }),
    )
}

/// The end of a tool call, with what it returned.
fn tool_update(session: &str, id: &str, status: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session,
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": status,
                "content": [{ "type": "content", "content": { "type": "text", "text": text } }]
            }
        }
    })
}

/// One open conversation.
struct Session {
    /// The devd conversation it became, once a prompt started it.
    thread: Option<String>,
    /// The folder the client is working in.
    cwd: String,
}

fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.clone(), "result": result })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.clone(), "error": { "code": code, "message": message } })
}

/// Dispatch a capability as the person's ACP client. `Approval::Granted` is used only when the
/// client answered a `session/request_permission`; everything else goes through `Approval::None`,
/// so devd still refuses what it would refuse anywhere else.
fn call(app: &App, id: &str, args: Value, approval: Approval) -> Result<Value> {
    let res = app.registry.dispatch(
        &app.ctx(),
        &InvokeRequest {
            id: id.to_string(),
            args,
            dry_run: false,
        },
        approval,
    )?;
    Ok(res.outcome.map(|o| o.data).unwrap_or(Value::Null))
}

fn write_line(out: &mut impl Write, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *out, value).map_err(|e| DevdError::internal(e.to_string()))?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Serve ACP on stdin/stdout until the client closes the pipe.
pub fn serve(app: App) -> Result<i32> {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() && tx.send(line).is_err() {
                break;
            }
        }
    });

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut sessions: HashMap<String, Session> = HashMap::new();
    let mut next: u64 = 0;

    while let Ok(line) = rx.recv() {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = write_line(&mut out, &error(&Value::Null, -32700, "the message is not JSON"));
            continue;
        };
        let method = msg["method"].as_str().unwrap_or("");
        let id = msg["id"].clone();
        let params = &msg["params"];
        // A notification has no id and never gets an answer.
        let is_request = !id.is_null();
        match method {
            "initialize" => {
                if is_request {
                    write_line(&mut out, &result(&id, initialize_result()))?;
                }
            }
            "authenticate" => {
                if is_request {
                    write_line(&mut out, &result(&id, json!({})))?;
                }
            }
            "session/new" => {
                if !is_request {
                    continue;
                }
                let cwd = params["cwd"].as_str().unwrap_or("").trim().to_string();
                if cwd.is_empty() || !std::path::Path::new(&cwd).is_absolute() {
                    write_line(
                        &mut out,
                        &error(&id, -32602, "`cwd` is an absolute path for the session"),
                    )?;
                    continue;
                }
                next += 1;
                let session = format!("sess_{next:016x}");
                sessions.insert(
                    session.clone(),
                    Session {
                        thread: None,
                        cwd,
                    },
                );
                write_line(&mut out, &result(&id, json!({ "sessionId": session })))?;
            }
            "session/prompt" => {
                if !is_request {
                    continue;
                }
                let session_id = params["sessionId"].as_str().unwrap_or("").to_string();
                if !sessions.contains_key(&session_id) {
                    write_line(
                        &mut out,
                        &error(&id, -32002, "unknown session; call session/new first"),
                    )?;
                    continue;
                }
                match run_prompt(&app, &mut sessions, &session_id, params, &mut out, &rx) {
                    Ok(stop) => write_line(&mut out, &result(&id, json!({ "stopReason": stop })))?,
                    Err(e) => write_line(
                        &mut out,
                        &error(&id, -32603, &e.to_string()),
                    )?,
                }
            }
            "session/cancel" => {
                // Cancellation while a prompt runs is handled inside `run_prompt`; outside one
                // there is nothing to stop.
            }
            "" => {}
            other => {
                if is_request {
                    write_line(
                        &mut out,
                        &error(&id, -32601, &format!("devd does not implement `{other}`")),
                    )?;
                }
            }
        }
    }
    Ok(0)
}

/// Follow one prompt turn to its end, streaming what devd does.
fn run_prompt(
    app: &App,
    sessions: &mut HashMap<String, Session>,
    session_id: &str,
    params: &Value,
    out: &mut impl Write,
    rx: &Receiver<String>,
) -> Result<&'static str> {
    let text = prompt_text(params)?;
    let cwd = sessions
        .get(session_id)
        .map(|s| s.cwd.clone())
        .unwrap_or_default();
    let thread = sessions.get(session_id).and_then(|s| s.thread.clone());

    let sent = call(
        app,
        "chat.send",
        json!({ "message": text, "thread": thread, "project": cwd }),
        Approval::None,
    )?;
    let tid = sent["id"]
        .as_str()
        .or_else(|| sent["thread"]["id"].as_str())
        .ok_or_else(|| DevdError::internal("chat.send returned no conversation"))?
        .to_string();
    if let Some(session) = sessions.get_mut(session_id) {
        session.thread = Some(tid.clone());
    }

    let mut after: i64 = 0;
    let mut said = HashSet::new();
    let mut open_tools: HashMap<String, String> = HashMap::new();
    // A client may close its end of the pipe while a turn is still running. The work was asked
    // for and is already in the record; it is finished (and its result is still written to the
    // closed pipe harmlessly) rather than abandoned halfway.
    let mut client_gone = false;
    loop {
        if client_gone {
            std::thread::sleep(Duration::from_millis(250));
        }
        let v = call(app, "chat.get", json!({ "thread": tid, "after": after }), Approval::None)?;
        let mut saw_error = false;
        for e in v["events"].as_array().cloned().unwrap_or_default() {
            after = e["id"].as_i64().unwrap_or(after);
            saw_error |= e["kind"] == json!("error");
            stream_event(session_id, &e, out, &mut open_tools)?;
        }
        // An approval the conversation is waiting on becomes the client's decision.
        let mut decided = false;
        for request in v["pending"].as_array().cloned().unwrap_or_default() {
            let request_id = request["id"].as_str().unwrap_or("").to_string();
            if request_id.is_empty() || !said.insert(format!("approval:{request_id}")) {
                continue;
            }
            decided = true;
            match ask_permission(out, rx, session_id, &request)? {
                Decision::Allow => {
                    let _ = call(
                        app,
                        "approval.decide",
                        json!({ "id": request_id, "approve": true }),
                        Approval::Granted,
                    );
                }
                Decision::Deny => {
                    let _ = call(
                        app,
                        "approval.decide",
                        json!({ "id": request_id, "approve": false }),
                        Approval::Granted,
                    );
                }
                Decision::Cancel => {
                    let _ = call(app, "chat.stop", json!({ "thread": tid }), Approval::None);
                    return Ok("cancelled");
                }
            }
        }
        if decided {
            // The decision let the conversation carry on; read its state again rather than
            // treating the pause that asked for it as the end of the turn.
            continue;
        }
        let state = v["thread"]["state"].as_str().unwrap_or("");
        let running = v["running"].as_bool().unwrap_or(false);
        if !running && (state == "idle" || state == "error" || state == "waiting_approval") {
            if state == "waiting_approval" {
                // A request appeared that the loop above could not ask about; nothing more will
                // happen in this turn.
                return Ok("end_turn");
            }
            if state == "error" {
                if !saw_error {
                    let why = v["thread"]["error"]
                        .as_str()
                        .unwrap_or("the agent stopped with an error");
                    write_line(out, &message_chunk(session_id, &format!("msg_{tid}_error"), why))?;
                }
                // The agent could not carry out the turn. ACP calls that a refusal, which a
                // client can show differently from a normal end of turn.
                return Ok("refusal");
            }
            return Ok("end_turn");
        }
        // Sleep in short steps, listening for a cancel.
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if msg["method"].as_str() == Some("session/cancel")
                    && msg["params"]["sessionId"].as_str() == Some(session_id)
                {
                    let _ = call(app, "chat.stop", json!({ "thread": tid }), Approval::None);
                    return Ok("cancelled");
                }
                if !msg["id"].is_null() {
                    write_line(
                        out,
                        &error(&msg["id"], -32603, "this agent is busy with a prompt turn"),
                    )?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => client_gone = true,
        }
    }
}

/// Translate one devd chat event into ACP updates.
fn stream_event(
    session: &str,
    event: &Value,
    out: &mut impl Write,
    open_tools: &mut HashMap<String, String>,
) -> Result<()> {
    let id = event["id"].as_i64().unwrap_or(0);
    match event["kind"].as_str().unwrap_or("") {
        "assistant" => {
            for block in event["data"]["content"].as_array().cloned().unwrap_or_default() {
                match block["type"].as_str().unwrap_or("") {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or("").to_string();
                        if !text.is_empty() {
                            write_line(out, &message_chunk(session, &format!("msg_{session}_{id}"), &text))?;
                        }
                    }
                    "tool_use" => {
                        let call_id = block["id"].as_str().unwrap_or("").to_string();
                        if call_id.is_empty() {
                            continue;
                        }
                        let (name, detail) = if block["name"] == "devd_invoke" {
                            (
                                block["input"]["id"].as_str().unwrap_or("devd_invoke").to_string(),
                                block["input"]["args"].to_string(),
                            )
                        } else {
                            (
                                block["name"].as_str().unwrap_or("tool").to_string(),
                                block["input"].to_string(),
                            )
                        };
                        open_tools.insert(call_id.clone(), name.clone());
                        let label = if detail.len() > 160 {
                            format!("{name} {}", devd_core::util::truncate(&detail, 160))
                        } else {
                            format!("{name} {detail}")
                        };
                        write_line(out, &tool_call(session, &call_id, &label, "other", "in_progress"))?;
                    }
                    _ => {}
                }
            }
        }
        "tool_result" => {
            let call_id = event["data"]["tool_use_id"].as_str().unwrap_or("").to_string();
            if !call_id.is_empty() {
                let failed = event["data"]["is_error"].as_bool().unwrap_or(false);
                let body = event["data"]["content"].as_str().unwrap_or("");
                let body = devd_core::util::truncate(body, 4000);
                open_tools.remove(&call_id);
                write_line(
                    out,
                    &tool_update(session, &call_id, if failed { "failed" } else { "completed" }, &body),
                )?;
            }
        }
        "notice" => {
            let text = event["data"]["text"].as_str().unwrap_or("");
            if !text.is_empty() {
                write_line(out, &message_chunk(session, &format!("msg_{session}_{id}"), text))?;
            }
        }
        "error" => {
            let text = event["data"]["message"].as_str().unwrap_or("the agent stopped");
            write_line(out, &message_chunk(session, &format!("msg_{session}_{id}"), text))?;
        }
        _ => {}
    }
    Ok(())
}

/// What the client said about one tool call.
enum Decision {
    Allow,
    Deny,
    Cancel,
}

/// Ask the client to approve one operation, and wait for its answer.
fn ask_permission(
    out: &mut impl Write,
    rx: &Receiver<String>,
    session: &str,
    request: &Value,
) -> Result<Decision> {
    let call_id = request["id"].as_str().unwrap_or("").to_string();
    let capability = request["capability"].as_str().unwrap_or("an operation");
    let mut title = format!("Allow devd to run `{capability}`");
    if let Some(steps) = request["plan"]["steps"].as_array() {
        let steps: Vec<String> = steps
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        if !steps.is_empty() {
            title = format!("{title}: {}", steps.join("; "));
        }
    }
    let request_id = format!("perm_{call_id}");
    let ask = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/request_permission",
        "params": {
            "sessionId": session,
            "toolCall": {
                "toolCallId": call_id,
                "title": devd_core::util::truncate(&title, 300),
                "kind": "other",
                "status": "pending",
                "rawInput": request["args"].clone(),
            },
            "options": [
                { "optionId": "allow", "name": "Allow this once", "kind": "allow_once" },
                { "optionId": "reject", "name": "Do not run it", "kind": "reject_once" }
            ]
        }
    });
    write_line(out, &ask)?;
    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(line) => {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if msg["method"].as_str() == Some("session/cancel") {
                    return Ok(Decision::Cancel);
                }
                if msg["id"]
                    .as_str()
                    .is_some_and(|id| id == format!("perm_{call_id}"))
                {
                    let outcome = &msg["result"]["outcome"];
                    return Ok(match outcome["outcome"].as_str().unwrap_or("") {
                        "selected" if outcome["optionId"].as_str() == Some("allow") => Decision::Allow,
                        "cancelled" => Decision::Cancel,
                        _ => Decision::Deny,
                    });
                }
                if !msg["id"].is_null() {
                    write_line(
                        out,
                        &error(&msg["id"], -32603, "waiting for permission for another call"),
                    )?;
                }
            }
            Err(RecvTimeoutError::Timeout) => return Ok(Decision::Deny),
            Err(RecvTimeoutError::Disconnected) => return Ok(Decision::Cancel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reports_the_baseline_honestly() {
        let v = initialize_result();
        assert_eq!(v["protocolVersion"], 1);
        assert_eq!(v["agentCapabilities"]["loadSession"], false);
        assert_eq!(v["agentCapabilities"]["promptCapabilities"]["image"], false);
        assert_eq!(v["agentInfo"]["name"], "devd");
        assert!(v["authMethods"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_prompt_is_its_text_blocks_and_richer_blocks_are_refused() {
        let text = prompt_text(&json!({ "prompt": [
            { "type": "text", "text": "Recreate the checkout page." },
            { "type": "text", "text": "Start with the empty state." }
        ]}))
        .unwrap();
        assert!(text.contains("checkout") && text.contains("empty state"));
        let err = prompt_text(&json!({ "prompt": [{ "type": "image", "data": "x", "mimeType": "image/png" }] }))
            .unwrap_err();
        assert!(err.to_string().contains("image"), "{err}");
        assert!(prompt_text(&json!({ "prompt": [] })).is_err());
    }

    #[test]
    fn updates_use_the_shapes_a_client_reads() {
        let chunk = message_chunk("sess_1", "msg_1", "hello");
        assert_eq!(chunk["method"], "session/update");
        assert_eq!(chunk["params"]["sessionId"], "sess_1");
        assert_eq!(chunk["params"]["update"]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(chunk["params"]["update"]["content"]["text"], "hello");
        let call = tool_call("sess_1", "call_1", "fs.write", "other", "in_progress");
        assert_eq!(call["params"]["update"]["toolCallId"], "call_1");
        let done = tool_update("sess_1", "call_1", "completed", "wrote 3 files");
        assert_eq!(done["params"]["update"]["status"], "completed");
    }

    #[test]
    fn an_error_keeps_the_requests_id() {
        let e = error(&json!(7), -32602, "bad params");
        assert_eq!(e["id"], 7);
        assert_eq!(e["error"]["code"], -32602);
    }
}
