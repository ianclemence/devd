//! `devd run --task task.json`: one prepared piece of work, one report line, an exit code.
//!
//! The rules of the contract (what a document may hold, what the report says, what each
//! exit code means) are in `devd_agent::task` and tested there. This is only the loop: send
//! the work, watch it, stop at the deadline, and say how it ended.
//!
//! Progress goes to standard error so a pipeline can read standard output as one JSON line.

use std::io::Read;
use std::time::{Duration, Instant};

use devd_agent::task::{self, Report, Status, Tokens};
use serde_json::{json, Value};

use crate::app::App;

/// How long reading the document may take. Separate from the run's own deadline.
const READ_LIMIT: Duration = Duration::from_secs(10);

/// How often the conversation is looked at.
const POLL: Duration = Duration::from_millis(500);

/// Run the task at `path` (`-` for standard input). Returns the process exit code.
pub fn run(app: &App, path: &str, check_only: bool) -> i32 {
    let bytes = match read(path) {
        Ok(b) => b,
        Err(why) => return finish(Report { error: Some(why), ..Report::of(Status::Invalid, None) }),
    };
    let task = match task::parse(&bytes) {
        Ok(t) => t,
        Err(why) => return finish(Report { error: Some(why.to_string()), ..Report::of(Status::Invalid, None) }),
    };
    if check_only {
        println!("{}", json!({ "version": task::VERSION, "valid": true, "taskId": task.task_id }));
        return 0;
    }

    let started = Instant::now();
    let limit = Duration::from_millis(task.max_duration_ms);
    let mut report = Report::of(Status::Failed, Some(task.task_id.clone()));
    let project = task.project.as_ref().map(|p| if std::path::Path::new(p).is_absolute() { p.clone() } else { std::env::current_dir().map(|c| c.join(p).display().to_string()).unwrap_or_else(|_| p.clone()) });
    let sent = app.data(
        "chat.send",
        json!({ "message": task.prompt, "project": project, "provider": task.provider, "model": task.model, "effort": task.effort, "permission": task.permission.clone().unwrap_or_else(|| "auto".to_string()) }),
    );
    let thread = match sent {
        Ok(v) => v["id"].as_str().unwrap_or("").to_string(),
        Err(e) => {
            report.error = Some(format!("the work could not be started: {e}"));
            report.duration_ms = started.elapsed().as_millis() as u64;
            return finish(report);
        }
    };
    report.thread = Some(thread.clone());
    report.link = Some(devd_core::deeplink::conversation(&thread));
    eprintln!("devd run: started {} ({})", task.task_id, report.link.as_deref().unwrap_or(""));

    let mut after = 0i64;
    let mut last_said = String::new();
    let mut missing: Vec<String> = Vec::new();
    loop {
        let v = match app.data("chat.get", json!({ "thread": thread, "after": after })) {
            Ok(v) => v,
            Err(e) => {
                report.error = Some(format!("the conversation could not be read: {e}"));
                break;
            }
        };
        for e in v["events"].as_array().cloned().unwrap_or_default() {
            after = e["id"].as_i64().unwrap_or(after);
            note(&e, &mut last_said, &mut missing);
        }
        record_use(&mut report, &v["thread"]);
        let state = v["thread"]["state"].as_str().unwrap_or("");
        let running = v["running"].as_bool().unwrap_or(false);

        if state == "waiting_approval" && !running {
            let pending: Vec<Value> = v["pending"].as_array().cloned().unwrap_or_default();
            if app.assume_yes {
                // The caller said yes to everything in advance (`--yes`); it is recorded like any approval.
                for r in &pending {
                    let _ = app.data("approval.decide", json!({ "id": r["id"], "approve": true }));
                }
                continue;
            }
            // Nobody is here to decide. Say which approval, and leave it waiting for a person.
            report.status = Status::NeedsPerson;
            report.pending_approvals = pending.iter().filter_map(|r| r["id"].as_str().map(str::to_string)).collect();
            report.error = Some(format!("the agent is waiting for approval to run {}", pending.iter().filter_map(|r| r["capability"].as_str()).collect::<Vec<_>>().join(", ")));
            break;
        }
        if !running && (state == "idle" || state == "error") {
            if state == "error" {
                report.error = Some(v["thread"]["error"].as_str().unwrap_or("the agent stopped with an error").to_string());
            } else {
                let unproven = app.store.run_for_thread(&thread).ok().flatten().map(|r| r.state == "unverified").unwrap_or(false);
                if unproven {
                    report.status = Status::Unproven;
                    report.missing = std::mem::take(&mut missing);
                } else {
                    report.status = Status::Completed;
                }
            }
            break;
        }
        if started.elapsed() >= limit {
            // Stopped, and said so. Not retried: it may already have changed things.
            let _ = app.data("chat.stop", json!({ "thread": thread }));
            report.status = Status::TimedOut;
            report.error = Some(format!("it was still working after {} ms and was stopped", task.max_duration_ms));
            break;
        }
        std::thread::sleep(POLL);
    }
    report.exit_code = report.status.exit_code();
    report.summary = last_said;
    report.duration_ms = started.elapsed().as_millis() as u64;
    finish(report)
}

