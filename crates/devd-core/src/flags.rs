//! Feature flags: switches for staged rollouts.
//!
//! A new behaviour that touches every agent (a gate on finishing, a check before data
//! leaves) should be able to ship switched on for the person testing it, and be switched
//! off by one setting if it misbehaves, without a new build. Each flag is declared once,
//! here, with a type and a default, so there is one list of what can be switched.
//!
//! A flag's value comes from, in order:
//!
//! 1. the environment, `DEVD_FEATURE_<KEY>`, for one run (a CI job, a bug report);
//! 2. the setting saved with `flags.set`, for this machine;
//! 3. the default in this file.
//!
//! A value that does not read (an environment variable holding "maybe") is ignored and the
//! next source is used, and the answer says it was ignored: a typo must not silently turn a
//! safety check off. A key that is not declared here cannot be set at all.

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{DevdError, Result};
use crate::state::Store;

/// What a flag holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// On or off.
    Bool(bool),
    /// A number within bounds.
    Int {
        /// Used when nothing says otherwise.
        default: i64,
        /// Smallest allowed.
        min: i64,
        /// Largest allowed.
        max: i64,
    },
}

/// One declared flag.
#[derive(Debug, Clone, Copy)]
pub struct Flag {
    /// Dotted lowercase key: `agent.end_gate`.
    pub key: &'static str,
    /// What it does, in a sentence.
    pub summary: &'static str,
    /// Its type and default.
    pub kind: Kind,
}

/// Every flag.
pub const ALL: &[Flag] = &[
    Flag { key: "agent.end_gate", summary: "Before the assistant ends a turn, check its plan and proof and send it back to finish", kind: Kind::Bool(true) },
    // One clear reminder, then the person decides. Sending a model back again and again to "prove" it is done is
    // what turned a stuck task into minutes of thinking and a wrong answer; the record says what is left either way.
    Flag { key: "agent.stop_budget", summary: "How many times one prompt may be sent back to finish (0 turns the gate off)", kind: Kind::Int { default: 1, min: 0, max: 10 } },
    Flag { key: "agent.draw", summary: "Let the assistant draw tables, charts and buttons in the window instead of describing them", kind: Kind::Bool(true) },
    Flag { key: "agent.draw_fixes", summary: "How many times a drawing that does not check is sent back to be written again (0 never sends it back)", kind: Kind::Int { default: 2, min: 0, max: 4 } },
    Flag { key: "ui.site_icons", summary: "Show a site's own icon before each web link in an answer, fetched from the site when the link is on screen", kind: Kind::Bool(true) },
    Flag { key: "agent.flow_guard", summary: "Hold anything that would send private data to a wider audience until a person approves it", kind: Kind::Bool(true) },
    Flag { key: "audit.chain", summary: "Show whether the audit record's hash chain still holds", kind: Kind::Bool(true) },
];

/// The flag with this key.
pub fn find(key: &str) -> Option<&'static Flag> {
    ALL.iter().find(|f| f.key == key)
}

/// The environment variable that overrides a flag: `agent.end_gate` is `DEVD_FEATURE_AGENT_END_GATE`.
pub fn env_name(key: &str) -> String {
    let mut out = String::from("DEVD_FEATURE_");
    let mut gap = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_uppercase());
            gap = false;
        } else {
            gap = true;
        }
    }
    out
}

/// Where a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The default in this file.
    Default,
    /// The setting saved on this machine.
    Setting,
    /// The environment of this process.
    Environment,
}

/// A flag's value and what it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    /// The key.
    pub key: &'static str,
    /// What it does.
    pub summary: &'static str,
    /// The value in force.
    pub value: Value,
    /// Where it came from.
    pub source: Source,
    /// The default, so a change can be undone by eye.
    pub default: Value,
    /// A value that was set but did not read, and was passed over.
    pub ignored: Vec<String>,
    /// The environment variable that overrides it.
    pub env: String,
}

fn read(kind: Kind, text: &str) -> Option<Value> {
    let t = text.trim();
    match kind {
        Kind::Bool(_) => match t.to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" => Some(json!(true)),
            "0" | "false" | "off" | "no" => Some(json!(false)),
            _ => None,
        },
        Kind::Int { min, max, .. } => t.parse::<i64>().ok().filter(|n| (min..=max).contains(n)).map(|n| json!(n)),
    }
}

fn default_of(kind: Kind) -> Value {
    match kind {
        Kind::Bool(d) => json!(d),
        Kind::Int { default, .. } => json!(default),
    }
}

/// A flag's value given what the environment and the saved setting say. Pure.
pub fn resolve(flag: &'static Flag, env: Option<&str>, setting: Option<&str>) -> Resolved {
    let mut ignored = Vec::new();
    let mut pick = |name: &str, raw: Option<&str>, source: Source| -> Option<(Value, Source)> {
        let raw = raw?;
        match read(flag.kind, raw) {
            Some(v) => Some((v, source)),
            None => {
                ignored.push(format!("{name} is `{raw}`, which is not a valid {}; it was ignored", if matches!(flag.kind, Kind::Bool(_)) { "on/off value".to_string() } else { "number in range".to_string() }));
                None
            }
        }
    };
    let envname = env_name(flag.key);
    let chosen = pick(&envname, env, Source::Environment).or_else(|| pick("the saved setting", setting, Source::Setting));
    let (value, source) = chosen.unwrap_or((default_of(flag.kind), Source::Default));
    Resolved { key: flag.key, summary: flag.summary, value, source, default: default_of(flag.kind), ignored, env: envname }
}

