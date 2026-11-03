//! What devd does on its own, and the records that keep it honest.
//!
//! These tests are about the changed model of supervision:
//!
//! * every piece of team work carries a **mandate** - why it may run, to whom it
//!   was given, under which rules - so resuming can ask whether it still holds;
//! * a run that cannot continue answers with **one question** and the actions
//!   that resolve it, and the Inbox and the run view read the same answer;
//! * work devd would do is **proposed**, not done, and a proposal is one row
//!   until a person accepts it;
//! * the queue keeps what must reach outside the process, retries it with
//!   backoff, and reports a kind that keeps failing.
//!
//! They run through the real registry, so a failure names a rule of the product
//! rather than a detail of a mock.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use devd_agent::intake::Decision;
use devd_core::capability::Ctx;
use devd_core::{Actor, Approval, EventBus, Paths, Registry, Settings, Store, Surface};

struct Env {
    /// Kept alive for the whole test: the paths under it are what a capability
    /// resolves against.
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    reg: Arc<Registry>,
    human: Ctx,
    agent: Ctx,
}

fn ctx_for(root: &std::path::Path, store: Arc<Store>, events: EventBus, actor: Actor) -> Ctx {
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

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
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
    std::fs::create_dir_all(root.join("www").join("proj")).unwrap();
    Env {
        human: ctx_for(&root, store.clone(), events.clone(), Actor::human(Surface::Http)),
        agent: ctx_for(&root, store, events, Actor::agent("devd-team")),
        _dir: dir,
        root,
        reg,
    }
}

/// The project folder both the person and the agent work in.
fn project(e: &Env) -> String {
    e.root.join("www").join("proj").display().to_string()
}

fn human_call(e: &Env, id: &str, args: Value) -> devd_core::Result<Value> {
    let r = e.reg.dispatch(
        &e.human,
        &devd_core::InvokeRequest {
            id: id.into(),
            args,
            dry_run: false,
        },
        Approval::None,
    )?;
    Ok(serde_json::to_value(r.outcome.unwrap().data).unwrap_or(Value::Null))
}

fn agent_call(e: &Env, id: &str, args: Value) -> devd_core::Result<Value> {
    let r = e.reg.dispatch(
        &e.agent,
        &devd_core::InvokeRequest {
            id: id.into(),
            args,
            dry_run: false,
        },
        Approval::None,
    )?;
    Ok(serde_json::to_value(r.outcome.unwrap().data).unwrap_or(Value::Null))
}

// ---------------------------------------------------------------- suggestions

#[test]
fn a_suggestion_is_recorded_and_nothing_runs() {
    let e = env();
    let dir = project(&e);
    let before = e.human.store.list_runs(100).unwrap().len();
    let v = agent_call(
        &e,
        "team.suggest",
        json!({
            "project": dir,
            "role": "implementer",
            "title": "Split the parser",
            "why": "the parse function is 900 lines and changed in four of the last five commits",
            "plan": ["extract the tokenizer", "keep the tests passing"],
            "accepts": ["the parser test suite passes"],
        }),
    )
    .unwrap();
    assert_eq!(v["state"], "open");
    assert_eq!(v["role_title"], "Implementer");
    // A proposal is a row, not work: no run was created.
    assert_eq!(e.human.store.list_runs(100).unwrap().len(), before);
    let list = agent_call(&e, "team.suggestions", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["title"], "Split the parser");
}

