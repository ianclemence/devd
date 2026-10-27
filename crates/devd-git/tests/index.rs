//! A Read capability must not write. `git.status` used to index the repository it read, so a project a
//! person had removed from devd came back the next time its page was opened.

use std::sync::Arc;

use serde_json::json;

use devd_core::capability::{Actor, Approval, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::state::Store;
use devd_core::{InvokeRequest, Paths, Registry};
use devd_git::GitService;

#[test]
fn reading_a_status_does_not_index_the_repository_but_tracking_does() {
    let dir = std::env::temp_dir().join(format!("devd-git-index-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = Arc::new(GitService::discover(None).expect("git is on PATH"));
    let repo = git.init(&dir, "main").expect("init");

    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(Paths::rooted(dir.join("home"))),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        run: None,
        dry_run: false,
    };
    let mut reg = Registry::new();
    devd_git::caps::register(&mut reg, git).unwrap();

    let status = InvokeRequest {
        id: "git.status".into(),
        args: json!({ "repo": repo.display().to_string() }),
        dry_run: false,
    };
    reg.dispatch(&ctx, &status, Approval::None).unwrap();
    assert!(
        ctx.store.list_repos().unwrap().is_empty(),
        "a read must not write an index row"
    );

    let track = InvokeRequest {
        id: "git.repo.track".into(),
        args: json!({ "repo": repo.display().to_string() }),
        dry_run: false,
    };
    reg.dispatch(&ctx, &track, Approval::None).unwrap();
    assert_eq!(
        ctx.store.list_repos().unwrap().len(),
        1,
        "tracking is the explicit way to index"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
