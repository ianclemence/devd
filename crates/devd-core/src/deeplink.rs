//! `devd://` addresses: how a run, a conversation, a project or a site is named in text.
//!
//! The address is the window's own route (`/runs/<id>`) behind the `devd://` scheme, so
//! one function in the window reads what these write. They are what a command prints in
//! its `link` field, what an agent puts in a reply, and what an operating system hands to
//! the app when someone clicks one.

use serde_json::{json, Value};

/// Percent-encode what cannot sit in one path segment: a project is a directory, and its
/// separators would otherwise be mistaken for more segments.
pub fn encode(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// A team run.
pub fn run(id: &str) -> String {
    format!("devd://runs/{}", encode(id))
}

/// A conversation with the assistant.
pub fn conversation(id: &str) -> String {
    format!("devd://assistant/{}", encode(id))
}

/// A project, by its directory.
pub fn project(dir: &str) -> String {
    format!("devd://projects/{}", encode(dir))
}

/// A site, by its name.
pub fn site(name: &str) -> String {
    format!("devd://sites/{}", encode(name))
}

/// `value` with a `link` field, when it is an object that has none. Anything else is returned as it was.
pub fn with_link(mut value: Value, link: String) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.entry("link").or_insert(json!(link));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_thing_has_an_address() {
        assert_eq!(run("9f3a21"), "devd://runs/9f3a21");
        assert_eq!(conversation("abc-1"), "devd://assistant/abc-1");
        assert_eq!(site("luna.test"), "devd://sites/luna.test");
    }

    #[test]
    fn a_directory_stays_one_segment() {
        assert_eq!(project(r"D:\www\luna"), "devd://projects/D%3A%5Cwww%5Cluna");
        assert_eq!(project("/home/me/my app"), "devd://projects/%2Fhome%2Fme%2Fmy%20app");
    }

    #[test]
    fn a_link_is_added_once_and_only_to_an_object() {
        let v = with_link(json!({ "id": "x" }), run("x"));
        assert_eq!(v["link"], "devd://runs/x");
        let kept = with_link(json!({ "link": "mine" }), run("x"));
        assert_eq!(kept["link"], "mine");
        assert_eq!(with_link(json!([1]), run("x")), json!([1]));
    }
}
