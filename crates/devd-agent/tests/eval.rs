//! Does the agent finish representative work, and is what it reports true?
//!
//! The unit tests prove that parts behave. These scenarios drive the whole
//! assistant - the conversation loop, the capability registry, the evidence
//! record and the completion policy - from a goal to a settled run, and measure
//! the outcome:
//!
//! * did the run end in the state the evidence justifies (completed, or
//!   unverified), and never in one it does not;
//! * how much evidence backs it, and how many calls it took;
//! * how long it took, and how many times a person had to step in.
//!
//! # Replay and live
//!
//! Each scenario has a script for the model, so it is deterministic and runs in
//! CI. A script can be replaced by a **recorded cassette**: set
//! `DEVD_EVAL_RECORD=1` with `DEEPSEEK_API_KEY` and the scenario talks to the real
//! provider while the raw responses are saved to `tests/cassettes/<name>.json`
//! (never the request, never the key). Replay then serves that file.
//!
//! `live_provider_scenarios` is opt-in (`DEVD_EVAL_LIVE=1`, `--ignored`): it
//! sends the same goals to a real model. Its assertions are about the *truthfulness
//! of the record*, not about what a particular model happens to do, because
//! model behaviour is not deterministic and a CI result must not depend on it.
//! A mocked scenario says nothing about how intelligent a model is.
//!
//! Every scenario writes its measurements to `$CARGO_TARGET_TMPDIR/eval/<name>.json`.
//! A failure also writes the whole conversation there, so it can be read after
//! the fact.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::Registry;

// ------------------------------------------------------------------ a model server

