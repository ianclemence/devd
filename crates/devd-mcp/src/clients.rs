//! Adding devd to the coding agents a person already uses.
//!
//! `devd mcp` is an MCP server, but an agent only uses it once it is listed in that agent's own
//! configuration, which is a different file in a different format for each: JSON for Claude,
//! Cursor, Windsurf and VS Code, TOML for Codex. This module edits those files, and it does so
//! the way a guest should in someone else's house:
//!
//! * It touches only its own entry (`devd`). Every other byte of the file stays exactly as it
//!   was, including its order, its comments (where the format has them) and its spacing; the
//!   edit is made on the text, not by parsing the file and writing it back.
//! * It never creates the home of a program that is not installed: a client whose folder does
//!   not exist is reported as not found rather than given a config it will never read.
//! * A file it cannot read as what it should be is left alone and named in the answer, so the
//!   person can add the entry by hand instead of finding a file replaced.
//! * The previous contents are kept beside the file (`.devd-backup`) before anything is written.
//!
//! The operations are human-only. A file in another program's configuration decides what that
//! program will run, so an agent must not be able to widen its own reach by writing one.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::{DevdError, Result};

/// The entry's name in every client's list of servers.
pub const ENTRY: &str = "devd";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// A JSON object that holds the servers under this key.
    Json { key: &'static str, typed: bool },
    /// Codex's `[mcp_servers.<name>]` tables.
    Toml,
}

/// One coding agent devd knows how to add itself to.
#[derive(Debug, Clone)]
pub struct Client {
    pub id: &'static str,
    pub name: &'static str,
    pub config: PathBuf,
    format: Format,
}

/// Where a client stands.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub id: &'static str,
    pub name: &'static str,
    pub config: String,
    /// The program's folder exists, so it is probably installed.
    pub found: bool,
    /// Its config already has a `devd` entry.
    pub added: bool,
    /// The entry points at the devd that would be written now.
    pub current: bool,
    /// Why nothing could be read or edited, if so.
    pub problem: Option<String>,
}

/// The clients, with their config files under `home` (and `appdata` where the platform keeps
/// application data apart from the home folder).
pub fn clients(home: &Path, appdata: &Path) -> Vec<Client> {
    vec![
        Client { id: "claude-code", name: "Claude Code", config: home.join(".claude.json"), format: Format::Json { key: "mcpServers", typed: true } },
        Client { id: "claude-desktop", name: "Claude Desktop", config: appdata.join("Claude").join("claude_desktop_config.json"), format: Format::Json { key: "mcpServers", typed: false } },
        Client { id: "cursor", name: "Cursor", config: home.join(".cursor").join("mcp.json"), format: Format::Json { key: "mcpServers", typed: false } },
        Client { id: "vscode", name: "VS Code", config: appdata.join("Code").join("User").join("mcp.json"), format: Format::Json { key: "servers", typed: true } },
        Client { id: "windsurf", name: "Windsurf", config: home.join(".codeium").join("windsurf").join("mcp_config.json"), format: Format::Json { key: "mcpServers", typed: false } },
        Client { id: "codex", name: "Codex", config: home.join(".codex").join("config.toml"), format: Format::Toml },
    ]
}

/// The clients on this machine.
pub fn this_machine() -> Vec<Client> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default();
    let appdata = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"))
    };
    clients(&home, &appdata)
}

/// The program a client should run: the `devd` command line, which is what speaks MCP. The
/// window is a different program (`devd-desktop`), so when this process is the window the
/// sibling `devd` is used.
pub fn command_for(current_exe: &Path) -> PathBuf {
    let name = current_exe.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name == "devd-desktop" {
        let sibling = current_exe.with_file_name(if cfg!(windows) { "devd.exe" } else { "devd" });
        if sibling.is_file() {
            return sibling;
        }
    }
    current_exe.to_path_buf()
}

// ---------------------------------------------------------------- reading

fn read(path: &Path) -> std::result::Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("could not read {}: {e}", path.display())),
    }
}