#[test]
fn proposing_the_same_thing_twice_is_one_question() {
    let e = env();
    let dir = project(&e);
    let one = json!({ "project": dir, "title": "Add a health endpoint", "why": "the uptime monitor found no probe to call" });
    let a = agent_call(&e, "team.suggest", one.clone()).unwrap();
    let b = agent_call(&e, "team.suggest", one).unwrap();
    assert_eq!(a["id"], b["id"], "the second call points at the first proposal");
    assert_eq!(b["duplicate"], true);
    assert_eq!(
        agent_call(&e, "team.suggestions", json!({ "state": "open" }))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_suggestion_needs_a_reason_a_person_can_judge() {
    let e = env();
    let dir = project(&e);
    // A proposal with no observation behind it is refused before it is recorded.
    let err = agent_call(&e, "team.suggest", json!({ "project": dir, "title": "Do things", "why": "because" })).unwrap_err();
    assert!(err.to_string().contains("what you noticed"), "{err}");
    let err = agent_call(&e, "team.suggest", json!({ "project": dir, "title": "Do things" })).unwrap_err();
    assert!(err.code() == "invalid_input", "the schema refuses a proposal with no reason: {err}");
}

#[test]
fn accepting_a_suggestion_starts_a_run_that_still_waits_for_plan_approval() {
    let e = env();
    let dir = project(&e);
    let s = agent_call(&e, "team.suggest", json!({ "project": dir, "title": "Add a health endpoint", "why": "the monitor found no probe to call" })).unwrap();
    let id = s["id"].as_str().unwrap();
    // An agent cannot turn its own proposal into work.
    assert_eq!(
        agent_call(&e, "team.accept_suggestion", json!({ "id": id })).unwrap_err().code(),
        "permission_denied"
    );
    let v = human_call(&e, "team.accept_suggestion", json!({ "id": id })).unwrap();
    assert_eq!(v["run"]["state"], "planning", "the accepted proposal becomes a run that plans first");
    assert_eq!(
        human_call(&e, "team.suggestions", json!({ "state": "accepted" })).unwrap()[0]["run_id"],
        v["run"]["id"]
    );
}

#[test]
fn a_dismissed_suggestion_is_kept_as_a_record() {
    let e = env();
    let dir = project(&e);
    let s = agent_call(&e, "team.suggest", json!({ "project": dir, "title": "Rewrite in Rust", "why": "the current runtime is slower than the alternative benchmark" })).unwrap();
    let id = s["id"].as_str().unwrap();
    assert_eq!(
        agent_call(&e, "team.dismiss_suggestion", json!({ "id": id })).unwrap_err().code(),
        "permission_denied",
        "only a person decides"
    );
    human_call(&e, "team.dismiss_suggestion", json!({ "id": id })).unwrap();
    assert_eq!(e.human.store.get_suggestion(id).unwrap().unwrap().state, "dismissed");
}

// ------------------------------------------------------------- intake rules

#[test]
fn a_rule_starts_in_observe_mode_and_says_what_it_would_do() {
    let e = env();
    let dir = project(&e);
    let v = human_call(&e, "intake.set", json!({
        "project": dir,
        "name": "nightly",
        "trigger": "check.failed",
        "goal": "Make the nightly check pass again and add a regression test",
        "condition": { "min_failures": 3, "cooldown_minutes": 60 },
    }))
    .unwrap();
    assert_eq!(v["mode"], "observe", "a new rule observes until a person says otherwise");
    assert_eq!(v["condition_text"], "after 3 times in a row, at most once every 60 minutes");
    // Only a person writes a rule: it decides what devd may start on its own.
    assert_eq!(
        agent_call(&e, "intake.set", json!({ "project": dir, "name": "x", "trigger": "health.down", "goal": "restart the site now" })).unwrap_err().code(),
        "permission_denied"
    );
}

#[test]
fn a_rule_may_not_invent_an_event() {
    let e = env();
    let dir = project(&e);
    let err = human_call(&e, "intake.set", json!({ "project": dir, "name": "vibes", "trigger": "vibes.good", "goal": "do something nice for the team" })).unwrap_err();
    assert!(err.to_string().contains("not an event devd watches"), "{err}");
}

#[test]
fn a_rule_counts_before_it_proposes_and_proposes_once() {
    let e = env();
    let dir = project(&e);
    human_call(&e, "intake.set", json!({
        "project": dir,
        "name": "nightly",
        "trigger": "check.failed",
        "goal": "Make the nightly check pass again",
        "condition": { "min_failures": 2, "cooldown_minutes": 30 },
    }))
    .unwrap();
    e.human.store.record_intake(&dir, "nightly", 1, false).unwrap();
    let rule = e.human.store.list_intake_rules(Some(&dir)).unwrap()[0].clone();
    assert_eq!(rule.countdown, 1, "one event seen");
    // The second event reaches the count the rule asked for, and it acts.
    assert_eq!(
        devd_agent::intake::judge(&rule, rule.countdown, chrono::Utc::now()),
        Decision::Fire
    );
    devd_agent::intake::observe(&e.human, &rule, "the check exited 1").unwrap();
    let open = e.human.store.list_suggestions(Some("open"), 10).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].source, "intake");
    assert_eq!(open[0].source_ref.as_deref(), Some("nightly"));
    // Observing twice does not fill the inbox with the same proposal.
    devd_agent::intake::observe(&e.human, &rule, "the check exited 1 again").unwrap();
    assert_eq!(e.human.store.list_suggestions(Some("open"), 10).unwrap().len(), 1);
}