/// One scripted model turn.
#[derive(Clone, Debug)]
enum Turn {
    /// Call a capability through `devd_invoke`.
    Call { id: &'static str, args: Value },
    /// Answer in words and stop.
    Say(&'static str),
}

fn openai_body(turn: &Turn, n: usize) -> String {
    match turn {
        Turn::Call { id, args } => json!({
            "choices": [{ "finish_reason": "tool_calls", "message": { "content": null, "tool_calls": [{
                "id": format!("call_{n}"), "type": "function",
                "function": { "name": "devd_invoke", "arguments": json!({ "id": id, "args": args }).to_string() }
            }]}}],
            "usage": { "prompt_tokens": 100, "completion_tokens": 20 }
        })
        .to_string(),
        Turn::Say(text) => json!({
            "choices": [{ "finish_reason": "stop", "message": { "content": text } }],
            "usage": { "prompt_tokens": 120, "completion_tokens": 10 }
        })
        .to_string(),
    }
}

/// The three spellings a path takes inside a recorded response: as is, escaped once
/// (inside a JSON string) and escaped twice (a JSON string inside a JSON string,
/// which is how tool-call arguments travel).
fn path_forms(path: &str) -> [String; 3] {
    let once = path.replace('\\', "\\\\");
    let twice = once.replace('\\', "\\\\");
    [path.to_string(), once, twice]
}

/// The spellings of the project's path a response can carry: the path with and
/// without Windows' `\\?\` prefix, each in the three escapings.
fn spellings(project: &str) -> Vec<(String, String)> {
    let stripped = project.trim_start_matches("\\\\?\\").to_string();
    let mut out = Vec::new();
    for (tag, base) in [("FULL", project.to_string()), ("PLAIN", stripped)] {
        let forms = path_forms(&base);
        for (level, form) in forms.into_iter().enumerate() {
            out.push((format!("{{{{{tag}{level}}}}}"), form));
        }
    }
    out
}

/// Replace the project's real path with placeholders, so a cassette recorded in
/// one temporary directory replays in another.
fn templatize(text: &str, project: &str) -> String {
    let mut all = spellings(project);
    // Longest first, so the plain form inside a longer one is not cut short.
    all.sort_by_key(|(_, form)| std::cmp::Reverse(form.len()));
    let mut out = text.to_string();
    for (tag, form) in all {
        if !form.is_empty() {
            out = out.replace(&form, &tag);
        }
    }
    out
}

fn instantiate(text: &str, project: &str) -> String {
    let mut out = text.to_string();
    for (tag, form) in spellings(project) {
        out = out.replace(&tag, &form);
    }
    out
}

/// Where the raw responses of a recorded scenario live.
fn cassette_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cassettes")
        .join(format!("{name}.json"))
}

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

/// Serve responses in order: from the cassette when one exists, otherwise from
/// the script. The last response repeats if the agent asks for more.
fn serve(name: &str, script: Vec<Turn>) -> Server {
    serve_hooked(name, script, Vec::new())
}

/// Replay a recorded cassette in `project`.
fn replay(name: &str, project: &Path) -> Server {
    serve_inner(name, Vec::new(), Vec::new(), Some(project.display().to_string()))
}

/// Like [`serve`], and runs a hook just before answering request number `n`:
/// the way a scenario makes something happen "while the model was thinking".
fn serve_hooked(
    name: &str,
    script: Vec<Turn>,
    hooks: Vec<(usize, Box<dyn Fn() + Send>)>,
) -> Server {
    serve_inner(name, script, hooks, None)
}

fn serve_inner(
    name: &str,
    script: Vec<Turn>,
    hooks: Vec<(usize, Box<dyn Fn() + Send>)>,
    project: Option<String>,
) -> Server {
    let recorded: Option<Vec<String>> = std::fs::read_to_string(cassette_path(name))
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok());
    let recorded = recorded.map(|r| {
        r.into_iter()
            .map(|t| instantiate(&t, project.as_deref().unwrap_or("")))
            .collect::<Vec<_>>()
    });
    let responses: Vec<String> = recorded.unwrap_or_else(|| {
        script
            .iter()
            .enumerate()
            .map(|(n, t)| openai_body(t, n))
            .collect()
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        for (n, stream) in listener.incoming().enumerate() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let (head_end, len) = loop {
                let got = stream.read(&mut tmp).unwrap_or(0);
                if got == 0 {
                    break (buf.len(), 0);
                }
                buf.extend_from_slice(&tmp[..got]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let got = stream.read(&mut tmp).unwrap_or(0);
                if got == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..got]);
            }
            let body: Value =
                serde_json::from_slice(&buf[head_end.min(buf.len())..]).unwrap_or(Value::Null);
            seen.lock().unwrap().push(body);
            for (at, hook) in &hooks {
                if *at == n {
                    hook();
                }
            }
            // Past the end of the script the model stops. Repeating the last
            // answer would, for a recorded tool call, loop forever.
            let text = responses
                .get(n)
                .cloned()
                .unwrap_or_else(|| openai_body(&Turn::Say("(the script is exhausted)"), n));
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    Server { base, requests }
}

// ------------------------------------------------------------------ the environment

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    reg: Arc<Registry>,
    human: Ctx,
    project: PathBuf,
}

fn ctx_for(root: &Path, store: Arc<Store>, events: EventBus, actor: Actor) -> Ctx {
    let paths = Paths::rooted(root.join("home"));
    paths.ensure().unwrap();
    let mut settings = Settings::default();
    settings.www_root = Some(root.join("www"));
    Ctx {
        store,
        paths: Arc::new(paths),
        settings: Arc::new(settings),
        events,
        actor,
        run: None,
        dry_run: false,
    }
}

