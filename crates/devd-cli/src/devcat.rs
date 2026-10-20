//! The `new`, `stacks`, `tools` and `project` command surfaces.
//!
//! Nothing here holds framework logic. The wizard reads the stack's option
//! schema from `stacks.describe` and asks about whatever options that stack
//! declares, so a new stack definition appears here without a code change.

use std::io::{BufRead, IsTerminal, Write};

use serde_json::{json, Map, Value};

use devd_core::capability::{Approval, InvokeRequest};
use devd_core::{DevdError, Result};

use crate::app::App;
use crate::{NewArgs, ProjectCommand, StacksCommand, ToolsCommand};

impl App {
    /// Dispatch without rendering, for commands that post-process the result.
    pub fn data(&self, id: &str, args: Value) -> Result<Value> {
        let res = self.registry.dispatch(
            &self.ctx(),
            &InvokeRequest { id: id.to_string(), args, dry_run: false },
            Approval::None,
        )?;
        Ok(res.outcome.map(|o| o.data).unwrap_or(Value::Null))
    }
}

fn prompt(label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(d) if !d.is_empty() => eprint!("{label} [{d}]: "),
        _ => eprint!("{label}: "),
    }
    std::io::stderr().flush().ok();
    let mut line = String::new();
    let n = std::io::stdin().lock().read_line(&mut line).map_err(|e| DevdError::internal(e.to_string()))?;
    if n == 0 {
        return Err(DevdError::invalid("input ended before the questions were answered"));
    }
    let t = line.trim();
    Ok(if t.is_empty() { default.unwrap_or("").to_string() } else { t.to_string() })
}

fn yes_no(label: &str, default: bool) -> Result<bool> {
    let a = prompt(&format!("{label} (y/n)"), Some(if default { "y" } else { "n" }))?;
    Ok(matches!(a.to_ascii_lowercase().as_str(), "y" | "yes" | "true"))
}

fn choose(label: &str, items: &[(String, String)], default: usize) -> Result<usize> {
    eprintln!("{label}");
    for (i, (_, text)) in items.iter().enumerate() {
        eprintln!("  {}{}) {}", if i == default { "*" } else { " " }, i + 1, text);
    }
    loop {
        let a = prompt("Choose a number", Some(&(default + 1).to_string()))?;
        if let Ok(n) = a.parse::<usize>() {
            if n >= 1 && n <= items.len() {
                return Ok(n - 1);
            }
        }
        if let Some(i) = items.iter().position(|(v, _)| *v == a) {
            return Ok(i);
        }
        eprintln!("  enter a number from 1 to {}", items.len());
    }
}