#[test]
fn a_rule_whose_condition_arrives_as_text_is_stored_as_an_object() {
    let e = env();
    let dir = project(&e);
    // The shell keeps the braces, so the value reaches devd as a string. It must
    // be read as JSON, not indexed as if it were an object: a panic here would
    // take the whole daemon down from a single bad argument.
    let v = human_call(
        &e,
        "intake.set",
        json!({
            "project": dir,
            "name": "nightly",
            "trigger": "health.down",
            "goal": "Restart the site and say why it stopped",
            "condition": "{\"min_failures\":3,\"cooldown_minutes\":15}",
        }),
    )
    .unwrap();
    assert_eq!(v["condition"]["min_failures"], 3);
    assert_eq!(v["min_failures"], 3);
    assert_eq!(v["condition_text"], "after 3 times in a row, at most once every 15 minutes");
    // And a condition that is not an object is refused with words, not a panic.
    let err = human_call(
        &e,
        "intake.set",
        json!({ "project": dir, "name": "second", "trigger": "health.down", "goal": "Restart the site now", "condition": "not json" }),
    )
    .unwrap_err();
    assert!(err.to_string().contains("condition"), "{err}");
}

#[test]
fn rules_are_listed_with_their_events_for_a_person_to_read() {
    let e = env();
    let dir = project(&e);
    human_call(&e, "intake.set", json!({ "project": dir, "name": "down", "trigger": "health.down", "goal": "Restart the site and say why it stopped" })).unwrap();
    let v = agent_call(&e, "intake.list", json!({ "project": dir })).unwrap();
    assert_eq!(v["rules"].as_array().unwrap().len(), 1);
    assert_eq!(v["rules"][0]["trigger_text"], "a watched site is down");
    assert_eq!(v["triggers"].as_array().unwrap().len(), 3);
}

// --------------------------------------------------------------- the mandate

#[test]
fn a_mandate_says_who_allowed_the_work_and_under_which_rules() {
    let e = env();
    let dir = project(&e);
    let run = e.human.store.create_run("Add search", "add search to the site", Some(&dir), 2, "human:http").unwrap();
    let version = devd_core::policy_version(&e.human.store, &run.id, &[]);
    let m = devd_core::Mandate::new("explicit", "human:http", &run.id, "implementer", &["src".to_string()], &version);
    assert_eq!(m.basis, "explicit");
    assert!(m.sentence().contains("http asked for this"));
    assert_eq!(m.role, "implementer");
    assert_eq!(m.scope, vec!["src".to_string()]);
    assert_eq!(
        devd_core::Mandate::new("schedule", "human:http", &run.id, "qa", &[], &version).sentence(),
        "a schedule set up by http asked for this"
    );
    assert!(devd_core::Mandate::new("inferred", "agent:devd-team", &run.id, "qa", &[], &version)
        .sentence()
        .contains("a person accepted it"));
    assert!(devd_core::Mandate::new("system", "devd:harness", &run.id, "qa", &[], &version)
        .sentence()
        .contains("verify the run"));
}

