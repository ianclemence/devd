//! Model providers: presets, the official model catalogue, and the two wire
//! formats devd speaks (Anthropic Messages and OpenAI-compatible chat
//! completions).
//!
//! Official model data comes from the vendors themselves. For Anthropic that is
//! the models overview page in the documentation (parsed from its Markdown
//! table, no key needed) and `GET /v1/models` (authoritative for an account).
//! Every other provider is read from its `/models` endpoint, since those vendors
//! publish model lists in the API rather than in a parseable page.

use std::io::{BufRead, BufReader, Read};
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

/// A model in a preset's maintained catalogue, for providers without a listing
/// endpoint or before the user has fetched the live list.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PresetModel {
    /// The id to send to the API.
    pub id: String,
    /// Display name.
    pub display: String,
    /// Context window in tokens, when the vendor states one.
    pub context_window: Option<i64>,
    /// Maximum output tokens, when the vendor states one.
    pub max_output: Option<i64>,
    /// One line about the model.
    pub note: String,
}

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
    /// `cloud`, `local` or `aggregator`. The interface groups by this.
    pub category: String,
    /// Display order. Lower comes first.
    pub order: u32,
    /// A model the vendor recommends for this kind of work, if any. It is only
    /// a suggestion: devd never uses it until the provider's own list contains
    /// it or the user chooses it.
    pub suggested_model: Option<String>,
    /// Tool calling the vendor documents: `documented`, `model` (depends on the
    /// model chosen) or `none`. This is a description of the API, not a claim
    /// that a particular model has passed a probe; `provider.verify` does that.
    pub tools: String,
    /// How `list_models` finds models: `anthropic`, `openai`, `ollama` or
    /// `catalog` (the maintained list below).
    pub discovery: String,
    /// The maintained fallback list. Empty when the provider has a listing API.
    pub catalog: Vec<PresetModel>,
}

impl Preset {
    fn category(mut self, category: &str) -> Self {
        self.category = category.into();
        self
    }
    fn order(mut self, order: u32) -> Self {
        self.order = order;
        self
    }
    fn suggested(mut self, model: &str) -> Self {
        self.suggested_model = Some(model.into());
        self
    }
    fn tools(mut self, tools: &str) -> Self {
        self.tools = tools.into();
        self
    }
    fn discovery(mut self, discovery: &str) -> Self {
        self.discovery = discovery.into();
        self
    }
    fn catalog(mut self, models: Vec<PresetModel>) -> Self {
        self.catalog = models;
        self
    }
}

fn model(id: &str, display: &str, context: Option<i64>, max_output: Option<i64>, note: &str) -> PresetModel {
    PresetModel {
        id: id.into(),
        display: display.into(),
        context_window: context,
        max_output,
        note: note.into(),
    }
}

/// How a preset is authenticated.
struct Key<'a> {
    env: Option<&'a str>,
    required: bool,
    url: Option<&'a str>,
}

/// Where to read about a preset.
struct Docs<'a> {
    page: &'a str,
    models_from: &'a str,
    notes: &'a str,
}

fn base_preset(id: &str, kind: &str, label: &str, base: &str, key: Key, docs: Docs) -> Preset {
    let Key { env, required: requires_key, url: key_url } = key;
    let Docs { page: docs, models_from, notes } = docs;
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
        category: "cloud".into(),
        order: 100,
        suggested_model: None,
        tools: "model".into(),
        discovery: "openai".into(),
        catalog: Vec::new(),
    }
}

