//! The evaluation suite for the run, evidence and approval model.
//!
//! These tests are deterministic and offline. They exercise the same public
//! boundary the HTTP API, MCP server and assistant cross: one registry dispatch,
//! one durable run, one approval row, one event log. They do not call a model and
//! they do not touch the network.
//!
//! What is covered:
//!
//! * a workflow's calls are recorded against a run with normalized evidence;
//! * arguments that do not match the declared schema never reach a handler;
//! * a cancelled run and an expired deadline stop work centrally;
//! * an idempotency key replays the first success instead of repeating it;
//! * approvals are durable, single-use, expiring and declined explicitly;
//! * run events replay from a cursor and page backwards without duplication;
//! * a failed run can be continued but a cancelled one cannot be reopened;
//! * acceptance criteria force verification, and a claim alone does not pass.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use devd_core::capability::{
    Actor, Approval, Ctx, Effect, InvokeRequest, Outcome, Plan, Surface, Tier,
};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::runs::{
    evaluate, evidence_for, CompletionPolicy, EvidenceKind, RunContext, RunMode, RunState,
};
use devd_core::state::Store;
use devd_core::store_runs::{NewApproval, NewEvidence, RunSpec};
use devd_core::{DispatchOptions, Registry};

#[derive(Debug, Deserialize, JsonSchema)]
struct CountArgs {
    n: i64,
}

fn fixture() -> (Arc<Store>, Ctx, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::rooted(dir.path().join("devd"));
    paths.ensure().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.migrate().unwrap();
    let ctx = Ctx {
        store: store.clone(),
        paths: Arc::new(paths),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        run: None,
        dry_run: false,
    };
    (store, ctx, dir)
}

fn start(store: &Store, goal: &str) -> String {
    store
        .start_run(RunSpec {
            goal,
            mode: RunMode::Api,
            project: None,
            actor: "human:test",
            thread_id: None,
            team_run_id: None,
            acceptance: &[],
        })
        .unwrap()
        .id
}

fn registry(counter: Arc<AtomicUsize>) -> Registry {
    let mut reg = Registry::new();
    reg.define::<CountArgs>("fs.write", "Record a change")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .handlers(
            |_, a| Ok(Plan::new("fs.write", format!("change by {}", a.n))),
            |_, a| Ok(Outcome::with_data("changed", json!({ "n": a.n }))),
        )
        .unwrap();
    reg.define::<CountArgs>("eval.count", "Count once")
        .tier(Tier::Write)
        .effect(Effect::Write)
        .agent_safe()
        .idempotent()
        .handlers(
            |_, a| Ok(Plan::new("eval.count", format!("count {}", a.n))),
            move |_, a| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Outcome::with_data("counted", json!({ "n": a.n })))
            },
        )
        .unwrap();
    reg.define::<CountArgs>("project.run", "Check the change")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .handlers(
            |_, _| Ok(Plan::new("project.run", "verify")),
            |_, _| {
                Ok(Outcome::with_data(
                    "checked",
                    json!({ "task": "test", "exit_code": 0 }),
                ))
            },
        )
        .unwrap();
    reg
}

fn request(id: &str, args: serde_json::Value) -> InvokeRequest {
    InvokeRequest {
        id: id.to_string(),
        args,
        dry_run: false,
    }
}

