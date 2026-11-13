//! The model catalogue: what each model is, can do and costs.
//!
//! Asking a provider for its model list answers "what may this key use", and
//! only when the key works, the network is up and the vendor has such an
//! endpoint. It does not answer what a model costs (most lists carry no
//! prices), and it answers nothing at all before a key exists, which is exactly
//! when a person is choosing a provider. Those gaps were the "cannot fetch
//! models" reports, and they are why every conversation recorded a cost of
//! nothing: the price was never known.
//!
//! So devd reads the open catalogue at <https://models.dev>, which several
//! agent tools share, and uses it in three layers:
//!
//! 1. **A copy compiled into devd.** It is always there: offline, before any
//!    key, on first start.
//! 2. **A cached copy** in devd's data directory, written by
//!    [`refresh`], which replaces the compiled one when it is the newer of the
//!    two. A cache fetched last month must not hide the models a devd
//!    released this week knows about, and the time of a fetch cannot say
//!    which copy holds the newer facts; the newest model each one lists can.
//! 3. **The provider's own list**, which still decides what this key may use
//!    and wins wherever it states a fact itself.
//!
//! A fact is only ever taken from the catalogue for the provider it is listed
//! under, or, for a provider the catalogue does not know (a private endpoint),
//! from the vendor that makes the model. Nothing is guessed: a model the
//! catalogue does not list stays unknown.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use serde::Serialize;
use serde_json::Value;

use devd_core::{CatalogueModel, DevdError, Paths, Provider, Result};

/// Where the catalogue is published. The flat shape; see [`parse`].
pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// The copy compiled in, generated from a models.dev catalogue for the
/// providers devd has presets for.
const BUNDLED: &str = include_str!("../data/models-dev.json");

/// What the catalogue says about one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Facts {
    /// The id sent to the provider.
    pub id: String,
    /// The name a person reads.
    pub name: String,
    /// Whether it calls tools.
    pub tools: Option<bool>,
    /// Whether it accepts images.
    pub vision: Option<bool>,
    /// Whether it reasons before it answers.
    pub reasoning: Option<bool>,
    /// Input window in tokens.
    pub context: Option<i64>,
    /// Output limit in tokens.
    pub output: Option<i64>,
    /// US dollars per million input tokens.
    pub input_price: Option<f64>,
    /// US dollars per million output tokens.
    pub output_price: Option<f64>,
    /// US dollars per million tokens read from the prompt cache.
    pub cache_read_price: Option<f64>,
    /// US dollars per million tokens written to the prompt cache.
    pub cache_write_price: Option<f64>,
    /// Release date, `YYYY-MM-DD`.
    pub released: Option<String>,
    /// The vendor has said it is going away.
    pub deprecated: bool,
}

/// One provider's entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    /// The provider's name.
    pub name: String,
    /// The environment variables its key is conventionally kept in.
    pub env: Vec<String>,
    /// Its models, newest first.
    pub models: Vec<Facts>,
}

/// The whole catalogue.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Catalogue {
    providers: BTreeMap<String, Entry>,
    /// `bundled` or `cached`.
    pub origin: &'static str,
}