/// Print the one line, and return its exit code.
fn finish(report: Report) -> i32 {
    println!("{}", report.line());
    report.exit_code
}

/// Read the document from a file or standard input, within the size and time limits.
fn read(path: &str) -> Result<Vec<u8>, String> {
    let limit = (task::MAX_BYTES + 1) as u64;
    if path == "-" {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let r = std::io::stdin().lock().take(limit).read_to_end(&mut buf).map(|_| buf);
            let _ = tx.send(r);
        });
        return match rx.recv_timeout(READ_LIMIT) {
            Ok(Ok(b)) => Ok(b),
            Ok(Err(e)) => Err(format!("could not read the task from standard input: {e}")),
            Err(_) => Err("the task was not finished arriving on standard input within 10 seconds: close it after the document".to_string()),
        };
    }
    let file = std::fs::File::open(path).map_err(|e| format!("could not open {path}: {e}"))?;
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf).map_err(|e| format!("could not read {path}: {e}"))?;
    Ok(buf)
}

/// Print what happened to standard error, and remember the agent's last words.
fn note(e: &Value, last_said: &mut String, missing: &mut Vec<String>) {
    match e["kind"].as_str().unwrap_or("") {
        "assistant" => {
            for b in e["data"]["content"].as_array().cloned().unwrap_or_default() {
                match b["type"].as_str().unwrap_or("") {
                    "text" => {
                        let text = b["text"].as_str().unwrap_or("").trim();
                        if !text.is_empty() {
                            *last_said = text.to_string();
                            eprintln!("agent: {}", first_line(text));
                        }
                    }
                    "tool_use" => eprintln!("  -> {}", b["input"]["id"].as_str().or(b["name"].as_str()).unwrap_or("a tool")),
                    _ => {}
                }
            }
        }
        "notice" => {
            let text = e["data"]["text"].as_str().unwrap_or("");
            eprintln!("  * {text}");
            // devd's own words when it settled the run: what the record still lacks.
            if let Some(rest) = text.strip_prefix("Finished, but not verified: ") {
                *missing = rest.trim_end_matches('.').split("; ").map(str::to_string).collect();
            }
        }
        "error" => eprintln!("  ! {}", e["data"]["message"].as_str().unwrap_or("")),
        _ => {}
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(140).collect::<String>())
    } else {
        line.to_string()
    }
}

fn record_use(report: &mut Report, thread: &Value) {
    report.tokens = Tokens { input: thread["input_tokens"].as_u64().unwrap_or(0), output: thread["output_tokens"].as_u64().unwrap_or(0) };
    report.cost_usd = thread["cost_usd"].as_f64();
}
