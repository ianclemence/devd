//! What a model may be used for, what it can do, and what it costs.
//!
//! Three questions, each answered from a durable place rather than guessed at
//! the moment of the call:
//!
//! * **Privacy.** May this provider see this project at all? A person can say
//!   "local models only", or allow or block providers by id.
//! * **Cost.** How much has this run spent, and is it over its budget?
//! * **Capability.** Does this model take tools, how large is its window, what
//!   does a token cost? The catalogue says what a provider reported; a small
//!   table of well-known model families fills in what no provider reports.
//!
//! The settings are plain keys in the state store (`policy.*`), so the window,
//! the command line and the agent all read the same ones.

use std::net::IpAddr;

use serde::Serialize;

use devd_core::state::Store;
use devd_core::Provider;
use devd_core::{DevdError, Result};

/// `true` limits every call to providers on this machine.
pub const LOCAL_ONLY: &str = "policy.local_only";
/// Comma-separated provider ids that may be used. Empty means no restriction.
pub const ALLOW: &str = "policy.allow_providers";
/// Comma-separated provider ids that may never be used.
pub const BLOCK: &str = "policy.block_providers";
/// Most a single run may spend, in US dollars, as far as prices are known.
pub const MAX_COST: &str = "policy.max_cost_usd_per_run";

/// The rules in force.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Policy {
    /// Only providers on this machine.
    pub local_only: bool,
    /// Providers that may be used; empty means any.
    pub allow: Vec<String>,
    /// Providers that may not be used.
    pub block: Vec<String>,
    /// Per-run spending limit, in US dollars.
    pub max_cost_usd: Option<f64>,
}

fn list(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl Policy {
    /// Read the rules from the store.
    pub fn load(store: &Store) -> Self {
        let get = |key: &str| store.get_setting(key).ok().flatten();
        Self {
            local_only: get(LOCAL_ONLY).is_some_and(|v| matches!(v.trim(), "true" | "1" | "yes")),
            allow: list(get(ALLOW)),
            block: list(get(BLOCK)),
            max_cost_usd: get(MAX_COST)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| *v > 0.0),
        }
    }

    /// Whether a provider may be used, and if not, why.
    pub fn permits(&self, provider: &Provider) -> Result<()> {
        if self.block.iter().any(|b| b == &provider.id) {
            return Err(DevdError::denied(format!(
                "{} is blocked by your privacy policy ({BLOCK})",
                provider.label
            )));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|a| a == &provider.id) {
            return Err(DevdError::denied(format!(
                "{} is not on your list of allowed providers ({ALLOW})",
                provider.label
            )));
        }
        if self.local_only && !is_local_url(&provider.base_url) {
            return Err(DevdError::denied(format!(
                "{} is not on this machine, and your privacy policy ({LOCAL_ONLY}) allows only local models",
                provider.label
            )));
        }
        Ok(())
    }

    /// Whether a run that has spent `spent` dollars may make another call.
    pub fn within_budget(&self, spent: f64) -> Result<()> {
        match self.max_cost_usd {
            Some(limit) if spent >= limit => Err(DevdError::denied(format!(
                "this run has spent ${spent:.4}, which reaches the limit of ${limit:.2} ({MAX_COST})"
            ))),
            _ => Ok(()),
        }
    }
}

/// Whether a base URL points at this machine.
pub fn is_local_url(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// What is known about a model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelCaps {
    /// Whether it calls tools. `None` when nothing says.
    pub tools: Option<bool>,
    /// Whether it accepts images.
    pub vision: Option<bool>,
    /// Whether it reasons before it answers.
    pub reasoning: Option<bool>,
    /// Input window in tokens.
    pub context_window: Option<i64>,
    /// Output limit in tokens.
    pub max_output: Option<i64>,
    /// US dollars per million input tokens.
    pub input_price: Option<f64>,
    /// US dollars per million output tokens.
    pub output_price: Option<f64>,
    /// US dollars per million tokens read from the prompt cache, when published.
    pub cache_read_price: Option<f64>,
    /// US dollars per million tokens written to the prompt cache, when published.
    pub cache_write_price: Option<f64>,
    /// Where the answer came from: `catalogue`, `models.dev`, `family` or `unknown`.
    pub source: String,
}

/// Facts about well-known model families that providers do not report.
///
/// Matched on the model id, longest pattern first. It is deliberately short:
/// anything a provider's own listing says takes precedence, and an unknown
/// model is reported as unknown rather than assumed to work.
type Traits = (Option<bool>, Option<bool>, Option<bool>);