/// Read a catalogue.
///
/// models.dev has published two shapes and both are accepted, field by field,
/// so a refresh keeps working whichever one the address serves:
///
/// * flat: `tool_call`, `cost.input`, `limit.context`, `modalities.input: ["image"]`
/// * nested: `capabilities.tools.supported`, `cost.text.input.price`,
///   `modalities.input: ["image/png"]`
///
/// A model that cannot be read is skipped; it never fails the catalogue.
pub fn parse(text: &str) -> Result<Catalogue> {
    let root: Value = serde_json::from_str(text).map_err(|e| DevdError::invalid(format!("the model catalogue is not JSON: {e}")))?;
    let object = root.as_object().ok_or_else(|| DevdError::invalid("the model catalogue is not an object of providers"))?;
    let mut providers = BTreeMap::new();
    for (key, provider) in object {
        let Some(models) = provider["models"].as_object() else { continue };
        let mut list: Vec<Facts> = models.iter().filter_map(|(id, model)| facts(id, model)).collect();
        if list.is_empty() {
            continue;
        }
        list.sort_by(|a, b| b.released.cmp(&a.released).then(a.id.cmp(&b.id)));
        let env = provider["env"].as_array().map(|v| v.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
        providers.insert(key.clone(), Entry { name: provider["name"].as_str().unwrap_or(key).to_string(), env, models: list });
    }
    if providers.is_empty() {
        return Err(DevdError::invalid("the model catalogue lists no providers"));
    }
    Ok(Catalogue { providers, origin: "cached" })
}

fn facts(key: &str, m: &Value) -> Option<Facts> {
    // Embedding, image and speech models are not something to converse with.
    if m["type"].as_str().is_some_and(|t| t != "chat") {
        return None;
    }
    let outputs = m["modalities"]["output"].as_array();
    if outputs.is_some_and(|out| !out.iter().any(|o| o.as_str().is_some_and(|o| o.starts_with("text")))) {
        return None;
    }
    let id = m["id"].as_str().unwrap_or(key).to_string();
    let price = |flat: &Value, nested: &Value| flat.as_f64().or_else(|| nested.as_f64());
    let text = &m["cost"]["text"];
    let cache_write = match &text["input"]["cache"]["write"] {
        // The nested shape prices each cache lifetime; the short one is the default.
        Value::Object(by_ttl) => by_ttl.get("5m").or_else(|| by_ttl.values().next()).and_then(Value::as_f64),
        other => other.as_f64(),
    };
    let inputs = m["modalities"]["input"].as_array();
    Some(Facts {
        name: m["name"].as_str().unwrap_or(&id).to_string(),
        tools: m["tool_call"].as_bool().or_else(|| m["capabilities"]["tools"]["supported"].as_bool()),
        vision: inputs.map(|list| list.iter().any(|i| i.as_str().is_some_and(|i| i.starts_with("image")))),
        reasoning: m["reasoning"].as_bool().or_else(|| m["capabilities"]["reasoning"]["supported"].as_bool()),
        context: m["limit"]["context"].as_i64(),
        output: m["limit"]["output"].as_i64(),
        input_price: price(&m["cost"]["input"], &text["input"]["price"]),
        output_price: price(&m["cost"]["output"], &text["output"]["price"]),
        cache_read_price: price(&m["cost"]["cache_read"], &text["input"]["cache"]["read"]),
        cache_write_price: m["cost"]["cache_write"].as_f64().or(cache_write),
        released: m["release_date"].as_str().map(str::to_string),
        deprecated: m["status"].as_str() == Some("deprecated"),
        id,
    })
}

/// The catalogue's name for a provider devd knows by another.
///
/// devd's preset ids are its own; the catalogue's are models.dev's. A provider
/// a person made themselves is recognised by the host it points at.
pub fn key_for(provider: &Provider) -> Option<&'static str> {
    const BY_ID: &[(&str, &str)] = &[
        ("anthropic", "anthropic"),
        ("openai", "openai"),
        ("deepseek", "deepseek"),
        ("moonshot", "moonshotai"),
        ("gemini", "google"),
        ("xai", "xai"),
        ("mistral", "mistral"),
        ("groq", "groq"),
        ("together", "togetherai"),
        ("fireworks", "fireworks-ai"),
        ("openrouter", "openrouter"),
        ("cerebras", "cerebras"),
        ("perplexity", "perplexity"),
        ("zai", "zai"),
        ("deepinfra", "deepinfra"),
        ("nvidia", "nvidia"),
        ("huggingface", "huggingface"),
        ("cohere", "cohere"),
        ("sambanova", "sambanova"),
        ("ollama-cloud", "ollama-cloud"),
    ];
    const BY_HOST: &[(&str, &str)] = &[
        ("api.anthropic.com", "anthropic"),
        ("api.openai.com", "openai"),
        ("api.deepseek.com", "deepseek"),
        ("api.moonshot.ai", "moonshotai"),
        ("generativelanguage.googleapis.com", "google"),
        ("api.x.ai", "xai"),
        ("api.mistral.ai", "mistral"),
        ("api.groq.com", "groq"),
        ("api.together.xyz", "togetherai"),
        ("api.fireworks.ai", "fireworks-ai"),
        ("openrouter.ai", "openrouter"),
        ("api.cerebras.ai", "cerebras"),
        ("api.perplexity.ai", "perplexity"),
        ("api.z.ai", "zai"),
        ("api.deepinfra.com", "deepinfra"),
        ("integrate.api.nvidia.com", "nvidia"),
        ("router.huggingface.co", "huggingface"),
        ("api.cohere.ai", "cohere"),
        ("api.cohere.com", "cohere"),
        ("api.sambanova.ai", "sambanova"),
        ("ollama.com", "ollama-cloud"),
    ];
    if let Some((_, key)) = BY_ID.iter().find(|(id, _)| *id == provider.id) {
        return Some(key);
    }
    let host = provider.base_url.split("://").nth(1).unwrap_or(&provider.base_url).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    BY_HOST.iter().find(|(known, _)| *known == host).map(|(_, key)| *key)
}

