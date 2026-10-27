//! A deploy to a "server", end to end, with the real SSH layer and a stand-in `ssh` that runs the remote command on this
//! machine. It checks the whole releases flow: the archive that ships, the shared `.env` and `storage`, the atomic
//! switch, the number of releases kept, and rollback. Linux only: the remote script uses GNU `mv -T` and symlinks, which
//! is what the servers run; a Windows machine has neither, so there the test is skipped and says so.

#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::{Site, Store};
use devd_ops::remote::{self, LinkSpec};
use devd_ops::servers;

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A stand-in for ssh: drop the options, run the last argument as a shell command, pass standard input through.
fn fake_ssh(dir: &Path) -> std::path::PathBuf {
    let shim = dir.join("fake-ssh");
    write(
        &shim,
        "#!/bin/sh\nwhile [ $# -gt 1 ]; do\n  case \"$1\" in\n    -p|-i|-o) shift 2;;\n    *) shift;;\n  esac\ndone\n# what is left: the command (the user@host before it was consumed by the loop's last shift)\nexec sh -c \"$1\"\n",
    );
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    shim
}

fn wait(ctx: &Ctx, id: &str) -> devd_ops::deploy::Deployment {
    let until = Instant::now() + Duration::from_secs(90);
    loop {
        let d = devd_ops::deploy::get(ctx, id).unwrap();
        if d.state != "running" || Instant::now() > until {
            return d;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_site_is_deployed_switched_pruned_and_rolled_back() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let home = base.join("home");
    write(&home.join(".ssh").join("testkey"), "not a real key");
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    std::env::set_var("DEVD_SSH", fake_ssh(base));

    let paths = Paths::rooted(base.join("devd"));
    paths.ensure().unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        run: None,
        dry_run: false,
    };

    let project = base.join("www").join("shop");
    write(
        &project.join("public").join("index.php"),
        "<?php echo 'v1';",
    );
    write(&project.join("composer.json"), "{}");
    write(
        &project.join(".env"),
        "APP_NAME=Shop\nAPP_ENV=local\nAPP_DEBUG=true\nDB_PASSWORD=secret\n",
    );
    write(&project.join("vendor").join("autoload.php"), "x");
    ctx.store
        .upsert_site(&Site {
            id: "1".into(),
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

    servers::add(
        &ctx,
        "box",
        "203.0.113.7",
        "deploy",
        None,
        "testkey",
        Some("hetzner"),
    )
    .unwrap();
    let connected = servers::connect(&ctx, "box").unwrap();
    assert!(
        connected["login_user"].as_str().is_some(),
        "{connected}"
    );

    let remote_dir = base.join("srv").join("shop");
    let l = remote::make_link(
        &ctx,
        &LinkSpec {
            site: "shop".into(),
            server: "box".into(),
            path: remote_dir.display().to_string(),
            domain: Some("shop.example.com".into()),
            keep: Some(2),
            steps: Some(vec!["echo built > BUILT.txt".into()]),
            build: Some(vec![]),
            include: Some(vec![]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(l.id, "shop@box");

    // The first deploy has no shared .env yet: it stops before anything is switched.
    let d = remote::deploy(&ctx, "shop@box", "manual").unwrap();
    let d = wait(&ctx, &d.id);
    assert_eq!(
        d.state,
        "failed",
        "{:?}",
        devd_ops::deploy::log(&ctx, &d.id, 40)
    );
    assert!(
        !remote_dir.join("current").exists(),
        "a failed deploy must not switch"
    );

    // Send the .env, then deploy.
    let pushed = remote::env_push(&ctx, "shop@box", false).unwrap();
    assert!(pushed["set_for_production"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "APP_DEBUG"));
    assert!(
        remote::env_push(&ctx, "shop@box", false).is_err(),
        "it does not replace an existing .env"
    );
    let shared_env = std::fs::read_to_string(remote_dir.join("shared").join(".env")).unwrap();
    assert!(
        shared_env.contains("APP_ENV=production")
            && shared_env.contains("APP_DEBUG=false")
            && shared_env.contains("APP_URL=https://shop.example.com"),
        "{shared_env}"
    );

    let mut seen = Vec::new();
    for i in 0..3 {
        write(
            &project.join("public").join("index.php"),
            &format!("<?php echo 'v{}';", i + 2),
        );
        std::thread::sleep(Duration::from_millis(1100)); // release names are to the second
        let d = remote::deploy(&ctx, "shop@box", "manual").unwrap();
        let done = wait(&ctx, &d.id);
        assert_eq!(
            done.state,
            "success",
            "{:?}",
            devd_ops::deploy::log(&ctx, &done.id, 60)
        );
        seen.push(std::fs::read_link(remote_dir.join("current")).unwrap());
    }
    let current = remote_dir.join("current");
    assert!(
        std::fs::read_to_string(current.join("public").join("index.php"))
            .unwrap()
            .contains("v4"),
        "the newest release is live"
    );
    assert!(
        current.join("BUILT.txt").is_file(),
        "the steps ran inside the release"
    );
    assert!(current.join(".devd-commit").is_file());
    assert!(std::fs::read_link(current.join("storage"))
        .unwrap()
        .ends_with("shared/storage"));
    assert_eq!(
        std::fs::read_to_string(current.join(".env")).unwrap(),
        shared_env,
        ".env is the shared one, not a shipped copy"
    );
    assert!(
        !current.join("vendor").join("autoload.php").exists(),
        "local dependencies are not shipped"
    );
    let kept = std::fs::read_dir(remote_dir.join("releases"))
        .unwrap()
        .count();
    assert_eq!(kept, 2, "keep = 2");

    let listing = remote::releases(&ctx, "shop@box").unwrap();
    assert_eq!(listing["releases"].as_array().unwrap().len(), 2);
    assert_eq!(
        listing["releases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["current"] == true)
            .count(),
        1
    );

    // Roll back to the release before.
    let done = remote::rollback(&ctx, "shop@box", None).unwrap();
    assert!(
        done.output.contains("current -> releases/"),
        "{}",
        done.output
    );
    assert!(
        std::fs::read_to_string(remote_dir.join("current").join("public").join("index.php"))
            .unwrap()
            .contains("v3")
    );
    assert!(
        remote::rollback(&ctx, "shop@box", None).is_err(),
        "there is nothing before the oldest release"
    );

    let history = devd_ops::deploy::history(&ctx, "shop", 10).unwrap();
    assert_eq!(history.iter().filter(|h| h.state == "success").count(), 3);
    assert!(history
        .iter()
        .all(|h| h.target.as_deref() == Some(&format!("box:{}", remote_dir.display()))));
}