/// An environment with a git project whose `test` task is `rustc --version`,
/// registered through an extension stack - a check any machine that builds this
/// repository can run.
fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let store = Store::open_in_memory().unwrap();
    store.migrate().unwrap();
    let store = Arc::new(store);
    let events = EventBus::new();
    let hub = devd_agent::Hub::new(ctx_for(&root, store.clone(), events.clone(), Actor::system()));
    let mut reg = Registry::new();
    devd_stacks::caps::register(&mut reg).unwrap();
    let team = devd_agent::team::Team::new(
        hub.clone(),
        ctx_for(&root, store.clone(), events.clone(), Actor::system()),
    );
    devd_agent::caps::register(&mut reg, hub.clone(), team.clone()).unwrap();
    let reg = Arc::new(reg);
    hub.attach(reg.clone());
    team.attach(reg.clone());

    let stacks = root.join("home").join("stacks");
    std::fs::create_dir_all(&stacks).unwrap();
    std::fs::write(
        stacks.join("echo.yaml"),
        r#"
id: echoer
name: Echoer
language: shell
category: language
summary: Evaluation stack.
docs: [{title: Docs, url: "https://example.com/docs"}]
verified: {method: docs, against: test, date: "2026-10-20"}
versions: [{id: one, label: One, value: "1", default: true}]
init: {creates_dir: true, steps: []}
tasks:
  - {id: test, title: Test, kind: test, run: [rustc, --version]}
detect:
  rules: [{file: marker.txt, weight: 9, evidence: marker.txt}]
"#,
    )
    .unwrap();
    let project = root.join("www").join("proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("marker.txt"), "x").unwrap();
    std::fs::write(project.join("parser.txt"), "broken\n").unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&project)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]);

    let env = Env {
        human: ctx_for(&root, store, events, Actor::human(Surface::Http)),
        _dir: dir,
        root,
        reg,
        project,
    };
    call(
        &env,
        "project.import",
        json!({ "path": env.project.display().to_string(), "stack": "echoer" }),
    )
    .unwrap();
    env
}

fn call(e: &Env, id: &str, args: Value) -> devd_core::Result<Value> {
    let r = e.reg.dispatch(
        &e.human,
        &InvokeRequest {
            id: id.into(),
            args,
            dry_run: false,
        },
        Approval::None,
    )?;
    Ok(r.outcome.map(|o| o.data).unwrap_or(Value::Null))
}

fn add_provider(e: &Env, base: &str, key: &str, model: &str) {
    call(
        e,
        "provider.save",
        json!({ "id": "eval", "kind": "openai-compatible", "label": "Eval", "base_url": base, "api_key": key, "default_model": model }),
    )
    .unwrap();
}

// ------------------------------------------------------------------ measuring

#[derive(Debug)]
struct Measured {
    state: String,
    thread_state: String,
    model_calls: usize,
    tool_calls: usize,
    failed_tool_calls: usize,
    evidence: usize,
    evidence_passed: usize,
    change_evidence: usize,
    verifying_evidence: usize,
    approvals_needed: usize,
    duration_ms: u128,
    missing: Vec<String>,
}

impl Measured {
    fn json(&self, name: &str) -> Value {
        json!({
            "scenario": name,
            "run_state": self.state,
            "thread_state": self.thread_state,
            "model_calls": self.model_calls,
            "tool_calls": self.tool_calls,
            "failed_tool_calls": self.failed_tool_calls,
            "evidence": self.evidence,
            "evidence_passed": self.evidence_passed,
            "evidence_coverage": if self.tool_calls == 0 { 1.0 } else { self.evidence as f64 / self.tool_calls as f64 },
            "change_evidence": self.change_evidence,
            "verifying_evidence": self.verifying_evidence,
            "human_interventions": self.approvals_needed,
            "duration_ms": self.duration_ms,
            "missing": self.missing,
        })
    }
}

