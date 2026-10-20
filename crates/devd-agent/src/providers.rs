//! Model providers: presets, the official model catalogue, and the two wire
//! formats devd speaks (Anthropic Messages and OpenAI-compatible chat
//! completions).
//!
//! Official model data comes from the vendors themselves. For Anthropic that is
//! the models overview page in the documentation (parsed from its Markdown
//! table, no key needed) and `GET /v1/models` (authoritative for an account).
//! Every other provider is read from its `/models` endpoint, since those vendors
//! publish model lists in the API rather than in a parseable page.

use std::time::{Duration, Instant};

use regex::Regex;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

use devd_core::{util, CatalogueModel, DevdError, Provider, Result};

/// The Anthropic API version header every request carries.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The page the Anthropic model table is read from.
pub const ANTHROPIC_MODELS_DOC: &str = "https://platform.claude.com/docs/en/about-claude/models/overview.md";

/// A ready-made provider configuration.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Preset {
    /// Suggested id.
    pub id: String,
    /// `anthropic`, `openai` or `openai-compatible`.
    pub kind: String,
    /// Display name.
    pub label: String,
    /// API base URL.
    pub base_url: String,
    /// The environment variable the vendor's tools conventionally read.
    pub key_env: Option<String>,
    /// Whether a key is required.
    pub requires_key: bool,
    /// Where to create a key.
    pub key_url: Option<String>,
    /// API documentation.
    pub docs_url: String,
    /// How devd lists models for it.
    pub models_from: String,
    /// What to know before using it.
    pub notes: String,
}

fn preset(id: &str, kind: &str, label: &str, base: &str, env: Option<&str>, requires_key: bool, key_url: Option<&str>, docs: &str, models_from: &str, notes: &str) -> Preset {
    Preset {
        id: id.into(),
        kind: kind.into(),
        label: label.into(),
        base_url: base.into(),
        key_env: env.map(|s| s.into()),
        requires_key,
        key_url: key_url.map(|s| s.into()),
        docs_url: docs.into(),
        models_from: models_from.into(),
        notes: notes.into(),
    }
}

/// The providers devd knows how to set up.
pub fn presets() -> Vec<Preset> {
    vec![
        preset("anthropic", "anthropic", "Anthropic (Claude)", "https://api.anthropic.com", Some("ANTHROPIC_API_KEY"), true,
            Some("https://platform.claude.com/settings/keys"), "https://platform.claude.com/docs/en/api/overview",
            "docs table (no key) and GET /v1/models (with key)",
            "Messages API with tool use. Requests send x-api-key and anthropic-version: 2023-06-01. This is the provider the built-in agent is tuned for."),
        preset("openai", "openai", "OpenAI", "https://api.openai.com/v1", Some("OPENAI_API_KEY"), true,
            Some("https://platform.openai.com/api-keys"), "https://platform.openai.com/docs/api-reference/chat",
            "GET /v1/models (needs a key)", "Chat Completions with function tools."),
        preset("gemini", "openai-compatible", "Google Gemini (OpenAI-compatible)", "https://generativelanguage.googleapis.com/v1beta/openai", Some("GEMINI_API_KEY"), true,
            Some("https://aistudio.google.com/apikey"), "https://ai.google.dev/gemini-api/docs/openai",
            "GET /models (needs a key)", "Google's OpenAI compatibility layer; model ids may be prefixed `models/`."),
        preset("openrouter", "openai-compatible", "OpenRouter", "https://openrouter.ai/api/v1", Some("OPENROUTER_API_KEY"), true,
            Some("https://openrouter.ai/keys"), "https://openrouter.ai/docs/api-reference/overview",
            "GET /models (public list)", "One key for many vendors; ids look like `anthropic/claude-sonnet-5-5`."),
        preset("groq", "openai-compatible", "Groq", "https://api.groq.com/openai/v1", Some("GROQ_API_KEY"), true,
            Some("https://console.groq.com/keys"), "https://console.groq.com/docs/openai", "GET /models (needs a key)", "Fast inference of open models."),
        preset("ollama", "openai-compatible", "Ollama (local)", "http://localhost:11434/v1", None, false,
            None, "https://github.com/ollama/ollama/blob/main/docs/openai.md", "GET /v1/models (local, no key)",
            "Runs on this machine; nothing leaves it. Tool use depends on the model you pull."),
        preset("lm-studio", "openai-compatible", "LM Studio (local)", "http://localhost:1234/v1", None, false,
            None, "https://lmstudio.ai/docs/app/api/endpoints/openai", "GET /v1/models (local, no key)", "Start the local server in LM Studio first."),
        preset("custom", "openai-compatible", "Custom OpenAI-compatible endpoint", "https://", None, false,
            None, "https://platform.openai.com/docs/api-reference/chat", "GET /models", "Any server implementing /models and /chat/completions."),
    ]
}