fn family(model: &str) -> Traits {
    let id = model.to_lowercase();
    // (pattern, tools, vision, reasoning)
    #[allow(clippy::type_complexity)]
    const FAMILIES: &[(&str, Option<bool>, Option<bool>, Option<bool>)] = &[
        // Explicit multimodal names first: a model id that says it sees must
        // win over a family guess, and these also cover the smaller vendors.
        ("pixtral", Some(true), Some(true), None),
        ("mistral-small-3", Some(true), Some(true), None),
        ("llava", Some(true), Some(true), None),
        ("internvl", Some(true), Some(true), None),
        ("molmo", Some(true), Some(true), None),
        ("idefics", Some(true), Some(true), None),
        ("cogvlm", Some(true), Some(true), None),
        ("moondream", Some(true), Some(true), None),
        ("minicpm-v", Some(true), Some(true), None),
        ("qwen3-vl", Some(true), Some(true), None),
        ("qwen2.5-vl", Some(true), Some(true), None),
        ("qwen2-vl", Some(true), Some(true), None),
        ("qwen-vl", Some(true), Some(true), None),
        ("glm-4.1v", Some(true), Some(true), None),
        ("glm-4v", Some(true), Some(true), None),
        ("llama-4", Some(true), Some(true), None),
        ("gemma-3", Some(true), Some(true), None),
        ("phi-4-multimodal", Some(true), Some(true), None),
        ("phi-3.5-vision", Some(true), Some(true), None),
        ("gpt-4.1", Some(true), Some(true), None),
        ("gpt-4-turbo", Some(true), Some(true), None),
        ("-vl", Some(true), Some(true), None),
        ("vision", Some(true), Some(true), None),
        ("multimodal", Some(true), Some(true), None),
        ("omni", Some(true), Some(true), None),
        ("deepseek-reasoner", Some(false), Some(false), Some(true)),
        ("deepseek", Some(true), Some(false), None),
        ("claude", Some(true), Some(true), None),
        ("gemini", Some(true), Some(true), None),
        ("gpt-4o", Some(true), Some(true), Some(false)),
        ("gpt-5", Some(true), Some(true), Some(true)),
        ("^o1", Some(true), Some(false), Some(true)),
        ("^o3", Some(true), Some(true), Some(true)),
        ("^o4", Some(true), Some(true), Some(true)),
        ("mistral", Some(true), None, None),
        ("llama", None, None, None),
        ("qwen", Some(true), None, None),
        ("kimi", Some(true), None, None),
        ("glm", Some(true), None, None),
    ];
    FAMILIES
        .iter()
        .find(|(pattern, ..)| match pattern.strip_prefix('^') {
            Some(prefix) => id.starts_with(prefix),
            None => id.contains(pattern),
        })
        .map(|(_, tools, vision, reasoning)| (*tools, *vision, *reasoning))
        .unwrap_or((None, None, None))
}

/// What is known about `model` as served by `provider`.
///
/// Three sources, most particular first: the entry the provider's own listing
/// produced (and a price a person set by hand), then the open catalogue at
/// models.dev, then the family table for behaviour neither states. A fact is
/// taken from the first source that has one, field by field.
pub fn model_caps(store: &Store, provider: &Provider, model: &str) -> ModelCaps {
    let entry = store
        .list_catalogue(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.model_id == model)
        // A provider's own listing is the best source for that provider.
        .max_by_key(|m| m.source == format!("provider:{}", provider.id));
    let open = crate::modelsdev::current();
    let known = open.find(provider, model);
    let (family_tools, family_vision, family_reasoning) = family(model);
    let listed_tools = entry.as_ref().and_then(|m| m.tools);
    let listed_vision = entry.as_ref().and_then(|m| m.vision);
    let listed_reasoning = entry.as_ref().and_then(|m| m.reasoning);
    let tools = listed_tools.or(known.and_then(|k| k.tools)).or(family_tools);
    let vision = listed_vision.or(known.and_then(|k| k.vision)).or(family_vision);
    let reasoning = listed_reasoning.or(known.and_then(|k| k.reasoning)).or(family_reasoning);
    let from_catalogue = listed_tools.is_some() || listed_vision.is_some() || listed_reasoning.is_some();
    let source = if from_catalogue || entry.is_some() {
        "catalogue"
    } else if known.is_some() {
        "models.dev"
    } else if tools.is_some() || vision.is_some() || reasoning.is_some() {
        "family"
    } else {
        "unknown"
    };
    ModelCaps {
        tools,
        vision,
        reasoning,
        context_window: entry.as_ref().and_then(|m| m.context_window).or(known.and_then(|k| k.context)),
        max_output: entry.as_ref().and_then(|m| m.max_output).or(known.and_then(|k| k.output)),
        input_price: entry.as_ref().and_then(|m| m.input_price).or(known.and_then(|k| k.input_price)),
        output_price: entry.as_ref().and_then(|m| m.output_price).or(known.and_then(|k| k.output_price)),
        cache_read_price: known.and_then(|k| k.cache_read_price),
        cache_write_price: known.and_then(|k| k.cache_write_price),
        source: source.to_string(),
    }
}