/// The providers devd knows how to set up, ordered by how useful they are as a
/// first choice for this product. Anthropic first because the built-in agent's
/// prompts and tool loop were developed against the Messages API.
pub fn presets() -> Vec<Preset> {
    vec![
        base_preset("anthropic", "anthropic", "Anthropic (Claude)", "https://api.anthropic.com", Key { env: Some("ANTHROPIC_API_KEY"), required: true, url: Some("https://platform.claude.com/settings/keys") }, Docs { page: "https://platform.claude.com/docs/en/api/overview", models_from: "GET /v1/models with the key; the documentation table needs no key", notes: "Messages API with tool use and prompt caching. Requests send x-api-key and anthropic-version: 2023-06-01. This is the wire format the built-in agent is tuned for." })
            .order(1).discovery("anthropic").tools("documented").suggested("claude-sonnet-4-5"),
        base_preset("openai", "openai", "OpenAI", "https://api.openai.com/v1", Key { env: Some("OPENAI_API_KEY"), required: true, url: Some("https://platform.openai.com/api-keys") }, Docs { page: "https://platform.openai.com/docs/api-reference/chat", models_from: "GET /v1/models with the key", notes: "Chat Completions with function tools. Reasoning models are billed and limited differently; devd sends max_completion_tokens for this provider." })
            .order(2).tools("documented"),
        base_preset("deepseek", "openai-compatible", "DeepSeek", "https://api.deepseek.com/v1", Key { env: Some("DEEPSEEK_API_KEY"), required: true, url: Some("https://platform.deepseek.com/api_keys") }, Docs { page: "https://api-docs.deepseek.com/", models_from: "GET /models with the key", notes: "OpenAI-compatible chat with function calling on the chat models. The reasoning model streams its thinking and may reject tools; choose a chat model for agent work." })
            .order(3).tools("documented").suggested("deepseek-chat"),
        base_preset("moonshot", "openai-compatible", "Moonshot AI (Kimi)", "https://api.moonshot.ai/v1", Key { env: Some("MOONSHOT_API_KEY"), required: true, url: Some("https://platform.moonshot.ai/console/api-keys") }, Docs { page: "https://platform.moonshot.ai/docs/api/chat", models_from: "GET /models with the key", notes: "Kimi and Moonshot models over an OpenAI-compatible API with tool calling. The mainland endpoint is api.moonshot.cn." })
            .order(4).tools("documented").suggested("moonshot-v1-8k"),
        base_preset("zhipu", "openai-compatible", "Zhipu AI (GLM)", "https://open.bigmodel.cn/api/paas/v4", Key { env: Some("ZHIPU_API_KEY"), required: true, url: Some("https://open.bigmodel.cn/usercenter/apikeys") }, Docs { page: "https://open.bigmodel.cn/dev/api", models_from: "maintained list: the chat API has no public model listing endpoint", notes: "GLM models over an OpenAI-compatible chat endpoint with tool calling. Model ids are checked against the maintained list, and a model you type yourself is accepted." })
            .order(5).tools("documented").discovery("catalog")
            .catalog(vec![
                model("glm-4.6", "GLM-4.6", Some(200_000), Some(128_000), "Flagship reasoning and coding"),
                model("glm-4.5", "GLM-4.5", Some(128_000), Some(96_000), "Previous flagship"),
                model("glm-4.5-air", "GLM-4.5 Air", Some(128_000), Some(96_000), "Smaller, faster"),
                model("glm-4-plus", "GLM-4 Plus", Some(128_000), Some(4_096), "General chat"),
                model("glm-4-flash", "GLM-4 Flash", Some(128_000), Some(4_096), "Cheap, fast"),
            ])
            .suggested("glm-4.6"),
        base_preset("gemini", "gemini", "Google Gemini", "https://generativelanguage.googleapis.com/v1beta", Key { env: Some("GEMINI_API_KEY"), required: true, url: Some("https://aistudio.google.com/apikey") }, Docs { page: "https://ai.google.dev/gemini-api/docs/openai", models_from: "GET /models with the key", notes: "The native Gemini API (generateContent): real token limits from the model list, thought signatures kept with function calls, and function schemas reduced to what Gemini accepts. An older provider saved against the OpenAI-compatibility URL keeps working unchanged." })
            .order(6).tools("documented").suggested("gemini-2.5-flash"),
        base_preset("xai", "openai-compatible", "xAI (Grok)", "https://api.x.ai/v1", Key { env: Some("XAI_API_KEY"), required: true, url: Some("https://console.x.ai/team/default/api-keys") }, Docs { page: "https://docs.x.ai/docs/api-reference", models_from: "GET /models with the key", notes: "Grok models over an OpenAI-compatible API with function calling." })
            .order(7).tools("documented"),
        base_preset("mistral", "openai-compatible", "Mistral AI", "https://api.mistral.ai/v1", Key { env: Some("MISTRAL_API_KEY"), required: true, url: Some("https://console.mistral.ai/api-keys") }, Docs { page: "https://docs.mistral.ai/api/", models_from: "GET /models with the key", notes: "Mistral models over an OpenAI-compatible API with function calling." })
            .order(8).tools("documented"),
        base_preset("groq", "openai-compatible", "Groq", "https://api.groq.com/openai/v1", Key { env: Some("GROQ_API_KEY"), required: true, url: Some("https://console.groq.com/keys") }, Docs { page: "https://console.groq.com/docs/openai", models_from: "GET /models with the key", notes: "Fast inference of open models over an OpenAI-compatible API." })
            .order(9).tools("model"),
        base_preset("together", "openai-compatible", "Together AI", "https://api.together.xyz/v1", Key { env: Some("TOGETHER_API_KEY"), required: true, url: Some("https://api.together.ai/settings/api-keys") }, Docs { page: "https://docs.together.ai/docs/openai-api-compatibility", models_from: "GET /models (public list)", notes: "Many open models behind one OpenAI-compatible API; tool support depends on the model." })
            .order(10).tools("model"),
        base_preset("fireworks", "openai-compatible", "Fireworks AI", "https://api.fireworks.ai/inference/v1", Key { env: Some("FIREWORKS_API_KEY"), required: true, url: Some("https://fireworks.ai/account/api-keys") }, Docs { page: "https://docs.fireworks.ai/api-reference/introduction", models_from: "GET /models with the key", notes: "Open models with an OpenAI-compatible API; tool support depends on the model." })
            .order(11).tools("model"),
        base_preset("openrouter", "openai-compatible", "OpenRouter", "https://openrouter.ai/api/v1", Key { env: Some("OPENROUTER_API_KEY"), required: true, url: Some("https://openrouter.ai/keys") }, Docs { page: "https://openrouter.ai/docs/api-reference/overview", models_from: "GET /models (public list with prices and supported parameters)", notes: "One key for many vendors. Model ids look like `vendor/model`, and each model advertises whether it supports tools in the public list." })
            .order(20).category("aggregator").tools("model"),
        base_preset("ollama", "openai-compatible", "Ollama (local)", "http://localhost:11434/v1", Key { env: None, required: false, url: None }, Docs { page: "https://github.com/ollama/ollama/blob/main/docs/openai.md", models_from: "GET /v1/models, falling back to GET /api/tags", notes: "Runs on this machine; nothing leaves it. devd finds a running instance automatically. Tool calling depends on the model you pulled." })
            .order(30).category("local").tools("model").discovery("ollama"),
        base_preset("lm-studio", "openai-compatible", "LM Studio (local)", "http://localhost:1234/v1", Key { env: None, required: false, url: None }, Docs { page: "https://lmstudio.ai/docs/app/api/endpoints/openai", models_from: "GET /v1/models (local, no key)", notes: "Start the local server in LM Studio first. Tool calling depends on the model and the server's prompt template." })
            .order(31).category("local").tools("model"),
        base_preset("custom", "openai-compatible", "Custom OpenAI-compatible endpoint", "https://", Key { env: None, required: false, url: None }, Docs { page: "https://platform.openai.com/docs/api-reference/chat", models_from: "GET /models", notes: "Any server implementing /models and /chat/completions. Plain http is accepted only for localhost. Capabilities are unknown until you run Verify." })
            .order(40).category("local").tools("model"),
    ]
}

/// Look up a preset by id.
pub fn preset(id: &str) -> Option<Preset> {
    presets().into_iter().find(|p| p.id == id)
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
    build_agent_with(url, follow, 15, if follow { 300 } else { 30 })
}

/// A client for probing a local server. It must answer quickly or not at all:
/// the discovery step runs while the user is waiting.
fn quick_agent(url: &str) -> ureq::Agent {
    build_agent_with(url, true, 2, 2)
}