/// The text of a JSON entry named `key` inside the top-level object's member `container`, if
/// the file has one: `(start, end)` of the member's value, in bytes.
fn find_member(text: &str, within: (usize, usize), key: &str) -> Option<(usize, usize, usize)> {
    // `within` is the span of an object, braces included. Returns (key_start, value_start, value_end).
    let b = text.as_bytes();
    let mut i = within.0 + 1;
    let end = within.1;
    while i < end {
        // Skip whitespace and commas to the next key.
        while i < end && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        if i >= end || b[i] != b'"' {
            return None;
        }
        let key_start = i;
        let (name, after) = string_at(text, i)?;
        i = after;
        while i < end && (b[i].is_ascii_whitespace() || b[i] == b':') {
            i += 1;
        }
        let value_start = i;
        let value_end = value_end(text, i)?;
        if name == key {
            return Some((key_start, value_start, value_end));
        }
        i = value_end;
    }
    None
}

/// The string starting at `i` (a `"`), decoded enough to compare, and the index after it.
fn string_at(text: &str, i: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    let mut j = i + 1;
    let mut out = String::new();
    while j < b.len() {
        match b[j] {
            b'\\' => {
                out.push(*b.get(j + 1)? as char);
                j += 2;
            }
            b'"' => return Some((out, j + 1)),
            c => {
                out.push(c as char);
                j += 1;
            }
        }
    }
    None
}

/// The end (exclusive) of the JSON value that starts at `i`.
fn value_end(text: &str, i: usize) -> Option<usize> {
    let b = text.as_bytes();
    match *b.get(i)? {
        b'"' => string_at(text, i).map(|(_, e)| e),
        b'{' | b'[' => {
            let mut depth = 0i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => j = string_at(text, j)?.1 - 1,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']') && !b[j].is_ascii_whitespace() {
                j += 1;
            }
            Some(j)
        }
    }
}

/// The span of the top-level object: first `{` to its matching `}`.
fn root_object(text: &str) -> Option<(usize, usize)> {
    let start = text.find('{')?;
    value_end(text, start).map(|end| (start, end - 1))
}

/// A JSON file's text with the `devd` entry set under `key` (created if missing), touching
/// nothing else. `entry` is the pretty-printed value to write.
pub fn json_with_entry(text: &str, key: &str, entry: &Value) -> std::result::Result<String, String> {
    let blank = text.trim().is_empty();
    let source = if blank { "{}".to_string() } else { text.to_string() };
    serde_json::from_str::<Value>(&source).map_err(|e| format!("the file is not valid JSON ({e}); add the entry by hand"))?;
    let root = root_object(&source).ok_or("the file has no top-level object; add the entry by hand")?;
    let rendered = serde_json::to_string_pretty(entry).map_err(|e| e.to_string())?;
    let indent = |s: &str, pad: &str| s.lines().map(|l| format!("{pad}{l}")).collect::<Vec<_>>().join("\n").trim_start().to_string();

    // The servers object, if there is one.
    if let Some((_, vs, ve)) = find_member(&source, root, key) {
        if source.as_bytes()[vs] != b'{' {
            return Err(format!("`{key}` in the file is not an object; add the entry by hand"));
        }
        let servers = (vs, ve - 1);
        let member = format!("\"{ENTRY}\": {}", indent(&rendered, "    "));
        if let Some((ks, _, me)) = find_member(&source, servers, ENTRY) {
            // Replace the entry that is there.
            return Ok(format!("{}{}{}", &source[..ks], member, &source[me..]));
        }
        // Add it as the last member.
        let inner = &source[vs + 1..ve - 1];
        let empty = inner.trim().is_empty();
        let before = source[..ve - 1].trim_end();
        let sep = if empty { "" } else { "," };
        let close_pad = if source[..ve].rfind('\n').map(|n| source[n + 1..ve - 1].trim().is_empty()).unwrap_or(false) {
            let n = source[..ve].rfind('\n').unwrap_or(0);
            source[n + 1..ve - 1].to_string()
        } else {
            String::new()
        };
        return Ok(format!("{before}{sep}\n    {member}\n{close_pad}{}", &source[ve - 1..]));
    }

    // No servers object: add one as the last member of the root.
    let (rs, re) = root;
    let inner_empty = source[rs + 1..re].trim().is_empty();
    let before = source[..re].trim_end();
    let sep = if inner_empty { "" } else { "," };
    let block = format!("\"{key}\": {{\n    \"{ENTRY}\": {}\n  }}", indent(&rendered, "    "));
    Ok(format!("{before}{sep}\n  {block}\n{}", &source[re..]))
}