/// Whether a host is this machine.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]") || host.ends_with(".localhost")
}

/// Validate a base URL. API keys are sent to it, so plain HTTP is accepted only
/// for this machine.
pub fn validate_base_url(url: &str) -> Result<String> {
    let url = url.trim().trim_end_matches('/').to_string();
    let (scheme, rest) = url.split_once("://").ok_or_else(|| DevdError::invalid("the base URL must start with https://"))?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(DevdError::invalid("the base URL needs a host and must not contain credentials"));
    }
    if url.contains('?') || url.contains('#') || url.contains(' ') {
        return Err(DevdError::invalid("the base URL must not contain a query, fragment or spaces"));
    }
    let host = authority.rsplit_once(':').map(|(h, p)| if p.chars().all(|c| c.is_ascii_digit()) { h } else { authority }).unwrap_or(authority);
    match scheme {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        "http" => Err(DevdError::invalid("plain http is only allowed for this machine (localhost); use https so the API key is not sent in the clear")),
        _ => Err(DevdError::invalid("the base URL must use https")),
    }
}

/// The extra certificate authorities a managed network asks tools to trust
/// (the conventional environment variables), added to the built-in roots.
fn tls_config() -> Option<std::sync::Arc<ureq::rustls::ClientConfig>> {
    use base64::Engine;
    let path = ["DEVD_CA_BUNDLE", "SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|p| std::path::Path::new(p).is_file())?;
    let pem = std::fs::read_to_string(path).ok()?;
    let mut roots = ureq::rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for block in pem.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let Some(body) = block.split("-----END CERTIFICATE-----").next() else { continue };
        let b64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(b64) {
            let _ = roots.add(ureq::rustls::pki_types::CertificateDer::from(der));
        }
    }
    let provider = std::sync::Arc::new(ureq::rustls::crypto::ring::default_provider());
    let cfg = ureq::rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Some(std::sync::Arc::new(cfg))
}

/// Whether NO_PROXY excludes this host.
fn bypass_proxy(host: &str) -> bool {
    if is_loopback(host) {
        return true;
    }
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.')).filter(|e| !e.is_empty()).any(|e| e == "*" || host == e || host.ends_with(&format!(".{e}")))
}

pub(crate) fn agent_for(url: &str) -> ureq::Agent {
    build_agent(url, true)
}

/// The same client, but a redirect is returned to the caller so it can be checked.
pub(crate) fn agent_no_redirect(url: &str) -> ureq::Agent {
    build_agent(url, false)
}