fn build_agent_with(url: &str, follow: bool, connect_s: u64, read_s: u64) -> ureq::Agent {
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
    b.timeout_connect(Duration::from_secs(connect_s))
        .timeout_read(Duration::from_secs(read_s))
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

/// A response body that may arrive a piece at a time.
enum Answer {
    /// An ordinary JSON body, read in full before it is parsed.
    Whole(String),
    /// A server-sent event stream, read as the provider writes it.
    Events(BufReader<Box<dyn Read + Send + Sync>>),
}

struct LiveReply {
    status: u16,
    retry_after: Option<u64>,
    answer: Answer,
}

/// The streaming form of [`send`]: a successful event stream is handed over
/// still open, so each token can reach the window as it arrives. Anything else
/// (an error, or a provider that ignored `stream`) is a whole body.
fn send_live(url: &str, headers: &[(String, String)], body: &str) -> Result<LiveReply> {
    let a = agent_for(url);
    let mut req = a.request("POST", url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let res = req
        .set("content-type", "application/json")
        .send_string(body);
    match res {
        Ok(r) => {
            let status = r.status();
            let retry_after = r.header("retry-after").and_then(|v| v.parse::<u64>().ok());
            let events = r
                .header("content-type")
                .map(|c| c.to_ascii_lowercase().contains("text/event-stream"))
                .unwrap_or(false);
            if events {
                Ok(LiveReply {
                    status,
                    retry_after,
                    answer: Answer::Events(BufReader::new(r.into_reader())),
                })
            } else {
                let text = r.into_string().map_err(|e| {
                    DevdError::internal(format!(
                        "could not read the response from {}: {e}",
                        host_of(url)
                    ))
                })?;
                Ok(LiveReply {
                    status,
                    retry_after,
                    answer: Answer::Whole(text),
                })
            }
        }
        Err(ureq::Error::Status(code, r)) => {
            let retry_after = r.header("retry-after").and_then(|v| v.parse::<u64>().ok());
            Ok(LiveReply {
                status: code,
                retry_after,
                answer: Answer::Whole(r.into_string().unwrap_or_default()),
            })
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

fn key_hint(p: &Provider) -> String {
    match p.key_env.as_deref() {
        Some(env) => format!(" Paste a new key in Models, or set {env}."),
        None => " Paste a new key in Models.".to_string(),
    }
}

fn is_context_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    ["context length", "context_length", "maximum context", "too many tokens", "token limit", "input is too long", "prompt is too long"]
        .iter()
        .any(|needle| m.contains(needle))
}

fn status_error(p: &Provider, r: &Reply) -> DevdError {
    let msg = error_message(&r.body);
    match r.status {
        401 => DevdError::denied(format!("{} rejected the API key (HTTP 401): {msg}.{}", p.label, key_hint(p))),
        402 => DevdError::denied(format!(
            "{} says this account is out of credit or needs billing set up (HTTP 402): {msg}. Top up the account, or choose another provider.",
            p.label
        )),
        403 => DevdError::denied(format!(
            "{} refused this key or region (HTTP 403): {msg}. Check the key's permissions, or whether the vendor serves your country.",
            p.label
        )),
        404 => DevdError::not_found(format!(
            "{} returned 404: {msg}. The model id or base URL may be wrong, or the model is not enabled for this key.",
            p.label
        )),
        413 => DevdError::invalid(format!(
            "{} says the request is too large (HTTP 413): {msg}. Start a new conversation or shorten the input.",
            p.label
        )),
        429 => {
            let wait = r.retry_after.map(|s| format!(" Retry after {s}s.")).unwrap_or_default();
            let quota = msg.to_lowercase().contains("quota") || msg.to_lowercase().contains("credit");
            if quota {
                DevdError::conflict(format!("{} says this key has hit its quota (HTTP 429): {msg}.{}", p.label, wait))
            } else {
                DevdError::conflict(format!("{} is rate limiting this key (HTTP 429): {msg}.{wait}", p.label))
            }
        }
        400 | 422 if is_context_error(&msg) => DevdError::invalid(format!(
            "{} says the conversation no longer fits the model's context (HTTP {}): {msg}. Start a new conversation, choose a larger-context model, or shorten the input.",
            p.label, r.status
        )),
        400 | 422 => DevdError::invalid(format!("{} refused the request (HTTP {}): {msg}", p.label, r.status)),
        500 | 502 | 503 | 504 | 529 => DevdError::Timeout(format!(
            "{} is unavailable or overloaded (HTTP {}): {msg}. Try again shortly.",
            p.label, r.status
        )),
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
        "gemini" => {
            if let Some(k) = key {
                h.push(("x-goog-api-key".to_string(), k.to_string()));
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
    } else if p.kind == "gemini" {
        format!("{}/models?pageSize=1000", p.base_url.trim_end_matches('/'))
    } else {
        format!("{}/models", p.base_url.trim_end_matches('/'))
    }
}

/// The scheme and authority of a URL, for endpoints that live beside the API
/// rather than under its prefix (Ollama's `/api/tags`).
fn origin_of(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    match trimmed.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{authority}")
        }
        None => trimmed.to_string(),
    }
}

fn discovery_of(p: &Provider) -> String {
    match preset(&p.id) {
        Some(pre) => pre.discovery,
        None if p.kind == "anthropic" => "anthropic".into(),
        None => "openai".into(),
    }
}

/// Whether an id is plausibly a chat model. Every vendor's `/models` endpoint
/// also lists embeddings, speech, image and moderation models; offering those
/// in a chat picker is a trap.
fn is_chat_model(id: &str) -> bool {
    let id = id.to_lowercase();
    const NOT_CHAT: [&str; 18] = [
        "embed", "whisper", "tts", "dall-e", "dall·e", "moderation", "rerank", "transcribe",
        "realtime", "audio", "image", "sora", "clip", "guard", "ocr", "speech", "voice", "veo",
    ];
    !NOT_CHAT.iter().any(|needle| id.contains(needle))
}

/// The maintained list a preset carries, in catalogue form.
fn catalogue_from_preset(p: &Provider, pre: &Preset, now: &str) -> Vec<CatalogueModel> {
    pre.catalog
        .iter()
        .map(|m| CatalogueModel {
            source: format!("provider:{}", p.id),
            model_id: m.id.clone(),
            display: m.display.clone(),
            description: Some(format!("devd's maintained list: {}", m.note)),
            context_window: m.context_window,
            max_output: m.max_output,
            input_price: None,
            output_price: None,
            retirement: None,
            url: Some(pre.docs_url.clone()),
            fetched_at: now.to_string(),
        })
        .collect()
}

fn price_of(m: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| {
        let v = m.pointer(&format!("/pricing/{k}"))?;
        let per_token = v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64())?;
        Some(per_token * 1_000_000.0)
    })
}

fn normalise(p: &Provider, v: &Value, now: &str) -> Vec<CatalogueModel> {
    let items = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.get("models").and_then(|d| d.as_array()))
        .or_else(|| v.get("results").and_then(|d| d.as_array()))
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<CatalogueModel> = Vec::new();
    for m in &items {
        let Some(raw) = m.get("id").and_then(|x| x.as_str()).or_else(|| m.get("model").and_then(|x| x.as_str())) else {
            continue;
        };
        // Gemini's compatibility layer sometimes returns `models/gemini-…`.
        let id = raw.strip_prefix("models/").unwrap_or(raw).to_string();
        if id.is_empty() || out.iter().any(|x| x.model_id == id) {
            continue;
        }
        if p.kind != "anthropic" && !is_chat_model(&id) {
            continue;
        }
        let display = m
            .get("display_name")
            .or_else(|| m.get("name"))
            .and_then(|x| x.as_str())
            .unwrap_or(&id)
            .to_string();
        let context = ["max_input_tokens", "context_length", "context_window", "input_token_limit", "max_context_length"]
            .iter()
            .find_map(|k| m.get(*k).and_then(|x| x.as_i64()));
        let max_out = ["max_output_tokens", "max_tokens", "output_token_limit"]
            .iter()
            .find_map(|k| m.get(*k).and_then(|x| x.as_i64()));
        let mut description = m.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
        if let Some(params) = m.get("supported_parameters").and_then(|x| x.as_array()) {
            let tools = params.iter().any(|x| x.as_str() == Some("tools"));
            let note = if tools { "tools supported" } else { "tools not advertised" };
            description = Some(match description {
                Some(text) if !text.trim().is_empty() => format!("{text} · {note}"),
                _ => note.to_string(),
            });
        }
        out.push(CatalogueModel {
            source: format!("provider:{}", p.id),
            model_id: id,
            display,
            description,
            context_window: context,
            max_output: max_out,
            input_price: price_of(m, &["prompt", "input"]),
            output_price: price_of(m, &["completion", "output"]),
            retirement: m
                .get("deprecation_date")
                .or_else(|| m.get("retirement"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            url: Some(models_url(p)),
            fetched_at: now.to_string(),
        });
    }
    out.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    out
}

/// Ollama's native list. Used when the OpenAI-compatible layer is missing or
/// empty, which happens on older builds.
fn ollama_models(p: &Provider, now: &str) -> Result<Vec<CatalogueModel>> {
    let url = format!("{}/api/tags", origin_of(&p.base_url));
    let r = send("GET", &url, &[], None)?;
    if r.status != 200 {
        return Err(DevdError::internal(format!(
            "Ollama did not answer its native model list at {url} (HTTP {}). Is the server running?",
            r.status
        )));
    }
    let v: Value = serde_json::from_str(&r.body)
        .map_err(|e| DevdError::internal(format!("Ollama returned something that is not JSON: {e}")))?;
    Ok(parse_ollama(p, &v, &url, now))
}

fn parse_ollama(p: &Provider, v: &Value, url: &str, now: &str) -> Vec<CatalogueModel> {
    let mut out: Vec<CatalogueModel> = v
        .get("models")
        .and_then(|m| m.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|m| {
                    let id = m.get("model").or_else(|| m.get("name")).and_then(|x| x.as_str())?.to_string();
                    let size = m
                        .pointer("/details/parameter_size")
                        .and_then(|x| x.as_str())
                        .map(|s| format!(" · {s}"));
                    Some(CatalogueModel {
                        source: format!("provider:{}", p.id),
                        display: id.clone(),
                        description: Some(format!("local model{}", size.unwrap_or_default())),
                        model_id: id,
                        context_window: None,
                        max_output: None,
                        input_price: None,
                        output_price: None,
                        retirement: None,
                        url: Some(url.to_string()),
                        fetched_at: now.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    out
}

/// List the models an account can use, from the provider's own API.
///
/// Discovery is per provider, not one guess for all: Anthropic has its own
/// shape, Ollama has two endpoints and may not implement either, Zhipu has no
/// listing API at all, and everyone else speaks OpenAI's `/models`.
pub fn list_models(p: &Provider, key: Option<&str>) -> Result<Vec<CatalogueModel>> {
    let now = util::now();
    let pre = preset(&p.id);
    let discovery = discovery_of(p);
    if discovery == "catalog" {
        let models = pre
            .as_ref()
            .map(|pre| catalogue_from_preset(p, pre, &now))
            .unwrap_or_default();
        if models.is_empty() {
            return Err(DevdError::not_found(format!(
                "{} has no model listing endpoint and devd has no maintained list for it; type a model id yourself.",
                p.label
            )));
        }
        return Ok(models);
    }

    if discovery == "ollama" {
        let openai = list_models_openai(p, key, &now);
        if let Ok(models) = &openai {
            if !models.is_empty() {
                return openai;
            }
        }
        if let Ok(models) = ollama_models(p, &now) {
            if !models.is_empty() {
                return Ok(models);
            }
        }
        return match openai {
            Ok(_) => Err(DevdError::not_found(
                "Ollama is running but has no models. Pull one first, for example `ollama pull llama3.2`.",
            )),
            Err(e) => Err(e),
        };
    }

    match list_models_openai(p, key, &now) {
        Ok(models) => Ok(models),
        Err(e) => {
            // A provider with no listing endpoint can still be configured from
            // the maintained list, but an auth or rate-limit failure must not be
            // hidden behind it.
            let message = e.to_string().to_lowercase();
            let no_endpoint = message.contains("404") || message.contains("405") || message.contains("501");
            if no_endpoint {
                if let Some(pre) = pre.as_ref().filter(|pre| !pre.catalog.is_empty()) {
                    return Ok(catalogue_from_preset(p, pre, &now));
                }
            }
            Err(e)
        }
    }
}

fn list_models_openai(p: &Provider, key: Option<&str>, now: &str) -> Result<Vec<CatalogueModel>> {
    let r = send("GET", &models_url(p), &auth_headers(p, key), None)?;
    if r.status != 200 {
        return Err(status_error(p, &r));
    }
    let v: Value = serde_json::from_str(&r.body)
        .map_err(|e| DevdError::internal(format!("{} returned something that is not JSON: {e}", p.label)))?;
    if p.kind == "gemini" {
        return Ok(crate::gemini::models(&v, &format!("provider:{}", p.id), now));
    }
    Ok(normalise(p, &v, now))
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
    let page_link = Regex::new(r"\]\(([^)]*)\)").unwrap();
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
        for (col, head) in header.iter().enumerate().skip(1) {
            let Some(id_cell) = ids.get(col) else { continue };
            let id = plain(id_cell);
            if !id.starts_with("claude-") || out.iter().any(|m| m.model_id == id) {
                continue;
            }
            let cell = |label: &str| row(label).and_then(|r| r.get(col)).map(|c| plain(c)).filter(|c| !c.is_empty() && c != "-");
            let (inp, outp) = cell("Pricing").and_then(|p| price.captures(&p).map(|c| (c[1].parse().ok(), c[2].parse().ok()))).unwrap_or((None, None));
            let name = plain(head);
            let page = row("Model page").and_then(|r| r.get(col)).and_then(|c| page_link.captures(c).map(|m| m[1].to_string()));
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
    /// The endpoint answered and the configuration can be used.
    pub ok: bool,
    /// Whether the credentials were actually checked. A provider with a
    /// maintained model list and no listing endpoint cannot check them here.
    pub auth_checked: bool,
    /// How long it took.
    pub latency_ms: u128,
    /// How many models the account can see.
    pub models: usize,
    /// Which wire format devd speaks to it.
    pub wire: String,
    /// The URL that was contacted.
    pub endpoint: String,
    /// Where to create a key, when one is needed.
    pub hint: Option<String>,
    /// What happened, in a sentence.
    pub message: String,
}

fn wire_of(p: &Provider) -> &'static str {
    match p.kind.as_str() {
        "anthropic" => "anthropic-messages",
        "gemini" => "gemini-generate-content",
        _ => "openai-chat",
    }
}

fn key_required(p: &Provider) -> bool {
    match p.kind.as_str() {
        "anthropic" | "openai" | "gemini" => true,
        _ => preset(&p.id).map(|pre| pre.requires_key).unwrap_or(false),
    }
}

/// Check that the base URL is reachable and the key is accepted by asking for
/// the model list. A provider without a listing endpoint is reported as
/// configured but not authenticated, rather than as a success.
pub fn test(p: &Provider, key: Option<&str>) -> TestReport {
    let started = Instant::now();
    let endpoint = models_url(p);
    let wire = wire_of(p).to_string();
    let hint = preset(&p.id).and_then(|pre| pre.key_url);
    let discovery = discovery_of(p);
    if key.is_none() && key_required(p) {
        return TestReport {
            ok: false,
            auth_checked: false,
            latency_ms: 0,
            models: 0,
            wire,
            endpoint,
            hint,
            message: format!(
                "No API key is set for {}. Paste one in Models{}.",
                p.label,
                p.key_env.as_deref().map(|e| format!(" or set {e}")).unwrap_or_default()
            ),
        };
    }
    match list_models(p, key) {
        Ok(m) => {
            let auth_checked = discovery != "catalog";
            TestReport {
                ok: true,
                auth_checked,
                latency_ms: started.elapsed().as_millis(),
                models: m.len(),
                wire,
                endpoint: models_url(p),
                hint,
                message: if auth_checked {
                    format!(
                        "{} answered in {} ms; {} model(s) available to this key.",
                        p.label,
                        started.elapsed().as_millis(),
                        m.len()
                    )
                } else {
                    format!(
                        "{} has no model listing endpoint, so devd used its maintained list ({} models). Credentials are checked when you run Verify on a model.",
                        p.label,
                        m.len()
                    )
                },
            }
        }
        Err(e) => TestReport {
            ok: false,
            auth_checked: true,
            latency_ms: started.elapsed().as_millis(),
            models: 0,
            wire,
            endpoint,
            hint,
            message: e.to_string(),
        },
    }
}

/// The outcome of asking a model to answer and call a tool.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VerifyReport {
    /// The model answered at all.
    pub ok: bool,
    /// How long the round trip took.
    pub latency_ms: u128,
    /// The model id that was tested.
    pub model: String,
    /// The model produced text.
    pub chat: bool,
    /// The model called the probe tool. Only true when the provider actually
    /// returned a tool call, never because the vendor advertises the feature.
    pub tools: bool,
    /// The provider's stop reason.
    pub stop_reason: String,
    /// Input tokens billed for the probe.
    pub input_tokens: i64,
    /// Output tokens billed for the probe.
    pub output_tokens: i64,
    /// What happened, in a sentence.
    pub message: String,
}

/// Prove that one model works with this key: a short completion that must call
/// a named tool. This is a real request and costs a few tokens.
pub fn verify(p: &Provider, key: Option<&str>, model: &str) -> VerifyReport {
    let started = Instant::now();
    let model = model.trim().to_string();
    if model.is_empty() {
        return verify_failure(started, &model, "Choose a model first: verify runs one real request against it.".into());
    }
    let req = ChatRequest {
        model: model.clone(),
        system: "You are a connectivity probe. Follow the instruction exactly.".into(),
        messages: vec![json!({
            "role": "user",
            "content": [{ "type": "text", "text": "Call the echo tool with the word hello. Do not answer in prose." }]
        })],
        tools: vec![ToolSpec {
            name: "echo".into(),
            description: "Echo one word back to the caller.".into(),
            schema: json!({
                "type": "object",
                "properties": { "word": { "type": "string", "description": "The word to echo" } },
                "required": ["word"]
            }),
        }],
        tool_choice: Some("echo".into()),
        max_tokens: 64,
    };
    match chat(p, key, &req) {
        Ok(r) => {
            let chat = r.content.iter().any(|b| {
                b["type"] == "text" && b["text"].as_str().map(|t| !t.trim().is_empty()).unwrap_or(false)
            });
            let tools = r
                .content
                .iter()
                .any(|b| b["type"] == "tool_use" && b["name"].as_str() == Some("echo"));
            VerifyReport {
                ok: chat || tools,
                latency_ms: started.elapsed().as_millis(),
                model,
                chat,
                tools,
                stop_reason: r.stop_reason.clone(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                message: if tools {
                    format!(
                        "{} replied and called the echo tool in {} ms. Tool use is verified for this model.",
                        p.label,
                        started.elapsed().as_millis()
                    )
                } else if chat {
                    format!(
                        "{} replied in {} ms but did not call the tool. Text works; tool use is not verified for this model, so agent work may fail.",
                        p.label,
                        started.elapsed().as_millis()
                    )
                } else {
                    format!("{} answered with no usable content (stop reason `{}`).", p.label, r.stop_reason)
                },
            }
        }
        Err(e) => verify_failure(started, &model, e.to_string()),
    }
}

fn verify_failure(started: Instant, model: &str, message: String) -> VerifyReport {
    VerifyReport {
        ok: false,
        latency_ms: started.elapsed().as_millis(),
        model: model.to_string(),
        chat: false,
        tools: false,
        stop_reason: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        message,
    }
}

/// A local inference server found on this machine.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LocalServer {
    /// The preset that configures it.
    pub preset: String,
    /// Display name.
    pub label: String,
    /// The base URL to save.
    pub base_url: String,
    /// Whether it answered.
    pub running: bool,
    /// How many models it has, when it answered.
    pub models: usize,
    /// What happened.
    pub message: String,
}

/// Probe the well-known local inference servers. No network traffic leaves the
/// machine: the two URLs are loopback by construction.
pub fn discover_local() -> Vec<LocalServer> {
    let candidates = [
        ("ollama", "Ollama", "http://127.0.0.1:11434"),
        ("lm-studio", "LM Studio", "http://127.0.0.1:1234"),
    ];
    candidates
        .iter()
        .map(|(preset, label, origin)| {
            let url = format!("{origin}/v1/models");
            let count = quick_models(&url);
            LocalServer {
                preset: preset.to_string(),
                label: label.to_string(),
                base_url: format!("{origin}/v1"),
                running: count.is_some(),
                models: count.unwrap_or(0),
                message: match count {
                    Some(n) if n > 0 => format!("{label} is running with {n} model(s)."),
                    Some(_) => format!("{label} is running but has no models loaded."),
                    None => format!("{label} is not answering on {origin}."),
                },
            }
        })
        .collect()
}

fn quick_models(url: &str) -> Option<usize> {
    let agent = quick_agent(url);
    let r = agent.get(url).call().ok()?;
    let body = r.into_string().ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    Some(
        v.get("data")
            .and_then(|d| d.as_array())
            .or_else(|| v.get("models").and_then(|d| d.as_array()))
            .map(|a| a.len())
            .unwrap_or(0),
    )
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
    /// Force one tool to be called, by name. `None` leaves the choice to the
    /// model. The adapters turn this into each vendor's own shape.
    pub tool_choice: Option<String>,
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
        if let Some(name) = &r.tool_choice {
            body["tool_choice"] = json!({ "type": "tool", "name": name });
        }
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
        if let Some(name) = &r.tool_choice {
            body["tool_choice"] = json!({ "type": "function", "function": { "name": name } });
        }
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
    chat_with(p, key, req, &mut |_| {})
}

/// Send one request and report the answer as it arrives.
///
/// `on` receives each piece of visible text as the provider produces it, so a
/// surface can show the reply being written instead of a silence followed by a
/// whole paragraph. The full answer is what `chat` returns either way: the
/// callback is a view of the answer, never a substitute for it.
pub fn chat_stream(
    p: &Provider,
    key: Option<&str>,
    req: &ChatRequest,
    on: &mut dyn FnMut(&str),
) -> Result<ChatResponse> {
    chat_with(p, key, req, on)
}

fn chat_with(
    p: &Provider,
    key: Option<&str>,
    req: &ChatRequest,
    on: &mut dyn FnMut(&str),
) -> Result<ChatResponse> {
    let (url, body) = if p.kind == "gemini" {
        (
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse",
                p.base_url.trim_end_matches('/'),
                req.model
            ),
            crate::gemini::body(req),
        )
    } else if p.kind == "anthropic" {
        (
            format!("{}/v1/messages", p.base_url.trim_end_matches('/')),
            anthropic_body(req),
        )
    } else {
        (
            format!("{}/chat/completions", p.base_url.trim_end_matches('/')),
            openai_body(p, req),
        )
    };
    let headers = auth_headers(p, key);
    let mut text = body;
    // Ask for the streamed form. The adapters parse whichever form comes back,
    // so a provider that ignores `stream` still answers correctly. Gemini
    // streams by endpoint, not by a flag in the body.
    if p.kind != "gemini" {
        text["stream"] = json!(true);
    }
    let text = text.to_string();
    let mut last: Option<DevdError> = None;
    for attempt in 0..3u32 {
        let r = send_live(&url, &headers, &text)?;
        if r.status == 200 {
            let answer = match r.answer {
                Answer::Events(reader) => {
                    return if p.kind == "gemini" {
                        crate::gemini::parse_stream(reader, on)
                    } else if p.kind == "anthropic" {
                        parse_anthropic_stream(reader, on)
                    } else {
                        parse_openai_stream(reader, on)
                    };
                }
                Answer::Whole(body) => {
                    // A provider may ignore `stream` and send the ordinary JSON
                    // answer. Some, though, stream while calling it plain text.
                    if body
                        .lines()
                        .any(|line| line.trim_start().starts_with("data:"))
                    {
                        return if p.kind == "gemini" {
                            crate::gemini::parse_stream(std::io::Cursor::new(body), on)
                        } else if p.kind == "anthropic" {
                            parse_anthropic_stream(std::io::Cursor::new(body), on)
                        } else {
                            parse_openai_stream(std::io::Cursor::new(body), on)
                        };
                    }
                    body
                }
            };
            let v: Value = serde_json::from_str(&answer).map_err(|e| {
                DevdError::internal(format!(
                    "{} returned something that is not JSON: {e}",
                    p.label
                ))
            })?;
            let answer = if p.kind == "gemini" {
                crate::gemini::parse(&v)?
            } else if p.kind == "anthropic" {
                parse_anthropic(&v)?
            } else {
                parse_openai(&v)?
            };
            // A provider that ignored `stream` still gets the same visible
            // result: its text is reported once, in full.
            let full = text_of(&answer.content);
            if !full.is_empty() {
                on(&full);
            }
            return Ok(answer);
        }
        let Answer::Whole(body) = r.answer else {
            return Err(DevdError::internal(format!(
                "{} returned HTTP {} with an unreadable stream",
                p.label, r.status
            )));
        };
        let reply = Reply {
            status: r.status,
            body,
            retry_after: r.retry_after,
        };
        let transient = matches!(reply.status, 429 | 500 | 502 | 503 | 504 | 529);
        let err = status_error(p, &reply);
        if !transient || attempt == 2 {
            return Err(err);
        }
        let wait = reply.retry_after.unwrap_or(1u64 << attempt).min(20);
        std::thread::sleep(Duration::from_secs(wait));
        last = Some(err);
    }
    Err(last.unwrap_or_else(|| DevdError::internal("request failed")))
}

/// Read a server-sent event stream as it arrives.
///
/// Events are separated by a blank line and may spread their payload over
/// several `data:` lines. Reading line by line is what makes the answer appear
/// while the provider is still writing it; waiting for the whole body would
/// only replay it after the fact.
pub(crate) fn for_each_event(
    mut reader: impl BufRead,
    mut on_data: impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut data = String::new();
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| DevdError::internal(format!("the provider's stream stopped: {e}")))?;
        if read == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !data.is_empty() {
                on_data(&data)?;
                data.clear();
            }
            continue;
        }
        if let Some(payload) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(payload.strip_prefix(' ').unwrap_or(payload));
        }
        // `event:`, `id:`, `retry:` and comments describe the frame; the JSON
        // inside `data:` already carries its own type, so they need no parsing.
    }
    if !data.is_empty() {
        on_data(&data)?;
    }
    Ok(())
}

fn parse_anthropic_stream(reader: impl BufRead, on: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
    let mut blocks: Vec<Value> = Vec::new();
    let mut tool_json: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let mut stop_reason = String::from("end_turn");
    let (mut input_tokens, mut output_tokens) = (0i64, 0i64);
    let mut saw_message_stop = false;
    let mut feed = |payload: &str| -> Result<()> {
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return Ok(());
        };
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                input_tokens += v
                    .pointer("/message/usage/input_tokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0)
                    + v.pointer("/message/usage/cache_read_input_tokens")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0)
                    + v.pointer("/message/usage/cache_creation_input_tokens")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0);
            }
            "content_block_start" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                let block = v["content_block"].clone();
                // A streamed tool call arrives as an empty shell whose input is
                // filled by the deltas below. A text block may already carry
                // its first words.
                if block["type"] == "tool_use" {
                    while blocks.len() <= index {
                        blocks.push(Value::Null);
                    }
                    let mut shell = block;
                    shell["input"] = json!({});
                    blocks[index] = shell;
                } else {
                    while blocks.len() <= index {
                        blocks.push(Value::Null);
                    }
                    let first = block["text"].as_str().unwrap_or("");
                    if !first.is_empty() {
                        on(first);
                    }
                    blocks[index] = block;
                }
            }
            "content_block_delta" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                let Some(block) = blocks.get_mut(index) else {
                    return Ok(());
                };
                match v
                    .pointer("/delta/type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                {
                    "text_delta" => {
                        let piece = v
                            .pointer("/delta/text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if !piece.is_empty() {
                            let so_far = block["text"].as_str().unwrap_or("").to_string();
                            block["text"] = json!(format!("{so_far}{piece}"));
                            on(piece);
                        }
                    }
                    "input_json_delta" => {
                        let piece = v
                            .pointer("/delta/partial_json")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        tool_json.entry(index).or_default().push_str(piece);
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = v["index"].as_u64().unwrap_or(0) as usize;
                if let Some(raw) = tool_json.remove(&index) {
                    if let Some(block) = blocks.get_mut(index) {
                        if block["type"] == "tool_use" {
                            block["input"] =
                                serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}));
                        }
                    }
                }
            }
            "message_delta" => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(|t| t.as_str()) {
                    stop_reason = r.to_string();
                }
                output_tokens = v
                    .pointer("/usage/output_tokens")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(output_tokens);
            }
            "message_stop" => saw_message_stop = true,
            "error" => {
                let msg = v
                    .pointer("/error/message")
                    .and_then(|t| t.as_str())
                    .unwrap_or("the stream failed");
                return Err(DevdError::internal(format!(
                    "the provider's stream failed: {msg}"
                )));
            }
            _ => {}
        }
        Ok(())
    };
    for_each_event(reader, &mut feed)?;
    let content: Vec<Value> = blocks.into_iter().filter(|b| !b.is_null()).collect();
    if content.is_empty() && !saw_message_stop {
        return Err(DevdError::internal("the streamed answer had no content"));
    }
    Ok(ChatResponse {
        content,
        stop_reason,
        input_tokens,
        output_tokens,
    })
}