#[test]
fn a_workflow_records_calls_and_only_real_evidence_completes_it() {
    let (store, ctx, _dir) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let reg = registry(calls.clone());
    let run_id = start(&store, "fix the parser");
    let step = store
        .add_step(&run_id, "task", "change and verify", &json!({}))
        .unwrap();

    let ctx = Ctx {
        run: Some(RunContext::step(&run_id, &step.id).with_store(store.clone())),
        ..ctx
    };
    let change = reg
        .dispatch(
            &ctx,
            &request("fs.write", json!({ "n": 1 })),
            Approval::Granted,
        )
        .unwrap();
    assert!(!change.dry_run);

    // The registry normalizes the outcome into evidence at the boundary.
    let draft = evidence_for("fs.write", &json!({ "n": 1 }), change.outcome.as_ref());
    assert_eq!(draft.kind, EvidenceKind::Change);
    assert!(draft.passed);

    let policy = CompletionPolicy::for_goal("fix the parser", &[]);
    let after_change = evaluate(&policy, &store.list_evidence(&run_id, 100).unwrap());
    assert_eq!(
        after_change.state,
        RunState::Unverified,
        "a change without a check is not completion"
    );

    // A verification is normalized from what it observed (`exit_code`), not
    // from the model saying that it checked.
    let verify = reg
        .dispatch(
            &ctx,
            &request("project.run", json!({ "n": 0, "task": "test" })),
            Approval::Granted,
        )
        .unwrap();
    let verify_draft = evidence_for(
        "project.run",
        &json!({ "task": "test" }),
        verify.outcome.as_ref(),
    );
    assert_eq!(verify_draft.kind, EvidenceKind::Verify);
    assert!(verify_draft.passed);
    let verdict = evaluate(&policy, &store.list_evidence(&run_id, 100).unwrap());
    assert!(verdict.passed(), "{:?}", verdict.missing);

    let timeline = store.run_timeline(&run_id).unwrap();
    assert_eq!(
        timeline["tool_calls"].as_array().unwrap().len(),
        2,
        "{timeline}"
    );
    assert_eq!(timeline["evidence"].as_array().unwrap().len(), 2);
    assert_eq!(timeline["steps"].as_array().unwrap().len(), 1);
    store.finish_step(&step.id, "done", None).unwrap();
}

#[test]
fn malformed_arguments_are_refused_before_the_handler() {
    let (store, ctx, _dir) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let reg = registry(calls.clone());
    let run_id = start(&store, "count");
    let ctx = Ctx {
        run: Some(RunContext::new(&run_id).with_store(store.clone())),
        ..ctx
    };

    let err = reg
        .dispatch(
            &ctx,
            &request("eval.count", json!({ "n": "not a number" })),
            Approval::Granted,
        )
        .expect_err("bad arguments must fail");
    assert!(
        matches!(err, devd_core::DevdError::Invalid { .. }),
        "{err:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the handler never ran");
}

#[test]
fn cancellation_and_deadlines_stop_work_at_the_boundary() {
    let (store, ctx, _dir) = fixture();
    let reg = registry(Arc::new(AtomicUsize::new(0)));
    let run_id = start(&store, "work");
    store
        .request_run_cancel(&run_id, "human:test", Some("stop"))
        .unwrap();

    let ctx = Ctx {
        run: Some(RunContext::new(&run_id).with_store(store.clone())),
        ..ctx
    };
    let err = reg
        .dispatch(
            &ctx,
            &request("fs.write", json!({ "n": 1 })),
            Approval::Granted,
        )
        .expect_err("a stopped run cannot start work");
    assert!(matches!(err, devd_core::DevdError::Denied(_)), "{err:?}");

    let run_id = start(&store, "later");
    let mut run = RunContext::new(&run_id);
    run.deadline = Some(Instant::now() - Duration::from_secs(1));
    let ctx = Ctx {
        run: Some(run),
        ..ctx
    };
    let err = reg
        .dispatch(
            &ctx,
            &request("fs.write", json!({ "n": 1 })),
            Approval::Granted,
        )
        .expect_err("an expired deadline cannot start work");
    assert!(matches!(err, devd_core::DevdError::Timeout(_)), "{err:?}");

    // The local flag is honoured even without a store attached.
    let run_id = start(&store, "flagged");
    let flag = Arc::new(AtomicBool::new(true));
    let mut run = RunContext::new(&run_id);
    run.cancel = flag;
    let ctx = Ctx {
        run: Some(run),
        ..ctx
    };
    assert!(reg
        .dispatch(
            &ctx,
            &request("fs.write", json!({ "n": 1 })),
            Approval::Granted
        )
        .is_err());
}