#[test]
fn a_policy_version_changes_when_the_rules_the_run_was_accepted_under_change() {
    let store = Store::open_in_memory().unwrap();
    let run = store.create_run("Add search", "add search", None, 2, "human:http").unwrap();
    let before = devd_core::policy_version(&store, &run.id, &[]);
    // The same rules give the same version, which is what makes a comparison mean something.
    assert_eq!(before, devd_core::policy_version(&store, &run.id, &[]));
    // A new definition of done is a different set of rules.
    store.set_goal_loop(&run.id, &["the search box returns results".to_string()], 2).unwrap();
    let after = devd_core::policy_version(&store, &run.id, &[]);
    assert_ne!(before, after);
    // So is a change to the working agreement.
    assert_ne!(after, devd_core::policy_version(&store, &run.id, &["tests first".to_string()]));
}

// ------------------------------------------------------------------- the queue

#[test]
fn a_notification_is_a_row_that_is_retried_and_can_fail_visibly() {
    let e = env();
    // A target that wants health events and cannot be reached.
    e.human
        .store
        .ops_put("notify", "t1", "t1", &json!({
            "channel": "webhook",
            "url": "https://127.0.0.1:9/hook",
            "events": ["health.down"],
        }))
        .unwrap();
    devd_ops::notify::send(&e.human, "health.down", "blog", "blog is down", json!({}));
    let now = (chrono::Utc::now() + chrono::Duration::seconds(2)).to_rfc3339();
    let due = e.human.store.due_effects(&now, 10).unwrap();
    assert_eq!(due.len(), 1, "the message is queued, not lost");
    assert_eq!(due[0].kind, "notify.send");
    assert_eq!(due[0].payload["site"], "blog");
    // Delivering it fails, and the row records that it will be tried again.
    let never = |_: &Ctx, _: &devd_core::Schedule| Err::<(), String>("not used".into());
    assert_eq!(devd_ops::effects::tick(&e.human, &never), 1);
    let later = (chrono::Utc::now() + chrono::Duration::minutes(1)).to_rfc3339();
    let due = e.human.store.due_effects(&later, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert!(due[0].attempts >= 1);
    assert!(due[0].last_error.is_some(), "the failure is visible, not swallowed");
    // It is not treated as a problem until it has failed three times in a row: a
    // one-off network error is not an unhealthy devd.
    let row = |e: &Env| {
        devd_ops::effects::health(&e.human)
            .into_iter()
            .find(|r| r["kind"] == "notify.send")
            .expect("a health entry per kind")
    };
    assert_eq!(row(&e)["failing"], 0);
    // Keep failing it, as the backoff would.
    for _ in 0..2 {
        e.human
            .store
            .fail_effect("notify.send", &due[0].key, "still refused", &later)
            .unwrap();
    }
    let health = row(&e);
    assert_eq!(health["state"], "error", "three failures in a row is a problem worth telling a person about");
    assert_eq!(health["failing"], 1);
}

#[test]
fn a_queue_that_keeps_working_is_not_reported_as_unhealthy() {
    let e = env();
    e.human.store.enqueue_effect("notify.send", "k", &json!({})).unwrap();
    e.human.store.settle_effect("notify.send", "k").unwrap();
    let health = devd_ops::effects::health(&e.human);
    let row = health.iter().find(|r| r["kind"] == "notify.send").unwrap();
    assert_eq!(row["state"], "ok", "a finished row is not a problem");
    assert_eq!(row["failing"], 0);
}

// -------------------------------------------------------------- the question

#[test]
fn a_paused_run_asks_one_question_and_the_inbox_and_run_view_read_the_same_one() {
    let e = env();
    let dir = project(&e);
    let run = e.human.store.create_run("Add search", "add search", Some(&dir), 1, "human:http").unwrap();
    let a = e.human.store.add_task(&run.id, devd_core::store_team::NewTask { key: "search", role: "implementer", title: "Add the search box", instructions: "add a search box and prove it with the tests", ..Default::default() }).unwrap();
    e.human.store.update_team_task(&a.id, devd_core::store_team::TaskUpdate { state: "running", ..Default::default() }).unwrap();
    e.human.store.update_team_task(&a.id, devd_core::store_team::TaskUpdate { state: "unverified", error: Some("no check passed"), evidence: Some(&json!([{ "kind": "change", "ok": true }])), ..Default::default() }).unwrap();
    e.human.store.set_run_state(&run.id, "paused", None, Some("Needs you: search (unverified): no check passed")).unwrap();
    // `team.get` is what both screens read.
    let v = agent_call(&e, "team.get", json!({ "run": run.id })).unwrap();
    let needs = &v["needs"];
    assert_eq!(needs["reason"], "not-proved");
    assert_eq!(needs["task_key"], "search");
    let actions: Vec<&str> = needs["actions"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
    assert_eq!(actions[0], "retry", "the first action is the one that unblocks the work");
    assert!(actions.contains(&"accept"));
    assert!(actions.contains(&"look"));
    // Every action that changes something names the capability that does it.
    for a in needs["actions"].as_array().unwrap() {
        if a["id"] == "retry" || a["id"] == "accept" {
            assert!(a["capability"].as_str().is_some_and(|c| c.starts_with("team.")), "{a}");
        }
    }
}

#[test]
fn a_healthy_run_has_no_question() {
    let e = env();
    let dir = project(&e);
    let run = e.human.store.create_run("Add search", "add search", Some(&dir), 1, "human:http").unwrap();
    let v = agent_call(&e, "team.get", json!({ "run": run.id })).unwrap();
    assert!(v["needs"].is_null());
}

// ------------------------------------------------------------- the heartbeat

#[test]
fn a_task_heartbeat_is_the_only_thing_that_moves_and_resuming_resets_it() {
    let e = env();
    let dir = project(&e);
    let run = e.human.store.create_run("Add search", "add search", Some(&dir), 1, "human:http").unwrap();
    let t = e.human.store.add_task(&run.id, devd_core::store_team::NewTask { key: "search", role: "implementer", title: "Add the search box", instructions: "add a search box and prove it with the tests", ..Default::default() }).unwrap();
    assert!(t.last_step_at.is_none(), "a task that has not started has no heartbeat");
    e.human.store.update_team_task(&t.id, devd_core::store_team::TaskUpdate { state: "running", ..Default::default() }).unwrap();
    let running = e.human.store.get_team_task(&t.id).unwrap().unwrap();
    assert!(running.last_step_at.is_some(), "starting the task is the first step");
    assert!(!devd_agent::team::stalled(&running, chrono::Utc::now(), Duration::from_secs(3600)), "a task that just started is not stalled");
    // A task that has done nothing for longer than the window is.
    let old = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    let mut t = running.clone();
    t.last_step_at = Some(old.clone());
    assert!(devd_agent::team::stalled(&t, chrono::Utc::now(), Duration::from_secs(3600)));
    assert_eq!(e.human.store.bump_stall(&t.id).unwrap(), 1);
    assert_eq!(e.human.store.bump_stall(&t.id).unwrap(), 2);
    // A person resuming the task starts the count again.
    e.human.store.reset_stalls(&t.id).unwrap();
    let reset = e.human.store.get_team_task(&t.id).unwrap().unwrap();
    assert_eq!(reset.stalls, 0);
    assert!(reset.last_step_at.is_none());
}

#[test]
fn the_watchdog_waits_longer_than_a_model_call_is_allowed_to_take() {
    // A worker waiting on its own provider is slow, not stuck: the watchdog's
    // window is a multiple of the provider's own timeout, so it cannot fire on a
    // model that is still answering.
    assert_eq!(devd_agent::team::STALL_FACTOR, 2);
    assert!(devd_agent::team::STALL_FACTOR as u64 * devd_agent::providers::READ_TIMEOUT_SECS > devd_agent::providers::READ_TIMEOUT_SECS, "the window must exceed one provider timeout");
    assert!(devd_agent::team::STALL_FLOOR <= Duration::from_secs(600));
}