fn build_agent(url: &str, follow: bool) -> ureq::Agent {
    let host = host_of(url).split(':').next().unwrap_or("").to_string();
    let mut b = ureq::AgentBuilder::new();
    if !bypass_proxy(&host) {
        let var = if url.starts_with("https") { ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] } else { ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] };
        if let Some(p) = var.iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).and_then(|v| ureq::Proxy::new(v).ok()) {
            b = b.proxy(p);
        }
    }
    if let Some(cfg) = tls_config() {
        b = b.tls_config(cfg);
    }
    if !follow {
        b = b.redirects(0);
    }
    b.timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(if follow { 300 } else { 30 }))
        .timeout_write(Duration::from_secs(60))
        .user_agent(&format!("devd/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

/// A response from a provider.
struct Reply {
    status: u16,
    body: String,
    retry_after: Option<u64>,
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_string()
}

fn send(method: &str, url: &str, headers: &[(String, String)], body: Option<&str>) -> Result<Reply> {
    let a = agent_for(url);
    let mut req = a.request(method, url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let res = match body {
        Some(b) => req.set("content-type", "application/json").send_string(b),
        None => req.call(),
    };
    match res {
        Ok(r) => {
            let status = r.status();
            Ok(Reply { status, retry_after: None, body: r.into_string().map_err(|e| DevdError::internal(format!("could not read the response from {}: {e}", host_of(url))))? })
        }
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r.header("retry-after").and_then(|v| v.parse::<u64>().ok());
            Ok(Reply { status: code, retry_after, body: r.into_string().unwrap_or_default() })
        }
        Err(e) => Err(DevdError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            format!("could not reach {}: {e}", host_of(url)),
        ))),
    }
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message").and_then(|m| m.as_str()).map(|s| s.to_string()).or_else(|| v.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| util::truncate(body.trim(), 300))
}

fn status_error(p: &Provider, r: &Reply) -> DevdError {
    let msg = error_message(&r.body);
    match r.status {
        401 | 403 => DevdError::denied(format!("{} rejected the API key (HTTP {}): {msg}. Check or replace it in Providers.", p.label, r.status)),
        404 => DevdError::not_found(format!("{} returned 404: {msg}. The model id or base URL may be wrong.", p.label)),
        400 | 422 => DevdError::invalid(format!("{} refused the request (HTTP {}): {msg}", p.label, r.status)),
        429 => DevdError::conflict(format!("{} is rate limiting this key (HTTP 429): {msg}{}", p.label, r.retry_after.map(|s| format!(". Retry after {s}s")).unwrap_or_default())),
        s if s >= 500 => DevdError::Timeout(format!("{} is unavailable or overloaded (HTTP {s}): {msg}. Try again shortly.", p.label)),
        s => DevdError::internal(format!("{} returned HTTP {s}: {msg}", p.label)),
    }
}

fn auth_headers(p: &Provider, key: Option<&str>) -> Vec<(String, String)> {
    let mut h = Vec::new();
    match p.kind.as_str() {
        "anthropic" => {
            h.push(("anthropic-version".to_string(), ANTHROPIC_VERSION.to_string()));
            if let Some(k) = key {
                h.push(("x-api-key".to_string(), k.to_string()));
            }
        }
        _ => {
            if let Some(k) = key {
                h.push(("authorization".to_string(), format!("Bearer {k}")));
            }
        }
    }
    h
}

fn models_url(p: &Provider) -> String {
    if p.kind == "anthropic" {
        format!("{}/v1/models?limit=1000", p.base_url.trim_end_matches('/'))
    } else {
        format!("{}/models", p.base_url.trim_end_matches('/'))
    }
}

