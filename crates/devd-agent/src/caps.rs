//! Capabilities of the built-in agent: providers and models, conversations,
//! approvals, the inbox, file tools and the health summary.
//!
//! Who may do what:
//!
//! * Providers hold credentials, so everything that writes them is human-only.
//!   Agents can read the list (keys are never part of it) and the model
//!   catalogue.
//! * A person decides agents' requests; an agent can only queue them and poll.
//! * File tools are agent-safe inside authorised directories; deleting needs
//!   approval.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Capability, Ctx, Effect, Outcome, Plan, Tier};
use devd_core::doctor::{self, Level};
use devd_core::{util, DevdError, Provider, Registry, Result};

use crate::chat::{Hub, SendRequest};
use crate::providers;
use crate::{fs, secrets};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct NoArgs {}

#[allow(clippy::too_many_arguments)]
fn add<A, P, Q>(reg: &mut Registry, id: &str, summary: &str, description: &str, tier: Tier, effect: Effect, agent_safe: bool, human_only: bool, plan: P, apply: Q) -> Result<()>
where
    A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
    P: Fn(&Ctx, &A) -> Result<Plan> + Send + Sync + 'static,
    Q: Fn(&Ctx, &A) -> Result<Outcome> + Send + Sync + 'static,
{
    let cap = Capability::builder(id, summary)
        .description(description)
        .tier(tier)
        .effect(effect)
        .agent_safe(agent_safe)
        .human_only(human_only)
        .dry_run(true)
        .idempotent(matches!(tier, Tier::Read))
        .args_schema(serde_json::to_value(schemars::schema_for!(A)).map_err(|e| DevdError::internal(format!("schema for {id}: {e}")))?)
        .handler(
            move |ctx: &Ctx, args: &Value| plan(ctx, &ctx.parse::<A>(args)?),
            move |ctx: &Ctx, args: &Value| apply(ctx, &ctx.parse::<A>(args)?),
        )
        .build()?;
    reg.register(cap)
}

fn simple(id: &str, text: impl Into<String>) -> Result<Plan> {
    Ok(Plan::new(id, text))
}

