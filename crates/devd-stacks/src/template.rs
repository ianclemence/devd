//! Command-line templating: placeholders and conditions.
//!
//! Commands are lists of arguments, never shell strings, so a project name or
//! option value can never be interpreted as shell syntax.

use std::collections::BTreeMap;

use serde_json::Value;

use devd_core::{DevdError, Result};

use crate::model::{ArgItem, Cond};

/// Everything a template can refer to.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// Option values by id, including the pseudo options `version`,
    /// `package_manager`, `git` and `install`.
    pub values: BTreeMap<String, Value>,
    /// Single-valued variables: `name`, `dir`, `parent`, `version`, ...
    pub vars: BTreeMap<String, String>,
    /// Multi-valued tokens that expand to several arguments: `pm.run`, `py`.
    pub multi: BTreeMap<String, Vec<String>>,
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(as_text).collect::<Vec<_>>().join(","),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Evaluate a condition.
pub fn eval_cond(c: &Cond, env: &Env) -> bool {
    let v = env.values.get(&c.option).cloned().unwrap_or(Value::Null);
    let mut r = true;
    if let Some(eq) = &c.equals {
        r &= match (eq, &v) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            _ => as_text(eq) == as_text(&v),
        };
    }
    if let Some(list) = &c.in_ {
        r &= list.iter().any(|x| as_text(x) == as_text(&v));
    }
    if let Some(needle) = &c.contains {
        r &= match &v {
            Value::Array(a) => a.iter().any(|x| as_text(x) == *needle),
            Value::String(s) => s == needle,
            _ => false,
        };
    }
    if let Some(ne) = c.nonempty {
        let empty = match &v {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Bool(b) => !*b,
            _ => false,
        };
        r &= ne != empty;
    }
    r != c.not
}

/// Expand a single-valued template string.
pub fn expand_text(s: &str, env: &Env) -> Result<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        // Only `{identifier}` is a placeholder; other braces (JSON, code) are literal.
        let key = after.find('}').map(|j| &after[..j]).filter(|k| {
            !k.is_empty()
                && k.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
                && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
        });
        match key {
            Some(k) => {
                out.push_str(&lookup(k, env)?);
                rest = &after[k.len() + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup(key: &str, env: &Env) -> Result<String> {
    if let Some(id) = key.strip_prefix("opt.") {
        return env
            .values
            .get(id)
            .map(as_text)
            .ok_or_else(|| DevdError::internal(format!("unknown option `{id}` in template")));
    }
    if let Some(v) = env.vars.get(key) {
        return Ok(v.clone());
    }
    Err(DevdError::internal(format!("unknown placeholder `{{{key}}}`")))
}

/// Expand a list of argument items to concrete arguments.
pub fn expand(items: &[ArgItem], env: &Env) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for item in items {
        match item {
            ArgItem::Lit(s) => {
                if s.starts_with('{') && s.ends_with('}') && s.matches('{').count() == 1 {
                    let key = &s[1..s.len() - 1];
                    if let Some(m) = env.multi.get(key) {
                        out.extend(m.iter().cloned());
                        continue;
                    }
                }
                out.push(expand_text(s, env)?);
            }
            ArgItem::If {
                when,
                args,
                otherwise,
            } => {
                let branch = if when.conds().iter().all(|c| eval_cond(c, env)) { args } else { otherwise };
                out.extend(expand(branch, env)?);
            }
        }
    }
    Ok(out)
}

/// A display form of an argument list that a person can paste into a terminal.
pub fn display(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty()
                && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@,+%^".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env() -> Env {
        let mut e = Env::default();
        e.values.insert("ts".into(), json!(true));
        e.values.insert("features".into(), json!(["a", "b"]));
        e.values.insert("pm".into(), json!("pnpm"));
        e.vars.insert("name".into(), "my app".into());
        e.multi.insert("pm.run".into(), vec!["pnpm".into(), "run".into()]);
        e
    }

    fn items(y: &str) -> Vec<ArgItem> {
        serde_yaml::from_str(y).unwrap()
    }

    #[test]
    fn conditions_and_multi_tokens_expand() {
        let a = items("[\"{pm.run}\", dev, \"{name}\", {when: {option: ts, equals: true}, args: [--ts], else: [--js]}, {when: {option: features, contains: b}, args: [\"--f={opt.features}\"]}]");
        assert_eq!(
            expand(&a, &env()).unwrap(),
            vec!["pnpm", "run", "dev", "my app", "--ts", "--f=a,b"]
        );
    }

    #[test]
    fn unknown_placeholders_are_errors_not_blanks() {
        assert!(expand_text("{nope}", &env()).is_err());
        assert!(expand_text("{opt.nope}", &env()).is_err());
    }

    #[test]
    fn values_are_arguments_never_shell_syntax() {
        let mut e = env();
        e.vars.insert("name".into(), "x; rm -rf /".into());
        let out = expand(&items("[echo, \"{name}\"]"), &e).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(display(&out), "echo 'x; rm -rf /'");
    }
}