/// List the models an account can use, from the provider's own API.
pub fn list_models(p: &Provider, key: Option<&str>) -> Result<Vec<CatalogueModel>> {
    let r = send("GET", &models_url(p), &auth_headers(p, key), None)?;
    if r.status != 200 {
        return Err(status_error(p, &r));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|e| DevdError::internal(format!("{} returned something that is not JSON: {e}", p.label)))?;
    let now = util::now();
    let items = v.get("data").and_then(|d| d.as_array()).cloned().or_else(|| v.get("models").and_then(|d| d.as_array()).cloned()).unwrap_or_default();
    let mut out: Vec<CatalogueModel> = items
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|x| x.as_str())?.to_string();
            let display = m.get("display_name").or_else(|| m.get("name")).and_then(|x| x.as_str()).unwrap_or(&id).to_string();
            let context = m.get("max_input_tokens").or_else(|| m.get("context_length")).or_else(|| m.get("context_window")).and_then(|x| x.as_i64());
            let max_out = m.get("max_tokens").or_else(|| m.get("max_output_tokens")).and_then(|x| x.as_i64());
            let price = |k: &str| m.pointer(&format!("/pricing/{k}")).and_then(|x| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64())).map(|x| x * 1_000_000.0);
            Some(CatalogueModel {
                source: format!("provider:{}", p.id),
                model_id: id,
                display,
                description: m.get("description").and_then(|x| x.as_str()).map(|s| s.to_string()),
                context_window: context,
                max_output: max_out,
                input_price: price("prompt"),
                output_price: price("completion"),
                retirement: m.get("deprecation_date").and_then(|x| x.as_str()).map(|s| s.to_string()),
                url: Some(models_url(p)),
                fetched_at: now.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    Ok(out)
}

fn plain(cell: &str) -> String {
    let link = Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap();
    link.replace_all(cell, "$1").replace('`', "").replace("\\_", "_").trim().to_string()
}

fn tokens(text: &str) -> Option<i64> {
    let re = Regex::new(r"(?i)([\d.,]+)\s*([KM])\b").unwrap();
    let c = re.captures(text)?;
    let n: f64 = c[1].replace(',', "").parse().ok()?;
    Some((n * if c[2].eq_ignore_ascii_case("m") { 1_000_000.0 } else { 1_000.0 }) as i64)
}

/// Parse the model comparison tables from the Anthropic models overview page.
pub fn parse_anthropic_docs(markdown: &str, now: &str) -> Vec<CatalogueModel> {
    let price = Regex::new(r"\$([\d.]+)\s*/\s*input MTok,\s*(?:From\s*)?\$([\d.]+)\s*/\s*output MTok").unwrap();
    let mut out: Vec<CatalogueModel> = Vec::new();
    let mut table: Vec<Vec<String>> = Vec::new();
    let mut tables: Vec<Vec<Vec<String>>> = Vec::new();
    for line in markdown.lines().chain(std::iter::once("")) {
        let t = line.trim();
        if t.starts_with('|') {
            let cells: Vec<String> = t.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            if cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')) {
                continue;
            }
            table.push(cells);
        } else if !table.is_empty() {
            tables.push(std::mem::take(&mut table));
        }
    }
    for t in tables {
        let Some(header) = t.first() else { continue };
        let row = |label: &str| t.iter().find(|r| plain(&r[0]).eq_ignore_ascii_case(label));
        let Some(ids) = row("Claude API ID") else { continue };
        for col in 1..header.len() {
            let Some(id_cell) = ids.get(col) else { continue };
            let id = plain(id_cell);
            if !id.starts_with("claude-") || out.iter().any(|m| m.model_id == id) {
                continue;
            }
            let cell = |label: &str| row(label).and_then(|r| r.get(col)).map(|c| plain(c)).filter(|c| !c.is_empty() && c != "-");
            let (inp, outp) = cell("Pricing").and_then(|p| price.captures(&p).map(|c| (c[1].parse().ok(), c[2].parse().ok()))).unwrap_or((None, None));
            let name = plain(&header[col]);
            let page = row("Model page").and_then(|r| r.get(col)).and_then(|c| Regex::new(r"\]\(([^)]*)\)").unwrap().captures(c).map(|m| m[1].to_string()));
            out.push(CatalogueModel {
                source: "anthropic-docs".into(),
                model_id: id,
                display: name,
                description: cell("Description"),
                context_window: cell("Context window").and_then(|c| tokens(&c)),
                max_output: cell("Max output").and_then(|c| tokens(&c)),
                input_price: inp,
                output_price: outp,
                retirement: cell("Retirement"),
                url: page.or_else(|| Some(ANTHROPIC_MODELS_DOC.trim_end_matches(".md").to_string())),
                fetched_at: now.to_string(),
            });
        }
    }
    out
}