fn parse_openai_stream(reader: impl BufRead, on: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
    let mut text = String::new();
    let mut calls: Vec<(String, String, String)> = Vec::new(); // id, name, arguments
    let mut finish = String::new();
    let (mut input_tokens, mut output_tokens) = (0i64, 0i64);
    let mut feed = |payload: &str| -> Result<()> {
        if payload.trim() == "[DONE]" {
            return Ok(());
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return Ok(());
        };
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            input_tokens = u["prompt_tokens"].as_i64().unwrap_or(input_tokens);
            output_tokens = u["completion_tokens"].as_i64().unwrap_or(output_tokens);
        }
        let Some(choice) = v.pointer("/choices/0") else {
            return Ok(());
        };
        if let Some(piece) = choice.pointer("/delta/content").and_then(|c| c.as_str()) {
            if !piece.is_empty() {
                text.push_str(piece);
                on(piece);
            }
        }
        for c in choice
            .pointer("/delta/tool_calls")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let index = c["index"].as_u64().unwrap_or(0) as usize;
            while calls.len() <= index {
                calls.push((String::new(), String::new(), String::new()));
            }
            let slot = &mut calls[index];
            if let Some(id) = c["id"].as_str().filter(|s| !s.is_empty()) {
                slot.0 = id.to_string();
            }
            if let Some(name) = c
                .pointer("/function/name")
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
            {
                slot.1 = name.to_string();
            }
            if let Some(args) = c.pointer("/function/arguments").and_then(|a| a.as_str()) {
                slot.2.push_str(args);
            }
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            finish = f.to_string();
        }
        Ok(())
    };
    for_each_event(reader, &mut feed)?;
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    for (i, (id, name, args)) in calls.into_iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        content.push(json!({
            "type": "tool_use",
            "id": if id.is_empty() { format!("call_{i}") } else { id },
            "name": name,
            "input": serde_json::from_str::<Value>(&args).unwrap_or_else(|_| json!({})),
        }));
    }
    if content.is_empty() && finish.is_empty() {
        return Err(DevdError::internal("the streamed answer had no content"));
    }
    let has_calls = content.iter().any(|b| b["type"] == "tool_use");
    Ok(ChatResponse {
        content,
        stop_reason: if has_calls {
            "tool_use".into()
        } else if finish == "length" {
            "max_tokens".into()
        } else {
            "end_turn".into()
        },
        input_tokens,
        output_tokens,
    })
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
    fn a_streamed_answer_is_read_over_the_wire() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            // Consume the whole request first. Closing a socket with unread
            // request bytes turns the answer into a reset on Windows.
            let mut request = Vec::new();
            let mut tmp = [0u8; 4096];
            let mut end = 0usize;
            let mut want = 0usize;
            loop {
                let n = socket.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&tmp[..n]);
                if end == 0 {
                    if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        end = i + 4;
                        let head = String::from_utf8_lossy(&request[..i]);
                        want = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                    }
                }
                if end > 0 && request.len() >= end + want {
                    break;
                }
            }
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Luna is \"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"a music app.\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4}}\n\n\
data: [DONE]\n\n";
            let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
            let _ = socket.write_all(head.as_bytes());
            let _ = socket.write_all(body.as_bytes());
        });

        let p = Provider {
            id: "test".into(),
            kind: "openai-compatible".into(),
            label: "Test".into(),
            base_url: base,
            key_env: None,
            default_model: Some("model-1".into()),
            is_default: false,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let req = ChatRequest {
            model: "model-1".into(),
            system: "s".into(),
            messages: vec![
                json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }),
            ],
            tools: vec![],
            tool_choice: None,
            max_tokens: 32,
        };
        let mut seen = String::new();
        let answer = chat_stream(&p, None, &req, &mut |piece| seen.push_str(piece)).unwrap();
        assert_eq!(
            seen, "Luna is a music app.",
            "each delta reaches the caller as it is read"
        );
        assert_eq!(text_of(&answer.content), "Luna is a music app.");
        assert_eq!((answer.input_tokens, answer.output_tokens), (3, 4));
    }

    #[test]
    fn an_openai_stream_is_rebuilt_from_its_deltas() {
        let body = "\
data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Luna is a \"}}]}

data: {\"choices\":[{\"delta\":{\"content\":\"React Native app.\"}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}

data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"README.md\\\"}\"}}]}}]}

data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7}}

