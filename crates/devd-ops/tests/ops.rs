//! The operations against a real scratch site: a folder, a store, and the functions the capabilities call.

use std::sync::Arc;
use std::time::{Duration, Instant};

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::{Site, Store};
use devd_ops::{cron, daemons, deploy, env, notify};

fn ctx(tag: &str) -> (Ctx, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("devd-ops-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(&dir);
    paths.ensure().unwrap();
    let project = dir.join("www").join("shop");
    std::fs::create_dir_all(project.join("public")).unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        run: None,
        dry_run: false,
    };
    ctx.store
        .upsert_site(&Site {
            id: "id-1".into(),
            name: "shop".into(),
            host: "shop.test".into(),
            root: project.display().to_string(),
            web_server: "nginx".into(),
            php: None,
            tls_mode: "off".into(),
            db_name: None,
            upstream: None,
            serving: None,
            created_at: "2026-10-23T00:00:00Z".into(),
        })
        .unwrap();
    (ctx, project)
}

fn wait_for_deploy(ctx: &Ctx, id: &str) -> deploy::Deployment {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let d = deploy::get(ctx, id).unwrap();
        if d.state != "running" || Instant::now() > until {
            return d;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn the_env_file_is_changed_in_place_with_a_backup_and_secrets_stay_hidden() {
    let (ctx, root) = ctx("env");
    std::fs::write(
        root.join(".env"),
        "# shop\nAPP_ENV=local\nDB_PASSWORD=hunter2\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(env::path_of(&root)).unwrap();
    let set = std::collections::BTreeMap::from([("APP_ENV".to_string(), "production".to_string())]);
    let backup = env::write_with_backup(&root, &env::with_set(&text, &set).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read_to_string(backup).unwrap(), text);
    let now = env::variables(&std::fs::read_to_string(env::path_of(&root)).unwrap());
    assert_eq!(now["APP_ENV"], "production");
    assert_eq!(
        env::masked("DB_PASSWORD", &now["DB_PASSWORD"]),
        "[hidden, 7 characters]"
    );
    let _ = ctx;
}

#[test]
fn a_deploy_script_is_saved_run_recorded_and_notified() {
    let (ctx, root) = ctx("deploy");
    // The default script follows the project: this one has nothing to install, so it only says it is done.
    let (default, saved) = deploy::script(&ctx, "shop").unwrap();
    assert!(!saved && default.contains("Application deployed!"));

    deploy::set_script(
        &ctx,
        "shop",
        "# a deploy\ncd /somewhere/on/a/server\necho first > out.txt\necho second >> out.txt\n",
    )
    .unwrap();
    assert!(deploy::script(&ctx, "shop").unwrap().1);

    notify::add(
        &ctx,
        "inbox",
        None,
        &["deploy.finished".to_string(), "deploy.failed".to_string()],
        None,
    )
    .unwrap();
    let started = deploy::start(&ctx, "shop", "manual", None).unwrap();
    assert_eq!(started.state, "running");
    assert!(
        deploy::start(&ctx, "shop", "manual", None).is_err(),
        "one deploy at a time"
    );
    let done = wait_for_deploy(&ctx, &started.id);
    assert_eq!(
        done.state,
        "success",
        "{:?}",
        deploy::log(&ctx, &started.id, 50)
    );
    let out = std::fs::read_to_string(root.join("out.txt")).unwrap();
    assert!(out.contains("first") && out.contains("second"));
    let log = deploy::log(&ctx, &started.id, 50).unwrap().join("\n");
    assert!(log.contains("skipped: cd /somewhere/on/a/server"), "{log}");
    assert_eq!(deploy::history(&ctx, "shop", 10).unwrap().len(), 1);
    assert!(
        ctx.store
            .list_inbox(false, 10)
            .unwrap()
            .iter()
            .any(|m| m.subject.starts_with("deploy.finished")),
        "the inbox was told"
    );
}

#[test]
fn a_failing_line_stops_the_deploy_and_names_itself() {
    let (ctx, root) = ctx("fail");
    let bad = if cfg!(windows) { "exit /b 3" } else { "exit 3" };
    deploy::set_script(
        &ctx,
        "shop",
        &format!("echo before > before.txt\n{bad}\necho after > after.txt\n"),
    )
    .unwrap();
    let d = deploy::start(&ctx, "shop", "manual", None).unwrap();
    let done = wait_for_deploy(&ctx, &d.id);
    assert_eq!(done.state, "failed");
    assert_eq!(done.failed_line.as_deref(), Some(bad));
    assert!(
        root.join("before.txt").is_file() && !root.join("after.txt").exists(),
        "the line after the failure never ran"
    );
}

#[test]
fn the_push_hook_needs_the_secret_and_is_off_until_turned_on() {
    let (ctx, root) = ctx("hook");
    assert!(
        deploy::set_hook(&ctx, "shop", true).is_err(),
        "link the repository first"
    );
    // A repository link needs a Git repository.
    assert!(deploy::repo_link(&ctx, "shop", None, Some("main")).is_err());
    std::fs::create_dir_all(root.join(".git")).unwrap();
    ctx.store
        .ops_put(
            deploy::REPO,
            "shop",
            "shop",
            &serde_json::json!({ "branch": "main", "deploy_on_push": false, "hook_token": null }),
        )
        .unwrap();
    assert!(!deploy::hook_ok(&ctx, "shop", ""));
    let on = deploy::set_hook(&ctx, "shop", true).unwrap();
    let path = on["path"].as_str().unwrap().to_string();
    let token = path.rsplit('/').next().unwrap().to_string();
    assert!(deploy::hook_ok(&ctx, "shop", &token));
    assert!(!deploy::hook_ok(&ctx, "shop", "wrong"));
    assert!(!deploy::hook_ok(&ctx, "other", &token));
    deploy::set_hook(&ctx, "shop", false).unwrap();
    assert!(
        !deploy::hook_ok(&ctx, "shop", &token),
        "off means off, and the old secret is dead"
    );
}

#[test]
fn cron_jobs_are_added_run_logged_and_removed() {
    let (ctx, root) = ctx("cron");
    assert!(cron::add(&ctx, "shop", "not a schedule", "echo hi").is_err());
    assert!(cron::add(&ctx, "nope", "* * * * *", "echo hi").is_err());
    let job = cron::add(&ctx, "shop", "*/5 * * * *", "echo ran > cron.txt").unwrap();
    assert!(job.next_run_at.is_some() && job.enabled);
    let (after, ran) = cron::run_now(&ctx, &job.id).unwrap();
    assert_eq!(ran.exit, 0);
    assert_eq!(after.last_exit, Some(0));
    assert!(std::fs::read_to_string(root.join("cron.txt"))
        .unwrap()
        .contains("ran"));
    assert!(cron::log(&ctx, &job.id, 20)
        .unwrap()
        .join("\n")
        .contains("echo ran"));
    let off = cron::set_enabled(&ctx, &job.id, false).unwrap();
    assert!(!off.enabled && off.next_run_at.is_none());
    cron::remove(&ctx, &job.id).unwrap();
    assert!(cron::list(&ctx, Some("shop")).unwrap().is_empty());
}

#[test]
fn a_cron_job_that_was_due_long_ago_is_skipped_not_run_late() {
    let (ctx, root) = ctx("stale");
    let job = cron::add(&ctx, "shop", "* * * * *", "echo late > late.txt").unwrap();
    let item = ctx.store.ops_get(cron::KIND, &job.id).unwrap().unwrap();
    let mut data = item.data.clone();
    data["next_run_at"] =
        serde_json::json!((chrono::Local::now() - chrono::Duration::hours(3)).to_rfc3339());
    ctx.store
        .ops_put(cron::KIND, &job.id, "shop", &data)
        .unwrap();
    assert_eq!(cron::tick(&ctx), 0);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!root.join("late.txt").exists());
    let again = cron::show(&ctx, &job.id).unwrap();
    assert!(
        again.next_run_at.as_deref().unwrap() > chrono::Local::now().to_rfc3339().as_str(),
        "the next time is in the future"
    );
}

#[test]
fn a_due_cron_job_runs_once_when_it_is_ticked() {
    let (ctx, root) = ctx("due");
    let job = cron::add(&ctx, "shop", "* * * * *", "echo tick >> ticks.txt").unwrap();
    let item = ctx.store.ops_get(cron::KIND, &job.id).unwrap().unwrap();
    let mut data = item.data.clone();
    data["next_run_at"] =
        serde_json::json!((chrono::Local::now() - chrono::Duration::seconds(20)).to_rfc3339());
    ctx.store
        .ops_put(cron::KIND, &job.id, "shop", &data)
        .unwrap();
    assert_eq!(cron::tick(&ctx), 1);
    assert_eq!(
        cron::tick(&ctx),
        0,
        "the run was claimed before the thread started"
    );
    let until = Instant::now() + Duration::from_secs(30);
    while !root.join("ticks.txt").exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(root.join("ticks.txt").is_file());
}

#[test]
fn a_worker_command_is_checked_before_a_service_is_made() {
    let (ctx, _root) = ctx("worker");
    let spec = |command: Option<&str>, processes: u8| daemons::Spec {
        site: "shop".into(),
        name: "queue".into(),
        command: command.map(str::to_string),
        connection: None,
        queue: None,
        tries: None,
        sleep: None,
        max_time: None,
        processes,
    };
    // Not a Laravel project: no artisan file.
    assert!(daemons::add(&ctx, &spec(None, 1))
        .unwrap_err()
        .to_string()
        .contains("artisan"));
    assert!(daemons::add(&ctx, &spec(Some("npm run dev"), 0)).is_err());
    assert!(daemons::add(&ctx, &spec(Some("npm run dev"), 9)).is_err());
    let d = daemons::add(&ctx, &spec(Some("npm run dev"), 2)).unwrap();
    assert_eq!(d.services, vec!["worker-shop-queue", "worker-shop-queue-2"]);
    assert!(ctx
        .store
        .get_service("worker-shop-queue-2")
        .unwrap()
        .is_some());
    let launcher = ctx.paths.etc().join("workers").join(if cfg!(windows) {
        "shop-queue.cmd"
    } else {
        "shop-queue.sh"
    });
    let script = std::fs::read_to_string(&launcher).unwrap();
    assert!(
        script.contains("npm run dev") && script.contains("shop"),
        "{script}"
    );
    assert_eq!(d.command, "npm run dev");
    assert_eq!(daemons::list(&ctx, Some("shop")).unwrap().len(), 1);
    daemons::remove(&ctx, &d.id).unwrap();
    assert!(ctx
        .store
        .get_service("worker-shop-queue")
        .unwrap()
        .is_none());
    assert!(!launcher.exists(), "the launcher goes with the worker");
}

#[test]
fn an_artisan_project_gets_a_queue_worker_with_its_own_options() {
    let (ctx, root) = ctx("artisan");
    std::fs::write(root.join("artisan"), "<?php").unwrap();
    // The worker needs the site's PHP; with none installed the error says how to get one.
    let spec = daemons::Spec {
        site: "shop".into(),
        name: "queue".into(),
        command: None,
        connection: Some("redis".into()),
        queue: Some("emails,default".into()),
        tries: Some(5),
        sleep: None,
        max_time: None,
        processes: 1,
    };
    match daemons::argv(&ctx, &spec) {
        Ok(argv) => {
            let line = argv.join(" ");
            assert!(
                line.contains(
                    "queue:work redis --queue=emails,default --tries=5 --sleep=3 --max-time=3600"
                ),
                "{line}"
            );
        }
        Err(e) => assert!(e.to_string().contains("PHP"), "{e}"),
    }
    let bad = daemons::Spec {
        connection: Some("a b".into()),
        ..spec
    };
    assert!(daemons::argv(&ctx, &bad).is_err());
}

#[test]
fn notify_targets_are_validated_scoped_and_listed_without_their_secrets() {
    let (ctx, _root) = ctx("notify");
    assert!(notify::add(&ctx, "slack", Some("http://hooks.slack.com/x"), &[], None).is_err());
    assert!(notify::add(&ctx, "pager", None, &[], None).is_err());
    assert!(notify::add(&ctx, "inbox", None, &["deploy.exploded".to_string()], None).is_err());
    let t = notify::add(
        &ctx,
        "slack",
        Some("https://hooks.slack.com/services/T0/B0/SECRET"),
        &["cron.failed".to_string()],
        Some("shop"),
    )
    .unwrap();
    assert_eq!(t.destination, "hooks.slack.com");
    let listed = serde_json::to_string(&notify::list(&ctx).unwrap()).unwrap();
    assert!(!listed.contains("SECRET"), "{listed}");
    notify::remove(&ctx, &t.id).unwrap();
    assert!(notify::list(&ctx).unwrap().is_empty());
}

#[test]
fn a_deployment_whose_process_is_gone_is_marked_interrupted_and_does_not_block_the_next() {
    let (ctx, _root) = ctx("interrupted");
    ctx.store
        .ops_put(deploy::DEPLOY, "stale001", "shop", &serde_json::json!({ "state": "running", "trigger": "manual", "started_at": devd_core::util::now(), "pid": 4294967u32 }))
        .unwrap();
    let d = deploy::get(&ctx, "stale001").unwrap();
    assert_eq!(d.state, "failed");
    assert!(d.failed_line.unwrap().starts_with("interrupted"));
    // It no longer stops a new deploy from starting.
    deploy::set_script(&ctx, "shop", "echo fine > fine.txt\n").unwrap();
    let started = deploy::start(&ctx, "shop", "manual", None).unwrap();
    assert_eq!(wait_for_deploy(&ctx, &started.id).state, "success");
    assert_eq!(deploy::wait(&ctx, &started.id).unwrap().state, "success");
}

#[test]
fn deploy_variables_are_validated_filled_into_the_script_and_masked_when_shown() {
    let (ctx, root) = ctx("vars");
    let set = |pairs: &[(&str, Option<&str>)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert!(
        deploy::set_vars(&ctx, "shop", &set(&[("lower", Some("x"))])).is_err(),
        "names are capitals"
    );
    assert!(
        deploy::set_vars(&ctx, "shop", &set(&[("PATH", Some("x"))])).is_err(),
        "devd's own names are reserved"
    );
    assert!(deploy::set_vars(&ctx, "shop", &set(&[("A", Some("line\0break"))])).is_err());
    let now = deploy::set_vars(
        &ctx,
        "shop",
        &set(&[
            ("GREETING", Some("hello there")),
            ("API_TOKEN", Some("s3cret")),
        ]),
    )
    .unwrap();
    assert_eq!(now.len(), 2);
    deploy::set_script(&ctx, "shop", "echo {GREETING} > greeting.txt\n").unwrap();
    let d = deploy::start(&ctx, "shop", "manual", None).unwrap();
    assert_eq!(wait_for_deploy(&ctx, &d.id).state, "success");
    assert!(std::fs::read_to_string(root.join("greeting.txt"))
        .unwrap()
        .contains("hello there"));
    let after = deploy::set_vars(&ctx, "shop", &set(&[("GREETING", None)])).unwrap();
    assert_eq!(after.keys().collect::<Vec<_>>(), vec!["API_TOKEN"]);
    assert_eq!(
        devd_ops::env::masked("API_TOKEN", &after["API_TOKEN"]),
        "[hidden, 6 characters]"
    );
}

#[test]
fn a_scheduled_deploy_runs_when_due_is_missed_when_late_and_can_be_cancelled() {
    let (ctx, root) = ctx("schedule");
    assert!(
        deploy::parse_when("tomorrow").is_err()
            && deploy::parse_when("in 0m").is_err()
            && deploy::parse_when("in 90d").is_err()
            && deploy::parse_when("in 5x").is_err()
    );
    assert!(
        deploy::parse_when("in 30m").unwrap()
            > chrono::Local::now() + chrono::Duration::minutes(29)
    );
    assert!(
        deploy::schedule(&ctx, "shop", "2001-01-01T00:00:00+00:00", None).is_err(),
        "the past is refused"
    );
    assert!(deploy::schedule(&ctx, "nope", "in 5m", None).is_err());
    deploy::set_script(&ctx, "shop", "echo scheduled > scheduled.txt\n").unwrap();

    let later = deploy::schedule(&ctx, "shop", "in 2h", Some("after lunch")).unwrap();
    deploy::unschedule(&ctx, later["id"].as_str().unwrap()).unwrap();
    assert!(
        deploy::unschedule(&ctx, later["id"].as_str().unwrap()).is_err(),
        "already cancelled"
    );

    let force = |id: &str, at: chrono::DateTime<chrono::Local>| {
        let item = ctx.store.ops_get(deploy::SCHEDULE, id).unwrap().unwrap();
        let mut d = item.data.clone();
        d["at"] = serde_json::json!(at.to_rfc3339());
        ctx.store.ops_put(deploy::SCHEDULE, id, "shop", &d).unwrap();
    };
    let due = deploy::schedule(&ctx, "shop", "in 5m", None).unwrap();
    let late = deploy::schedule(&ctx, "shop", "in 6m", None).unwrap();
    force(
        due["id"].as_str().unwrap(),
        chrono::Local::now() - chrono::Duration::seconds(30),
    );
    force(
        late["id"].as_str().unwrap(),
        chrono::Local::now() - chrono::Duration::hours(3),
    );
    assert_eq!(deploy::tick_scheduled(&ctx), 1);
    let listed = deploy::scheduled(&ctx, Some("shop")).unwrap();
    let state = |id: &str| {
        listed.iter().find(|s| s["id"] == id).unwrap()["state"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(state(late["id"].as_str().unwrap()), "missed");
    let started = deploy::scheduled(&ctx, Some("shop"))
        .unwrap()
        .into_iter()
        .find(|s| s["id"] == due["id"])
        .unwrap();
    assert_eq!(started["state"], "started");
    let dep = started["deployment"].as_str().unwrap().to_string();
    assert_eq!(wait_for_deploy(&ctx, &dep).state, "success");
    assert!(root.join("scheduled.txt").is_file());
    assert_eq!(deploy::tick_scheduled(&ctx), 0, "nothing is run twice");
}

#[test]
fn a_staging_site_is_a_worktree_on_its_own_branch_that_can_be_promoted_and_removed() {
    // Git needs to know who is committing; the test says so for itself only.
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "devd test"),
        ("GIT_AUTHOR_EMAIL", "t@example.invalid"),
        ("GIT_COMMITTER_NAME", "devd test"),
        ("GIT_COMMITTER_EMAIL", "t@example.invalid"),
    ] {
        std::env::set_var(k, v);
    }
    let (ctx, root) = ctx("staging");
    let git = |dir: &std::path::Path, args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    std::fs::write(root.join("index.html"), "v1").unwrap();
    std::fs::write(
        root.join(".env"),
        "APP_ENV=local\nAPP_URL=http://shop.test\n",
    )
    .unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["add", "index.html"]);
    git(&root, &["commit", "-q", "-m", "first"]);

    let v = deploy::staging_create(&ctx, "shop", None).unwrap();
    assert_eq!(v["staging_site"], "shop-staging");
    let staging = root.parent().unwrap().join("shop-staging");
    assert!(staging.join("index.html").is_file() && staging.join(".git").exists());
    assert_eq!(
        git(&staging, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "staging"
    );
    let env = std::fs::read_to_string(staging.join(".env")).unwrap();
    assert!(
        env.contains("APP_ENV=staging") && env.contains("shop-staging.test"),
        "{env}"
    );
    assert!(ctx.store.get_site("shop-staging").unwrap().is_some());
    assert!(
        deploy::staging_create(&ctx, "shop", None).is_err(),
        "only one staging site per site"
    );

    // Work on staging, then ship it.
    std::fs::write(staging.join("index.html"), "v2").unwrap();
    git(&staging, &["commit", "-q", "-am", "second"]);
    deploy::set_script(&ctx, "shop", "echo promoted > promoted.txt\n").unwrap();
    std::fs::write(root.join("dirty.txt"), "x").unwrap();
    git(&root, &["add", "dirty.txt"]);
    assert!(
        deploy::staging_promote(&ctx, "shop")
            .unwrap_err()
            .to_string()
            .contains("uncommitted"),
        "production must be clean"
    );
    git(&root, &["reset", "-q", "dirty.txt"]);
    std::fs::remove_file(root.join("dirty.txt")).unwrap();
    let d = deploy::staging_promote(&ctx, "shop").unwrap();
    assert_eq!(wait_for_deploy(&ctx, &d.id).state, "success");
    assert_eq!(
        std::fs::read_to_string(root.join("index.html")).unwrap(),
        "v2"
    );
    assert!(root.join("promoted.txt").is_file());

    let gone = deploy::staging_remove(&ctx, "shop").unwrap();
    assert_eq!(gone["removed"], "shop-staging");
    assert!(!staging.exists() && ctx.store.get_site("shop-staging").unwrap().is_none());
    assert!(deploy::staging_list(&ctx).unwrap().is_empty());
}
