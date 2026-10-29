//! The native Gemini wire format (`generateContent`).
//!
//! Google also offers an OpenAI-compatibility layer, and that remains available
//! as an `openai-compatible` provider. The native API is used when it matters:
//! it returns the real input and output token limits, it carries the thought
//! signatures that newer models require to be sent back with a function call,
//! and its function schemas are the ones the models are trained on.
//!
//! Messages use the same neutral content blocks as every other adapter
//! (`text`, `tool_use`, `tool_result`); this module converts them to and from
//! Gemini's `contents`, `parts` and `functionCall` shapes.

use std::io::BufRead;

use serde_json::{json, Value};

use devd_core::store_ai::CatalogueModel;
use devd_core::{DevdError, Result};

use crate::providers::{ChatRequest, ChatResponse};

/// Keys Gemini's function-declaration schema does not accept.
const UNSUPPORTED_SCHEMA_KEYS: &[&str] = &[
    "$schema",
    "$id",
    "$ref",
    "$defs",
    "definitions",
    "additionalProperties",
    "unevaluatedProperties",
    "patternProperties",
    "title",
    "default",
    "examples",
    "const",
    "oneOf",
    "allOf",
    "not",
];

/// Reduce a JSON Schema to the subset Gemini accepts.
///
/// Gemini rejects a request outright for a keyword it does not know, so the
/// schema is rewritten rather than passed through. `anyOf` is kept when every
/// branch survives, which is how optional values are usually written.
pub fn sanitize_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                if UNSUPPORTED_SCHEMA_KEYS.contains(&key.as_str()) {
                    continue;
                }
                match key.as_str() {
                    "properties" => {
                        if let Value::Object(props) = value {
                            let cleaned: serde_json::Map<String, Value> = props
                                .iter()
                                .map(|(k, v)| (k.clone(), sanitize_schema(v)))
                                .collect();
                            out.insert(key.clone(), Value::Object(cleaned));
                        }
                    }
                    // `"type": ["string", "null"]` becomes a nullable string.
                    "type" => match value {
                        Value::Array(types) => {
                            let named: Vec<&str> = types
                                .iter()
                                .filter_map(Value::as_str)
                                .filter(|t| *t != "null")
                                .collect();
                            if let Some(first) = named.first() {
                                out.insert("type".into(), json!(first));
                            }
                            if types.iter().any(|t| t == "null") {
                                out.insert("nullable".into(), json!(true));
                            }
                        }
                        other => {
                            out.insert("type".into(), other.clone());
                        }
                    },
                    _ => {
                        out.insert(key.clone(), sanitize_schema(value));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_schema).collect()),
        other => other.clone(),
    }
}

fn function_name_for(messages: &[Value], tool_use_id: &str) -> String {
    for m in messages {
        if let Some(blocks) = m["content"].as_array() {
            for b in blocks {
                if b["type"] == "tool_use" && b["id"] == tool_use_id {
                    return b["name"].as_str().unwrap_or("tool").to_string();
                }
            }
        }
    }
    "tool".to_string()
}

fn response_object(content: &Value) -> Value {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    };
    // Gemini wants an object. A JSON object result is passed through as one.
    match serde_json::from_str::<Value>(&text) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({ "output": text }),
    }
}

/// The request body for `generateContent` / `streamGenerateContent`.
pub fn body(r: &ChatRequest) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    for m in &r.messages {
        let role = m["role"].as_str().unwrap_or("user");
        let blocks: Vec<Value> = m["content"].as_array().cloned().unwrap_or_default();
        let mut parts: Vec<Value> = Vec::new();
        if role == "assistant" {
            for b in &blocks {
                match b["type"].as_str() {
                    Some("text") => {
                        if let Some(t) = b["text"].as_str().filter(|t| !t.is_empty()) {
                            parts.push(json!({ "text": t }));
                        }
                    }
                    Some("tool_use") => {
                        let mut part = json!({ "functionCall": { "name": b["name"], "args": b["input"] } });
                        // Newer models reject a function call that arrives
                        // without the signature they issued with it.
                        if let Some(sig) = b.get("thought_signature").and_then(Value::as_str) {
                            part["thoughtSignature"] = json!(sig);
                        }
                        parts.push(part);
                    }
                    _ => {}
                }
            }
            if !parts.is_empty() {
                contents.push(json!({ "role": "model", "parts": parts }));
            }
        } else {
            for b in &blocks {
                match b["type"].as_str() {
                    Some("tool_result") => {
                        let id = b["tool_use_id"].as_str().unwrap_or("");
                        parts.push(json!({
                            "functionResponse": {
                                "name": function_name_for(&r.messages, id),
                                "response": response_object(&b["content"]),
                            }
                        }));
                    }
                    Some("text") => {
                        if let Some(t) = b["text"].as_str().filter(|t| !t.is_empty()) {
                            parts.push(json!({ "text": t }));
                        }
                    }
                    _ => {}
                }
            }
            if !parts.is_empty() {
                contents.push(json!({ "role": "user", "parts": parts }));
            }
        }
    }

    let mut body = json!({
        "systemInstruction": { "parts": [{ "text": r.system }] },
        "contents": contents,
        "generationConfig": { "maxOutputTokens": r.max_tokens },
    });
    if !r.tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": r.tools.iter().map(|t| json!({
                "name": t.name,
                "description": t.description,
                "parameters": sanitize_schema(&t.schema),
            })).collect::<Vec<_>>()
        }]);
        if let Some(name) = &r.tool_choice {
            body["toolConfig"] = json!({
                "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [name] }
            });
        }
    }
    body
}