/// What a call cost, when both prices are known.
pub fn cost_usd(caps: &ModelCaps, input_tokens: i64, output_tokens: i64) -> Option<f64> {
    let input = caps.input_price?;
    let output = caps.output_price?;
    Some((input * input_tokens as f64 + output * output_tokens as f64) / 1_000_000.0)
}

/// What a call cost, with prompt-cache tokens priced as the provider bills them.
///
/// The provider's list price is for ordinary input. A cache read is cheaper and
/// a cache write is dearer. The model's own published cache prices are used when
/// the catalogue has them; otherwise the vendor's published ratio to the input
/// price stands in. Either applies only to tokens the provider reported as
/// cached. `input` already includes them, so the uncached part is the remainder.
pub fn cost_usd_cached(
    caps: &ModelCaps,
    provider_kind: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_write: i64,
) -> Option<f64> {
    let input = caps.input_price?;
    let output = caps.output_price?;
    let (read_mult, write_mult) = if provider_kind == "anthropic" {
        (0.1, 1.25)
    } else if provider_kind == "gemini" {
        (0.25, 1.0)
    } else if provider_kind == "deepseek" {
        // A cache hit costs a tenth of a miss; DeepSeek charges no premium to write the cache.
        (0.1, 1.0)
    } else {
        // OpenAI-compatible listings report a cached prompt token at half price.
        (0.5, 1.0)
    };
    let cached = (cache_read + cache_write).min(input_tokens.max(0));
    let uncached = (input_tokens - cached).max(0);
    let read = caps.cache_read_price.unwrap_or(input * read_mult);
    let write = caps.cache_write_price.unwrap_or(input * write_mult);
    Some((input * uncached as f64 + read * cache_read as f64 + write * cache_write as f64 + output * output_tokens as f64) / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, url: &str) -> Provider {
        Provider {
            id: id.into(),
            kind: "openai-compatible".into(),
            label: id.to_uppercase(),
            base_url: url.into(),
            key_env: None,
            default_model: None,
            is_default: false,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn local_means_loopback_and_nothing_that_merely_looks_like_it() {
        assert!(is_local_url("http://localhost:11434/v1"));
        assert!(is_local_url("http://127.0.0.1:1234/v1"));
        assert!(is_local_url("http://[::1]:8080"));
        assert!(is_local_url("http://models.localhost/v1"));
        assert!(!is_local_url("https://api.openai.com/v1"));
        assert!(!is_local_url("http://localhost.evil.example/v1"));
        assert!(!is_local_url("http://127.0.0.1.evil.example/v1"));
        assert!(!is_local_url("http://user@evil.example/localhost"));
    }

    #[test]
    fn a_local_only_policy_refuses_a_hosted_provider_and_says_why() {
        let policy = Policy {
            local_only: true,
            ..Policy::default()
        };
        assert!(policy.permits(&provider("ollama", "http://localhost:11434/v1")).is_ok());
        let err = policy
            .permits(&provider("openai", "https://api.openai.com/v1"))
            .unwrap_err();
        assert_eq!(err.code(), "permission_denied");
        assert!(err.to_string().contains(LOCAL_ONLY));
    }

    #[test]
    fn allow_and_block_lists_apply_by_id_and_block_wins() {
        let allow = Policy {
            allow: vec!["anthropic".into()],
            ..Policy::default()
        };
        assert!(allow.permits(&provider("anthropic", "https://x")).is_ok());
        assert!(allow.permits(&provider("openai", "https://x")).is_err());

        let both = Policy {
            allow: vec!["openai".into()],
            block: vec!["openai".into()],
            ..Policy::default()
        };
        assert!(both.permits(&provider("openai", "https://x")).is_err());
        assert!(Policy::default().permits(&provider("anything", "https://x")).is_ok());
    }

    #[test]
    fn the_budget_stops_the_call_that_would_follow_the_limit() {
        let policy = Policy {
            max_cost_usd: Some(1.0),
            ..Policy::default()
        };
        assert!(policy.within_budget(0.99).is_ok());
        assert!(policy.within_budget(1.0).is_err());
        assert!(Policy::default().within_budget(1e9).is_ok());
    }

    #[test]
    fn the_policy_is_read_from_the_settings_store() {
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        assert_eq!(Policy::load(&store), Policy::default());
        store.set_setting(LOCAL_ONLY, "true").unwrap();
        store.set_setting(ALLOW, "ollama, lm-studio").unwrap();
        store.set_setting(MAX_COST, "2.5").unwrap();
        let policy = Policy::load(&store);
        assert!(policy.local_only);
        assert_eq!(policy.allow, vec!["ollama", "lm-studio"]);
        assert_eq!(policy.max_cost_usd, Some(2.5));
        store.set_setting(MAX_COST, "-3").unwrap();
        assert_eq!(Policy::load(&store).max_cost_usd, None);
    }

    #[test]
    fn a_catalogue_entry_wins_and_a_family_fills_the_rest() {
        use devd_core::store_ai::CatalogueModel;
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        let p = provider("deepseek", "https://api.deepseek.com/v1");
        store
            .replace_catalogue(
                "provider:deepseek",
                &[CatalogueModel {
                    source: "provider:deepseek".into(),
                    model_id: "deepseek-chat".into(),
                    display: "DeepSeek Chat".into(),
                    description: None,
                    context_window: Some(128_000),
                    max_output: Some(8_000),
                    input_price: Some(0.27),
                    output_price: Some(1.10),
                    retirement: None,
                    url: None,
                    fetched_at: String::new(),
                    ..Default::default()
                }],
            )
            .unwrap();
        let caps = model_caps(&store, &p, "deepseek-chat");
        assert_eq!(caps.source, "catalogue");
        assert_eq!(caps.context_window, Some(128_000));
        assert_eq!(caps.tools, Some(true));
        let cost = cost_usd(&caps, 1_000_000, 100_000).unwrap();
        assert!((cost - 0.38).abs() < 1e-9, "{cost}");

        // The reasoning model is known not to take tools, whatever the family.
        assert_eq!(model_caps(&store, &p, "deepseek-reasoner").tools, Some(false));
        // An unfamiliar model is unknown, not assumed capable.
        let unknown = model_caps(&store, &p, "some-new-model");
        assert_eq!(unknown.source, "unknown");
        assert_eq!(unknown.tools, None);
        assert_eq!(cost_usd(&unknown, 10, 10), None);
    }

    #[test]
    fn a_provider_listing_can_change_what_the_family_says() {
        use devd_core::store_ai::CatalogueModel;
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        let p = provider("mystery", "https://example.test/v1");
        store
            .replace_catalogue(
                "provider:mystery",
                &[
                    CatalogueModel {
                        source: "provider:mystery".into(),
                        model_id: "mystery-9".into(),
                        display: "Mystery 9".into(),
                        vision: Some(true),
                        tools: Some(true),
                        reasoning: Some(false),
                        ..Default::default()
                    },
                    CatalogueModel {
                        source: "provider:mystery".into(),
                        model_id: "claude-text-only".into(),
                        display: "Claude text only".into(),
                        vision: Some(false),
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
        let caps = model_caps(&store, &p, "mystery-9");
        assert_eq!(caps.vision, Some(true), "the listing knows what the family does not");
        assert_eq!(caps.reasoning, Some(false));
        assert_eq!(caps.source, "catalogue");
        // An explicit false from the provider beats the family's true.
        assert_eq!(model_caps(&store, &p, "claude-text-only").vision, Some(false));
    }

    #[test]
    fn vision_is_read_from_the_model_name_when_the_provider_is_silent() {
        // A provider listing cannot describe behaviour for every vendor, so the
        // family table is the fallback. Unknown vision names must not be blind.
        assert_eq!(family("qwen2.5-vl-72b").1, Some(true));
        assert_eq!(family("llama-3.2-11b-vision").1, Some(true));
        assert_eq!(family("pixtral-large").1, Some(true));
        assert_eq!(family("deepseek-chat").1, Some(false));
        assert_eq!(family("some-new-model").1, None);
    }
}
