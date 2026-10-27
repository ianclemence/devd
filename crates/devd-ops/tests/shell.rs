//! Which PHP a launcher runs from which folder, against a scratch home with two PHP versions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::{Runtime, Site, Store};
use devd_ops::shell;

fn install_php(ctx: &Ctx, version: &str) {
    let root = ctx.paths.runtime_version("php", version);
    let exe = root.join(devd_runtime::executable_relative("php").unwrap());
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"").unwrap();
    ctx.store
        .upsert_runtime(&Runtime {
            id: format!("php-{version}"),
            kind: "php".into(),
            version: version.into(),
            path: root.display().to_string(),
            digest: None,
            pinned: false,
            installed_at: "2026-10-24T00:00:00Z".into(),
        })
        .unwrap();
}

fn site(ctx: &Ctx, name: &str, root: &Path, php: Option<&str>) {
    ctx.store
        .upsert_site(&Site {
            id: format!("id-{name}"),
            name: name.into(),
            host: format!("{name}.test"),
            root: root.display().to_string(),
            web_server: "nginx".into(),
            php: php.map(str::to_string),
            tls_mode: "off".into(),
            db_name: None,
            upstream: None,
            serving: None,
            created_at: "2026-10-24T00:00:00Z".into(),
        })
        .unwrap();
}

/// A home with PHP 8.3.1 and 8.4.2, a site `shop` pinned to 8.3 whose web root is `public`,
/// and a folder `work/shopping` that only starts with the same letters.
fn setup(tag: &str) -> (Ctx, PathBuf) {
    let dir = std::env::temp_dir().join(format!("devd-shell-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(&dir);
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
    install_php(&ctx, "8.3.1");
    install_php(&ctx, "8.4.2");
    let work = dir.join("work");
    std::fs::create_dir_all(work.join("shop").join("public")).unwrap();
    std::fs::create_dir_all(work.join("shop").join("app")).unwrap();
    std::fs::create_dir_all(work.join("shopping")).unwrap();
    std::fs::create_dir_all(work.join("elsewhere")).unwrap();
    // `shop` serves PHP (it has an entry point); `tool` is a Go module that an older devd pinned to PHP 8.3.
    std::fs::write(work.join("shop").join("public").join("index.php"), "<?php").unwrap();
    std::fs::create_dir_all(work.join("tool")).unwrap();
    std::fs::write(work.join("tool").join("go.mod"), "module tool").unwrap();
    site(&ctx, "shop", &work.join("shop").join("public"), Some("8.3"));
    site(&ctx, "tool", &work.join("tool"), Some("8.3"));
    (ctx, work)
}

/// The version folder in a launcher path.
///
/// The executable is `php.exe` directly in the version folder on Windows and
/// `bin/php` inside it on Unix, so the folder is found by its shape rather than
/// by counting components.
fn version(program: &Path) -> String {
    for part in program.ancestors().filter_map(|p| p.file_name()) {
        let name = part.to_string_lossy();
        let mut chars = name.chars();
        if chars.next().is_some_and(|c| c.is_ascii_digit()) && name.contains('.') {
            return name.into_owned();
        }
    }
    panic!("no version folder in {}", program.display());
}

#[test]
fn a_folder_in_a_site_runs_the_php_that_site_is_served_with() {
    let (ctx, work) = setup("site");
    for inside in [
        work.join("shop"),
        work.join("shop").join("app"),
        work.join("shop").join("public"),
    ] {
        let r = shell::resolve(&ctx, "php", &inside).unwrap();
        assert_eq!(version(&r.program), "8.3.1", "{}", inside.display());
        assert!(
            r.reason.contains("`shop`") && r.reason.contains("8.3.1"),
            "{}",
            r.reason
        );
    }
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn a_folder_that_only_shares_the_start_of_a_name_is_not_in_the_site() {
    let (ctx, work) = setup("lookalike");
    let r = shell::resolve(&ctx, "php", &work.join("shopping")).unwrap();
    assert_eq!(
        version(&r.program),
        "8.4.2",
        "the default, not the pinned 8.3 of `shop`: {}",
        r.reason
    );
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn a_runtime_pin_on_a_folder_applies_to_everything_under_it() {
    let (ctx, work) = setup("pin");
    ctx.store
        .set_runtime_pin(&work.join("elsewhere").display().to_string(), "php", "8.3")
        .unwrap();
    let deep = work.join("elsewhere").join("a").join("b");
    std::fs::create_dir_all(&deep).unwrap();
    let r = shell::resolve(&ctx, "php", &deep).unwrap();
    assert_eq!(version(&r.program), "8.3.1", "{}", r.reason);
    assert!(r.reason.contains("pinned"), "{}", r.reason);
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn outside_any_site_or_pin_the_default_php_runs() {
    let (ctx, work) = setup("default");
    let r = shell::resolve(&ctx, "php", &work.join("elsewhere")).unwrap();
    assert_eq!(version(&r.program), "8.4.2");
    assert!(r.reason.contains("default"), "{}", r.reason);
    // The program's own folders come first, so `composer` finds `php` and a script's `php` is this one.
    assert_eq!(r.path_dirs[0], r.program.parent().unwrap());
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn composer_runs_its_phar_with_the_php_of_the_folder_and_says_so_when_it_is_missing() {
    let (ctx, work) = setup("composer");
    let err = shell::resolve(&ctx, "composer", &work.join("shop"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("composer.install"), "{err}");

    let phar = devd_core::shims::composer_phar(&ctx.paths);
    std::fs::create_dir_all(phar.parent().unwrap()).unwrap();
    std::fs::write(&phar, b"phar").unwrap();
    let r = shell::resolve(&ctx, "composer", &work.join("shop")).unwrap();
    assert_eq!(version(&r.program), "8.3.1", "the site's PHP runs Composer");
    assert_eq!(r.prefix, vec![phar.display().to_string()]);

    assert!(shell::resolve(&ctx, "node", &work)
        .unwrap_err()
        .to_string()
        .contains("no launcher for `node`"));
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn a_leftover_pin_on_a_site_that_serves_no_php_does_not_steer_the_terminal() {
    let (ctx, work) = setup("unused-pin");
    let r = shell::resolve(&ctx, "php", &work.join("tool")).unwrap();
    assert_eq!(
        version(&r.program),
        "8.4.2",
        "`tool` serves no PHP, so its 8.3 pin is unused and the default applies: {}",
        r.reason
    );
    assert!(r.reason.contains("default"), "{}", r.reason);
    // The same pin on a site that does serve PHP still decides.
    assert_eq!(
        version(
            &shell::resolve(&ctx, "php", &work.join("shop"))
                .unwrap()
                .program
        ),
        "8.3.1"
    );
    let _ = std::fs::remove_dir_all(ctx.paths.root());
}