fn option_value_from_flag(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// `devd new`.
pub fn new_project(app: &App, a: NewArgs) -> Result<()> {
    let interactive = std::io::stdin().is_terminal() && !app.json && (a.stack.is_none() || a.name.is_none() || a.interactive);
    let mut req = Map::new();

    // 1. Stack.
    let stacks = app.data("stacks.list", json!({}))?;
    let list: Vec<&Value> = stacks.as_array().map(|v| v.iter().filter(|s| s["creatable"] == true).collect()).unwrap_or_default();
    let stack_id = match &a.stack {
        Some(s) => s.clone(),
        None if interactive => {
            let items: Vec<(String, String)> = list
                .iter()
                .map(|s| (s["id"].as_str().unwrap_or("").to_string(), format!("{:<18} {:<11} {}", s["name"].as_str().unwrap_or(""), s["category"].as_str().unwrap_or(""), s["summary"].as_str().unwrap_or(""))))
                .collect();
            items[choose("What do you want to create?", &items, 0)?].0.clone()
        }
        None => return Err(DevdError::invalid("name a stack (see `devd stacks list`) or run in a terminal for the guided flow")),
    };
    let desc = app.data("stacks.describe", json!({"stack": stack_id}))?;
    let def = &desc["stack"];
    req.insert("stack".into(), json!(stack_id));
    if interactive {
        eprintln!("\n{} - {}", def["name"].as_str().unwrap_or(""), def["summary"].as_str().unwrap_or(""));
        if let Some(d) = def["docs"].get(0) {
            eprintln!("Docs: {}", d["url"].as_str().unwrap_or(""));
        }
    }

    // 2. Version and package manager.
    let versions = def["versions"].as_array().cloned().unwrap_or_default();
    let version = match &a.stack_version {
        Some(v) => Some(v.clone()),
        None if interactive && versions.len() > 1 => {
            let items: Vec<(String, String)> = versions.iter().map(|v| (v["id"].as_str().unwrap_or("").to_string(), format!("{}{}", v["label"].as_str().unwrap_or(""), if v["default"] == true { " (default)" } else { "" }))).collect();
            let d = versions.iter().position(|v| v["default"] == true).unwrap_or(0);
            Some(items[choose("Which version?", &items, d)?].0.clone())
        }
        None => None,
    };
    if let Some(v) = version {
        req.insert("version".into(), json!(v));
    }
    let pms = def["package_managers"].as_array().cloned().unwrap_or_default();
    let pm = match &a.pm {
        Some(p) => Some(p.clone()),
        None if interactive && pms.len() > 1 => {
            let items: Vec<(String, String)> = pms.iter().map(|p| (p.as_str().unwrap_or("").to_string(), p.as_str().unwrap_or("").to_string())).collect();
            Some(items[choose("Package manager?", &items, 0)?].0.clone())
        }
        None => None,
    };
    if let Some(p) = pm {
        req.insert("package_manager".into(), json!(p));
    }

    // 3. Name and destination.
    let name = match &a.name {
        Some(n) => n.clone(),
        None if interactive => prompt("Project name", None)?,
        None => return Err(DevdError::invalid("name the project: devd new <stack> <name>")),
    };
    req.insert("name".into(), json!(name));
    if let Some(d) = &a.dest {
        req.insert("destination".into(), json!(d));
    } else if interactive {
        let d = prompt("Create it in (blank for the projects root)", None)?;
        if !d.is_empty() {
            req.insert("destination".into(), json!(d));
        }
    }

    // 4. Options.
    let mut options = Map::new();
    for kv in &a.option {
        let (k, v) = kv.split_once('=').ok_or_else(|| DevdError::invalid(format!("-o needs key=value, got `{kv}`")))?;
        options.insert(k.to_string(), option_value_from_flag(v));
    }
    let opts = def["options"].as_array().cloned().unwrap_or_default();
    if interactive && !opts.is_empty() {
        let advanced = opts.iter().any(|o| o["advanced"] == true) && yes_no("Show advanced options?", false)?;
        for o in &opts {
            let id = o["id"].as_str().unwrap_or("");
            if options.contains_key(id) || (o["advanced"] == true && !advanced) {
                continue;
            }
            eprintln!("\n{}{}", o["label"].as_str().unwrap_or(id), o["flag"].as_str().map(|f| format!("  [{f}]")).unwrap_or_default());
            if let Some(h) = o["help"].as_str().filter(|h| !h.is_empty()) {
                eprintln!("  {h}");
            }
            let default = &o["default"];
            match o["kind"].as_str().unwrap_or("") {
                "bool" => {
                    let v = yes_no(o["label"].as_str().unwrap_or(id), default.as_bool().unwrap_or(false))?;
                    options.insert(id.into(), json!(v));
                }
                "select" => {
                    let choices = o["choices"].as_array().cloned().unwrap_or_default();
                    let items: Vec<(String, String)> = choices.iter().map(|c| (c["value"].as_str().unwrap_or("").to_string(), c["label"].as_str().unwrap_or("").to_string())).collect();
                    let d = items.iter().position(|(v, _)| Some(v.as_str()) == default.as_str()).unwrap_or(0);
                    options.insert(id.into(), json!(items[choose("", &items, d)?].0));
                }
                "multi" => {
                    let choices = o["choices"].as_array().cloned().unwrap_or_default();
                    let dflt: Vec<String> = default.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
                    eprintln!("  choices: {}", choices.iter().map(|c| c["value"].as_str().unwrap_or("")).collect::<Vec<_>>().join(", "));
                    let v = prompt("  comma separated", Some(&dflt.join(",")))?;
                    options.insert(id.into(), json!(v));
                }
                _ => {
                    let v = prompt("  value", default.as_str())?;
                    options.insert(id.into(), json!(v));
                }
            }
        }
    }
    if !options.is_empty() {
        req.insert("options".into(), Value::Object(options));
    }
    req.insert("git".into(), json!(!a.no_git));
    if a.no_install {
        req.insert("install".into(), json!(false));
    } else if interactive && def["init"]["install_toggle"] == true {
        let v = yes_no("Install dependencies now?", true)?;
        req.insert("install".into(), json!(v));
    }
    if !a.extra.is_empty() {
        req.insert("extra_args".into(), json!(a.extra));
    }

    // 5. Review.
    let mut request = Value::Object(req.clone());
    let plan = app.data("stacks.validate", request.clone())?;
    if !app.json {
        print_review(&plan);
    }
    if plan["valid"] != true {
        return Err(DevdError::invalid("the request is not valid; fix the problems above"));
    }
    if app.plan_only {
        return Ok(());
    }
    if interactive && a.extra.is_empty() && yes_no("Add or change arguments on the command above?", false)? {
        let extra = prompt("Extra arguments (space separated)", None)?;
        let args: Vec<String> = extra.split_whitespace().map(|s| s.to_string()).collect();
        req.insert("extra_args".into(), json!(args));
        request = Value::Object(req.clone());
        let again = app.data("stacks.validate", request.clone())?;
        print_review(&again);
    }
    if plan["ready"] != true {
        return Err(DevdError::conflict("prerequisites are missing; install them with `devd tools install <tool>`"));
    }
    if interactive && !app.assume_yes && !yes_no("Create the project?", true)? {
        eprintln!("nothing was created");
        return Ok(());
    }
    app.invoke("project.init", request)
}

fn print_review(plan: &Value) {
    println!("\nReview");
    println!("  stack:     {} ({})", plan["stack_name"].as_str().unwrap_or(""), plan["version"].as_str().unwrap_or(""));
    println!("  directory: {}", plan["project_dir"].as_str().unwrap_or(""));
    if let Some(pm) = plan["package_manager"].as_str().filter(|s| !s.is_empty()) {
        println!("  packages:  {pm}");
    }
    println!("  command:   {}", plan["command"].as_str().unwrap_or(""));
    if let Some(steps) = plan["steps"].as_array() {
        for s in steps {
            println!("    - {}: {}", s["title"].as_str().unwrap_or(""), s["display"].as_str().unwrap_or(""));
        }
    }
    if let Some(p) = plan["prerequisites"].as_array() {
        println!("  prerequisites:");
        for q in p {
            let st = q["status"].as_str().unwrap_or("");
            println!(
                "    {} {} {}{}",
                if st == "ok" { "ok     " } else { "PROBLEM" },
                q["name"].as_str().unwrap_or(""),
                q["found"].as_str().map(|f| format!("{f} ")).unwrap_or_default(),
                if st == "ok" { String::new() } else { format!("({st}; {})", q["fix"].as_str().unwrap_or("")) }
            );
        }
    }
    if let Some(is) = plan["issues"].as_array() {
        for i in is {
            println!("  PROBLEM {}: {}", i["field"].as_str().unwrap_or(""), i["message"].as_str().unwrap_or(""));
        }
    }
    if let Some(w) = plan["warnings"].as_array() {
        for x in w {
            println!("  note: {}", x.as_str().unwrap_or(""));
        }
    }
    println!();
}

/// `devd stacks`.
pub fn stacks(app: &App, cmd: StacksCommand) -> Result<()> {
    match cmd {
        StacksCommand::List { category } => app.invoke("stacks.list", json!({ "category": category })),
        StacksCommand::Show { id } => app.invoke("stacks.describe", json!({ "stack": id })),
        StacksCommand::Schema => app.invoke("stacks.schema", json!({})),
        StacksCommand::Matrix { markdown } => {
            if markdown {
                let reg = devd_stacks::StackRegistry::with_extensions(&app.paths.root().join("stacks"));
                print!("{}", devd_stacks::matrix::markdown(&reg));
                Ok(())
            } else {
                app.invoke("stacks.matrix", json!({}))
            }
        }
        StacksCommand::Runs { subject } => app.invoke("stacks.runs", json!({ "subject": subject })),
        StacksCommand::Cancel { run_id } => app.invoke("stacks.cancel", json!({ "run_id": run_id })),
    }
}

/// `devd tools`.
pub fn tools(app: &App, cmd: ToolsCommand) -> Result<()> {
    match cmd {
        ToolsCommand::List { category } => app.invoke("toolchain.list", json!({ "category": category })),
        ToolsCommand::Check { stack, version, pm } => app.invoke("toolchain.check", json!({ "stack": stack, "version": version, "package_manager": pm })),
        ToolsCommand::Plan { tool, remove } => app.invoke("toolchain.plan", json!({ "tool": tool, "action": if remove { "remove" } else { "install" } })),
        ToolsCommand::Install { tool } => app.invoke("toolchain.install", json!({ "tool": tool })),
        ToolsCommand::Remove { tool } => app.invoke("toolchain.remove", json!({ "tool": tool })),
        ToolsCommand::Status { tool } => app.invoke("toolchain.status", json!({ "tool": tool })),
        ToolsCommand::Start { tool } => app.invoke("toolchain.service", json!({ "tool": tool, "action": "start" })),
        ToolsCommand::Stop { tool } => app.invoke("toolchain.service", json!({ "tool": tool, "action": "stop" })),
        ToolsCommand::Restart { tool } => app.invoke("toolchain.service", json!({ "tool": tool, "action": "restart" })),
        ToolsCommand::Logs { tool } => app.invoke("toolchain.logs", json!({ "tool": tool })),
        ToolsCommand::Devices => app.invoke("toolchain.devices", json!({})),
        ToolsCommand::Services => app.invoke("services.inventory", json!({})),
        ToolsCommand::Decide { tool, mode, port, clear } => app.invoke("services.decide", json!({ "tool": tool, "mode": mode, "port": port, "clear": clear })),
    }
}

fn abs(p: &str) -> String {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        p.to_string()
    } else {
        std::env::current_dir().map(|c| c.join(path).display().to_string()).unwrap_or_else(|_| p.to_string())
    }
}

