//! Central JSON Schema validation for capability arguments.
//!
//! Typed capabilities already deserialize their arguments, but many handlers
//! take a raw [`serde_json::Value`](serde_json::Value). Validating only inside a
//! handler means a malformed call can reach a handler that forgets to check, and
//! it means an alias, an MCP call or a scheduled invocation can take a different
//! path to the same operation. This module is the one boundary every caller
//! crosses, so "the arguments were checked" is a property of dispatch rather
//! than a habit of each capability.
//!
//! The implementation covers the keywords `schemars` emits for devd's argument
//! structs plus the useful validation subset: `$ref`, `type`, `properties`,
//! `required`, `additionalProperties`, `items`, `enum`, `const`, `anyOf`,
//! `oneOf`, `allOf`, `not`, and the string/number bounds. Unknown keywords are
//! ignored deliberately: a schema that says more than this validator understands
//! must not make every call fail.

use serde_json::Value;

use crate::error::{DevdError, Result};

/// Maximum nesting accepted while walking a schema. A self-referential schema
/// would otherwise recurse forever.
const MAX_DEPTH: usize = 64;

/// Validate `args` against a capability's argument schema.
///
/// Returns a structured [`DevdError::Invalid`] naming each failing path, so the
/// caller (human or model) can correct the call without guessing.
pub fn validate(schema: &Value, args: &Value) -> Result<()> {
    let mut errors = Vec::new();
    check(schema, schema, args, "$", 0, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DevdError::invalid(format!(
            "arguments do not match the capability schema: {}",
            errors.join("; ")
        )))
    }
}