/// Fetch the official Anthropic model list from the documentation. Needs no key.
pub fn fetch_anthropic_docs() -> Result<Vec<CatalogueModel>> {
    fetch_docs_from(ANTHROPIC_MODELS_DOC)
}

/// The same, from a given URL (tests point it at a local server).
pub fn fetch_docs_from(url: &str) -> Result<Vec<CatalogueModel>> {
    let r = send("GET", url, &[], None)?;
    if r.status != 200 {
        return Err(DevdError::internal(format!("the Anthropic documentation returned HTTP {} for {url}", r.status)));
    }
    let models = parse_anthropic_docs(&r.body, &util::now());
    if models.is_empty() {
        return Err(DevdError::internal("the Anthropic models page was fetched but no model table was found in it; its layout may have changed"));
    }
    Ok(models)
}

/// The outcome of testing a provider.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TestReport {
    /// The provider answered and accepted the credentials.
    pub ok: bool,
    /// How long it took.
    pub latency_ms: u128,
    /// How many models the account can see.
    pub models: usize,
    /// What happened, in a sentence.
    pub message: String,
}

/// Check that the base URL is reachable and the key is accepted.
pub fn test(p: &Provider, key: Option<&str>) -> TestReport {
    let started = Instant::now();
    if key.is_none() && !is_loopback(&host_of(&p.base_url).split(':').next().unwrap_or("").to_string()) && p.kind != "openai-compatible" {
        return TestReport { ok: false, latency_ms: 0, models: 0, message: format!("no API key is set for {}", p.label) };
    }
    match list_models(p, key) {
        Ok(m) => TestReport { ok: true, latency_ms: started.elapsed().as_millis(), models: m.len(), message: format!("connected to {}; {} model(s) available to this key", p.label, m.len()) },
        Err(e) => TestReport { ok: false, latency_ms: started.elapsed().as_millis(), models: 0, message: e.to_string() },
    }
}

// ------------------------------------------------------------------- chat

/// A tool the model may call.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// JSON Schema of the input.
    pub schema: Value,
}

/// One request to a model. Messages use Anthropic-style content blocks as the
/// neutral format; the OpenAI adapter converts them.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Model id.
    pub model: String,
    /// System prompt.
    pub system: String,
    /// `{role, content: [blocks]}` messages.
    pub messages: Vec<Value>,
    /// Tools.
    pub tools: Vec<ToolSpec>,
    /// Output limit.
    pub max_tokens: u32,
}

/// A model's answer.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// Content blocks (`text`, `tool_use`, and for Anthropic any others, kept verbatim).
    pub content: Vec<Value>,
    /// `end_turn`, `tool_use`, `max_tokens`, ...
    pub stop_reason: String,
    /// Input tokens.
    pub input_tokens: i64,
    /// Output tokens.
    pub output_tokens: i64,
}

fn anthropic_body(r: &ChatRequest) -> Value {
    let mut body = json!({
        "model": r.model,
        "max_tokens": r.max_tokens,
        // The system prompt is stable across turns, so mark it cacheable.
        "system": [{ "type": "text", "text": r.system, "cache_control": { "type": "ephemeral" } }],
        "messages": r.messages,
    });
    if !r.tools.is_empty() {
        body["tools"] = json!(r.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema })).collect::<Vec<_>>());
    }
    body
}

fn text_of(blocks: &[Value]) -> String {
    blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")
}

