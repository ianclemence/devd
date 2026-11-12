//! Tests that tie the window's literals to the things they name.
//!
//! Two classes of mistake are cheap to make and expensive to find by clicking:
//! an icon the assets do not have (it draws nothing, silently), and a capability
//! or argument key that the registry does not offer (it fails only when the
//! button is pressed). These tests read the window's own source and compare it
//! with the compiled asset table and the real registry.

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use devd_core::paths::Paths;
use serde_json::Value;

/// Every Rust file under `src`, because a name can be used from any screen.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                out.push((path, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

/// The index of the bracket that closes the one at `open`, respecting strings.
fn matching(text: &str, open: usize, opening: char, closing: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in text[open..].char_indices() {
        let at = open + index;
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            c if c == opening => depth += 1,
            c if c == closing => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// The strings a call passes, in order, ignoring ones nested deeper.
fn top_level_strings(body: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < body.len() {
        let ch = body[i..].chars().next().unwrap();
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                i += ch.len_utf8();
            }
            ')' | ']' | '}' => {
                depth -= 1;
                i += ch.len_utf8();
            }
            '"' if depth == 0 => {
                let start = i;
                let mut j = i + 1;
                let mut escaped = false;
                while j < body.len() {
                    let c = body[j..].chars().next().unwrap();
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                    j += c.len_utf8();
                }
                out.push((body[start + 1..j].to_string(), start, j + 1));
                i = j + 1;
            }
            _ => i += ch.len_utf8(),
        }
    }
    out
}

/// The top-level keys of the object literal inside `json!({ … })` in `body`.
fn json_keys(body: &str) -> Vec<String> {
    let Some(at) = body.find("json!") else { return Vec::new() };
    let Some(paren) = body[at..].find('(').map(|n| at + n) else { return Vec::new() };
    let Some(close) = matching(body, paren, '(', ')') else { return Vec::new() };
    let inner = &body[paren + 1..close];
    let Some(brace) = inner.find('{') else { return Vec::new() };
    let Some(end) = matching(inner, brace, '{', '}') else { return Vec::new() };
    let object = &inner[brace + 1..end];
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < object.len() {
        let ch = object[i..].chars().next().unwrap();
        match ch {
            '"' => {
                let start = i + 1;
                let mut j = start;
                let mut escaped = false;
                while j < object.len() {
                    let c = object[j..].chars().next().unwrap();
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                    j += c.len_utf8();
                }
                if j < object.len() && depth == 0 && object[j + 1..].trim_start().starts_with(':') {
                    keys.push(object[start..j].to_string());
                }
                i = j + 1;
            }
            '{' | '[' => {
                depth += 1;
                i += ch.len_utf8();
            }
            '}' | ']' => {
                depth -= 1;
                i += ch.len_utf8();
            }
            _ => i += ch.len_utf8(),
        }
    }
    keys
}

/// The capability literal in a call, and the keys its argument object names.
fn calls(text: &str, needle: &str, cap_index: usize) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(at) = text[from..].find(needle) {
        let start = from + at;
        let Some(paren) = text[start..].find('(').map(|n| start + n) else { break };
        let Some(close) = matching(text, paren, '(', ')') else { break };
        let body = &text[paren + 1..close];
        let strings = top_level_strings(body);
        if let Some((cap, _, _)) = strings.get(cap_index) {
            out.push((cap.clone(), json_keys(body)));
        }
        from = close + 1;
    }
    out
}

/// The capabilities and argument keys a `FormSpec` names.
fn forms(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(at) = text[from..].find("FormSpec {") {
        let start = from + at;
        let Some(end) = matching(text, start + "FormSpec ".len(), '{', '}') else { break };
        let body = &text[start..=end];
        let cap = body.find("capability:").and_then(|at| top_level_strings(&body[at..]).into_iter().next()).map(|(s, _, _)| s);
        if let Some(cap) = cap {
            let mut keys = json_keys(body);
            for field in ["Field::text(", "Field::optional(", "Field::number("] {
                let mut at = 0usize;
                while let Some(found) = body[at..].find(field) {
                    let after = at + found + field.len();
                    if let Some((key, _, _)) = top_level_strings(&body[after..]).into_iter().next() {
                        keys.push(key);
                    }
                    at = after;
                }
            }
            out.push((cap, keys));
        }
        from = end + 1;
    }
    out
}

/// A registry with every built-in capability, in a scratch data home.
fn registry() -> (devd_bootstrap::Bootstrap, PathBuf) {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("devd-app-truth-{}-{nanos}", std::process::id()));
    let bootstrap = devd_bootstrap::Bootstrap::at(Paths::rooted(&dir)).expect("the registry assembles");
    (bootstrap, dir)
}

#[test]
fn every_icon_named_in_the_window_exists_in_the_asset_table() {
    let mut missing = Vec::new();
    for (path, text) in sources() {
        for pattern in ["icon(\"", ".icon(\"", "ui::empty(\""] {
            let mut from = 0usize;
            while let Some(at) = text[from..].find(pattern) {
                let start = from + at + pattern.len();
                let Some(end) = text[start..].find('"').map(|n| start + n) else { break };
                let name = &text[start..end];
                if !name.is_empty() && !crate::assets::has_icon(name) {
                    missing.push(format!("{}: {name}", path.display()));
                }
                from = end + 1;
            }
        }
    }
    assert!(missing.is_empty(), "icons drawn but not compiled in: {missing:#?}");
}

#[test]
fn every_capability_and_argument_named_by_a_screen_really_exists() {
    let (bootstrap, dir) = registry();
    let schemas: BTreeMap<String, Value> = bootstrap.registry.list().iter().map(|cap| (cap.id.clone(), cap.args_schema.clone())).collect();

    let mut bad_caps = Vec::new();
    let mut bad_keys = Vec::new();
    // The server describes itself at this address; it is not a capability.
    let known = |id: &str| id == "api.capabilities" || schemas.contains_key(id);

    for (path, full) in sources() {
        // Unit tests name made-up capabilities and fields on purpose; only the
        // code that ships is checked against the real registry.
        let text = full.split("#[cfg(test)]").next().unwrap_or(&full).to_string();
        let mut pairs: Vec<(String, Vec<String>)> = Vec::new();
        for (needle, index) in [
            ("Act::new(", 1),
            ("Mutation::new(", 0),
            ("QueryKey::new(", 0),
            ("read_with(cx, ", 0),
            ("read(cx, ", 0),
        ] {
            for (cap, keys) in calls(&text, needle, index) {
                pairs.push((cap.to_string(), keys));
            }
        }
        pairs.extend(forms(&text));

        for (cap, keys) in pairs {
            if !cap.contains('.') {
                continue;
            }
            if !known(&cap) {
                bad_caps.push(format!("{}: {cap}", path.display()));
                continue;
            }
            let Some(schema) = schemas.get(&cap) else { continue };
            let Some(properties) = schema["properties"].as_object() else { continue };
            for key in keys {
                if !properties.contains_key(&key) {
                    bad_keys.push(format!("{}: {cap} has no argument `{key}`", path.display()));
                }
            }
        }
    }

    assert!(bad_caps.is_empty(), "capabilities a screen names but devd does not have: {bad_caps:#?}");
    assert!(bad_keys.is_empty(), "arguments a screen sends but a capability does not accept: {bad_keys:#?}");
    let _ = std::fs::remove_dir_all(dir);
}