/// A model id without what varies between two names for the same model: the
/// reseller's `vendor/` prefix, a `:tag`, a trailing snapshot date, `-latest`.
fn canonical(id: &str) -> String {
    let id = id.to_ascii_lowercase();
    let id = id.rsplit('/').next().unwrap_or(&id);
    let id = id.split(':').next().unwrap_or(id);
    let id = id.strip_suffix("-latest").unwrap_or(id);
    // `-20250929` and `-2025-09-29`.
    let bytes = id.as_bytes();
    let dated = |len: usize, dashes: &[usize]| {
        bytes.len() > len && {
            let tail = &bytes[bytes.len() - len..];
            tail[0] == b'-' && tail.iter().enumerate().skip(1).all(|(i, b)| if dashes.contains(&i) { *b == b'-' } else { b.is_ascii_digit() })
        }
    };
    if dated(11, &[5, 8]) {
        id[..id.len() - 11].to_string()
    } else if dated(9, &[]) {
        id[..id.len() - 9].to_string()
    } else {
        id.to_string()
    }
}

/// The vendors that make models, in the order their own list is trusted for a
/// model served by somebody the catalogue does not know.
const MAKERS: &[&str] = &["anthropic", "openai", "google", "deepseek", "xai", "mistral", "moonshotai", "zai", "cohere"];

impl Catalogue {
    /// A provider's entry, by the catalogue's key.
    pub fn provider(&self, key: &str) -> Option<&Entry> {
        self.providers.get(key)
    }

    /// The release date of the newest model it lists: how current it is.
    pub fn as_of(&self) -> Option<&str> {
        self.providers.values().flat_map(|e| e.models.iter()).filter_map(|m| m.released.as_deref()).max()
    }

    /// How many providers and models it holds.
    pub fn size(&self) -> (usize, usize) {
        (self.providers.len(), self.providers.values().map(|e| e.models.len()).sum())
    }

    /// What the catalogue says about `model` as `provider` serves it.
    ///
    /// The provider's own entry is searched first, by exact id and then by the
    /// id without its snapshot date. A provider the catalogue does not list is
    /// answered from the vendor that makes the model.
    pub fn find(&self, provider: &Provider, model: &str) -> Option<&Facts> {
        let within = |entry: &'_ Entry| -> Option<usize> {
            entry.models.iter().position(|m| m.id == model).or_else(|| {
                let want = canonical(model);
                entry.models.iter().position(|m| canonical(&m.id) == want)
            })
        };
        if let Some(entry) = key_for(provider).and_then(|key| self.providers.get(key)) {
            if let Some(at) = within(entry) {
                return entry.models.get(at);
            }
        }
        MAKERS.iter().filter_map(|key| self.providers.get(*key)).find_map(|entry| within(entry).and_then(|at| entry.models.get(at)))
    }

    /// A provider's models as catalogue rows, for showing before a key exists
    /// and when the provider's own list cannot be reached.
    pub fn rows_for(&self, provider: &Provider, now: &str) -> Vec<CatalogueModel> {
        let Some(entry) = key_for(provider).and_then(|key| self.providers.get(key)) else { return Vec::new() };
        // Deprecated models are left out of a picker, unless they are all the
        // vendor has: a deprecated model still answers until it is retired.
        let current: Vec<CatalogueModel> = entry.models.iter().filter(|m| !m.deprecated).map(|m| row(provider, m, now)).collect();
        if current.is_empty() { entry.models.iter().map(|m| row(provider, m, now)).collect() } else { current }
    }
}