/// A JSON file's text without its `devd` entry. `None` when there was none.
pub fn json_without_entry(text: &str, key: &str) -> Option<String> {
    let root = root_object(text)?;
    let (_, vs, ve) = find_member(text, root, key)?;
    if text.as_bytes()[vs] != b'{' {
        return None;
    }
    let (ks, _, me) = find_member(text, (vs, ve - 1), ENTRY)?;
    // Take the comma that joins it to a neighbour, and the line it sat on.
    let mut start = ks;
    let mut end = me;
    let after = text[end..].trim_start();
    if after.starts_with(',') {
        end += text[end..].find(',')? + 1;
    } else if let Some(p) = text[..start].trim_end().strip_suffix(',') {
        start = p.len();
    }
    // Remove the whitespace that led to it.
    let lead = text[..start].len() - text[..start].trim_end_matches([' ', '\t']).len();
    let mut out = format!("{}{}", &text[..start - lead], &text[end..]);
    out = out.replace(",\n\n", ",\n");
    Some(out)
}

fn toml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn toml_header(line: &str) -> Option<&str> {
    let t = line.trim();
    (t.starts_with('[') && t.ends_with(']')).then_some(t)
}

/// A Codex config's text with `[mcp_servers.devd]` set to run `command`, other tables untouched.
pub fn toml_with_entry(text: &str, command: &str) -> String {
    let without = toml_without_entry(text).unwrap_or_else(|| text.to_string());
    let mut out = without.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("[mcp_servers.{ENTRY}]\ncommand = {}\nargs = [\"mcp\"]\n", toml_quote(command)));
    out
}

/// A Codex config's text without the devd table and its sub-tables. `None` when there was none.
pub fn toml_without_entry(text: &str) -> Option<String> {
    let own = format!("[mcp_servers.{ENTRY}]");
    let sub = format!("[mcp_servers.{ENTRY}.");
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    let mut found = false;
    for line in text.lines() {
        if let Some(h) = toml_header(line) {
            skipping = h == own || h.starts_with(&sub);
            found |= skipping;
        }
        if !skipping {
            out.push(line);
        }
    }
    if !found {
        return None;
    }
    let mut s = out.join("\n").trim_end().to_string();
    if !s.is_empty() {
        s.push('\n');
    }
    Some(s)
}

// ---------------------------------------------------------------- the operations

fn entry_value(client: &Client, command: &str) -> Value {
    match client.format {
        Format::Json { typed: true, .. } => json!({ "type": "stdio", "command": command, "args": ["mcp"] }),
        _ => json!({ "command": command, "args": ["mcp"] }),
    }
}

/// Where each client stands, for the window and for an agent asking.
pub fn status(all: &[Client], command: &Path) -> Vec<Status> {
    let cmd = command.display().to_string();
    all.iter()
        .map(|c| {
            let found = c.config.exists() || c.config.parent().is_some_and(|p| p.exists());
            let mut s = Status { id: c.id, name: c.name, config: c.config.display().to_string(), found, added: false, current: false, problem: None };
            match read(&c.config) {
                Ok(Some(text)) => match c.format {
                    Format::Json { key, .. } => match serde_json::from_str::<Value>(&text) {
                        Ok(v) => {
                            if let Some(e) = v.get(key).and_then(|k| k.get(ENTRY)) {
                                s.added = true;
                                s.current = e.get("command").and_then(Value::as_str).is_some_and(|x| x == cmd);
                            }
                        }
                        Err(e) => s.problem = Some(format!("not valid JSON ({e})")),
                    },
                    Format::Toml => {
                        s.added = text.lines().any(|l| toml_header(l) == Some(&format!("[mcp_servers.{ENTRY}]")));
                        s.current = s.added && text.contains(&toml_quote(&cmd));
                    }
                },
                Ok(None) => {}
                Err(e) => s.problem = Some(e),
            }
            s
        })
        .collect()
}