fn openai_body(p: &Provider, r: &ChatRequest) -> Value {
    let mut msgs = vec![json!({ "role": "system", "content": r.system })];
    for m in &r.messages {
        let role = m["role"].as_str().unwrap_or("user");
        let blocks: Vec<Value> = m["content"].as_array().cloned().unwrap_or_default();
        if role == "assistant" {
            let calls: Vec<Value> = blocks
                .iter()
                .filter(|b| b["type"] == "tool_use")
                .map(|b| json!({ "id": b["id"], "type": "function", "function": { "name": b["name"], "arguments": b["input"].to_string() } }))
                .collect();
            let mut msg = json!({ "role": "assistant", "content": text_of(&blocks) });
            if !calls.is_empty() {
                msg["tool_calls"] = json!(calls);
                if msg["content"] == "" {
                    msg["content"] = Value::Null;
                }
            }
            msgs.push(msg);
        } else {
            for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                let content = match &b["content"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                msgs.push(json!({ "role": "tool", "tool_call_id": b["tool_use_id"], "content": content }));
            }
            let t = text_of(&blocks);
            if !t.is_empty() {
                msgs.push(json!({ "role": "user", "content": t }));
            }
        }
    }
    let mut body = json!({ "model": r.model, "messages": msgs });
    body[if p.kind == "openai" { "max_completion_tokens" } else { "max_tokens" }] = json!(r.max_tokens);
    if !r.tools.is_empty() {
        body["tools"] = json!(r.tools.iter().map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } })).collect::<Vec<_>>());
    }
    body
}