#[test]
fn an_idempotency_key_replays_the_first_success() {
    let (store, ctx, _dir) = fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let reg = registry(calls.clone());
    let run_id = start(&store, "count once");
    let ctx = Ctx {
        run: Some(RunContext::new(&run_id).with_store(store.clone())),
        ..ctx
    };
    let opts = DispatchOptions {
        idempotency_key: Some("count-key"),
        ..Default::default()
    };

    let first = reg
        .dispatch_with(
            &ctx,
            &request("eval.count", json!({ "n": 7 })),
            Approval::Granted,
            &opts,
        )
        .unwrap();
    let second = reg
        .dispatch_with(
            &ctx,
            &request("eval.count", json!({ "n": 7 })),
            Approval::Granted,
            &opts,
        )
        .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1, "the side effect ran once");
    assert_eq!(first.outcome.unwrap().data, second.outcome.unwrap().data);
    // The replay is still one authoritative call in the timeline, not two.
    assert_eq!(store.list_tool_calls(&run_id, 20).unwrap().len(), 1);
}

#[test]
fn approvals_are_durable_single_use_and_expire() {
    let (store, _ctx, _dir) = fixture();
    let run_id = start(&store, "deploy");
    let args = json!({ "target": "box" });
    let hash = devd_core::util::sha256_hex(args.to_string().as_bytes());
    let (token, record) = store
        .create_approval(NewApproval {
            run_id: Some(&run_id),
            task_id: None,
            tool_call_id: None,
            capability: "deploy.run",
            args: &args,
            args_hash: &hash,
            plan: None,
            plan_hash: None,
            scope: "single",
            tier: "admin",
            risk: "sensitive",
            actor: "agent:test",
            requested_by: "agent:test",
        })
        .unwrap();
    assert_eq!(record.state, "pending");
    assert!(!record.id.contains(&token), "the token is not stored");

    let used = store.consume_approval(&token).unwrap();
    assert_eq!(used.state, "consumed");
    let again = store
        .consume_approval(&token)
        .expect_err("a token is single use");
    assert!(
        matches!(again, devd_core::DevdError::Denied(_)),
        "{again:?}"
    );

    // A second request can be declined, and a declined row cannot be consumed.
    let (token2, record2) = store
        .create_approval(NewApproval {
            run_id: Some(&run_id),
            task_id: None,
            tool_call_id: None,
            capability: "deploy.run",
            args: &args,
            args_hash: &hash,
            plan: None,
            plan_hash: None,
            scope: "single",
            tier: "admin",
            risk: "sensitive",
            actor: "agent:test",
            requested_by: "agent:test",
        })
        .unwrap();
    store
        .decline_approval(&record2.id, "human:test", Some("not now"))
        .unwrap();
    assert!(store.consume_approval(&token2).is_err());

    // Expiry is checked when the decision is made, not only by a sweeper.
    let (token3, record3) = store
        .create_approval(NewApproval {
            run_id: Some(&run_id),
            task_id: None,
            tool_call_id: None,
            capability: "deploy.run",
            args: &args,
            args_hash: &hash,
            plan: None,
            plan_hash: None,
            scope: "single",
            tier: "admin",
            risk: "sensitive",
            actor: "agent:test",
            requested_by: "agent:test",
        })
        .unwrap();
    store
        .reschedule_approval(&record3.id, "2000-01-01T00:00:00Z")
        .unwrap();
    let expired = store
        .consume_approval(&token3)
        .expect_err("expired approval");
    assert!(
        matches!(expired, devd_core::DevdError::Denied(_)),
        "{expired:?}"
    );
    assert_eq!(
        store.get_approval(&record3.id).unwrap().unwrap().state,
        "expired"
    );
}

#[test]
fn run_events_replay_both_ways_without_gaps_or_duplicates() {
    let (store, _ctx, _dir) = fixture();
    let run_id = start(&store, "timeline");
    for i in 0..10 {
        store
            .append_run_event(
                &run_id,
                None,
                "step",
                format!("event {i}"),
                json!({ "i": i }),
            )
            .unwrap();
    }

    let first = store.list_run_events(&run_id, 0, 4).unwrap();
    assert_eq!(first.len(), 4);
    let second = store
        .list_run_events(&run_id, first.last().unwrap().seq, 4)
        .unwrap();
    assert_eq!(second.len(), 4);
    assert!(second.first().unwrap().seq > first.last().unwrap().seq);

    let tail = store.tail_run_events(&run_id, 4).unwrap();
    let before = store
        .list_run_events_before(&run_id, tail.first().unwrap().seq, 4)
        .unwrap();
    assert_eq!(before.last().unwrap().seq, tail.first().unwrap().seq - 1);
    assert!(before.iter().all(|e| e.seq < tail[0].seq));

    // The global cursor is stable across runs and advances monotonically.
    let all = store.events_after(0, 100).unwrap();
    assert!(all.windows(2).all(|w| w[0].id < w[1].id));
    let after = store.events_after(all[3].id, 100).unwrap();
    assert_eq!(after.len(), all.len() - 4);
    assert_eq!(after[0].id, all[4].id);
}

