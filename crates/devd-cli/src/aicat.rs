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
use crate::{ApprovalsCommand, ChatArgs, InboxCommand, ModelsCommand, ProviderCommand, ScheduleCommand, SkillsCommand, TeamCommand, TemplateCommand};

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
/// `devd chats ...`: managing the conversations that exist.
pub fn chats(app: &App, cmd: crate::ChatsCommand) -> Result<()> {
    use crate::ChatsCommand as C;
    match cmd {
        C::List => app.invoke("chat.list", json!({})),
        C::Delete { thread } => app.invoke("chat.delete", json!({ "thread": thread })),
        C::Prune { days } => app.invoke("chat.prune", json!({ "days": days })),
        C::ClearInbox => app.invoke("inbox.clear", json!({})),
    }
}

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
        ModelsCommand::Roles => app.invoke("models.roles", json!({})),
        ModelsCommand::Assign { role, models, project } => {
            let mut list = Vec::new();
            for m in &models {
                let (p, model) = m.split_once(':').ok_or_else(|| DevdError::invalid(format!("`{m}` should look like provider:model")))?;
                list.push(json!({ "provider": p, "model": model }));
            }
            app.invoke("models.assign", json!({ "role": role, "models": list, "project": project }))
        }
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

fn abs_path(p: &str) -> String {
    if std::path::Path::new(p).is_absolute() {
        p.to_string()
    } else {
        std::env::current_dir().map(|c| c.join(p).display().to_string()).unwrap_or_else(|_| p.to_string())
    }
}

/// One line per task, the way the terminal shows a run.
fn print_run(v: &Value) {
    let r = &v["run"];
    println!("{}  [{}]  {}", r["id"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""));
    if let Some(e) = r["error"].as_str() {
        println!("  ! {e}");
    }
    for t in v["tasks"].as_array().cloned().unwrap_or_default() {
        let ev = t["evidence"].as_array().cloned().unwrap_or_default();
        let verified = ev.iter().filter(|f| f["kind"] == "verify" && f["ok"] == true).count();
        println!("  {:<17} {:<12} {:<12} {}{}", t["state"].as_str().unwrap_or(""), t["role"].as_str().unwrap_or(""), t["key"].as_str().unwrap_or(""), t["title"].as_str().unwrap_or(""), if verified > 0 { format!("  ({verified} verification run(s) passed)") } else { String::new() });
        if let Some(e) = t["error"].as_str().filter(|_| matches!(t["state"].as_str(), Some("unverified" | "blocked" | "failed"))) {
            println!("      {e}");
        }
    }
    if let Some(s) = r["summary"].as_str() {
        println!("\n{s}");
    }
}