fn row(provider: &Provider, m: &Facts, now: &str) -> CatalogueModel {
    CatalogueModel {
        source: format!("models.dev:{}", provider.id),
        model_id: m.id.clone(),
        display: m.name.clone(),
        description: m.deprecated.then(|| "The vendor has marked this model deprecated.".to_string()),
        context_window: m.context,
        max_output: m.output,
        input_price: m.input_price,
        output_price: m.output_price,
        retirement: None,
        url: Some("https://models.dev".to_string()),
        fetched_at: now.to_string(),
        tools: m.tools,
        vision: m.vision,
        reasoning: m.reasoning,
    }
}

/// Fill what a provider's own list left unsaid from the catalogue.
///
/// The provider's word stands wherever it gave one. The catalogue supplies the
/// rest, which for most vendors is every price and limit.
pub fn enrich(catalogue: &Catalogue, provider: &Provider, models: &mut [CatalogueModel]) {
    for model in models {
        let Some(known) = catalogue.find(provider, &model.model_id) else { continue };
        model.context_window = model.context_window.or(known.context);
        model.max_output = model.max_output.or(known.output);
        model.input_price = model.input_price.or(known.input_price);
        model.output_price = model.output_price.or(known.output_price);
        model.tools = model.tools.or(known.tools);
        model.vision = model.vision.or(known.vision);
        model.reasoning = model.reasoning.or(known.reasoning);
        if model.display == model.model_id {
            model.display = known.name.clone();
        }
    }
}

fn bundled() -> Arc<Catalogue> {
    static BUNDLED_CATALOGUE: OnceLock<Arc<Catalogue>> = OnceLock::new();
    BUNDLED_CATALOGUE
        .get_or_init(|| {
            let mut catalogue = parse(BUNDLED).unwrap_or_default();
            catalogue.origin = "bundled";
            Arc::new(catalogue)
        })
        .clone()
}

fn slot() -> &'static RwLock<Option<Arc<Catalogue>>> {
    static CURRENT: OnceLock<RwLock<Option<Arc<Catalogue>>>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new(None))
}

fn cache_file(paths: &Paths) -> PathBuf {
    paths.root().join("cache").join("models-dev.json")
}

/// Use the cached copy in devd's data directory, if there is a readable one.
/// Called once when the agent starts; without it the compiled copy is used.
pub fn load(paths: &Paths) {
    let Some(cached) = read_cache(&cache_file(paths)) else { return };
    if let Ok(mut current) = slot().write() {
        *current = newer_than_bundled(cached).map(Arc::new);
    }
}

/// A fetched catalogue, unless the one compiled in knows a newer model.
fn newer_than_bundled(fetched: Catalogue) -> Option<Catalogue> {
    (fetched.as_of() >= bundled().as_of()).then_some(fetched)
}

fn read_cache(path: &Path) -> Option<Catalogue> {
    parse(&std::fs::read_to_string(path).ok()?).ok()
}

/// The catalogue in use: the cached copy when one was loaded or fetched, the
/// compiled one otherwise.
pub fn current() -> Arc<Catalogue> {
    slot().read().ok().and_then(|current| current.clone()).unwrap_or_else(bundled)
}

/// What a refresh found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refreshed {
    /// Providers in the fetched catalogue.
    pub providers: usize,
    /// Models in it.
    pub models: usize,
    /// Whether it differs from what was in use.
    pub changed: bool,
}

/// Fetch the catalogue, check it reads, and start using it.
///
/// A catalogue that does not parse, or that has lost most of its providers, is
/// refused and the one in use stays: a bad download must not empty every model
/// picker.
pub fn refresh(paths: &Paths) -> Result<Refreshed> {
    let text = crate::providers::fetch_text(SOURCE_URL)?;
    install(paths, &text)
}

