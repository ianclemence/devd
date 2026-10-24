//! What devd needs on this machine, and how to get it.
//!
//! One list, one answer, whichever interface asks: the things devd cannot work
//! without, the things it can suggest, and the exact operation that fixes each
//! one. The fix is expressed as a capability call with its arguments, so the
//! ordinary permission model applies to it: the window shows a plan and asks,
//! the terminal asks, the CLI asks, and an agent is refused.
//!
//! Nothing here installs anything. It only says what is missing and what would
//! fix it, which is why it can be called from a read-only surface.

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::Result;

use crate::secrets;

/// One thing devd needs, or can use, on this machine.
#[derive(Debug, Clone)]
pub struct Requirement {
    /// Stable id, e.g. `tool:git` or `service:nginx`.
    pub id: String,
    /// What it is called in the interface.
    pub title: String,
    /// `tool`, `service`, `runtime`, `provider` or `system`.
    pub kind: &'static str,
    /// Why devd wants it. One sentence, in plain words.
    pub why: String,
    /// `ready`, `missing` or `broken`.
    pub state: &'static str,
    /// Whether devd cannot work without it.
    pub required: bool,
    /// The measured detail, when there is one: a version, a path, a message.
    pub detail: Option<String>,
    /// What would fix it: a capability to call, a view to open, or a command to
    /// run by hand.
    pub fix: Option<Value>,
}

impl Requirement {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "kind": self.kind,
            "why": self.why,
            "state": self.state,
            "required": self.required,
            "detail": self.detail,
            "fix": self.fix,
        })
    }
}

/// Everything devd needs, in the order a person would want to deal with it:
/// what is broken first, then what is missing, then what is ready.
pub fn list(ctx: &Ctx) -> Result<Value> {
    let mut reqs: Vec<Requirement> = Vec::new();

    // ---- Git: every repository feature, and the agent team's worktrees.
    let git = devd_git::doctor_check(ctx.settings.git_executable.as_deref());
    if git.level != devd_core::doctor::Level::Ok {
        reqs.push(Requirement {
            id: "tool:git".into(),
            title: "Git".into(),
            kind: "tool",
            why: "Every repository feature, and the isolated copies the agent team works in, needs Git.".into(),
            state: if git.level == devd_core::doctor::Level::Fail { "missing" } else { "broken" },
            required: true,
            detail: None,
            fix: Some(json!({
                "label": "Install Git",
                "capability": "toolchain.install",
                "args": { "tool": "git" }
            })),
        });
    }

    // ---- The PHP runtime devd serves sites with.
    //
    // devd serves sites through a FastCGI listener it runs, and that
    // listener comes from a runtime devd downloaded and verified. A PHP that
    // is already on the machine is real and it is usable, but devd cannot
    // manage it - so it is named, not hidden: the person must be able to see
    // that a PHP exists and still be told why devd wants its own.
    let runtimes = devd_runtime::list(ctx).unwrap_or_default();
    let managed_php = runtimes
        .iter()
        .find(|r| r.runtime.kind == "php" && r.managed)
        .map(|r| r.runtime.version.clone());
    let found_php = runtimes
        .iter()
        .find(|r| r.runtime.kind == "php" && !r.managed)
        .map(|r| (r.runtime.version.clone(), r.executable.clone()));
    if managed_php.is_none() {
        let (why, detail) = match &found_php {
            Some((version, path)) => (
                format!(
                    "PHP {version} is installed on this machine, but not by devd. Sites are served through a FastCGI listener devd runs, and that listener is built from a runtime devd downloaded and verified."
                ),
                Some(format!("found at {path}")),
            ),
            None => (
                "Sites are served by PHP. devd downloads a verified build from windows.php.net.".to_string(),
                None,
            ),
        };
        reqs.push(Requirement {
            id: "runtime:php".into(),
            title: match &found_php {
                Some(_) => "PHP runtime devd manages".into(),
                None => "PHP".into(),
            },
            kind: "runtime",
            why,
            state: "missing",
            required: true,
            detail,
            fix: Some(json!({
                "label": "Install PHP",
                "capability": "runtime.install",
                "args": { "kind": "php", "version": "latest" }
            })),
        });
    }

    // ---- The services devd manages: the web server, the database, the
    // listener that runs PHP.
    for s in devd_services::list(ctx).unwrap_or_default() {
        if s.installed {
            continue;
        }
        let (why, required) = match s.name.as_str() {
            "nginx" => ("The web server that serves your sites over HTTP and HTTPS.".to_string(), true),
            "mariadb" => ("The local database devd offers for projects that need one.".to_string(), false),
            "php-cgi" => ("The listener that runs PHP for your sites.".to_string(), true),
            _ => (format!("The {} service devd manages.", s.name), false),
        };
        reqs.push(Requirement {
            id: format!("service:{}", s.name),
            title: match s.name.as_str() {
                "nginx" => "nginx".into(),
                "mariadb" => "MariaDB".into(),
                "php-cgi" => "PHP listener".into(),
                other => other.to_string(),
            },
            kind: "service",
            why,
            state: "missing",
            required,
            detail: None,
            fix: Some(json!({
                "label": format!("Install {}", s.name),
                "capability": "service.install",
                "args": { "name": s.name }
            })),
        });
    }

    // ---- The model provider: only the assistant needs it, but the assistant
    // is the product, so it is always offered.
    let providers = ctx.store.list_providers().unwrap_or_default();
    if providers.is_empty() {
        reqs.push(Requirement {
            id: "provider:model".into(),
            title: "A model provider".into(),
            kind: "provider",
            why: "The built-in assistant and the agent team need a model. Everything else in devd works without one.".into(),
            state: "missing",
            required: false,
            detail: None,
            fix: Some(json!({ "label": "Set up a provider", "view": "providers" })),
        });
    } else {
        let without_key: Vec<&str> = providers
            .iter()
            .filter(|p| secrets::read_key(&ctx.paths, p).is_none())
            .map(|p| p.id.as_str())
            .collect();
        if !without_key.is_empty() {
            reqs.push(Requirement {
                id: "provider:key".into(),
                title: "A model provider key".into(),
                kind: "provider",
                why: format!(
                    "{} has no API key, so the assistant cannot reach it.",
                    without_key.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
                ),
                state: "missing",
                required: false,
                detail: None,
                fix: Some(json!({ "label": "Add the key", "view": "providers" })),
            });
        }
    }

    // ---- The native window needs the WebView2 runtime; the CLI does not.
    #[cfg(windows)]
    if let Some(detail) = webview2_missing() {
        reqs.push(Requirement {
            id: "system:webview2".into(),
            title: "The WebView2 runtime".into(),
            kind: "system",
            why: "The devd window is drawn by the WebView2 runtime, which ships with current Edge but is not always installed.".into(),
            state: "missing",
            required: false,
            detail: Some(detail),
            fix: Some(json!({
                "label": "Install the WebView2 runtime",
                "url": "https://developer.microsoft.com/microsoft-edge/webview2/",
                "command": "MicrosoftEdgeWebview2Setup.exe /silent /install"
            })),
        });
    }

    // Broken first, then missing, then ready: the order a person acts in.
    reqs.sort_by_key(|r| match r.state {
        "broken" => 0,
        "missing" => 1,
        _ => 2,
    });
    let missing = reqs.iter().filter(|r| r.state != "ready").count();
    let required_missing = reqs.iter().filter(|r| r.state != "ready" && r.required).count();
    Ok(json!({
        "requirements": reqs.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
        "missing": missing,
        "required_missing": required_missing,
        "complete": required_missing == 0,
    }))
}