data: [DONE]
";
        let mut seen = String::new();
        let answer =
            parse_openai_stream(std::io::Cursor::new(body), &mut |p| seen.push_str(p)).unwrap();
        assert_eq!(
            seen, "Luna is a React Native app.",
            "text arrives as it was written"
        );
        assert_eq!(text_of(&answer.content), "Luna is a React Native app.");
        assert_eq!(answer.stop_reason, "tool_use");
        let call = answer
            .content
            .iter()
            .find(|b| b["type"] == "tool_use")
            .expect("the tool call is rebuilt");
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["name"], "read_file");
        assert_eq!(
            call["input"]["path"], "README.md",
            "arguments are joined from their pieces"
        );
        assert_eq!((answer.input_tokens, answer.output_tokens), (11, 7));
    }

    #[test]
    fn an_anthropic_stream_is_rebuilt_from_its_events() {
        let body = "\
event: message_start
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":9}}}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Yes, I \"}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"read it.\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":0}

event: content_block_start
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"read_file\",\"input\":{}}}

event: content_block_delta
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\": \\\"README.md\\\"}\"}}

event: content_block_stop
data: {\"type\":\"content_block_stop\",\"index\":1}

event: message_delta
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}

event: message_stop
data: {\"type\":\"message_stop\"}
";
        let mut pieces = Vec::new();
        let answer = parse_anthropic_stream(std::io::Cursor::new(body), &mut |p| {
            pieces.push(p.to_string())
        })
        .unwrap();
        assert_eq!(pieces.join(""), "Yes, I read it.");
        assert_eq!(answer.content[0]["text"], "Yes, I read it.");
        assert_eq!(answer.content[1]["type"], "tool_use");
        assert_eq!(answer.content[1]["input"]["path"], "README.md");
        assert_eq!(answer.stop_reason, "tool_use");
        assert_eq!((answer.input_tokens, answer.output_tokens), (9, 5));
    }

    #[test]
    fn a_stream_that_fails_says_so_instead_of_answering_nothing() {
        let body = "data: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}\n";
        let err = parse_anthropic_stream(std::io::Cursor::new(body), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("overloaded"), "{err}");
        let err = parse_anthropic_stream(std::io::Cursor::new(""), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("no content"), "{err}");
    }

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
            tool_choice: Some("devd_groups".into()),
            max_tokens: 100,
        };
        let b = openai_body(&p, &req);
        assert_eq!(b["tool_choice"]["function"]["name"], "devd_groups");
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

    fn provider(id: &str) -> Provider {
        let pre = preset(id);
        Provider {
            id: id.into(),
            kind: pre.as_ref().map(|p| p.kind.clone()).unwrap_or_else(|| "openai-compatible".into()),
            label: pre.as_ref().map(|p| p.label.clone()).unwrap_or_else(|| id.into()),
            base_url: pre.as_ref().map(|p| p.base_url.clone()).unwrap_or_else(|| "https://example.test/v1".into()),
            key_env: pre.as_ref().and_then(|p| p.key_env.clone()),
            default_model: None,
            is_default: false,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn the_presets_cover_the_prioritised_providers_in_order() {
        let all = presets();
        assert!(all.len() >= 14, "{}", all.len());
        let ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(&ids[..6], &["anthropic", "openai", "deepseek", "moonshot", "zhipu", "gemini"]);
        assert!(ids.contains(&"openrouter"));
        assert!(ids.contains(&"ollama"));
        assert!(ids.contains(&"custom"));
        // Order is the display order, and ids are unique.
        let mut order: Vec<u32> = all.iter().map(|p| p.order).collect();
        let sorted = order.clone();
        order.sort_unstable();
        assert_eq!(order, sorted);
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
        for p in &all {
            if p.id == "custom" {
                continue;
            }
            assert!(
                validate_base_url(&p.base_url).is_ok(),
                "{} has an unusable base URL: {}",
                p.id,
                p.base_url
            );
            assert!(!p.notes.is_empty() && !p.models_from.is_empty(), "{}", p.id);
            assert!(["documented", "model", "none"].contains(&p.tools.as_str()), "{}", p.id);
        }
    }

    #[test]
    fn providers_without_a_listing_endpoint_have_a_maintained_catalogue() {
        let zhipu = preset("zhipu").unwrap();
        assert_eq!(zhipu.discovery, "catalog");
        assert!(!zhipu.catalog.is_empty());
        // Reading a catalogue provider does not touch the network.
        let models = list_models(&provider("zhipu"), None).unwrap();
        assert!(models.iter().any(|m| m.model_id == "glm-4.6"));
        assert!(models.iter().all(|m| m.source == "provider:zhipu" && m.context_window.is_some()));
    }

    #[test]
    fn the_model_list_hides_models_that_cannot_chat() {
        let p = provider("openai");
        let v = json!({"data": [
            {"id": "gpt-4o"},
            {"id": "o3"},
            {"id": "text-embedding-3-large"},
            {"id": "whisper-1"},
            {"id": "gpt-4o-audio-preview"},
            {"id": "dall-e-3"},
            {"id": "gpt-image-1"}
        ]});
        let ids: Vec<String> = normalise(&p, &v, "now").into_iter().map(|m| m.model_id).collect();
        assert_eq!(ids, vec!["gpt-4o", "o3"]);
    }

    #[test]
    fn openrouter_prices_and_tool_support_are_read_from_the_public_list() {
        let p = provider("openrouter");
        let v = json!({"data": [{
            "id": "anthropic/claude-sonnet-4-5",
            "name": "Claude Sonnet 4.5",
            "context_length": 200000,
            "pricing": {"prompt": "0.000003", "completion": "0.000015"},
            "supported_parameters": ["tools", "temperature"]
        }]});
        let m = normalise(&p, &v, "now");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].context_window, Some(200_000));
        assert!((m[0].input_price.unwrap() - 3.0).abs() < 0.001);
        assert!((m[0].output_price.unwrap() - 15.0).abs() < 0.001);
        assert!(m[0].description.as_deref().unwrap().contains("tools supported"));
    }

    #[test]
    fn gemini_ids_lose_the_models_prefix() {
        let p = provider("gemini");
        let v = json!({"data": [{"id": "models/gemini-2.5-flash"}]});
        let m = normalise(&p, &v, "now");
        assert_eq!(m[0].model_id, "gemini-2.5-flash");
    }

    #[test]
    fn ollama_native_tags_are_understood() {
        let p = provider("ollama");
        let v = json!({"models": [
            {"name": "llama3.2:latest", "model": "llama3.2:latest", "details": {"parameter_size": "3.2B"}},
            {"name": "qwen2.5-coder:7b", "model": "qwen2.5-coder:7b", "details": {"parameter_size": "7.6B"}}
        ]});
        let m = parse_ollama(&p, &v, "http://127.0.0.1:11434/api/tags", "now");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].model_id, "llama3.2:latest");
        assert!(m[0].description.as_deref().unwrap().contains("3.2B"));
        assert!(m[0].source == "provider:ollama");
    }

    #[test]
    fn a_quota_or_payment_failure_says_which_it_is() {
        let p = provider("deepseek");
        let payment = status_error(
            &p,
            &Reply { status: 402, body: r#"{"error":{"message":"Insufficient Balance"}}"#.into(), retry_after: None },
        );
        assert!(payment.to_string().contains("credit"), "{payment}");
        let limit = status_error(
            &p,
            &Reply { status: 429, body: r#"{"error":{"message":"quota exceeded"}}"#.into(), retry_after: Some(30) },
        );
        assert!(limit.to_string().contains("quota"), "{limit}");
        assert!(limit.to_string().contains("30"), "{limit}");
    }

    #[test]
    fn an_overflowing_context_is_explained_rather_than_called_invalid_input() {
        let p = provider("openai");
        let err = status_error(
            &p,
            &Reply {
                status: 400,
                body: r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#.into(),
                retry_after: None,
            },
        );
        let text = err.to_string();
        assert!(text.contains("context"), "{text}");
        assert!(text.contains("Start a new conversation"), "{text}");
    }

    #[test]
    fn forcing_a_tool_uses_each_wire_format() {
        let req = ChatRequest {
            model: "m".into(),
            system: "s".into(),
            messages: vec![json!({"role":"user","content":[{"type":"text","text":"hi"}]})],
            tools: vec![ToolSpec { name: "echo".into(), description: "e".into(), schema: json!({"type":"object"}) }],
            tool_choice: Some("echo".into()),
            max_tokens: 16,
        };
        let a = anthropic_body(&req);
        assert_eq!(a["tool_choice"]["type"], "tool");
        assert_eq!(a["tool_choice"]["name"], "echo");
        let o = openai_body(&provider("openai"), &req);
        assert_eq!(o["tool_choice"]["type"], "function");
        assert_eq!(o["tool_choice"]["function"]["name"], "echo");
    }

    #[test]
    fn a_missing_key_is_reported_before_any_request_is_made() {
        let report = test(&provider("anthropic"), None);
        assert!(!report.ok);
        assert_eq!(report.wire, "anthropic-messages");
        assert!(report.message.contains("No API key"), "{}", report.message);
        assert!(report.hint.is_some());
    }

    /// Live checks. They only run when the vendor's key is in the environment,
    /// so a test run never spends money by accident:
    ///
    ///   $env:ANTHROPIC_API_KEY=...; cargo test -p devd-agent -- --ignored live_
    ///
    /// Verification is per provider through `provider.verify`; this covers the
    /// listing endpoint and, when the key works, one model chosen by the vendor
    /// list.
    #[test]
    #[ignore]
    fn live_provider_round_trips() {
        for id in ["anthropic", "openai", "deepseek", "moonshot", "zhipu", "gemini", "xai", "mistral", "groq", "openrouter"] {
            let p = provider(id);
            let key = p.key_env.as_deref().and_then(|env| std::env::var(env).ok());
            if key.is_none() {
                continue;
            }
            let report = test(&p, key.as_deref());
            assert!(report.ok, "{}: {}", p.label, report.message);
            let models = list_models(&p, key.as_deref()).expect("listing");
            assert!(!models.is_empty(), "{} listed no models", p.label);
            let model = preset(id)
                .and_then(|pre| pre.suggested_model)
                .filter(|m| models.iter().any(|x| &x.model_id == m))
                .unwrap_or_else(|| models[0].model_id.clone());
            let verified = verify(&p, key.as_deref(), &model);
            assert!(verified.ok, "{} {}: {}", p.label, model, verified.message);
            eprintln!("{} / {}: tools={} — {}", p.label, model, verified.tools, verified.message);
        }
    }

    /// The local probe is loopback only and does not need a key. It passes
    /// whether or not a server is installed: the report must be honest either
    /// way.
    #[test]
    fn local_discovery_never_claims_a_server_that_is_not_there() {
        for found in discover_local() {
            assert!(found.base_url.starts_with("http://127.0.0.1:"));
            if !found.running {
                assert_eq!(found.models, 0);
                assert!(found.message.contains("not answering"));
            }
        }
    }
}