fn install(paths: &Paths, text: &str) -> Result<Refreshed> {
    let fetched = parse(text)?;
    let before = current();
    let (providers, models) = fetched.size();
    if providers * 2 < bundled().size().0 {
        return Err(DevdError::invalid(format!(
            "the fetched catalogue lists only {providers} provider(s), far fewer than the one devd ships with; keeping the current one"
        )));
    }
    let changed = fetched.providers != before.providers;
    let file = cache_file(paths);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside and renamed, so a reader never sees half a file.
    let partial = file.with_extension("json.part");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, &file)?;
    if let Ok(mut slot) = slot().write() {
        *slot = newer_than_bundled(fetched).map(Arc::new);
    }
    Ok(Refreshed { providers, models, changed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, base: &str) -> Provider {
        Provider {
            id: id.into(),
            kind: "openai-compatible".into(),
            label: id.into(),
            base_url: base.into(),
            key_env: None,
            default_model: None,
            is_default: false,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn the_compiled_catalogue_reads_and_covers_the_presets() {
        let catalogue = bundled();
        assert_eq!(catalogue.origin, "bundled");
        let (providers, models) = catalogue.size();
        assert!(providers >= 15 && models >= 300, "{providers} providers, {models} models");
        for preset in crate::providers::presets() {
            let local = matches!(preset.id.as_str(), "ollama" | "lm-studio" | "custom");
            let p = provider(&preset.id, &preset.base_url);
            assert_eq!(key_for(&p).and_then(|key| catalogue.provider(key)).is_some(), !local, "{}", preset.id);
        }
    }

    #[test]
    fn a_model_has_its_price_including_what_the_cache_costs() {
        let catalogue = bundled();
        let claude = catalogue.find(&provider("anthropic", "https://api.anthropic.com"), "claude-sonnet-5-5").expect("listed");
        assert_eq!(claude.tools, Some(true));
        assert_eq!(claude.vision, Some(true));
        assert!(claude.context.unwrap() >= 200_000);
        let (input, read, write) = (claude.input_price.unwrap(), claude.cache_read_price.unwrap(), claude.cache_write_price.unwrap());
        assert!(read < input && write > input, "a cache read is cheaper than input and a write dearer: {input} {read} {write}");
        // The model this machine's conversations ran on had no price at all before.
        let deepseek = catalogue.find(&provider("deepseek", "https://api.deepseek.com/v1"), "deepseek-flash").expect("listed");
        assert!(deepseek.input_price.is_some() && deepseek.output_price.is_some());
    }

    #[test]
    fn a_snapshot_date_or_a_reseller_prefix_does_not_hide_a_model() {
        assert_eq!(canonical("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(canonical("anthropic/claude-sonnet-4-5:beta"), "claude-sonnet-4-5");
        assert_eq!(canonical("gpt-4o-latest"), "gpt-4o");
        assert_eq!(canonical("gpt-4-1106"), "gpt-4-1106", "four digits are a version, not a date");
        let catalogue = bundled();
        let anthropic = provider("anthropic", "https://api.anthropic.com");
        assert!(catalogue.find(&anthropic, "claude-haiku-4-5-20251001").is_some());
        assert!(catalogue.find(&anthropic, "no-such-model").is_none());
    }

    #[test]
    fn a_private_endpoint_is_answered_from_the_vendor_that_makes_the_model() {
        let catalogue = bundled();
        let gateway = provider("work-gateway", "https://llm.internal.example/v1");
        assert_eq!(key_for(&gateway), None);
        let facts = catalogue.find(&gateway, "claude-opus-5-5").expect("Anthropic makes it");
        assert_eq!(facts.tools, Some(true));
        // A provider made by hand but pointing at a vendor is recognised by its host.
        assert_eq!(key_for(&provider("my-groq", "https://api.groq.com/openai/v1")), Some("groq"));
    }

    #[test]
    fn both_published_shapes_are_read() {
        let flat = r#"{"acme":{"name":"Acme","env":["ACME_KEY"],"models":{
            "a-1":{"id":"a-1","name":"A One","tool_call":true,"reasoning":false,"release_date":"2026-01-02",
                   "modalities":{"input":["text","image"],"output":["text"]},
                   "cost":{"input":3,"output":15,"cache_read":0.3,"cache_write":3.75},"limit":{"context":200000,"output":8192}},
            "a-embed":{"id":"a-embed","name":"Embed","modalities":{"input":["text"],"output":["embedding"]}}}}}"#;
        let nested = r#"{"acme":{"name":"Acme","env":["ACME_KEY"],"models":{
            "a-1":{"id":"a-1","type":"chat","name":"A One","release_date":"2026-01-02",
                   "modalities":{"input":["text","image/png"],"output":["text"]},
                   "capabilities":{"tools":{"supported":true},"reasoning":{"supported":false}},
                   "cost":{"text":{"input":{"price":3,"cache":{"read":0.3,"write":{"5m":3.75,"1h":6}}},"output":{"price":15}}},
                   "limit":{"context":200000,"output":8192}},
            "a-image":{"id":"a-image","type":"image","name":"Image"}}}}"#;
        let (a, b) = (parse(flat).unwrap(), parse(nested).unwrap());
        assert_eq!(a.provider("acme"), b.provider("acme"));
        let model = &a.provider("acme").unwrap().models[0];
        assert_eq!(a.provider("acme").unwrap().models.len(), 1, "only the chat model is kept");
        assert_eq!((model.vision, model.tools, model.reasoning), (Some(true), Some(true), Some(false)));
        assert_eq!((model.input_price, model.cache_read_price, model.cache_write_price), (Some(3.0), Some(0.3), Some(3.75)));
    }

    #[test]
    fn the_provider_s_own_word_stands_and_the_catalogue_fills_the_rest() {
        let catalogue = bundled();
        let p = provider("deepseek", "https://api.deepseek.com/v1");
        let mut listed = vec![CatalogueModel {
            source: "provider:deepseek".into(),
            model_id: "deepseek-flash".into(),
            display: "deepseek-flash".into(),
            description: None,
            context_window: Some(42),
            max_output: None,
            input_price: None,
            output_price: None,
            retirement: None,
            url: None,
            fetched_at: String::new(),
            tools: None,
            vision: None,
            reasoning: None,
        }];
        enrich(&catalogue, &p, &mut listed);
        assert_eq!(listed[0].context_window, Some(42), "what the provider said is kept");
        assert!(listed[0].input_price.is_some() && listed[0].tools.is_some(), "what it did not say is filled in");
        assert_ne!(listed[0].display, "deepseek-flash", "and it gets its proper name");
    }

    #[test]
    fn an_old_cache_does_not_hide_what_a_newer_devd_knows() {
        let newest = bundled().as_of().expect("the compiled catalogue has dated models").to_string();
        // The same catalogue with every model a year older, as a cache from an earlier devd would be.
        let year: i32 = newest[..4].parse().unwrap();
        let stale = parse(&BUNDLED.replace(&format!("\"{year}-"), &format!("\"{}-", year - 1)).replace(&format!("\"{}-", year + 1), &format!("\"{year}-"))).unwrap();
        assert!(stale.as_of().unwrap() < newest.as_str());
        assert!(newer_than_bundled(stale).is_none(), "the compiled copy stays in use");
        assert!(newer_than_bundled(parse(BUNDLED).unwrap()).is_some(), "an equally current fetch is used");
    }

    #[test]
    fn a_broken_or_gutted_download_is_refused_and_a_good_one_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path());
        assert!(install(&paths, "<html>rate limited</html>").is_err());
        assert!(install(&paths, r#"{"acme":{"models":{"a":{"id":"a","name":"A"}}}}"#).is_err(), "one provider is not the catalogue");
        assert!(!cache_file(&paths).exists(), "nothing bad was written");
        let good = install(&paths, BUNDLED).unwrap();
        assert!(good.models >= 300);
        assert!(read_cache(&cache_file(&paths)).is_some());
    }
}