/// Whether the WebView2 runtime is missing on this machine.
///
/// It is not on the PATH; the runtime installs itself under the EdgeWebView
/// folder. The per-machine and per-user locations are both checked, because
/// either one is enough to run the window.
#[cfg(windows)]
fn webview2_missing() -> Option<String> {
    let program_files = std::env::var("ProgramFiles").unwrap_or_default();
    let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let system = std::env::var("SystemRoot").unwrap_or_default();
    for root in [program_files_x86, program_files, local] {
        if root.is_empty() {
            continue;
        }
        let dir = std::path::Path::new(&root).join("Microsoft").join("EdgeWebView").join("Application");
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if entry.path().join("msedgewebview2.exe").is_file() {
                return None;
            }
        }
    }
    // Older installs put the runtime in the system directory itself.
    if !system.is_empty() && std::path::Path::new(&system).join("System32").join("msedgewebview2.exe").is_file() {
        return None;
    }
    Some("the WebView2 runtime is not installed on this machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_requirement_carries_a_fix_or_explains_itself() {
        let r = Requirement {
            id: "tool:git".into(),
            title: "Git".into(),
            kind: "tool",
            why: "why".into(),
            state: "missing",
            required: true,
            detail: None,
            fix: Some(json!({ "label": "Install Git" })),
        };
        let v = r.to_json();
        assert_eq!(v["id"], "tool:git");
        assert_eq!(v["state"], "missing");
        assert!(v["required"].as_bool().unwrap());
        assert!(v["fix"]["label"].is_string());
    }

    #[test]
    fn the_php_fix_names_a_version_because_runtime_install_requires_one() {
        use devd_core::capability::Actor;
        use devd_core::config::Settings;
        use devd_core::events::EventBus;
        use devd_core::paths::Paths;
        use devd_core::state::Store;
        use std::sync::Arc;

        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path().join("home"));
        paths.ensure().unwrap();
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        let ctx = Ctx { store: Arc::new(store), paths: Arc::new(paths), settings: Arc::new(Settings::default()), events: EventBus::new(), actor: Actor::agent("t"), dry_run: false };
        let all = list(&ctx).unwrap();
        let php = all["requirements"].as_array().unwrap().iter().find(|r| r["id"] == "runtime:php").expect("a machine with no managed PHP is asked to install it");
        assert_eq!(php["fix"]["capability"], "runtime.install");
        assert_eq!(php["fix"]["args"]["kind"], "php");
        assert!(php["fix"]["args"]["version"].is_string(), "the install capability rejects a call without a version: {php}");
    }

    #[cfg(windows)]
    #[test]
    fn the_webview2_probe_answers_without_panicking() {
        // Whether it is installed depends on the machine; the probe must simply
        // answer without touching anything.
        let _ = webview2_missing();
    }
}