// ---------------------------------------------------------------- arguments

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SaveProvider {
    /// Provider id: lowercase letters, digits and hyphens.
    id: String,
    /// Start from a preset (see provider.presets).
    #[serde(default)]
    preset: Option<String>,
    /// `anthropic`, `openai` or `openai-compatible`.
    #[serde(default)]
    kind: Option<String>,
    /// Display name.
    #[serde(default)]
    label: Option<String>,
    /// API base URL (https, or http for localhost).
    #[serde(default)]
    base_url: Option<String>,
    /// Name of an environment variable that holds the key (nothing is stored).
    #[serde(default)]
    key_env: Option<String>,
    /// The API key. Stored privately, never returned or logged.
    #[serde(default)]
    api_key: Option<String>,
    /// Model to use when a conversation names none.
    #[serde(default)]
    default_model: Option<String>,
    /// Use this provider for new conversations.
    #[serde(default)]
    make_default: Option<bool>,
    /// Enabled (default true).
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct IdArgs {
    /// Provider id.
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CatalogueArgs {
    /// `anthropic-docs` or `provider:<id>`; omit for everything.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SendArgs {
    /// What to say.
    message: String,
    /// Continue this conversation.
    #[serde(default)]
    thread: Option<String>,
    /// Project directory the agent works in (new conversations).
    #[serde(default)]
    project: Option<String>,
    /// Provider id to use.
    #[serde(default)]
    provider: Option<String>,
    /// Model id to use.
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ThreadArgs {
    /// Conversation id.
    thread: String,
    /// Only events after this sequence number.
    #[serde(default)]
    after: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct UpdateThread {
    /// Conversation id.
    thread: String,
    /// New title.
    #[serde(default)]
    title: Option<String>,
    /// Provider id.
    #[serde(default)]
    provider: Option<String>,
    /// Model id.
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ListRequests {
    /// Only requests still waiting for a decision (default true).
    #[serde(default)]
    pending_only: Option<bool>,
    /// Only requests from this conversation.
    #[serde(default)]
    thread: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct DecideArgs {
    /// Request id.
    id: String,
    /// true to approve and run it, false to decline.
    approve: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PostArgs {
    /// Short subject.
    subject: String,
    /// What you want the person to know or decide.
    body: String,
    /// Project directory it concerns.
    #[serde(default)]
    project: Option<String>,
    /// You are waiting for an answer; poll inbox.list for the reply.
    #[serde(default)]
    needs_reply: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct InboxList {
    /// Only unresolved messages (default true).
    #[serde(default)]
    open_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ReplyArgs {
    /// Message id.
    id: String,
    /// Your answer; omit to just mark it resolved.
    #[serde(default)]
    reply: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsList {
    /// Absolute directory.
    path: String,
    /// Levels to descend (1-4, default 1).
    #[serde(default)]
    depth: Option<usize>,
    /// Maximum entries (default 200).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsRead {
    /// Absolute file path.
    path: String,
    /// First line to return, 1-based (default 1).
    #[serde(default)]
    offset: Option<usize>,
    /// Lines to return (default 400, at most 2000).
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsSearch {
    /// Absolute directory or file to search.
    path: String,
    /// Regular expression matched against each line.
    pattern: String,
    /// File-name glob such as `*.rs` or `src/**/*.ts`.
    #[serde(default)]
    glob: Option<String>,
    /// Maximum matches (default 50).
    #[serde(default)]
    limit: Option<usize>,
    /// Lines of context around each match (0-3).
    #[serde(default)]
    context: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsWrite {
    /// Absolute file path.
    path: String,
    /// The complete new content.
    content: String,
    /// Create missing parent directories.
    #[serde(default)]
    create_dirs: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsEdit {
    /// Absolute file path.
    path: String,
    /// Exact text to replace (must be unique unless replace_all).
    old: String,
    /// Replacement text.
    new: String,
    /// Replace every occurrence.
    #[serde(default)]
    replace_all: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsMove {
    /// Existing path.
    from: String,
    /// New path (must not exist).
    to: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FsDelete {
    /// Absolute path.
    path: String,
    /// Delete a non-empty directory with its contents.
    #[serde(default)]
    recursive: Option<bool>,
}

// ----------------------------------------------------------------- helpers

fn provider_view(ctx: &Ctx, p: &Provider) -> Value {
    let key = secrets::read_key(&ctx.paths, p);
    json!({
        "id": p.id, "kind": p.kind, "label": p.label, "base_url": p.base_url,
        "key_env": p.key_env, "default_model": p.default_model,
        "is_default": p.is_default, "enabled": p.enabled,
        "key_source": secrets::key_source(&ctx.paths, p),
        "key_hint": key.as_deref().map(secrets::mask),
        "has_key": key.is_some(),
        "needs_key": p.kind != "openai-compatible" || !p.base_url.contains("localhost") && !p.base_url.contains("127.0.0.1"),
        "created_at": p.created_at, "updated_at": p.updated_at,
    })
}

fn find_provider(ctx: &Ctx, id: &str) -> Result<Provider> {
    ctx.store.get_provider(id)?.ok_or_else(|| DevdError::not_found(format!("no provider `{id}`; see provider.list")))
}

/// What a health check means and what to do about it.
fn explain(name: &str, level: &str, detail: &str) -> (String, Value) {
    let (text, fix): (String, Value) = match name {
        "git" => ("Git is needed for every repository feature.".into(), json!({ "label": "Install Git", "capability": "toolchain.install", "args": { "tool": "git" } })),
        n if n.starts_with("port.") => (
            "Something else is using a port devd would serve sites on. Sites still work on the ports devd is configured for once the other program stops; nothing is broken in devd itself.".into(),
            json!({ "label": "See what is using it", "view": "system" }),
        ),
        "state" => (format!("devd's database could not be opened or is damaged ({detail}). Nothing is saved until this is fixed. Check free disk space and permissions on the devd directory."), json!({ "label": "Open system details", "view": "system" })),
        "settings" => ("The settings file is invalid, so defaults are being used. Fix the file named in the detail.".into(), json!({ "label": "Open system details", "view": "system" })),
        "layout" => ("devd cannot use its data directory. Check that it exists and is writable.".into(), json!({ "label": "Open system details", "view": "system" })),
        "ai.provider" => ("The assistant and in-app agents need a model provider. Add one and an API key to use them. Nothing else depends on it.".into(), json!({ "label": "Set up a provider", "view": "providers" })),
        "inbox.approvals" => ("Agents are waiting for you to approve or decline operations. They carry on with other work meanwhile.".into(), json!({ "label": "Review", "view": "inbox" })),
        "inbox.messages" => ("An agent asked you something and is waiting for the answer.".into(), json!({ "label": "Reply", "view": "inbox" })),
        _ => (if level == "fail" { "This check failed; see the detail.".into() } else { "For information.".into() }, json!({ "label": "Open system details", "view": "system" })),
    };
    (text, fix)
}

/// Register everything.
pub fn register(reg: &mut Registry, hub: Arc<Hub>) -> Result<()> {
    // ----------------------------------------------------------- providers
    add::<NoArgs, _, _>(reg, "provider.presets", "Provider presets", "Ready-made configurations for Anthropic, OpenAI, Gemini, OpenRouter, Groq, Ollama, LM Studio and any OpenAI-compatible endpoint, with where to get a key, the API documentation and how models are listed.",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("provider.presets", "list provider presets"),
        |_, _| Ok(Outcome::with_data("provider presets", serde_json::to_value(providers::presets())?)))?;

    add::<NoArgs, _, _>(reg, "provider.list", "List providers", "Configured model providers with whether a key is set and where it comes from. Keys are never returned, only the last four characters.",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("provider.list", "list providers"),
        |ctx, _| {
            let v: Vec<Value> = ctx.store.list_providers()?.iter().map(|p| provider_view(ctx, p)).collect();
            Ok(Outcome::with_data(format!("{} provider(s)", v.len()), json!(v)))
        })?;

    add::<SaveProvider, _, _>(reg, "provider.save", "Add or update a provider", "Creates or updates a provider, optionally from a preset, and stores its API key privately (owner-only file, never in the database, logs or API responses). Plain http is accepted only for localhost. Human only.",
        Tier::Write, Effect::Write, false, true,
        |_, a| simple("provider.save", format!("save provider `{}`{}", a.id, if a.api_key.is_some() { " and store its API key" } else { "" })),
        |ctx, a| {
            if !secrets::valid_id(&a.id) {
                return Err(DevdError::invalid("the provider id must be lowercase letters, digits and hyphens"));
            }
            let existing = ctx.store.get_provider(&a.id)?;
            let preset = a.preset.as_ref().and_then(|id| providers::presets().into_iter().find(|p| &p.id == id));
            if a.preset.is_some() && preset.is_none() {
                return Err(DevdError::not_found("unknown preset; see provider.presets"));
            }
            let pick = |given: &Option<String>, from_existing: Option<String>, from_preset: Option<String>| given.clone().or(from_existing).or(from_preset);
            let kind = pick(&a.kind, existing.as_ref().map(|p| p.kind.clone()), preset.as_ref().map(|p| p.kind.clone())).ok_or_else(|| DevdError::invalid("name a kind or a preset"))?;
            if !["anthropic", "openai", "openai-compatible"].contains(&kind.as_str()) {
                return Err(DevdError::invalid("kind must be anthropic, openai or openai-compatible"));
            }
            let base = pick(&a.base_url, existing.as_ref().map(|p| p.base_url.clone()), preset.as_ref().map(|p| p.base_url.clone())).ok_or_else(|| DevdError::invalid("a base URL is required"))?;
            let base_url = providers::validate_base_url(&base)?;
            let label = pick(&a.label, existing.as_ref().map(|p| p.label.clone()), preset.as_ref().map(|p| p.label.clone())).unwrap_or_else(|| a.id.clone());
            let key_env = pick(&a.key_env, existing.as_ref().and_then(|p| p.key_env.clone()), preset.as_ref().and_then(|p| p.key_env.clone())).filter(|v| !v.is_empty());
            if let Some(v) = &key_env {
                if !v.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                    return Err(DevdError::invalid("an environment variable name uses capital letters, digits and underscores"));
                }
            }
            let now = util::now();
            let first = ctx.store.list_providers()?.is_empty();
            let p = Provider {
                id: a.id.clone(), kind, label, base_url, key_env,
                default_model: a.default_model.clone().or_else(|| existing.as_ref().and_then(|e| e.default_model.clone())),
                is_default: a.make_default.unwrap_or(existing.as_ref().map(|e| e.is_default).unwrap_or(first)),
                enabled: a.enabled.unwrap_or(existing.as_ref().map(|e| e.enabled).unwrap_or(true)),
                created_at: existing.as_ref().map(|e| e.created_at.clone()).unwrap_or_else(|| now.clone()),
                updated_at: now,
            };
            ctx.store.upsert_provider(&p)?;
            if let Some(k) = &a.api_key {
                secrets::save_key(&ctx.paths, &p.id, k)?;
            }
            Ok(Outcome::with_data(format!("saved provider {}", p.label), provider_view(ctx, &p)))
        })?;

    add::<IdArgs, _, _>(reg, "provider.default", "Choose the default provider", "New conversations use this provider unless they name another. Human only.",
        Tier::Write, Effect::Write, false, true,
        |_, a| simple("provider.default", format!("make `{}` the default provider", a.id)),
        |ctx, a| {
            find_provider(ctx, &a.id)?;
            ctx.store.set_default_provider(&a.id)?;
            Ok(Outcome::ok(format!("{} is now the default provider", a.id)))
        })?;

    add::<IdArgs, _, _>(reg, "provider.remove", "Remove a provider", "Deletes the provider, its stored API key and its cached model list. Conversations that used it keep their history. Human only; asks for approval.",
        Tier::Destructive, Effect::Data, false, true,
        |_, a| Ok(Plan::new("provider.remove", format!("remove provider `{}` and its stored API key", a.id)).effect("the API key file is deleted; you will need to enter it again to restore the provider").destructive(true)),
        |ctx, a| {
            find_provider(ctx, &a.id)?;
            ctx.store.delete_provider(&a.id)?;
            secrets::delete_key(&ctx.paths, &a.id);
            Ok(Outcome::ok(format!("removed {}", a.id)))
        })?;

    add::<IdArgs, _, _>(reg, "provider.test", "Test a provider", "Checks that the base URL is reachable and the key is accepted by asking the provider for its model list. Reports the actual error (bad key, rate limit, unreachable host) rather than a generic failure.",
        Tier::Exec, Effect::Remote, false, false,
        |_, a| simple("provider.test", format!("contact `{}` and list its models", a.id)),
        |ctx, a| {
            let p = find_provider(ctx, &a.id)?;
            let key = secrets::read_key(&ctx.paths, &p);
            let r = providers::test(&p, key.as_deref());
            let mut o = Outcome::with_data(r.message.clone(), serde_json::to_value(&r)?);
            o.ok = r.ok;
            Ok(o)
        })?;

    add::<IdArgs, _, _>(reg, "provider.models", "Fetch a provider's models", "Reads the provider's own model list with its key (Anthropic GET /v1/models, others GET /models) and caches it. This is authoritative for the account: it shows exactly what this key may use.",
        Tier::Exec, Effect::Remote, false, false,
        |_, a| simple("provider.models", format!("list the models `{}` offers", a.id)),
        |ctx, a| {
            let p = find_provider(ctx, &a.id)?;
            let key = secrets::read_key(&ctx.paths, &p);
            let models = providers::list_models(&p, key.as_deref())?;
            ctx.store.replace_catalogue(&format!("provider:{}", p.id), &models)?;
            Ok(Outcome::with_data(format!("{} model(s) from {}", models.len(), p.label), serde_json::to_value(&models)?))
        })?;

    add::<NoArgs, _, _>(reg, "models.refresh_official", "Fetch Anthropic's official model list", "Reads the Claude models overview in Anthropic's documentation and caches each model's API id, description, pricing, context window, maximum output and retirement date. Needs no API key.",
        Tier::Write, Effect::Remote, true, false,
        |_, _| simple("models.refresh_official", format!("fetch {}", providers::ANTHROPIC_MODELS_DOC)),
        |ctx, _| {
            let models = providers::fetch_anthropic_docs()?;
            ctx.store.replace_catalogue("anthropic-docs", &models)?;
            Ok(Outcome::with_data(format!("{} official Claude model(s)", models.len()), serde_json::to_value(&models)?))
        })?;

    add::<CatalogueArgs, _, _>(reg, "models.catalogue", "The model catalogue", "Cached models from the official documentation and from each provider's API, with prices, context windows and where each came from. Run models.refresh_official or provider.models to update it.",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("models.catalogue", "read the cached model catalogue"),
        |ctx, a| {
            let m = ctx.store.list_catalogue(a.source.as_deref())?;
            Ok(Outcome::with_data(format!("{} model(s)", m.len()), serde_json::to_value(&m)?))
        })?;

    // ------------------------------------------------------------- chat
    {
        let hub = hub.clone();
        add::<SendArgs, _, _>(reg, "chat.send", "Talk to the built-in agent", "Sends a message to devd's built-in agent, which can use every capability devd has within the permission model: inspect and edit files, run tasks and tests, manage git, install tools. It works in the background; read progress with chat.get. Operations above its tier wait for your approval.",
            Tier::Exec, Effect::Process, false, false,
            |_, a| simple("chat.send", format!("send a message to the built-in agent{}", a.project.as_ref().map(|p| format!(" in {p}")).unwrap_or_default())),
            move |ctx, a| {
                let t = hub.send(ctx, SendRequest { thread_id: a.thread.clone(), message: a.message.clone(), project: a.project.clone(), provider: a.provider.clone(), model: a.model.clone() })?;
                Ok(Outcome::with_data("sent; the agent is working", serde_json::to_value(&t)?))
            })?;
    }
    add::<NoArgs, _, _>(reg, "chat.list", "List conversations", "Recent conversations with the built-in agent, with their state (idle, running, waiting_approval, error) and token use.",
        Tier::Read, Effect::Read, false, false,
        |_, _| simple("chat.list", "list conversations"),
        |ctx, _| {
            let t = ctx.store.list_threads(100)?;
            Ok(Outcome::with_data(format!("{} conversation(s)", t.len()), serde_json::to_value(&t)?))
        })?;
    {
        let hub = hub.clone();
        add::<ThreadArgs, _, _>(reg, "chat.get", "Read a conversation", "The conversation's events after a sequence number (poll with the last id you saw), its state and any operations waiting for approval.",
            Tier::Read, Effect::Read, false, false,
            |_, a| simple("chat.get", format!("read conversation {}", a.thread)),
            move |ctx, a| {
                let t = ctx.store.get_thread(&a.thread)?.ok_or_else(|| DevdError::not_found("no such conversation"))?;
                let ev = ctx.store.list_events(&a.thread, a.after.unwrap_or(0), 500)?;
                let pending = ctx.store.list_requests(true, Some(&a.thread), 50)?;
                Ok(Outcome::with_data(t.state.clone(), json!({ "thread": t, "events": ev, "pending": pending, "running": hub.is_running(&a.thread) })))
            })?;
    }
    {
        let hub = hub.clone();
        add::<ThreadArgs, _, _>(reg, "chat.stop", "Stop the agent", "Stops the agent after its current step and declines anything it was waiting to have approved.",
            Tier::Exec, Effect::Process, false, false,
            |_, a| simple("chat.stop", format!("stop conversation {}", a.thread)),
            move |ctx, a| {
                hub.stop(ctx, &a.thread)?;
                Ok(Outcome::ok("stopping"))
            })?;
    }
    add::<UpdateThread, _, _>(reg, "chat.update", "Rename a conversation or change its model", "Renames a conversation or switches its provider or model for the next message.",
        Tier::Write, Effect::Write, false, false,
        |_, a| simple("chat.update", format!("update conversation {}", a.thread)),
        |ctx, a| {
            ctx.store.get_thread(&a.thread)?.ok_or_else(|| DevdError::not_found("no such conversation"))?;
            ctx.store.update_thread(&a.thread, a.title.as_deref(), a.provider.as_deref(), a.model.as_deref())?;
            Ok(Outcome::ok("updated"))
        })?;
    add::<ThreadArgs, _, _>(reg, "chat.delete", "Delete a conversation", "Deletes a conversation and its log. Cannot be undone.",
        Tier::Destructive, Effect::Data, false, true,
        |_, a| Ok(Plan::new("chat.delete", format!("delete conversation {} and its log", a.thread)).destructive(true)),
        |ctx, a| {
            ctx.store.delete_thread(&a.thread)?;
            Ok(Outcome::ok("deleted"))
        })?;

    // -------------------------------------------------------- approvals
    add::<ListRequests, _, _>(reg, "approval.list", "List agent requests", "Operations agents asked a person to approve, with their plan and state. Pending ones wait in the inbox.",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("approval.list", "list requests"),
        |ctx, a| {
            let r = ctx.store.list_requests(a.pending_only.unwrap_or(true), a.thread.as_deref(), 100)?;
            Ok(Outcome::with_data(format!("{} request(s)", r.len()), serde_json::to_value(&r)?))
        })?;
    add::<IdArgs, _, _>(reg, "approval.get", "Check one request", "The state of a queued request (pending, approved, declined, failed) and, once approved, what the operation returned. Poll this instead of asking again.",
        Tier::Read, Effect::Read, true, false,
        |_, a| simple("approval.get", format!("read request {}", a.id)),
        |ctx, a| {
            let r = ctx.store.get_request(&a.id)?.ok_or_else(|| DevdError::not_found("no such request"))?;
            Ok(Outcome::with_data(r.state.clone(), serde_json::to_value(&r)?))
        })?;
    {
        let hub = hub.clone();
        add::<DecideArgs, _, _>(reg, "approval.decide", "Approve or decline an agent's request", "A person's decision on an operation an agent queued. Approving runs exactly the stored operation, as that agent, with your approval recorded in the audit trail. Human only.",
            Tier::Exec, Effect::Process, false, true,
            |ctx, a| {
                let r = ctx.store.get_request(&a.id)?.ok_or_else(|| DevdError::not_found("no such request"))?;
                let mut p = Plan::new("approval.decide", format!("{} `{}` requested by {}", if a.approve { "run" } else { "decline" }, r.capability, r.actor));
                if let Some(plan) = &r.plan {
                    if let Some(steps) = plan["steps"].as_array() {
                        for s in steps.iter().filter_map(|s| s.as_str()) {
                            p = p.step(s.to_string());
                        }
                    }
                }
                Ok(p.data(json!({ "request": r })))
            },
            move |ctx, a| {
                let r = hub.decide(ctx, &a.id, a.approve)?;
                let mut o = Outcome::with_data(format!("{} {}", r.capability, r.state), serde_json::to_value(&r)?);
                o.ok = r.state != "failed";
                Ok(o)
            })?;
    }

    // ------------------------------------------------------------ inbox
    add::<PostArgs, _, _>(reg, "inbox.post", "Message the person", "Leaves a message in the person's inbox: a finished piece of work, a question, a decision you need. Set needs_reply when you are waiting for an answer and poll inbox.list for it. Use sparingly; one clear message beats several.",
        Tier::Write, Effect::Write, true, false,
        |_, a| simple("inbox.post", format!("post `{}` to the inbox", a.subject)),
        |ctx, a| {
            if a.subject.trim().is_empty() || a.body.trim().is_empty() {
                return Err(DevdError::invalid("a subject and a body are required"));
            }
            let m = ctx.store.post_inbox(&ctx.actor.label(), a.project.as_deref(), util::truncate(a.subject.trim(), 200).as_str(), util::truncate(a.body.trim(), 8000).as_str(), a.needs_reply.unwrap_or(false))?;
            ctx.events.emit_with("inbox.message", m.subject.clone(), json!({ "id": m.id }));
            Ok(Outcome::with_data("posted", json!({ "id": m.id })))
        })?;
    add::<InboxList, _, _>(reg, "inbox.list", "Read the inbox", "Messages agents left for the person, with the person's replies.",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("inbox.list", "read the inbox"),
        |ctx, a| {
            let m = ctx.store.list_inbox(a.open_only.unwrap_or(true), 100)?;
            Ok(Outcome::with_data(format!("{} message(s)", m.len()), serde_json::to_value(&m)?))
        })?;
    add::<ReplyArgs, _, _>(reg, "inbox.reply", "Reply to an inbox message", "Answers a message from an agent (it reads the reply with inbox.list) and marks it resolved. Human only.",
        Tier::Write, Effect::Write, false, true,
        |_, a| simple("inbox.reply", format!("answer message {}", a.id)),
        |ctx, a| {
            ctx.store.get_inbox(&a.id)?.ok_or_else(|| DevdError::not_found("no such message"))?;
            ctx.store.answer_inbox(&a.id, a.reply.as_deref().map(str::trim).filter(|r| !r.is_empty()))?;
            Ok(Outcome::ok("resolved"))
        })?;

    // ------------------------------------------------------------- files
    add::<FsList, _, _>(reg, "fs.list", "List files", "Lists a directory (up to four levels), skipping build output and dependency folders, and flagging secrets files without reading them.",
        Tier::Read, Effect::Read, true, false,
        |_, a| simple("fs.list", format!("list {}", a.path)),
        |ctx, a| Ok(Outcome::with_data("listed", fs::list(ctx, &a.path, a.depth.unwrap_or(1), a.limit.unwrap_or(200))?)))?;
    add::<FsRead, _, _>(reg, "fs.read", "Read a file", "Reads a text file with line numbers. Secrets files (.env, keys) are refused. Large files can be read in parts with offset and limit.",
        Tier::Read, Effect::Read, true, false,
        |_, a| simple("fs.read", format!("read {}", a.path)),
        |ctx, a| Ok(Outcome::with_data("read", fs::read(ctx, &a.path, a.offset.unwrap_or(1), a.limit.unwrap_or(400))?)))?;
    add::<FsSearch, _, _>(reg, "fs.search", "Search file contents", "Regular-expression search across a project, skipping dependency folders, binaries and secrets files. Returns file, line and text.",
        Tier::Read, Effect::Read, true, false,
        |_, a| simple("fs.search", format!("search {} for `{}`", a.path, a.pattern)),
        |ctx, a| Ok(Outcome::with_data("searched", fs::search(ctx, &a.path, &a.pattern, a.glob.as_deref(), a.limit.unwrap_or(50), a.context.unwrap_or(0).min(3))?)))?;
    add::<FsWrite, _, _>(reg, "fs.write", "Write a file", "Creates a file or replaces it entirely. Prefer fs.edit for changes to existing files. Secrets files are refused.",
        Tier::Write, Effect::Write, true, false,
        |_, a| Ok(Plan::new("fs.write", format!("write {} ({} bytes)", a.path, a.content.len())).effect("replaces the file if it exists")),
        |ctx, a| Ok(Outcome::with_data("written", fs::write(ctx, &a.path, &a.content, a.create_dirs.unwrap_or(false))?)))?;
    add::<FsEdit, _, _>(reg, "fs.edit", "Edit a file", "Replaces exact text in a file. `old` must match exactly once (include surrounding lines to make it unique) unless replace_all is set. Read the file first so the text is current.",
        Tier::Write, Effect::Write, true, false,
        |_, a| simple("fs.edit", format!("edit {}", a.path)),
        |ctx, a| Ok(Outcome::with_data("edited", fs::edit(ctx, &a.path, &a.old, &a.new, a.replace_all.unwrap_or(false))?)))?;
    add::<FsMove, _, _>(reg, "fs.move", "Move or rename", "Moves a file or directory. Never overwrites an existing path.",
        Tier::Write, Effect::Write, true, false,
        |_, a| simple("fs.move", format!("move {} to {}", a.from, a.to)),
        |ctx, a| Ok(Outcome::with_data("moved", fs::mv(ctx, &a.from, &a.to)?)))?;
    add::<FsDelete, _, _>(reg, "fs.delete", "Delete a file or directory", "Deletes a file, or a directory (recursive for a non-empty one). Needs a person's approval.",
        Tier::Destructive, Effect::Data, true, false,
        |_, a| Ok(Plan::new("fs.delete", format!("delete {}{}", a.path, if a.recursive.unwrap_or(false) { " and everything in it" } else { "" })).destructive(true)),
        |ctx, a| Ok(Outcome::with_data("deleted", fs::delete(ctx, &a.path, a.recursive.unwrap_or(false))?)))?;

    // ------------------------------------------------------------ health
    add::<NoArgs, _, _>(reg, "health.status", "Health summary", "One verdict - ok, attention or problem - with every check, what it means in plain words and the fix to offer: environment checks, whether an AI provider is configured, and what is waiting for a person (approvals, agent questions).",
        Tier::Read, Effect::Read, true, false,
        |_, _| simple("health.status", "check health"),
        |ctx, _| {
            let mut checks = doctor::core_checks(&ctx.paths, &ctx.settings);
            checks.push(devd_git::doctor_check(ctx.settings.git_executable.as_deref()));
            let providers = ctx.store.list_providers()?;
            let provider_check = match providers.iter().find(|p| p.is_default && p.enabled).or_else(|| providers.iter().find(|p| p.enabled)) {
                None => doctor::Check::info("ai.provider", "no AI provider configured"),
                Some(p) if secrets::read_key(&ctx.paths, p).is_none() && p.kind != "openai-compatible" => doctor::Check::info("ai.provider", format!("{} has no API key", p.label)),
                Some(p) => doctor::Check::ok("ai.provider", format!("{} ready", p.label)),
            };
            checks.push(provider_check);
            let pending = ctx.store.list_requests(true, None, 200)?.len();
            if pending > 0 {
                checks.push(doctor::Check::info("inbox.approvals", format!("{pending} operation(s) waiting for your approval")));
            }
            let asks = ctx.store.list_inbox(true, 200)?.iter().filter(|m| m.needs_reply).count();
            if asks > 0 {
                checks.push(doctor::Check::info("inbox.messages", format!("{asks} agent question(s) waiting for your reply")));
            }
            let failing = checks.iter().filter(|c| c.level == Level::Fail).count();
            let attention = checks.iter().filter(|c| c.level == Level::Info).count();
            let level = if failing > 0 { "problem" } else if attention > 0 { "attention" } else { "ok" };
            let rows: Vec<Value> = checks
                .iter()
                .map(|c| {
                    let lvl = serde_json::to_value(c.level).ok().and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_default();
                    let (meaning, fix) = explain(&c.name, &lvl, &c.detail);
                    json!({ "name": c.name, "level": lvl, "detail": c.detail, "meaning": meaning, "fix": fix })
                })
                .collect();
            let headline = match level {
                "ok" => "Everything is working".to_string(),
                "attention" => format!("{attention} thing(s) need a look"),
                _ => format!("{failing} problem(s) need fixing"),
            };
            Ok(Outcome::with_data(headline.clone(), json!({ "level": level, "headline": headline, "failing": failing, "attention": attention, "checks": rows })))
        })?;

    Ok(())
}