#[test]
fn recovery_follows_the_lifecycle_and_cancelled_stays_closed() {
    let (store, _ctx, _dir) = fixture();
    let failed = start(&store, "failed work");
    store
        .transition_run(&failed, RunState::Executing, None)
        .unwrap();
    store
        .transition_run(&failed, RunState::Failed, Some("the test failed"))
        .unwrap();
    // A person continuing a failed run is allowed.
    let resumed = store
        .transition_run(&failed, RunState::Executing, None)
        .unwrap();
    assert_eq!(resumed.state, "executing");

    let cancelled = start(&store, "cancelled work");
    store
        .request_run_cancel(&cancelled, "human:test", Some("stop"))
        .unwrap();
    store
        .transition_run(&cancelled, RunState::Cancelled, None)
        .unwrap();
    let reopened = store.transition_run(&cancelled, RunState::Executing, None);
    assert!(reopened.is_err(), "a cancelled run is never reopened");

    // One conversation has one current run; asking again returns the same row.
    let a = store
        .ensure_run_for_thread("thread-1", "first", None, "human:test")
        .unwrap();
    let b = store
        .ensure_run_for_thread("thread-1", "again", None, "human:test")
        .unwrap();
    assert_eq!(a.id, b.id);
}

#[test]
fn acceptance_criteria_are_required_evidence_not_a_claim() {
    let (store, _ctx, _dir) = fixture();
    let acceptance = vec![
        "the parser accepts the new syntax".to_string(),
        "the old syntax still works".to_string(),
    ];
    let policy = CompletionPolicy::for_goal("implement the new syntax", &acceptance);
    assert!(policy.requires_change && policy.requires_verify);
    assert_eq!(policy.acceptance.len(), 2);

    let run_id = start(&store, "implement the new syntax");
    store
        .add_evidence(NewEvidence {
            run_id: &run_id,
            step_id: None,
            tool_call_id: None,
            artifact_id: None,
            kind: "change",
            capability: "fs.edit",
            passed: true,
            claim: Some("the parser accepts the new syntax"),
            summary: "edited the parser",
            detail: None,
        })
        .unwrap();
    // A model's sentence that it verified the change is not evidence of it.
    store
        .add_evidence(NewEvidence {
            run_id: &run_id,
            step_id: None,
            tool_call_id: None,
            artifact_id: None,
            kind: "structured",
            capability: "chat.reply",
            passed: true,
            claim: Some("the old syntax still works"),
            summary: "the model said it checked",
            detail: None,
        })
        .unwrap();
    let verdict = evaluate(&policy, &store.list_evidence(&run_id, 100).unwrap());
    assert_eq!(verdict.state, RunState::Unverified);
    assert!(
        verdict.missing.iter().any(|m| m.contains("verification")),
        "{:?}",
        verdict.missing
    );

    store
        .add_evidence(NewEvidence {
            run_id: &run_id,
            step_id: None,
            tool_call_id: None,
            artifact_id: None,
            kind: "verify",
            capability: "project.run",
            passed: true,
            claim: Some("the old syntax still works"),
            summary: "the parser test suite exited 0",
            detail: Some("exit 0"),
        })
        .unwrap();
    let verdict = evaluate(&policy, &store.list_evidence(&run_id, 100).unwrap());
    assert!(verdict.passed(), "{:?}", verdict.missing);
    // The acceptance criteria stay attached to the run, not only the policy.
    store
        .set_run_plan(&run_id, &json!({ "steps": [] }), Some("fingerprint"))
        .unwrap();
    let run = store.run_by_id(&run_id).unwrap().unwrap();
    assert!(run.plan_fingerprint.is_some());
}
