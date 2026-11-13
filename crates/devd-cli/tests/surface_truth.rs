//! The window's pages may only ask devd for what devd can do.
//!
//! `devd_app_core::surface` holds every capability call the project and site pages make. This test asks the
//! real binary for every capability and its argument schema, then checks each call against them: the
//! capability exists, every argument sent is one it accepts, and every argument it requires is sent. It is
//! the check that would have caught a page asking for `stack.list`, which has never existed.

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

/// Asked of the binary once for the whole file: two tests starting devd in one folder at the same time race to create it.
fn capabilities() -> &'static BTreeMap<String, Value> {
    static ALL: OnceLock<BTreeMap<String, Value>> = OnceLock::new();
    ALL.get_or_init(ask)
}

fn ask() -> BTreeMap<String, Value> {
    let home = std::env::temp_dir().join(format!("devd-surface-truth-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&home);
    let out = Command::new(env!("CARGO_BIN_EXE_devd")).args(["--json", "capabilities"]).env("DEVD_HOME", &home).output().expect("devd runs");
    let _ = std::fs::remove_dir_all(&home);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let list: Vec<Value> = serde_json::from_slice(&out.stdout).expect("a JSON list of capabilities");
    list.into_iter().map(|c| (c["id"].as_str().unwrap().to_string(), c)).collect()
}

#[test]
fn every_call_a_page_makes_names_a_real_capability_with_real_arguments() {
    let known = capabilities();
    assert!(known.len() > 400, "the capability list looks truncated: {}", known.len());
    let mut problems = Vec::new();
    for call in devd_app_core::surface::samples() {
        let Some(cap) = known.get(call.cap) else {
            problems.push(format!("`{}` is not a capability devd has", call.cap));
            continue;
        };
        let schema = &cap["args_schema"];
        let properties = schema["properties"].as_object();
        let sent = call.args.as_object().expect("arguments are an object");
        for key in sent.keys() {
            if properties.is_some_and(|p| !p.contains_key(key)) {
                problems.push(format!("`{}` is sent `{key}`, which it does not accept (it takes {})", call.cap, properties.map(|p| p.keys().cloned().collect::<Vec<_>>().join(", ")).unwrap_or_default()));
            }
        }
        for required in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if !sent.contains_key(required) {
                problems.push(format!("`{}` requires `{required}`, which is not sent", call.cap));
            }
        }
    }
    assert!(problems.is_empty(), "the pages ask devd for things it cannot do:\n  {}", problems.join("\n  "));
}

#[test]
fn a_call_that_is_wrong_would_be_caught() {
    // The check has to be able to fail: the call that was once made by mistake is not in the list.
    let known = capabilities();
    assert!(known.contains_key("stacks.list"));
    assert!(!known.contains_key("stack.list"));
}
