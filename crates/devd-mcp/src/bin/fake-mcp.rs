//! A tiny Model Context Protocol server used by this crate's integration tests.
//!
//! It is deliberately a real process speaking the real protocol over its pipes: the test that matters
//! is that devd's client starts a server, initialises it and calls a tool, not that a mock returned the
//! value the test handed it.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(method) = value.get("method").and_then(Value::as_str) else { continue };
        // A notification (no id) gets no answer.
        let Some(id) = value.get("id").cloned() else { continue };
        let result: Value = match method {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake-mcp", "version": "1" }
            }),
            "tools/list" => json!({
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo the text argument back",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "text": { "type": "string" } },
                            "required": ["text"]
                        }
                    },
                    {
                        "name": "fail",
                        "description": "Report an error without doing anything",
                        "inputSchema": { "type": "object", "properties": {} }
                    }
                ]
            }),
            "tools/call" => {
                let name = value["params"]["name"].as_str().unwrap_or("");
                let args = &value["params"]["arguments"];
                match name {
                    "echo" => json!({
                        "content": [{ "type": "text", "text": args["text"].as_str().unwrap_or("") }],
                        "isError": false
                    }),
                    "fail" => json!({
                        "content": [{ "type": "text", "text": "it did not work" }],
                        "isError": true
                    }),
                    other => {
                        reply(&mut out, json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32602, "message": format!("unknown tool `{other}`") }
                        }));
                        continue;
                    }
                }
            }
            other => {
                reply(&mut out, json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("unknown method `{other}`") }
                }));
                continue;
            }
        };
        reply(&mut out, json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }
}

fn reply(out: &mut std::io::Stdout, value: Value) {
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}