fn setting_key(key: &str) -> String {
    format!("flag.{key}")
}

/// A flag as it stands now.
pub fn get(store: &Store, key: &str) -> Result<Resolved> {
    let flag = find(key).ok_or_else(|| DevdError::not_found(format!("there is no flag `{key}`")))?;
    // A cleared flag is stored empty: nothing was set, so nothing is passed over.
    let setting = store.get_setting(&setting_key(key))?.filter(|s| !s.trim().is_empty());
    Ok(resolve(flag, std::env::var(env_name(key)).ok().as_deref(), setting.as_deref()))
}

/// Whether a bool flag is on. A flag that cannot be read is its default: a broken settings table must not switch safety off.
pub fn on(store: &Store, key: &str) -> bool {
    match get(store, key) {
        Ok(r) => r.value.as_bool().unwrap_or(true),
        Err(_) => find(key).map(|f| matches!(f.kind, Kind::Bool(true))).unwrap_or(false),
    }
}

/// An integer flag's value, or its default.
pub fn number(store: &Store, key: &str) -> i64 {
    get(store, key).ok().and_then(|r| r.value.as_i64()).or_else(|| find(key).and_then(|f| if let Kind::Int { default, .. } = f.kind { Some(default) } else { None })).unwrap_or(0)
}

/// Every flag.
pub fn all(store: &Store) -> Vec<Resolved> {
    ALL.iter().filter_map(|f| get(store, f.key).ok()).collect()
}

/// Save a flag, or clear it with `None` so the default applies again.
pub fn set(store: &Store, key: &str, value: Option<&str>) -> Result<Resolved> {
    let flag = find(key).ok_or_else(|| DevdError::not_found(format!("there is no flag `{key}`; `flags.list` shows them")))?;
    match value {
        Some(v) => {
            let parsed = read(flag.kind, v).ok_or_else(|| DevdError::invalid(format!("`{v}` is not a valid value for {key}")))?;
            store.set_setting(&setting_key(key), parsed.to_string().trim_matches('"'))?;
        }
        None => store.set_setting(&setting_key(key), "")?,
    }
    get(store, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(key: &str) -> &'static Flag {
        find(key).unwrap()
    }

    #[test]
    fn the_environment_beats_the_setting_beats_the_default() {
        let f = flag("agent.end_gate");
        assert_eq!((resolve(f, None, None).value, resolve(f, None, None).source), (json!(true), Source::Default));
        let s = resolve(f, None, Some("off"));
        assert_eq!((s.value, s.source), (json!(false), Source::Setting));
        let e = resolve(f, Some("on"), Some("off"));
        assert_eq!((e.value, e.source), (json!(true), Source::Environment));
    }

    #[test]
    fn a_value_that_does_not_read_is_passed_over_and_said_so() {
        let f = flag("agent.end_gate");
        let r = resolve(f, Some("maybe"), Some("off"));
        assert_eq!((r.value, r.source), (json!(false), Source::Setting), "the next source is used");
        assert!(r.ignored[0].contains("DEVD_FEATURE_AGENT_END_GATE") && r.ignored[0].contains("maybe"), "{:?}", r.ignored);
        let both = resolve(f, Some("x"), Some("y"));
        assert_eq!((both.value, both.source, both.ignored.len()), (json!(true), Source::Default, 2), "a typo cannot turn a check off");
    }

    #[test]
    fn a_number_must_be_in_range() {
        let f = flag("agent.stop_budget");
        assert_eq!(resolve(f, Some("5"), None).value, json!(5));
        assert_eq!(resolve(f, Some("0"), None).value, json!(0));
        assert_eq!(resolve(f, Some("11"), None).value, json!(1), "out of range falls back to the default");
        assert_eq!(resolve(f, Some("-1"), None).source, Source::Default);
    }

    #[test]
    fn environment_names_are_unambiguous() {
        assert_eq!(env_name("agent.end_gate"), "DEVD_FEATURE_AGENT_END_GATE");
        assert_eq!(env_name("a-b.c"), "DEVD_FEATURE_A_B_C");
        let mut names: Vec<String> = ALL.iter().map(|f| env_name(f.key)).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "two flags share an environment variable");
    }

    #[test]
    fn every_flag_is_declared_with_a_lowercase_dotted_key_a_sentence_and_a_sane_default() {
        for f in ALL {
            assert!(f.key.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_') && f.key.contains('.'), "{}", f.key);
            assert!(f.summary.len() > 20 && !f.summary.ends_with('.'), "{}", f.key);
            if let Kind::Int { default, min, max } = f.kind {
                assert!(min <= default && default <= max, "{}", f.key);
            }
        }
    }

    #[test]
    fn flags_are_saved_cleared_and_checked_against_the_store() {
        let s = Store::open_in_memory().unwrap();
        assert!(on(&s, "agent.end_gate"));
        let r = set(&s, "agent.end_gate", Some("off")).unwrap();
        assert_eq!((r.value, r.source), (json!(false), Source::Setting));
        assert!(!on(&s, "agent.end_gate"));
        let cleared = set(&s, "agent.end_gate", None).unwrap();
        assert_eq!(cleared.source, Source::Default);
        assert!(set(&s, "agent.end_gate", Some("maybe")).is_err());
        assert!(set(&s, "nope.nothing", Some("on")).is_err());
        set(&s, "agent.stop_budget", Some("7")).unwrap();
        assert_eq!(number(&s, "agent.stop_budget"), 7);
        assert_eq!(all(&s).len(), ALL.len());
    }
}