/// What one response (or one streamed chunk) contributes.
#[derive(Default)]
struct Accumulator {
    content: Vec<Value>,
    finish: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    calls: usize,
}

impl Accumulator {
    fn feed(&mut self, v: &Value, on: &mut dyn FnMut(&str)) {
        if let Some(parts) = v.pointer("/candidates/0/content/parts").and_then(Value::as_array) {
            for part in parts {
                if part["thought"] == true {
                    continue;
                }
                if let Some(text) = part["text"].as_str().filter(|t| !t.is_empty()) {
                    on(text);
                    // Consecutive text chunks are one block.
                    if let Some(last) = self.content.last_mut().filter(|b| b["type"] == "text") {
                        let joined = format!("{}{}", last["text"].as_str().unwrap_or(""), text);
                        last["text"] = json!(joined);
                    } else {
                        self.content.push(json!({ "type": "text", "text": text }));
                    }
                }
                if let Some(call) = part.get("functionCall") {
                    let name = call["name"].as_str().unwrap_or("");
                    let mut block = json!({
                        "type": "tool_use",
                        // Gemini does not number its calls; the id only has to
                        // be unique within the conversation.
                        "id": format!("gemini_{}_{}", self.calls, name),
                        "name": name,
                        "input": call.get("args").cloned().unwrap_or_else(|| json!({})),
                    });
                    if let Some(sig) = part["thoughtSignature"].as_str() {
                        block["thought_signature"] = json!(sig);
                    }
                    self.calls += 1;
                    self.content.push(block);
                }
            }
        }
        if let Some(reason) = v.pointer("/candidates/0/finishReason").and_then(Value::as_str) {
            self.finish = Some(reason.to_string());
        }
        if let Some(n) = v.pointer("/usageMetadata/promptTokenCount").and_then(Value::as_i64) {
            self.input_tokens = n;
        }
        let out = v
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + v.pointer("/usageMetadata/thoughtsTokenCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
        if out > 0 {
            self.output_tokens = out;
        }
    }

    fn finish(self) -> Result<ChatResponse> {
        if self.content.is_empty() && self.finish.is_none() {
            return Err(DevdError::internal("Gemini returned no candidates"));
        }
        let stop_reason = if self.calls > 0 {
            "tool_use"
        } else if self.finish.as_deref() == Some("MAX_TOKENS") {
            "max_tokens"
        } else {
            "end_turn"
        };
        Ok(ChatResponse {
            content: self.content,
            stop_reason: stop_reason.to_string(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

/// Parse a complete (non-streamed) response.
pub fn parse(v: &Value) -> Result<ChatResponse> {
    if let Some(err) = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        return Err(DevdError::invalid(format!(
            "Gemini refused the prompt: {err}"
        )));
    }
    let mut acc = Accumulator::default();
    acc.feed(v, &mut |_| {});
    acc.finish()
}

/// Parse a `streamGenerateContent?alt=sse` stream.
pub fn parse_stream(
    reader: impl BufRead,
    on: &mut dyn FnMut(&str),
) -> Result<ChatResponse> {
    let mut acc = Accumulator::default();
    crate::providers::for_each_event(reader, |payload| {
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
                return Err(DevdError::invalid(format!(
                    "Gemini refused the prompt: {reason}"
                )));
            }
            acc.feed(&v, on);
        }
        Ok(())
    })?;
    acc.finish()
}

/// The model list from `GET /models`, keeping only models that can chat.
pub fn models(v: &Value, source: &str, now: &str) -> Vec<CatalogueModel> {
    let mut out = Vec::new();
    for m in v["models"].as_array().cloned().unwrap_or_default() {
        let methods = m["supportedGenerationMethods"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !methods.iter().any(|x| x == "generateContent") {
            continue;
        }
        let name = m["name"].as_str().unwrap_or("");
        let id = name.strip_prefix("models/").unwrap_or(name);
        if id.is_empty() || out.iter().any(|x: &CatalogueModel| x.model_id == id) {
            continue;
        }
        out.push(CatalogueModel {
            source: source.to_string(),
            model_id: id.to_string(),
            display: m["displayName"].as_str().unwrap_or(id).to_string(),
            description: m["description"].as_str().map(str::to_string),
            context_window: m["inputTokenLimit"].as_i64(),
            max_output: m["outputTokenLimit"].as_i64(),
            input_price: None,
            output_price: None,
            retirement: None,
            url: Some("https://ai.google.dev/gemini-api/docs/models".to_string()),
            fetched_at: now.to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolSpec;

    fn request() -> ChatRequest {
        ChatRequest {
            model: "gemini-2.5-flash".into(),
            system: "be brief".into(),
            messages: vec![
                json!({ "role": "user", "content": [{ "type": "text", "text": "list files" }] }),
                json!({ "role": "assistant", "content": [
                    { "type": "text", "text": "ok" },
                    { "type": "tool_use", "id": "gemini_0_fs_list", "name": "fs_list",
                      "input": { "path": "." }, "thought_signature": "sig-1" }
                ]}),
                json!({ "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "gemini_0_fs_list", "content": "a.txt\nb.txt" }
                ]}),
            ],
            tools: vec![ToolSpec {
                name: "fs_list".into(),
                description: "list".into(),
                schema: json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "additionalProperties": false,
                    "properties": { "path": { "type": ["string", "null"], "title": "Path" } },
                    "required": ["path"]
                }),
            }],
            tool_choice: None,
            max_tokens: 1000,
        }
    }

    #[test]
    fn the_schema_is_reduced_to_what_gemini_accepts() {
        let cleaned = sanitize_schema(&request().tools[0].schema);
        assert!(cleaned.get("$schema").is_none());
        assert!(cleaned.get("additionalProperties").is_none());
        assert_eq!(cleaned["properties"]["path"]["type"], "string");
        assert_eq!(cleaned["properties"]["path"]["nullable"], true);
        assert!(cleaned["properties"]["path"].get("title").is_none());
        assert_eq!(cleaned["required"][0], "path");
    }

    #[test]
    fn a_conversation_becomes_contents_with_calls_and_responses() {
        let b = body(&request());
        assert_eq!(b["systemInstruction"]["parts"][0]["text"], "be brief");
        assert_eq!(b["generationConfig"]["maxOutputTokens"], 1000);
        let contents = b["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][1]["functionCall"]["name"], "fs_list");
        // The signature the model issued goes back with its call.
        assert_eq!(contents[1]["parts"][1]["thoughtSignature"], "sig-1");
        // A result is answered under the function's name, not the call's id.
        let response = &contents[2]["parts"][0]["functionResponse"];
        assert_eq!(response["name"], "fs_list");
        assert_eq!(response["response"]["output"], "a.txt\nb.txt");
        assert_eq!(
            b["tools"][0]["functionDeclarations"][0]["name"],
            "fs_list"
        );
    }

    #[test]
    fn a_forced_tool_becomes_a_function_calling_config() {
        let mut r = request();
        r.tool_choice = Some("fs_list".into());
        let b = body(&r);
        assert_eq!(b["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        assert_eq!(
            b["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "fs_list"
        );
    }

    #[test]
    fn a_response_with_a_call_stops_for_tool_use_and_keeps_the_signature() {
        let v = json!({
            "candidates": [{ "content": { "parts": [
                { "text": "Looking." },
                { "functionCall": { "name": "fs_list", "args": { "path": "src" } }, "thoughtSignature": "abc" }
            ]}, "finishReason": "STOP" }],
            "usageMetadata": { "promptTokenCount": 12, "candidatesTokenCount": 5, "thoughtsTokenCount": 3 }
        });
        let r = parse(&v).unwrap();
        assert_eq!(r.stop_reason, "tool_use");
        assert_eq!(r.input_tokens, 12);
        assert_eq!(r.output_tokens, 8);
        assert_eq!(r.content[0]["text"], "Looking.");
        assert_eq!(r.content[1]["type"], "tool_use");
        assert_eq!(r.content[1]["input"]["path"], "src");
        assert_eq!(r.content[1]["thought_signature"], "abc");
    }

    #[test]
    fn a_stream_is_rebuilt_and_its_text_is_reported_as_it_arrives() {
        let sse = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}]}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo\"}]},\"finishReason\":\"MAX_TOKENS\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2}}\n\n",
        );
        let mut seen = String::new();
        let r = parse_stream(std::io::Cursor::new(sse), &mut |t| seen.push_str(t)).unwrap();
        assert_eq!(seen, "Hello");
        assert_eq!(r.content[0]["text"], "Hello");
        assert_eq!(r.stop_reason, "max_tokens");
        assert_eq!((r.input_tokens, r.output_tokens), (4, 2));
    }

    #[test]
    fn a_blocked_prompt_is_an_error_not_an_empty_answer() {
        let v = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert!(parse(&v).unwrap_err().to_string().contains("SAFETY"));
        assert!(parse(&json!({})).is_err());
    }

    #[test]
    fn only_models_that_can_generate_are_listed_with_their_real_limits() {
        let v = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash",
              "inputTokenLimit": 1048576, "outputTokenLimit": 65536,
              "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004",
              "supportedGenerationMethods": ["embedContent"] }
        ]});
        let m = models(&v, "provider:gemini", "now");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].model_id, "gemini-2.5-flash");
        assert_eq!(m[0].context_window, Some(1_048_576));
        assert_eq!(m[0].max_output, Some(65_536));
    }
}