/// `devd project`.
pub fn project(app: &App, cmd: ProjectCommand) -> Result<()> {
    match cmd {
        ProjectCommand::Detect { path, stack } => app.invoke("project.detect", json!({ "path": abs(&path), "stack": stack })),
        ProjectCommand::Import { path, stack, name } => app.invoke("project.import", json!({ "path": abs(&path), "stack": stack, "name": name })),
        ProjectCommand::List => app.invoke("project.list", json!({})),
        ProjectCommand::Forget { path } => app.invoke("project.forget", json!({ "path": abs(&path) })),
        ProjectCommand::Tasks { path } => app.invoke("project.tasks", json!({ "path": abs(&path) })),
        ProjectCommand::Run { path, task, background, foreground, args } => {
            let ctx_args = json!({ "path": abs(&path), "task": task, "args": args, "background": if background { Some(true) } else if foreground { Some(false) } else { None } });
            // Destructive tasks go through their own capability so approval is required.
            let tasks = app.data("project.tasks", json!({ "path": abs(&path) }))?;
            let destructive = tasks.as_array().and_then(|t| t.iter().find(|x| x["id"] == json!(task))).map(|t| t["risk"] == "destructive").unwrap_or(false);
            app.invoke(if destructive { "project.run_destructive" } else { "project.run" }, ctx_args)
        }
        ProjectCommand::Exec { path, timeout, argv } => {
            let a = json!({ "path": abs(&path), "argv": argv, "timeout_secs": timeout });
            // The engine decides whether this is destructive; try the safe path first.
            match app.data("project.exec", a.clone()) {
                Err(DevdError::Denied(m)) if m.contains("project.exec_destructive") => app.invoke("project.exec_destructive", a),
                _ => app.invoke("project.exec", a),
            }
        }
        ProjectCommand::Ps => app.invoke("task.list", json!({})),
        ProjectCommand::Stop { name } => app.invoke("task.stop", json!({ "name": name })),
        ProjectCommand::Logs { name, lines } => app.invoke("task.logs", json!({ "name": name, "lines": lines })),
    }
}