fn follow_run(app: &App, id: &str) -> Result<()> {
    let mut after = 0i64;
    let mut last_state = String::new();
    loop {
        let v = app.data("team.get", json!({ "run": id, "after": after }))?;
        for e in v["events"].as_array().cloned().unwrap_or_default() {
            after = e["id"].as_i64().unwrap_or(after);
            let d = &e["data"];
            let line = match e["kind"].as_str().unwrap_or("") {
                "started" if d.get("task").is_some() => format!("started {} ({})", d["task"].as_str().unwrap_or(""), d["role"].as_str().unwrap_or("")),
                "started" => "run started".to_string(),
                "plan" => format!("plan: {} task(s)", d["tasks"].as_array().map(|t| t.len()).unwrap_or(0)),
                "report" => format!("{} reports {}: {}", d["task"].as_str().unwrap_or(""), d["status"].as_str().unwrap_or(""), d["summary"].as_str().unwrap_or("").lines().next().unwrap_or("")),
                "accepted" => format!("accepted {} ({} verification run(s) passed)", d["task"].as_str().unwrap_or(""), d["verified"]),
                "rejected" | "unverified" => format!("NOT accepted {}: {}", d["task"].as_str().unwrap_or(""), d["why"].as_str().unwrap_or("")),
                "direction" => format!("instruction added: {}", d["text"].as_str().unwrap_or("")),
                other => format!("{other} {}", d),
            };
            println!("  {line}");
        }
        let st = v["run"]["state"].as_str().unwrap_or("").to_string();
        if st != last_state {
            last_state = st.clone();
            if st == "awaiting_plan" {
                print_run(&v);
                println!("\nThe plan needs your approval: devd team approve {id}");
                return Ok(());
            }
        }
        if matches!(st.as_str(), "done" | "failed" | "cancelled" | "paused") {
            println!();
            print_run(&v);
            return Ok(());
        }
        let pending = v["approvals"].as_array().map(|a| a.len()).unwrap_or(0);
        if pending > 0 {
            println!("\n{pending} operation(s) are waiting for your approval: devd approvals list");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
}

/// `devd team`.
pub fn team(app: &App, cmd: TeamCommand) -> Result<()> {
    match cmd {
        TeamCommand::Start { goal, project, new, parallel, yes_plan, done_when, rounds, detach } => {
            let run = app.data("team.start", json!({ "goal": goal, "project": project.as_deref().map(abs_path), "new_project": new, "parallel": parallel, "auto_approve": if yes_plan { Some(true) } else { None }, "done_when": if done_when.is_empty() { None } else { Some(done_when) }, "max_rounds": rounds }))?;
            let id = run["id"].as_str().unwrap_or("").to_string();
            if app.json || detach {
                println!("{}", serde_json::to_string_pretty(&run).unwrap_or_default());
                return Ok(());
            }
            println!("run {id} started; the planner is working");
            follow_run(app, &id)
        }
        TeamCommand::List => app.invoke("team.list", json!({})),
        TeamCommand::Show { run, follow } => {
            if follow && !app.json {
                return follow_run(app, &run);
            }
            if app.json {
                return app.invoke("team.get", json!({ "run": run }));
            }
            let v = app.data("team.get", json!({ "run": run }))?;
            print_run(&v);
            Ok(())
        }
        TeamCommand::Roles => app.invoke("team.roles", json!({})),
        TeamCommand::Approve { run } => {
            app.invoke("team.approve_plan", json!({ "run": run }))?;
            if !app.json {
                follow_run(app, &run)?;
            }
            Ok(())
        }
        TeamCommand::Say { run, text } => app.invoke("team.say", json!({ "run": run, "text": text })),
        TeamCommand::Pause { run } => app.invoke("team.pause", json!({ "run": run })),
        TeamCommand::Resume { run } => app.invoke("team.resume", json!({ "run": run })),
        TeamCommand::Cancel { run } => app.invoke("team.cancel", json!({ "run": run })),
        TeamCommand::Delete { run } => app.invoke("team.delete", json!({ "run": run })),
        TeamCommand::Prune { days } => app.invoke("team.prune", json!({ "days": days })),
        TeamCommand::Retry { task, note } => app.invoke("team.retry", json!({ "task": task, "note": note })),
        TeamCommand::Accept { task } => app.invoke("team.accept", json!({ "task": task })),
    }
}

/// `name=value` pairs as a JSON object.
fn pairs(items: &[String]) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut out = serde_json::Map::new();
    for item in items {
        let (k, v) = item.split_once('=').ok_or_else(|| devd_core::DevdError::invalid(format!("`{item}` should be name=value")))?;
        out.insert(k.trim().to_string(), json!(v));
    }
    Ok(out)
}

/// `devd template`.
pub fn template(app: &App, cmd: TemplateCommand) -> Result<()> {
    match cmd {
        TemplateCommand::List => app.invoke("template.list", json!({})),
        TemplateCommand::Show { id } => app.invoke("template.get", json!({ "id": id })),
        TemplateCommand::Run { id, project, new, params, done_when, yes_plan, detach } => {
            let run = app.data(
                "template.run",
                json!({
                    "id": id,
                    "project": project.as_deref().map(abs_path),
                    "new_project": new,
                    "params": pairs(&params)?,
                    "done_when": if done_when.is_empty() { None } else { Some(done_when) },
                    "auto_approve": if yes_plan { Some(true) } else { None },
                }),
            )?;
            let id = run["id"].as_str().unwrap_or("").to_string();
            if app.json || detach {
                println!("{}", serde_json::to_string_pretty(&run).unwrap_or_default());
                return Ok(());
            }
            println!("run {id} started; the planner is working");
            follow_run(app, &id)
        }
    }
}

/// `devd schedule`.
pub fn schedule(app: &App, cmd: ScheduleCommand) -> Result<()> {
    match cmd {
        ScheduleCommand::List => app.invoke("schedule.list", json!({})),
        ScheduleCommand::Create { name, spec, project, template, params, goal, done_when, yes_plan } => app.invoke(
            "schedule.create",
            json!({
                "name": name,
                "spec": spec,
                "project": abs_path(&project),
                "template": template,
                "params": pairs(&params)?,
                "goal": goal,
                "done_when": if done_when.is_empty() { None } else { Some(done_when) },
                "auto_approve": if yes_plan { Some(true) } else { None },
            }),
        ),
        ScheduleCommand::Enable { name } => app.invoke("schedule.enable", json!({ "name": name, "enabled": true })),
        ScheduleCommand::Disable { name } => app.invoke("schedule.enable", json!({ "name": name, "enabled": false })),
        ScheduleCommand::Run { name } => app.invoke("schedule.run_now", json!({ "name": name })),
        ScheduleCommand::Delete { name } => app.invoke("schedule.delete", json!({ "name": name })),
    }
}

/// `devd skills`.
pub fn skills(app: &App, cmd: SkillsCommand) -> Result<()> {
    match cmd {
        SkillsCommand::List { query, role, kind } => app.invoke("skills.list", json!({ "query": query, "role": role, "kind": kind })),
        SkillsCommand::Show { name } => {
            let s = app.data("skills.get", json!({ "name": name }))?;
            if app.json {
                println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            } else {
                println!("{}  ({}, for {})\n{}\n\n{}", s["name"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or(""), s["roles"].as_array().map(|r| r.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(), s["description"].as_str().unwrap_or(""), s["body"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        SkillsCommand::Add { name, description, kind, roles, file } => {
            let body = std::fs::read_to_string(&file).map_err(|e| DevdError::invalid(format!("cannot read {file}: {e}")))?;
            app.invoke("skills.save", json!({ "name": name, "description": description, "kind": kind, "roles": roles, "body": body }))
        }
        SkillsCommand::Remove { name } => app.invoke("skills.delete", json!({ "name": name })),
    }
}