fn parse_anthropic(v: &Value) -> Result<ChatResponse> {
    let content = v.get("content").and_then(|c| c.as_array()).cloned().ok_or_else(|| DevdError::internal("the response had no content"))?;
    Ok(ChatResponse {
        content,
        stop_reason: v["stop_reason"].as_str().unwrap_or("end_turn").to_string(),
        input_tokens: v.pointer("/usage/input_tokens").and_then(|x| x.as_i64()).unwrap_or(0)
            + v.pointer("/usage/cache_read_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0)
            + v.pointer("/usage/cache_creation_input_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
        output_tokens: v.pointer("/usage/output_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
    })
}

fn parse_openai(v: &Value) -> Result<ChatResponse> {
    let msg = v.pointer("/choices/0/message").ok_or_else(|| DevdError::internal("the response had no choices"))?;
    let mut content = Vec::new();
    if let Some(t) = msg["content"].as_str().filter(|t| !t.is_empty()) {
        content.push(json!({ "type": "text", "text": t }));
    }
    let mut has_calls = false;
    for c in msg["tool_calls"].as_array().cloned().unwrap_or_default() {
        has_calls = true;
        let args = c.pointer("/function/arguments").and_then(|a| a.as_str()).unwrap_or("{}");
        content.push(json!({
            "type": "tool_use",
            "id": c["id"].as_str().unwrap_or("call_0"),
            "name": c.pointer("/function/name").and_then(|n| n.as_str()).unwrap_or(""),
            "input": serde_json::from_str::<Value>(args).unwrap_or_else(|_| json!({})),
        }));
    }
    let finish = v.pointer("/choices/0/finish_reason").and_then(|f| f.as_str()).unwrap_or("stop");
    Ok(ChatResponse {
        content,
        stop_reason: if has_calls { "tool_use".into() } else if finish == "length" { "max_tokens".into() } else { "end_turn".into() },
        input_tokens: v.pointer("/usage/prompt_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
        output_tokens: v.pointer("/usage/completion_tokens").and_then(|x| x.as_i64()).unwrap_or(0),
    })
}

/// Send one request, retrying transient failures (rate limits, overload).
pub fn chat(p: &Provider, key: Option<&str>, req: &ChatRequest) -> Result<ChatResponse> {
    let (url, body) = if p.kind == "anthropic" {
        (format!("{}/v1/messages", p.base_url.trim_end_matches('/')), anthropic_body(req))
    } else {
        (format!("{}/chat/completions", p.base_url.trim_end_matches('/')), openai_body(p, req))
    };
    let headers = auth_headers(p, key);
    let text = body.to_string();
    let mut last: Option<DevdError> = None;
    for attempt in 0..3u32 {
        let r = send("POST", &url, &headers, Some(&text))?;
        if r.status == 200 {
            let v: Value = serde_json::from_str(&r.body).map_err(|e| DevdError::internal(format!("{} returned something that is not JSON: {e}", p.label)))?;
            return if p.kind == "anthropic" { parse_anthropic(&v) } else { parse_openai(&v) };
        }
        let transient = matches!(r.status, 429 | 500 | 502 | 503 | 504 | 529);
        let err = status_error(p, &r);
        if !transient || attempt == 2 {
            return Err(err);
        }
        let wait = r.retry_after.unwrap_or(1u64 << attempt).min(20);
        std::thread::sleep(Duration::from_secs(wait));
        last = Some(err);
    }
    Err(last.unwrap_or_else(|| DevdError::internal("request failed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
| Feature | [Claude Fable 5.1](https://x/fable) | Claude Haiku 5.5 |
| :--- | :--- | :--- |
| Description | For demanding reasoning | Fastest |
| Model page | [Claude Fable 5.1](https://platform.claude.com/docs/en/models/fable-5-1/overview) | [Claude Haiku 5.5](https://p/h) |
| [Pricing](https://p) | $10 / input MTok, $50 / output MTok | From $0.10 / input MTok, From $0.50 / output MTok |
| Claude API ID | `claude-fable-5-1` | `claude-haiku-5-5` |
| [Context window](https://c) | 1M tokens | 200k tokens |
| Max output | 128K tokens | 64K tokens |
| [Retirement](https://r) | Not sooner than September 1, 2027 | Not sooner than 2027 |
";

    #[test]
    fn the_official_model_table_is_parsed() {
        let m = parse_anthropic_docs(TABLE, "now");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].model_id, "claude-fable-5-1");
        assert_eq!(m[0].display, "Claude Fable 5.1");
        assert_eq!(m[0].input_price, Some(10.0));
        assert_eq!(m[0].output_price, Some(50.0));
        assert_eq!(m[0].context_window, Some(1_000_000));
        assert_eq!(m[0].max_output, Some(128_000));
        assert_eq!(m[1].input_price, Some(0.10));
        assert_eq!(m[1].context_window, Some(200_000));
        assert!(m[0].url.as_deref().unwrap().contains("fable-5-1"));
    }

    #[test]
    fn base_urls_protect_the_key() {
        assert!(validate_base_url("https://api.anthropic.com/").is_ok());
        assert!(validate_base_url("http://localhost:11434/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://api.example.com/v1").is_err());
        assert!(validate_base_url("https://user:pw@api.example.com").is_err());
        assert!(validate_base_url("https://api.example.com/v1?key=1").is_err());
        assert!(validate_base_url("ftp://x").is_err());
        assert!(validate_base_url("https://").is_err());
    }

    #[test]
    fn messages_convert_between_the_two_wire_formats() {
        let p = Provider { id: "o".into(), kind: "openai".into(), label: "O".into(), base_url: "https://x".into(), key_env: None, default_model: None, is_default: false, enabled: true, created_at: "".into(), updated_at: "".into() };
        let req = ChatRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![
                json!({"role":"user","content":[{"type":"text","text":"hi"}]}),
                json!({"role":"assistant","content":[{"type":"text","text":"ok"},{"type":"tool_use","id":"c1","name":"devd_groups","input":{}}]}),
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"done"}]}),
            ],
            tools: vec![ToolSpec { name: "devd_groups".into(), description: "d".into(), schema: json!({"type":"object"}) }],
            max_tokens: 100,
        };
        let b = openai_body(&p, &req);
        let m = b["messages"].as_array().unwrap();
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "devd_groups");
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[3]["tool_call_id"], "c1");
        assert_eq!(b["max_completion_tokens"], 100);
        let back = parse_openai(&json!({"choices":[{"message":{"content":null,"tool_calls":[{"id":"x","function":{"name":"n","arguments":"{\"a\":1}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2}})).unwrap();
        assert_eq!(back.stop_reason, "tool_use");
        assert_eq!(back.content[0]["input"]["a"], 1);
    }
}
