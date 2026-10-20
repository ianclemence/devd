//! The `chat`, `provider`, `models`, `inbox` and `approvals` command surfaces.
//!
//! Like every other command these only shape arguments for capabilities. The
//! one piece of its own logic is following a conversation: poll the log, print
//! what the agent said and did, and ask the person when it needs approval.

use std::io::{BufRead, IsTerminal, Write};
use std::time::Duration;

use serde_json::{json, Value};

use devd_core::{DevdError, Result};

use crate::app::App;
use crate::{ApprovalsCommand, ChatArgs, InboxCommand, ModelsCommand, ProviderCommand};

fn ask(prompt: &str) -> Option<String> {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim().to_string()),
    }
}

fn short(v: &Value, n: usize) -> String {
    let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
    devd_core::util::truncate(&s.replace('\n', " "), n)
}

fn show(e: &Value) {
    match e["kind"].as_str().unwrap_or("") {
        "user" => println!("you> {}", e["data"]["text"].as_str().unwrap_or("")),
        "assistant" => {
            for b in e["data"]["content"].as_array().cloned().unwrap_or_default() {
                match b["type"].as_str().unwrap_or("") {
                    "text" => println!("agent> {}", b["text"].as_str().unwrap_or("")),
                    "tool_use" => {
                        let input = &b["input"];
                        let what = if b["name"] == "devd_invoke" { format!("{} {}", input["id"].as_str().unwrap_or(""), short(&input["args"], 100)) } else { format!("{} {}", b["name"].as_str().unwrap_or(""), short(input, 100)) };
                        println!("  -> {what}");
                    }
                    _ => {}
                }
            }
        }
        "tool_result" => println!("  <- {}{}", if e["data"]["is_error"] == true { "(error) " } else { "" }, short(&e["data"]["content"], 160)),
        "notice" => println!("  * {}", e["data"]["text"].as_str().unwrap_or("")),
        "error" => println!("  ! {}", e["data"]["message"].as_str().unwrap_or("")),
        _ => {}
    }
}