fn check(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    errors: &mut Vec<String>,
) {
    if depth > MAX_DEPTH {
        errors.push(format!("{path}: schema nesting is too deep"));
        return;
    }
    let Some(obj) = schema.as_object() else {
        return;
    };

    // Local references are how schemars represents a nested struct.
    if let Some(reference) = obj.get("$ref").and_then(Value::as_str) {
        if let Some(target) = resolve(root, reference) {
            check(root, target, value, path, depth + 1, errors);
        }
        return;
    }

    if let Some(all) = obj.get("allOf").and_then(Value::as_array) {
        for sub in all {
            check(root, sub, value, path, depth + 1, errors);
        }
    }
    if let Some(any) = obj.get("anyOf").and_then(Value::as_array) {
        if !any
            .iter()
            .any(|sub| branch_ok(root, sub, value, path, depth + 1))
        {
            errors.push(format!("{path}: does not match any allowed shape"));
        }
    }
    if let Some(one) = obj.get("oneOf").and_then(Value::as_array) {
        let matches = one
            .iter()
            .filter(|sub| branch_ok(root, sub, value, path, depth + 1))
            .count();
        if matches != 1 {
            errors.push(format!(
                "{path}: must match exactly one allowed shape ({matches} matched)"
            ));
        }
    }
    if let Some(not) = obj.get("not") {
        if branch_ok(root, not, value, path, depth + 1) {
            errors.push(format!("{path}: matches a shape it must not match"));
        }
    }

    if let Some(choices) = obj.get("enum").and_then(Value::as_array) {
        if !choices.iter().any(|c| c == value) {
            errors.push(format!(
                "{path}: must be one of {}",
                short(&Value::Array(choices.clone()))
            ));
        }
    }
    if let Some(expected) = obj.get("const") {
        if expected != value {
            errors.push(format!("{path}: must be {}", short(expected)));
        }
    }

    if let Some(kind) = obj.get("type") {
        let types: Vec<&str> = match kind {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !types.is_empty() && !types.iter().any(|t| has_type(value, t)) {
            errors.push(format!(
                "{path}: expected {}, got {}",
                types.join(" or "),
                kind_of(value)
            ));
            return;
        }
    }

    match value {
        Value::Object(map) => {
            if let Some(required) = obj.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if !map.contains_key(key) {
                        errors.push(format!("{path}.{key}: is required"));
                    }
                }
            }
            if let Some(props) = obj.get("properties").and_then(Value::as_object) {
                for (key, sub) in props {
                    if let Some(v) = map.get(key) {
                        check(root, sub, v, &format!("{path}.{key}"), depth + 1, errors);
                    }
                }
            }
            if let Some(extra) = obj.get("additionalProperties") {
                match extra {
                    Value::Bool(false) => {
                        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
                            for key in map.keys() {
                                if !props.contains_key(key) {
                                    errors.push(format!("{path}.{key}: is not a known argument"));
                                }
                            }
                        }
                    }
                    Value::Object(_) => {
                        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
                            for (key, v) in map {
                                if !props.contains_key(key) {
                                    check(
                                        root,
                                        extra,
                                        v,
                                        &format!("{path}.{key}"),
                                        depth + 1,
                                        errors,
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Array(items) => {
            if let Some(min) = obj.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < min {
                    errors.push(format!("{path}: needs at least {min} item(s)"));
                }
            }
            if let Some(max) = obj.get("maxItems").and_then(Value::as_u64) {
                if (items.len() as u64) > max {
                    errors.push(format!("{path}: allows at most {max} item(s)"));
                }
            }
            if let Some(sub) = obj.get("items") {
                for (i, item) in items.iter().enumerate() {
                    check(root, sub, item, &format!("{path}[{i}]"), depth + 1, errors);
                }
            }
        }
        Value::String(s) => {
            let n = s.chars().count() as u64;
            if let Some(min) = obj.get("minLength").and_then(Value::as_u64) {
                if n < min {
                    errors.push(format!("{path}: must be at least {min} character(s)"));
                }
            }
            if let Some(max) = obj.get("maxLength").and_then(Value::as_u64) {
                if n > max {
                    errors.push(format!("{path}: must be at most {max} character(s)"));
                }
            }
            if let Some(pattern) = obj.get("pattern").and_then(Value::as_str) {
                match simple_match(pattern, s) {
                    Ok(false) => {
                        errors.push(format!("{path}: does not match the required pattern"))
                    }
                    Ok(true) => {}
                    // A pattern this validator cannot evaluate must not turn a
                    // legitimate call into a failure.
                    Err(()) => {}
                }
            }
        }
        Value::Number(n) => {
            if let Some(min) = obj.get("minimum").and_then(Value::as_f64) {
                if n.as_f64().map(|v| v < min).unwrap_or(false) {
                    errors.push(format!("{path}: must be at least {min}"));
                }
            }
            if let Some(max) = obj.get("maximum").and_then(Value::as_f64) {
                if n.as_f64().map(|v| v > max).unwrap_or(false) {
                    errors.push(format!("{path}: must be at most {max}"));
                }
            }
            if let Some(min) = obj.get("exclusiveMinimum").and_then(Value::as_f64) {
                if n.as_f64().map(|v| v <= min).unwrap_or(false) {
                    errors.push(format!("{path}: must be greater than {min}"));
                }
            }
            if let Some(max) = obj.get("exclusiveMaximum").and_then(Value::as_f64) {
                if n.as_f64().map(|v| v >= max).unwrap_or(false) {
                    errors.push(format!("{path}: must be less than {max}"));
                }
            }
        }
        _ => {}
    }
}

fn branch_ok(root: &Value, schema: &Value, value: &Value, path: &str, depth: usize) -> bool {
    let mut e = Vec::new();
    check(root, schema, value, path, depth, &mut e);
    e.is_empty()
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let pointer = reference.strip_prefix('#')?;
    // `#/definitions/X` and `#/$defs/X` are JSON Pointer paths already.
    root.pointer(pointer)
}

fn has_type(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value.as_f64().map(|f| f.fract() == 0.0).unwrap_or(false)
        }
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    crate::util::truncate(&text, 120)
}

/// A deliberately small pattern matcher for the anchored patterns schemars
/// generates (`^...$`) and the character classes devd uses. Anything else is
/// reported as unsupported and skipped rather than guessed at.
fn simple_match(pattern: &str, text: &str) -> std::result::Result<bool, ()> {
    let anchored = pattern.starts_with('^') && pattern.ends_with('$');
    if !anchored {
        return Err(());
    }
    let inner = &pattern[1..pattern.len() - 1];
    // Character class followed by a quantifier, e.g. `[A-Za-z0-9_-]+`.
    if let Some(rest) = inner.strip_prefix('[') {
        if let Some((class, tail)) = rest.split_once(']') {
            let repeated = tail == "+" || tail == "*" || tail == "?";
            if repeated {
                let min = if tail == "+" { 1 } else { 0 };
                if text.chars().count() < min {
                    return Ok(false);
                }
                if tail == "?" && text.chars().count() > 1 {
                    return Ok(false);
                }
                return Ok(text.chars().all(|c| class_allows(class, c)));
            }
        }
    }
    // Literal alternatives, e.g. `^(a|b|c)$`.
    if inner.starts_with('(') && inner.ends_with(')') {
        let choices = &inner[1..inner.len() - 1];
        return Ok(choices.split('|').any(|c| c == text));
    }
    // Escaped or literal text.
    if inner.chars().any(|c| "\\+*?[](){}".contains(c)) {
        return Err(());
    }
    Ok(inner == text)
}

fn class_allows(class: &str, c: char) -> bool {
    let bytes: Vec<char> = class.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len() && bytes[i + 1] == '-' {
            if c >= bytes[i] && c <= bytes[i + 2] {
                return true;
            }
            i += 3;
        } else {
            if c == bytes[i] {
                return true;
            }
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string", "minLength": 1 },
                "recursive": { "type": "boolean" },
                "mode": { "enum": ["fast", "safe"] },
                "limit": { "type": "integer", "minimum": 1, "maximum": 10 }
            }
        })
    }

    #[test]
    fn a_matching_object_passes() {
        validate(
            &schema(),
            &json!({ "path": "C:/x", "recursive": true, "limit": 4 }),
        )
        .unwrap();
        validate(&schema(), &json!({ "path": "C:/x", "mode": "safe" })).unwrap();
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        let e = validate(&schema(), &json!({})).unwrap_err();
        assert!(e.to_string().contains("$.path"), "{e}");
        assert!(e.to_string().contains("required"), "{e}");
    }

    #[test]
    fn a_wrong_type_is_refused_before_any_handler_runs() {
        let e = validate(&schema(), &json!({ "path": 3 })).unwrap_err();
        assert!(e.to_string().contains("expected string"), "{e}");
    }

    #[test]
    fn unknown_arguments_are_refused_when_the_schema_says_so() {
        let e = validate(&schema(), &json!({ "path": "x", "extra": 1 })).unwrap_err();
        assert!(e.to_string().contains("extra"), "{e}");
    }

    #[test]
    fn bounds_and_enums_are_enforced() {
        assert!(validate(&schema(), &json!({ "path": "x", "limit": 0 })).is_err());
        assert!(validate(&schema(), &json!({ "path": "x", "limit": 11 })).is_err());
        assert!(validate(&schema(), &json!({ "path": "x", "mode": "other" })).is_err());
    }

    #[test]
    fn nullable_and_nested_shapes_work() {
        let s = json!({
            "type": "object",
            "properties": {
                "when": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
                "items": { "type": "array", "items": { "$ref": "#/definitions/Item" } }
            },
            "definitions": {
                "Item": { "type": "object", "required": ["name"], "properties": { "name": { "type": "string" } } }
            }
        });
        validate(&s, &json!({ "when": null, "items": [{ "name": "a" }] })).unwrap();
        let e = validate(&s, &json!({ "items": [{ "name": 2 }] })).unwrap_err();
        assert!(e.to_string().contains("$.items[0].name"), "{e}");
    }

    #[test]
    fn a_pattern_this_validator_cannot_read_does_not_reject_valid_input() {
        let s = json!({ "type": "string", "pattern": "look(?=ahead)" });
        validate(&s, &json!("anything")).unwrap();
        let anchored = json!({ "type": "string", "pattern": "^[a-z]+$" });
        validate(&anchored, &json!("abc")).unwrap();
        assert!(validate(&anchored, &json!("AB")).is_err());
    }
}
