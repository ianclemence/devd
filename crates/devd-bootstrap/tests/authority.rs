//! Who may do what, checked against the whole registry that ships, not a toy one.
//!
//! The registry's approval rules are enforced in one place (`Registry::dispatch`), so these
//! tests go through it, as every interface does, with the actor an agent would have.

use devd_bootstrap::Bootstrap;
use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Surface};
use devd_core::paths::Paths;
use serde_json::{json, Value};

fn booted(tag: &str) -> Bootstrap {
    let dir = std::env::temp_dir().join(format!("devd-authority-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Bootstrap::at(Paths::rooted(dir)).expect("a fresh home boots")
}

fn as_actor(app: &Bootstrap, actor: Actor) -> Ctx {
    Ctx {
        store: app.store.clone(),
        paths: std::sync::Arc::new(app.paths.clone()),
        settings: app.settings.clone(),
        events: app.events.clone(),
        actor,
        dry_run: false,
    }
}

fn call(app: &Bootstrap, ctx: &Ctx, id: &str, args: Value, dry_run: bool, approval: Approval) -> devd_core::Result<devd_core::capability::InvokeResult> {
    app.registry.dispatch(ctx, &InvokeRequest { id: id.to_string(), args, dry_run }, approval)
}

/// A site with a folder, so operations that name a site have one.
fn with_site(app: &Bootstrap, name: &str) {
    let dir = app.paths.root().join("www").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.php"), "<?php").unwrap();
    app.store
        .upsert_site(&devd_core::state::Site {
            id: name.into(),
            name: name.into(),
            host: format!("{name}.test"),
            root: dir.display().to_string(),
            web_server: "nginx".into(),
            php: None,
            serving: None,
            tls_mode: "off".into(),
            db_name: None,
            upstream: None,
            created_at: "2026-10-24T00:00:00Z".into(),
        })
        .unwrap();
}

#[test]
fn an_agent_cannot_have_devd_run_a_command_it_wrote_without_a_persons_approval() {
    let app = booted("code");
    with_site(&app, "shop");
    let agent = as_actor(&app, Actor::agent("assistant"));

    for (id, args) in [
        ("cron.add", json!({ "site": "shop", "schedule": "* * * * *", "command": "echo hi" })),
        ("deploy.script_set", json!({ "site": "shop", "script": "echo hi" })),
        ("queue.add", json!({ "site": "shop", "name": "w", "command": "echo hi" })),
    ] {
        let err = call(&app, &agent, id, args.clone(), false, Approval::None).unwrap_err();
        assert!(err.is_approval_required(), "`{id}` ran for an agent without approval: {err:?}");

        // The plan, which shows the code, needs no approval: that is what the person reads.
        let plan = call(&app, &agent, id, args.clone(), true, Approval::None);
        assert!(plan.is_ok(), "`{id}` cannot be previewed: {:?}", plan.err());
        assert!(app.store.ops_list("cron", None).unwrap().is_empty(), "a refused call changed nothing");
    }
}

#[test]
fn the_same_operations_run_for_the_person_who_wrote_the_command() {
    let app = booted("person");
    with_site(&app, "shop");
    let person = as_actor(&app, Actor::human(Surface::Cli));
    let done = call(&app, &person, "cron.add", json!({ "site": "shop", "schedule": "* * * * *", "command": "echo hi" }), false, Approval::None);
    assert!(done.is_ok(), "a person is not asked to approve their own command: {:?}", done.err());
}

#[test]
fn once_a_person_approves_the_agents_call_it_runs() {
    let app = booted("approved");
    with_site(&app, "shop");
    let agent = as_actor(&app, Actor::agent("assistant"));
    let args = json!({ "site": "shop", "schedule": "*/5 * * * *", "command": "echo hi" });
    assert!(call(&app, &agent, "cron.add", args.clone(), false, Approval::None).is_err());
    assert!(call(&app, &agent, "cron.add", args, false, Approval::Granted).is_ok());
    assert_eq!(app.store.ops_list("cron", Some("shop")).unwrap().len(), 1);
}

#[test]
fn a_denied_call_never_reaches_its_handler() {
    let app = booted("denied");
    with_site(&app, "shop");
    let agent = as_actor(&app, Actor::agent("assistant"));
    let err = call(&app, &agent, "cron.add", json!({ "site": "shop", "schedule": "* * * * *", "command": "echo hi" }), false, Approval::Denied).unwrap_err();
    assert!(matches!(err, devd_core::DevdError::Denied(_)), "{err:?}");
    assert!(app.store.ops_list("cron", Some("shop")).unwrap().is_empty());
}

/// Any capability an agent may call that takes a command, script or program to run is either
/// gated or has been reviewed and is named here. A new one that is neither fails this test,
/// which is the point: the tier alone does not say whose code runs.
#[test]
fn every_agent_callable_capability_that_takes_code_is_gated_or_reviewed() {
    // Reviewed, and why they stay open to agents:
    //  - project.exec / project.run: the stack's own allow-list and the project's own tasks; the
    //    core of an agent's work loop. Interpreters on the allow-list can run inline code, which
    //    is recorded as an open decision in docs/security.md rather than silently accepted.
    //  - remote.steps_set: only stored; the steps run at remote.deploy, which is approval-gated and
    //    shows them in its plan.
    //  - project.init, template.run, team.start: take a goal or a stack name, not code.
    //  - nginx.site_set: directives are checked by `nginx -t`, and `include`, `listen`, certificates
    //    and anything that runs code inside nginx are refused (siteconf.rs).
    //  - remote.link: stores build steps that run at remote.deploy, which is approval-gated and
    //    shows them in its plan.
    //  - shell.resolve: its `command` is `php` or `composer`, and it runs nothing.
    let reviewed = [
        "project.exec",
        "project.run",
        "remote.steps_set",
        "project.init",
        "template.run",
        "team.start",
        "team.assign",
        "review.run",
        "nginx.site_set",
        "remote.link",
        "shell.resolve",
    ];
    let words = ["command", "script", "steps", "argv", "program", "directives"];
    let app = booted("structure");
    let mut open: Vec<String> = Vec::new();
    for cap in app.registry.list() {
        if !cap.agent_safe || cap.human_only || cap.agent_needs_approval || cap.tier.requires_approval() {
            continue;
        }
        let props = cap.args_schema.get("properties").and_then(|p| p.as_object());
        let takes_code = props.is_some_and(|p| p.keys().any(|k| words.contains(&k.as_str())));
        if takes_code && !reviewed.contains(&cap.id.as_str()) {
            open.push(cap.id.clone());
        }
    }
    assert!(
        open.is_empty(),
        "these capabilities let an agent supply code that devd runs, with no approval: {open:?}. Mark them .agent_needs_approval(), or review and list them here."
    );
}