/// Follow a conversation until the agent is idle, asking for approvals as they
/// arise. Returns false when it stopped to wait for an approval it could not ask for.
fn follow(app: &App, tid: &str, mut after: i64, interactive: bool) -> Result<bool> {
    loop {
        let v = app.data("chat.get", json!({ "thread": tid, "after": after }))?;
        for e in v["events"].as_array().cloned().unwrap_or_default() {
            after = e["id"].as_i64().unwrap_or(after);
            if e["kind"] != "user" {
                show(&e);
            }
        }
        let state = v["thread"]["state"].as_str().unwrap_or("");
        let running = v["running"].as_bool().unwrap_or(false);
        if state == "waiting_approval" && !running {
            for r in v["pending"].as_array().cloned().unwrap_or_default() {
                println!("\nThe agent wants to run `{}`", r["capability"].as_str().unwrap_or(""));
                if let Some(steps) = r["plan"]["steps"].as_array() {
                    for s in steps {
                        println!("  - {}", s.as_str().unwrap_or(""));
                    }
                }
                if let Some(effects) = r["plan"]["effects"].as_array() {
                    for s in effects {
                        println!("  ! {}", s.as_str().unwrap_or(""));
                    }
                }
                println!("  arguments: {}", short(&r["args"], 300));
                let yes = if app.assume_yes {
                    println!("approved (--yes)");
                    true
                } else if interactive {
                    matches!(ask("Approve? [y/N] ").as_deref(), Some("y") | Some("Y") | Some("yes"))
                } else {
                    println!("waiting for approval; decide it with: devd approvals approve {}", r["id"].as_str().unwrap_or(""));
                    return Ok(false);
                };
                app.data("approval.decide", json!({ "id": r["id"], "approve": yes }))?;
            }
            continue;
        }
        if !running && (state == "idle" || state == "error") {
            if state == "error" {
                return Err(DevdError::conflict(v["thread"]["error"].as_str().unwrap_or("the agent stopped with an error").to_string()));
            }
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// `devd chat`.
pub fn chat(app: &App, a: ChatArgs) -> Result<()> {
    let interactive = std::io::stdin().is_terminal() && !app.json;
    let mut thread = a.thread.clone();
    let mut message = a.message.clone();
    loop {
        let text = match message.take() {
            Some(m) => m,
            None if interactive => match ask("you> ") {
                Some(t) if !t.is_empty() && t != "/quit" && t != "/exit" => t,
                _ => return Ok(()),
            },
            None => return Err(DevdError::invalid("give a message: devd chat \"what you want\"")),
        };
        let sent = app.data("chat.send", json!({ "message": text, "thread": thread, "project": a.project.as_ref().map(|p| if std::path::Path::new(p).is_absolute() { p.clone() } else { std::env::current_dir().map(|c| c.join(p).display().to_string()).unwrap_or_else(|_| p.clone()) }), "provider": a.provider, "model": a.model }))?;
        let tid = sent["id"].as_str().unwrap_or("").to_string();
        thread = Some(tid.clone());
        if !follow(app, &tid, 0, interactive)? {
            return Ok(());
        }
        if !interactive {
            println!("\nconversation: {tid}  (continue with --thread {tid})");
            return Ok(());
        }
    }
}

/// `devd provider`.
pub fn provider(app: &App, cmd: ProviderCommand) -> Result<()> {
    match cmd {
        ProviderCommand::List => app.invoke("provider.list", json!({})),
        ProviderCommand::Presets => app.invoke("provider.presets", json!({})),
        ProviderCommand::Add { id, preset, base_url, key_env, model, default } => {
            // The key is read from the environment or stdin, never from an argument,
            // so it does not land in shell history or the process list.
            let key = std::env::var("DEVD_API_KEY").ok().filter(|k| !k.trim().is_empty()).or_else(|| {
                if std::io::stdin().is_terminal() && !app.json {
                    ask("API key (leave empty to use an environment variable instead): ").filter(|k| !k.is_empty())
                } else {
                    None
                }
            });
            app.invoke("provider.save", json!({ "id": id, "preset": preset, "base_url": base_url, "key_env": key_env, "api_key": key, "default_model": model, "make_default": if default { Some(true) } else { None } }))
        }
        ProviderCommand::Test { id } => app.invoke("provider.test", json!({ "id": id })),
        ProviderCommand::Models { id } => app.invoke("provider.models", json!({ "id": id })),
        ProviderCommand::Default { id } => app.invoke("provider.default", json!({ "id": id })),
        ProviderCommand::Remove { id } => app.invoke("provider.remove", json!({ "id": id })),
    }
}

/// `devd models`.
pub fn models(app: &App, cmd: ModelsCommand) -> Result<()> {
    match cmd {
        ModelsCommand::List { source } => app.invoke("models.catalogue", json!({ "source": source })),
        ModelsCommand::Refresh => app.invoke("models.refresh_official", json!({})),
    }
}

/// `devd inbox`.
pub fn inbox(app: &App, cmd: InboxCommand) -> Result<()> {
    match cmd {
        InboxCommand::List { all } => app.invoke("inbox.list", json!({ "open_only": !all })),
        InboxCommand::Reply { id, text } => app.invoke("inbox.reply", json!({ "id": id, "reply": if text.is_empty() { None } else { Some(text.join(" ")) } })),
    }
}

/// `devd approvals`.
pub fn approvals(app: &App, cmd: ApprovalsCommand) -> Result<()> {
    match cmd {
        ApprovalsCommand::List { all } => app.invoke("approval.list", json!({ "pending_only": !all })),
        ApprovalsCommand::Approve { id } => decide(app, &id, true),
        ApprovalsCommand::Decline { id } => decide(app, &id, false),
    }
}

/// Decide a request. When the request belongs to a conversation, stay until the
/// agent has used the answer: it runs inside this process, so leaving early
/// would cut it off.
fn decide(app: &App, id: &str, approve: bool) -> Result<()> {
    let req = app.data("approval.get", json!({ "id": id }))?;
    let thread = req["thread_id"].as_str().map(|s| s.to_string());
    let after = match &thread {
        Some(t) => app.data("chat.get", json!({ "thread": t }))?["events"].as_array().and_then(|e| e.last().and_then(|x| x["id"].as_i64())).unwrap_or(0),
        None => 0,
    };
    app.invoke("approval.decide", json!({ "id": id, "approve": approve }))?;
    if let Some(t) = thread {
        follow(app, &t, after, std::io::stdin().is_terminal() && !app.json)?;
    }
    Ok(())
}