fn artifact_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Send a goal and wait for the run to settle.
fn run_goal(e: &Env, name: &str, goal: &str, model_calls: Option<&Server>) -> Measured {
    let started = Instant::now();
    let sent = call(
        e,
        "chat.send",
        json!({ "message": goal, "project": e.project.display().to_string() }),
    )
    .unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();

    let deadline = Instant::now() + Duration::from_secs(60);
    let thread = loop {
        let v = call(e, "chat.get", json!({ "thread": tid })).unwrap();
        let state = v["thread"]["state"].as_str().unwrap_or("").to_string();
        if ["idle", "error"].contains(&state.as_str()) && !v["running"].as_bool().unwrap_or(true) {
            break v;
        }
        if Instant::now() > deadline {
            dump(name, &v);
            panic!("{name}: the conversation did not settle: {state}");
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let duration_ms = started.elapsed().as_millis();

    let store = &e.human.store;
    let run = store.run_for_thread(&tid).unwrap().expect("the conversation has a run");
    let evidence = store.list_evidence(&run.id, 10_000).unwrap();
    let calls = store.list_tool_calls(&run.id, 10_000).unwrap();
    let verdict = devd_core::evaluate(&run.completion_policy, &evidence);
    let kinds = run.completion_policy.verifying_kinds();
    let measured = Measured {
        state: run.state.clone(),
        thread_state: thread["thread"]["state"].as_str().unwrap_or("").to_string(),
        model_calls: model_calls.map(|s| s.requests.lock().unwrap().len()).unwrap_or(0),
        tool_calls: calls.len(),
        failed_tool_calls: calls.iter().filter(|c| !c.ok).count(),
        evidence: evidence.len(),
        evidence_passed: evidence.iter().filter(|x| x.passed).count(),
        change_evidence: evidence.iter().filter(|x| x.kind == "change" && x.passed).count(),
        verifying_evidence: evidence
            .iter()
            .filter(|x| x.passed && kinds.contains(&x.kind.as_str()))
            .count(),
        approvals_needed: store
            .list_requests(false, Some(&tid), 100)
            .map(|r| r.len())
            .unwrap_or(0),
        duration_ms,
        missing: verdict.missing,
    };
    let report = measured.json(name);
    println!("EVAL {report}");
    let _ = std::fs::write(
        artifact_dir().join(format!("{name}.json")),
        serde_json::to_string_pretty(&report).unwrap(),
    );
    dump(name, &thread);
    measured
}

fn dump(name: &str, conversation: &Value) {
    let _ = std::fs::write(
        artifact_dir().join(format!("{name}.conversation.json")),
        serde_json::to_string_pretty(conversation).unwrap_or_default(),
    );
}

// ------------------------------------------------------------------ the scenarios

#[test]
fn inspecting_a_project_completes_without_changing_it() {
    let e = env();
    let m = serve(
        "inspect",
        vec![
            Turn::Call { id: "fs.list", args: json!({ "path": e.project.display().to_string() }) },
            Turn::Call { id: "fs.read", args: json!({ "path": e.project.join("parser.txt").display().to_string() }) },
            Turn::Say("The project has a marker and a parser file; the parser is broken."),
        ],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "scripted");
    let r = run_goal(&e, "inspect", "Explain how this project is laid out", Some(&m));

    assert_eq!(r.state, "completed", "{r:?}");
    assert_eq!(r.change_evidence, 0, "an explanation must not change anything");
    assert_eq!(r.failed_tool_calls, 0);
    assert!(r.evidence >= 2, "reads are recorded as evidence: {r:?}");
    assert_eq!(r.model_calls, 3);
    assert_eq!(std::fs::read_to_string(e.project.join("parser.txt")).unwrap(), "broken\n");
}

#[test]
fn a_fix_that_was_never_checked_ends_unverified_not_completed() {
    let e = env();
    let file = e.project.join("parser.txt").display().to_string();
    let m = serve(
        "unchecked",
        vec![
            Turn::Call { id: "fs.write", args: json!({ "path": file, "content": "fixed\n" }) },
            Turn::Say("I fixed the parser and everything works."),
        ],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "scripted");
    let r = run_goal(&e, "unchecked", "Fix the failing test in parser.txt", Some(&m));

    // The model said it was fixed. The record says nothing proves it.
    assert_eq!(r.state, "unverified", "{r:?}");
    assert!(r.change_evidence >= 1);
    assert_eq!(r.verifying_evidence, 0);
    assert!(
        r.missing.iter().any(|m| m.contains("verification")),
        "the reason is stated: {:?}",
        r.missing
    );
}

#[test]
fn a_fix_that_was_checked_completes_on_the_strength_of_the_check() {
    let e = env();
    let file = e.project.join("parser.txt").display().to_string();
    let m = serve(
        "checked",
        vec![
            Turn::Call { id: "fs.write", args: json!({ "path": file, "content": "fixed\n" }) },
            Turn::Call { id: "project.run", args: json!({ "path": e.project.display().to_string(), "task": "test" }) },
            Turn::Say("Fixed, and the test task passed."),
        ],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "scripted");
    let r = run_goal(&e, "checked", "Fix the failing test in parser.txt", Some(&m));

    assert_eq!(r.state, "completed", "{r:?}");
    assert!(r.change_evidence >= 1, "{r:?}");
    assert!(r.verifying_evidence >= 1, "{r:?}");
    assert_eq!(r.failed_tool_calls, 0);
    assert_eq!(std::fs::read_to_string(e.project.join("parser.txt")).unwrap(), "fixed\n");
}

#[test]
fn a_change_made_by_a_program_the_registry_did_not_see_is_still_evidence() {
    let e = env();
    let file = e.project.join("parser.txt");
    let (project, f2) = (e.project.display().to_string(), file.clone());
    // The first thing the "model" does is run the project's test task. Before its
    // second answer, something edits the file behind the registry's back.
    let m = serve_hooked(
        "outside_change",
        vec![
            Turn::Call { id: "project.run", args: json!({ "path": project, "task": "test" }) },
            Turn::Say("Done."),
        ],
        vec![(
            1,
            Box::new(move || {
                let _ = std::fs::write(&f2, "edited by something else, longer than before\n");
            }),
        )],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "scripted");
    let r = run_goal(&e, "outside_change", "Fix the failing test in parser.txt", Some(&m));

    // The edit was not a recorded call, but the project differs from how the run found it.
    // With the check that did run, the code goal is now satisfied by what was seen.
    assert_eq!(r.state, "completed", "{r:?}");
    let evidence = e.human.store.run_for_thread(
        &e.human.store.list_threads(10).unwrap()[0].id,
    ).unwrap().unwrap();
    let all = e.human.store.list_evidence(&evidence.id, 100).unwrap();
    assert!(
        all.iter().any(|x| x.capability == devd_core::runs::OBSERVED_CAPABILITY),
        "observed change recorded: {all:?} / {r:?}"
    );
}

#[test]
fn a_malformed_call_is_rejected_before_it_does_anything_and_the_agent_recovers() {
    let e = env();
    let file = e.project.join("note.txt").display().to_string();
    let m = serve(
        "malformed",
        vec![
            // `content` is required and missing: refused by the schema, nothing is written.
            Turn::Call { id: "fs.write", args: json!({ "path": file }) },
            Turn::Call { id: "fs.write", args: json!({ "path": file, "content": "ok\n" }) },
            Turn::Say("Wrote the note."),
        ],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "scripted");
    let r = run_goal(&e, "malformed", "Create note.txt containing ok", Some(&m));

    assert_eq!(r.failed_tool_calls, 1, "exactly the malformed call failed: {r:?}");
    assert_eq!(std::fs::read_to_string(e.project.join("note.txt")).unwrap(), "ok\n");
    assert!(r.change_evidence >= 1);
    // The refusal reached the model as a result it could act on.
    let requests = m.requests.lock().unwrap();
    let second = &requests[1]["messages"];
    let tool_message = second
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .expect("the failure is reported back");
    assert!(
        tool_message["content"].as_str().unwrap().to_lowercase().contains("content"),
        "{tool_message}"
    );
}

#[test]
fn a_privacy_policy_keeps_a_hosted_provider_from_ever_seeing_the_project() {
    let e = env();
    // A hosted provider (not on this machine) and a policy that allows only local models.
    call(
        &e,
        "provider.save",
        json!({ "id": "hosted", "kind": "openai-compatible", "label": "Hosted", "base_url": "https://api.example.com/v1", "api_key": "sk-eval-0123456789", "default_model": "m" }),
    )
    .unwrap();
    call(&e, "policy.set", json!({ "local_only": true })).unwrap();

    let sent = call(
        &e,
        "chat.send",
        json!({ "message": "Explain this project", "project": e.project.display().to_string() }),
    )
    .unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    let v = loop {
        let v = call(&e, "chat.get", json!({ "thread": tid })).unwrap();
        if v["thread"]["state"] == "error" || v["thread"]["state"] == "idle" {
            break v;
        }
        assert!(Instant::now() < deadline, "did not settle: {v}");
        std::thread::sleep(Duration::from_millis(40));
    };
    assert_eq!(v["thread"]["state"], "error", "{v}");
    let error = v["thread"]["error"].as_str().unwrap_or("");
    assert!(error.contains("policy.local_only") || error.contains("local models"), "{error}");
}

#[test]
fn a_spending_limit_stops_a_run_before_its_next_model_call() {
    let e = env();
    let m = serve(
        "budget",
        vec![
            Turn::Call { id: "fs.list", args: json!({ "path": e.project.display().to_string() }) },
            Turn::Call { id: "fs.list", args: json!({ "path": e.project.display().to_string() }) },
            Turn::Say("never reached"),
        ],
    );
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "priced");
    // A model with a known price, and a limit its first call already reaches.
    e.human
        .store
        .replace_catalogue(
            "provider:eval",
            &[devd_core::CatalogueModel {
                source: "provider:eval".into(),
                model_id: "priced".into(),
                display: "Priced".into(),
                description: None,
                context_window: Some(100_000),
                max_output: None,
                input_price: Some(1_000_000.0),
                output_price: Some(1_000_000.0),
                retirement: None,
                url: None,
                fetched_at: String::new(),
            }],
        )
        .unwrap();
    call(&e, "policy.set", json!({ "max_cost_usd": 1.0 })).unwrap();

    let sent = call(
        &e,
        "chat.send",
        json!({ "message": "Explain this project", "project": e.project.display().to_string() }),
    )
    .unwrap();
    let tid = sent["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    let v = loop {
        let v = call(&e, "chat.get", json!({ "thread": tid })).unwrap();
        if v["thread"]["state"] == "error" || v["thread"]["state"] == "idle" {
            break v;
        }
        assert!(Instant::now() < deadline, "did not settle: {v}");
        std::thread::sleep(Duration::from_millis(40));
    };
    assert_eq!(v["thread"]["state"], "error", "{v}");
    assert!(
        v["thread"]["error"].as_str().unwrap_or("").contains("limit"),
        "{}",
        v["thread"]["error"]
    );
    // One call was made and paid for; the second was never sent.
    assert_eq!(m.requests.lock().unwrap().len(), 1);
}

// ------------------------------------------------------------------ recording and live

/// A session recorded from a real model replays to the same kind of outcome.
///
/// The cassette holds only the provider's responses. Replaying it drives the
/// real loop, registry and evidence record; what it proves is that the harness
/// handles real provider output (shapes, ids, argument encoding), not that any
/// model would behave this way again.
#[test]
fn a_recorded_session_replays_through_the_real_loop() {
    // The session was recorded on Windows, so the paths the model used inside the
    // project have Windows separators. Only the project's own folder is rewritten
    // for replay, so on another system those paths name nothing and the agent
    // rightly asks before touching them. Record a cassette there to replay there.
    if !cfg!(windows) {
        println!("this cassette was recorded on Windows; skipped on this platform");
        return;
    }
    if !cassette_path("inspect_recorded").exists() {
        println!("no cassette recorded yet; run record_inspect to make one");
        return;
    }
    let e = env();
    let m = replay("inspect_recorded", &e.project);
    add_provider(&e, &format!("{}/v1", m.base), "sk-eval-0123456789", "recorded");
    let r = run_goal(&e, "inspect_recorded", "Explain how this project is laid out", Some(&m));

    assert_eq!(r.state, "completed", "{r:?}");
    assert_eq!(r.change_evidence, 0, "an explanation must not change anything");
    assert_eq!(r.failed_tool_calls, 0, "{r:?}");
    assert!(r.evidence >= 1 && r.evidence == r.tool_calls, "every call left evidence: {r:?}");
}

/// Record a scenario's responses from the real provider into a cassette.
///
/// `DEVD_EVAL_RECORD=1 DEEPSEEK_API_KEY=... cargo test -p devd-agent --test eval -- --ignored record_inspect`
#[test]
#[ignore = "talks to a real provider; set DEVD_EVAL_RECORD=1 and DEEPSEEK_API_KEY"]
fn record_inspect() {
    if std::env::var("DEVD_EVAL_RECORD").ok().as_deref() != Some("1") {
        return;
    }
    let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else {
        panic!("DEEPSEEK_API_KEY is not set");
    };
    let e = env();
    // A proxy that forwards to the provider and keeps only the responses.
    let recorded: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let keep = recorded.clone();
    let upstream_key = key.clone();
    let project_text = e.project.display().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let (head_end, len) = loop {
                let got = stream.read(&mut tmp).unwrap_or(0);
                if got == 0 {
                    break (buf.len(), 0);
                }
                buf.extend_from_slice(&tmp[..got]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    break (i + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let got = stream.read(&mut tmp).unwrap_or(0);
                if got == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..got]);
            }
            let mut body: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
            // Ask for one whole JSON answer, which is what a cassette stores.
            body["stream"] = json!(false);
            let answer = ureq::post("https://api.deepseek.com/v1/chat/completions")
                .set("authorization", &format!("Bearer {upstream_key}"))
                .set("content-type", "application/json")
                .send_string(&body.to_string())
                .map(|r| r.into_string().unwrap_or_default())
                .unwrap_or_else(|e| json!({ "error": e.to_string() }).to_string());
            keep.lock().unwrap().push(templatize(&answer, &project_text));
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                answer.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    add_provider(&e, &format!("{base}/v1"), &key, "deepseek-chat");
    let r = run_goal(&e, "inspect_recorded", "Explain how this project is laid out", None);
    let out = cassette_path("inspect_recorded");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, serde_json::to_string_pretty(&*recorded.lock().unwrap()).unwrap()).unwrap();
    println!("recorded {} response(s) to {} ({r:?})", recorded.lock().unwrap().len(), out.display());
}

/// The same goals against a real model. Opt in with `DEVD_EVAL_LIVE=1`.
///
/// What is asserted is that the record is truthful whatever the model did: a run
/// that claims `completed` has the evidence its policy demands, and one that does
/// not is `unverified`, never `completed`.
#[test]
#[ignore = "talks to a real provider; set DEVD_EVAL_LIVE=1 and DEEPSEEK_API_KEY"]
fn live_provider_scenarios() {
    if std::env::var("DEVD_EVAL_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else {
        panic!("DEEPSEEK_API_KEY is not set");
    };
    for (name, goal) in [
        ("live_inspect", "Explain how this project is laid out"),
        ("live_fix", "Fix the failing test: parser.txt should contain the word fixed, then run the project's test task to check"),
    ] {
        let e = env();
        add_provider(&e, "https://api.deepseek.com/v1", &key, "deepseek-chat");
        let r = run_goal(&e, name, goal, None);
        assert!(
            ["completed", "unverified", "failed"].contains(&r.state.as_str()),
            "{name}: a run settles in a terminal state: {r:?}"
        );
        if r.state == "completed" {
            assert!(r.missing.is_empty(), "{name}: completed with evidence outstanding: {r:?}");
        }
        let _ = e.root;
    }
}
