//! `project.list` is the one list every surface shows. A folder a site serves is part of it even when
//! nobody imported it, otherwise the Sites screen is the only place those projects appear.

use std::sync::Arc;

use serde_json::json;

use devd_core::capability::{Actor, Approval, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::state::{Site, Store};
use devd_core::{InvokeRequest, Paths, Registry};

fn setup(tag: &str) -> (Ctx, Paths) {
    let dir = std::env::temp_dir().join(format!("devd-projects-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(&dir);
    paths.ensure().unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths.clone()),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        dry_run: false,
    };
    (ctx, paths)
}

fn site(name: &str, root: &std::path::Path) -> Site {
    Site {
        id: format!("id-{name}"),
        name: name.to_string(),
        host: format!("{name}.test"),
        root: root.display().to_string(),
        web_server: "nginx".into(),
        php: None,
        serving: None,
        tls_mode: "internal".into(),
        db_name: None,
        upstream: None,
        created_at: devd_core::util::now(),
    }
}

#[test]
fn a_folder_a_site_serves_is_a_project_in_the_one_list() {
    let (ctx, paths) = setup("sites");
    let shop = paths.www().join("shop");
    std::fs::create_dir_all(shop.join("public")).unwrap();
    std::fs::write(shop.join("public").join("index.php"), "<?php").unwrap();
    let notes = paths.www().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    ctx.store.upsert_site(&site("shop", &shop)).unwrap();
    ctx.store.upsert_site(&site("notes", &notes)).unwrap();

    let mut reg = Registry::new();
    devd_stacks::caps::register(&mut reg).unwrap();
    let result = reg
        .dispatch(&ctx, &InvokeRequest { id: "project.list".into(), args: json!({}), dry_run: false }, Approval::None)
        .unwrap();
    let rows = result.outcome.unwrap().data;
    let rows = rows.as_array().unwrap();

    for (name, folder) in [("shop", &shop), ("notes", &notes)] {
        let row = rows.iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("{name} is in the list: {rows:?}"));
        assert_eq!(row["source"], "site");
        assert_eq!(row["registered"], false);
        assert_eq!(row["sites"], json!([name]));
        assert_eq!(row["exists"], true);
        assert_eq!(row["path"], folder.display().to_string());
    }

    let _ = std::fs::remove_dir_all(ctx.paths.root());
}

#[test]
fn forgetting_a_project_removes_every_spelling_of_its_index_row() {
    let (ctx, paths) = setup("forget");
    let folder = paths.www().join("shop");
    std::fs::create_dir_all(&folder).unwrap();
    // The index may hold the folder with forward slashes while the caller uses backslashes.
    ctx.store
        .upsert_repo(&devd_core::state::Repo {
            id: "repo-shop".into(),
            path: folder.display().to_string().replace('\\', "/"),
            name: "shop".into(),
            remote_url: None,
            default_branch: Some("main".into()),
            last_seen_at: devd_core::util::now(),
        })
        .unwrap();

    let mut reg = Registry::new();
    devd_stacks::caps::register(&mut reg).unwrap();
    let result = reg
        .dispatch(
            &ctx,
            &InvokeRequest { id: "project.forget".into(), args: json!({ "path": folder.display().to_string() }), dry_run: false },
            Approval::None,
        )
        .unwrap();
    assert!(result.outcome.unwrap().message.contains("forgot"), "it says what it did");
    assert!(ctx.store.list_repos().unwrap().is_empty(), "the differently spelled row is gone too");

    let _ = std::fs::remove_dir_all(ctx.paths.root());
}