fn write_with_backup(path: &Path, new: &str, old: Option<&str>) -> Result<Option<PathBuf>> {
    let mut backup = None;
    if let Some(old) = old {
        let b = path.with_file_name(format!("{}.devd-backup", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        std::fs::write(&b, old)?;
        backup = Some(b);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    devd_core::util::atomic_write(path, new)?;
    Ok(backup)
}

/// Add devd to one client's config. Returns the file written and the backup of what was there.
pub fn add(client: &Client, command: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    let exists = client.config.exists();
    if !exists && !client.config.parent().is_some_and(|p| p.exists()) {
        return Err(DevdError::not_found(format!(
            "{} does not look installed: {} is not there. Open it once, then try again",
            client.name,
            client.config.parent().map(|p| p.display().to_string()).unwrap_or_default()
        )));
    }
    let old = read(&client.config).map_err(DevdError::invalid)?;
    let cmd = command.display().to_string();
    let new = match client.format {
        Format::Json { key, .. } => json_with_entry(old.as_deref().unwrap_or(""), key, &entry_value(client, &cmd))
            .map_err(|e| DevdError::invalid(format!("{}: {e}", client.config.display())))?,
        Format::Toml => toml_with_entry(old.as_deref().unwrap_or(""), &cmd),
    };
    if old.as_deref() == Some(new.as_str()) {
        return Ok((client.config.clone(), None));
    }
    let backup = write_with_backup(&client.config, &new, old.as_deref())?;
    Ok((client.config.clone(), backup))
}

/// Remove devd from one client's config. `false` when it was not there.
pub fn remove(client: &Client) -> Result<bool> {
    let Some(old) = read(&client.config).map_err(DevdError::invalid)? else { return Ok(false) };
    let new = match client.format {
        Format::Json { key, .. } => json_without_entry(&old, key),
        Format::Toml => toml_without_entry(&old),
    };
    match new {
        Some(new) => {
            write_with_backup(&client.config, &new, Some(&old))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Value {
        json!({ "command": "C:\\devd\\devd.exe", "args": ["mcp"] })
    }

    #[test]
    fn an_entry_is_added_to_an_existing_server_list_and_nothing_else_moves() {
        let before = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"files\": { \"command\": \"npx\", \"args\": [\"-y\", \"x\"] }\n  },\n  \"zeta\": 1\n}\n";
        let after = json_with_entry(before, "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["mcpServers"]["devd"]["args"][0], "mcp");
        assert_eq!(v["mcpServers"]["files"]["command"], "npx");
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["zeta"], 1);
        // The original lines are still there, in the same order, byte for byte.
        let a = after.find("\"theme\"").unwrap();
        let b = after.find("\"mcpServers\"").unwrap();
        let c = after.find("\"zeta\"").unwrap();
        assert!(a < b && b < c, "{after}");
        assert!(after.contains("\"files\": { \"command\": \"npx\", \"args\": [\"-y\", \"x\"] }"), "{after}");
    }

    #[test]
    fn a_missing_server_list_is_created_and_a_blank_file_becomes_one() {
        let a = json_with_entry("{\n  \"theme\": \"dark\"\n}\n", "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&a).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["devd"]["command"], "C:\\devd\\devd.exe");
        let blank = json_with_entry("", "servers", &entry()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&blank).unwrap()["servers"]["devd"]["args"][0], "mcp");
    }

    #[test]
    fn adding_twice_replaces_the_entry_instead_of_stacking_it() {
        let once = json_with_entry("{\"mcpServers\":{}}", "mcpServers", &entry()).unwrap();
        let newer = json!({ "command": "D:\\new\\devd.exe", "args": ["mcp"] });
        let twice = json_with_entry(&once, "mcpServers", &newer).unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["mcpServers"]["devd"]["command"], "D:\\new\\devd.exe");
        assert_eq!(twice.matches("\"devd\"").count(), 1, "{twice}");
    }

    #[test]
    fn a_file_that_is_not_json_is_refused_not_rewritten() {
        let err = json_with_entry("{ \"mcpServers\": { // a comment\n }", "mcpServers", &entry()).unwrap_err();
        assert!(err.contains("not valid JSON") || err.contains("by hand"), "{err}");
        assert!(json_with_entry("{\"mcpServers\": []}", "mcpServers", &entry()).unwrap_err().contains("not an object"));
    }

    #[test]
    fn removing_the_entry_leaves_the_others_and_the_file_valid() {
        let before = "{\n  \"mcpServers\": {\n    \"a\": { \"command\": \"x\" },\n    \"devd\": { \"command\": \"d\", \"args\": [\"mcp\"] },\n    \"b\": { \"command\": \"y\" }\n  },\n  \"keep\": true\n}\n";
        let after = json_without_entry(before, "mcpServers").unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert!(v["mcpServers"].get("devd").is_none());
        assert_eq!(v["mcpServers"]["a"]["command"], "x");
        assert_eq!(v["mcpServers"]["b"]["command"], "y");
        assert_eq!(v["keep"], true);
        let only = json_without_entry("{\"mcpServers\":{\"devd\":{\"command\":\"d\"}}}", "mcpServers").unwrap();
        assert!(serde_json::from_str::<Value>(&only).is_ok(), "{only}");
        assert_eq!(json_without_entry("{\"mcpServers\":{\"a\":{}}}", "mcpServers"), None);
    }

    #[test]
    fn a_string_that_looks_like_json_does_not_confuse_the_scanner() {
        let before = "{\n  \"note\": \"a } brace and a \\\"quote\\\" and \\\"devd\\\"\",\n  \"mcpServers\": {}\n}";
        let after = json_with_entry(before, "mcpServers", &entry()).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["note"], "a } brace and a \"quote\" and \"devd\"");
        assert!(v["mcpServers"]["devd"].is_object());
    }

    #[test]
    fn codex_gets_its_own_table_and_keeps_the_rest_of_the_file() {
        let before = "model = \"gpt-5\"\n\n# my servers\n[mcp_servers.files]\ncommand = \"npx\"\n\n[other]\nx = 1\n";
        let after = toml_with_entry(before, "C:\\devd\\devd.exe");
        assert!(after.contains("model = \"gpt-5\"") && after.contains("# my servers") && after.contains("[other]\nx = 1"));
        assert!(after.contains("[mcp_servers.devd]\ncommand = \"C:\\\\devd\\\\devd.exe\"\nargs = [\"mcp\"]"));
        let twice = toml_with_entry(&after, "D:\\new\\devd.exe");
        assert_eq!(twice.matches("[mcp_servers.devd]").count(), 1);
        assert!(twice.contains("D:\\\\new\\\\devd.exe") && !twice.contains("C:\\\\devd\\\\devd.exe"));
        let gone = toml_without_entry(&twice).unwrap();
        assert!(!gone.contains("devd") && gone.contains("[mcp_servers.files]") && gone.contains("[other]"));
        assert_eq!(toml_without_entry("[a]\nb = 1\n"), None);
    }

    #[test]
    fn the_window_is_never_what_a_client_is_told_to_run() {
        let d = std::env::temp_dir().join(format!("devd-clients-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (cli, win) = if cfg!(windows) { ("devd.exe", "devd-desktop.exe") } else { ("devd", "devd-desktop") };
        std::fs::write(d.join(cli), b"x").unwrap();
        assert_eq!(command_for(&d.join(win)), d.join(cli));
        assert_eq!(command_for(&d.join(cli)), d.join(cli));
    }

    #[test]
    fn a_client_that_is_not_installed_is_reported_and_not_given_a_config() {
        let d = std::env::temp_dir().join(format!("devd-clients-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let all = clients(&d, &d);
        let cursor = all.iter().find(|c| c.id == "cursor").unwrap();
        let err = add(cursor, Path::new("C:\\devd\\devd.exe")).unwrap_err();
        assert!(err.to_string().contains("does not look installed"), "{err}");
        assert!(!d.join(".cursor").exists());
        // Once its folder exists, the file is made, and it can be removed again.
        std::fs::create_dir_all(d.join(".cursor")).unwrap();
        add(cursor, Path::new("C:\\devd\\devd.exe")).unwrap();
        let st = status(&all, Path::new("C:\\devd\\devd.exe"));
        let s = st.iter().find(|s| s.id == "cursor").unwrap();
        assert!(s.found && s.added && s.current && s.problem.is_none());
        assert!(remove(cursor).unwrap());
        assert!(!remove(cursor).unwrap(), "removing twice is not an error");
    }
}
