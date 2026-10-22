//! The dashboard against the real registry and a real repository.
//!
//! The unit tests drive the controller with canned capability results, which
//! proves the logic but not the wiring. This one builds the actual registry over
//! a throwaway Git repository, tracks it, switches to the Changes view, and
//! renders — so a change to a capability's payload shape fails here rather than
//! in front of a person.
//!
//! The terminal itself is a [`TestBackend`]: it renders the same frames a real
//! terminal would, without needing one, which is the point.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Surface};
use devd_core::{EventBus, Paths, Registry, Settings, Store};
use devd_git::GitService;
use devd_tui::{Controller, RegistryBackend, View};

use ratatui::backend::TestBackend;
use ratatui::Terminal;

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(repo: &PathBuf, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .expect("git must be installed to run this test");
    assert!(status.success(), "git {args:?} failed");
}

fn fixture(name: &str) -> Fixture {
    // Unique per test: the tests in one binary share a process id and run in
    // parallel, so a shared path would have them stepping on each other.
    let root = std::env::temp_dir().join(format!("devd-tui-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();

    git(&repo, &["init", "--initial-branch", "main"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    git(&repo, &["config", "user.name", "devd test"]);
    git(&repo, &["config", "user.email", "devd@example.invalid"]);
    std::fs::write(repo.join("committed.txt"), "first\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "First commit"]);

    // One staged, one unstaged and one untracked change, so all three buckets
    // have something in them.
    std::fs::write(repo.join("committed.txt"), "second\n").unwrap();
    git(&repo, &["add", "committed.txt"]);
    std::fs::write(repo.join("committed.txt"), "third\n").unwrap();
    std::fs::write(repo.join("untracked.txt"), "new\n").unwrap();

    Fixture { root, repo }
}

/// A controller over the real registry.
///
/// `extra_checks` is empty: the Git check is what the CLI contributes, and this
/// test is about the Git views rather than the doctor.
fn controller(root: &std::path::Path) -> (Controller, Arc<Registry>, Ctx) {
    let paths = Paths::rooted(root.join("home"));
    paths.ensure().unwrap();
    let ctx = Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        // The real surface, so `can_prompt` and the tier rules see what they
        // would see in a terminal.
        actor: Actor::human(Surface::Tui),
        dry_run: false,
    };

    let mut registry = Registry::new();
    let git = Arc::new(GitService::discover(None).expect("git on PATH"));
    devd_git::caps::register(&mut registry, git).unwrap();
    let registry = Arc::new(registry);

    let backend = RegistryBackend::new(registry.clone(), ctx.clone(), Vec::new());
    (Controller::new(Box::new(backend)), registry, ctx)
}

fn draw(controller: &Controller) -> String {
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal
        .draw(|frame| devd_tui::view::render(frame, &controller.state))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

#[test]
fn the_dashboard_reads_a_real_repository_through_the_real_registry() {
    let fixture = fixture("dashboard");
    let (mut controller, registry, ctx) = controller(&fixture.root);

    // Track the repository the way the CLI would, then let the dashboard find it
    // on its own through `git.repo.list`.
    registry
        .dispatch(
            &ctx,
            &InvokeRequest {
                id: "git.repo.track".to_string(),
                args: serde_json::json!({ "repo": fixture.repo.display().to_string() }),
                dry_run: false,
            },
            Approval::None,
        )
        .expect("tracking a repository must succeed");

    controller.switch_view(View::Changes);

    let state = &controller.state;
    assert_eq!(
        state.projects.len(),
        1,
        "the project list did not pick up the tracked repository"
    );
    assert!(
        state.repo.as_deref().is_some_and(|r| r.contains("demo")),
        "no repository was selected: {:?}",
        state.repo
    );

    let status = state.status.as_ref().expect("status was not loaded");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.staged.len(), 1, "{status:?}");
    assert_eq!(status.unstaged.len(), 1, "{status:?}");
    assert_eq!(status.untracked.len(), 1, "{status:?}");

    let screen = draw(&controller);
    assert!(
        screen.contains("main: 1 staged, 1 unstaged, 1 untracked"),
        "{screen}"
    );
    assert!(screen.contains("committed.txt"), "{screen}");
    assert!(screen.contains("untracked.txt"), "{screen}");
    assert!(screen.contains("demo"), "{screen}");
}

#[test]
fn the_history_and_branch_views_show_what_git_has() {
    let fixture = fixture("history");
    let (mut controller, registry, ctx) = controller(&fixture.root);
    registry
        .dispatch(
            &ctx,
            &InvokeRequest {
                id: "git.repo.track".to_string(),
                args: serde_json::json!({ "repo": fixture.repo.display().to_string() }),
                dry_run: false,
            },
            Approval::None,
        )
        .unwrap();
    // Make the selection deterministic rather than relying on the first project.
    controller.state.repo = Some(fixture.repo.display().to_string());

    controller.switch_view(View::History);
    assert_eq!(controller.state.commits.len(), 1);
    assert_eq!(controller.state.commits[0].subject, "First commit");
    let screen = draw(&controller);
    assert!(screen.contains("First commit"), "{screen}");

    controller.switch_view(View::Branches);
    assert_eq!(controller.state.branches.len(), 1);
    assert!(controller.state.branches[0].current);
    let screen = draw(&controller);
    assert!(screen.contains("* main"), "{screen}");
}

#[test]
fn staging_from_the_dashboard_changes_the_real_index() {
    let fixture = fixture("staging");
    let (mut controller, registry, ctx) = controller(&fixture.root);
    registry
        .dispatch(
            &ctx,
            &InvokeRequest {
                id: "git.repo.track".to_string(),
                args: serde_json::json!({ "repo": fixture.repo.display().to_string() }),
                dry_run: false,
            },
            Approval::None,
        )
        .unwrap();

    controller.switch_view(View::Changes);
    // items(): staged (committed.txt), unstaged (committed.txt), untracked
    // (untracked.txt). Row 2 is the untracked file.
    controller.state.cursor = 2;
    assert_eq!(
        controller.state.selected_change().unwrap().path,
        "untracked.txt"
    );

    controller.handle_key(devd_tui::Key::Enter);

    // The stage happened in Git, not just in the panel.
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    let staged = String::from_utf8_lossy(&output.stdout);
    assert!(
        staged.contains("untracked.txt"),
        "the file was not staged in the repository: {staged}"
    );
}

#[test]
fn a_destructive_key_from_the_dashboard_asks_before_it_touches_the_working_tree() {
    let fixture = fixture("discard");
    let (mut controller, registry, ctx) = controller(&fixture.root);
    registry
        .dispatch(
            &ctx,
            &InvokeRequest {
                id: "git.repo.track".to_string(),
                args: serde_json::json!({ "repo": fixture.repo.display().to_string() }),
                dry_run: false,
            },
            Approval::None,
        )
        .unwrap();

    controller.switch_view(View::Changes);
    controller.state.cursor = 1; // committed.txt, unstaged
    assert_eq!(
        controller.state.selected_change().unwrap().path,
        "committed.txt"
    );

    controller.handle_key(devd_tui::Key::Char('d'));

    // Nothing has happened to the file yet: the registry produced a plan and the
    // person has been asked.
    let on_disk = std::fs::read_to_string(fixture.repo.join("committed.txt")).unwrap();
    assert_eq!(
        on_disk, "third\n",
        "the working tree changed before approval"
    );
    assert!(
        matches!(controller.state.overlay, devd_tui::Overlay::Approval(_)),
        "no confirmation was shown: {:?}",
        controller.state.overlay
    );

    // And declining leaves it alone, while recording the refusal.
    controller.handle_key(devd_tui::Key::Char('n'));
    let on_disk = std::fs::read_to_string(fixture.repo.join("committed.txt")).unwrap();
    assert_eq!(on_disk, "third\n", "a declined discard changed the file");

    let audit = ctx.store.list_audit(10).unwrap();
    assert!(
        audit
            .iter()
            .any(|row| row.capability == "git.discard" && !row.ok),
        "the refusal was not recorded: {audit:?}"
    );
}

#[test]
fn the_doctor_and_capability_views_are_filled_from_the_registry() {
    let fixture = fixture("registry");
    let (mut controller, _registry, _ctx) = controller(&fixture.root);

    controller.switch_view(View::Doctor);
    assert!(
        !controller.state.checks.is_empty(),
        "the doctor produced no checks"
    );

    controller.switch_view(View::Capabilities);
    assert!(
        controller.state.capabilities.len() >= 60,
        "the capability list is too small: {}",
        controller.state.capabilities.len()
    );
    // The order is the registry's own ranking, so assert on what is on the
    // screen rather than on a particular capability being in the first page.
    let screen = draw(&controller);
    assert!(screen.contains("Capabilities matching"), "{screen}");
    assert!(screen.contains("git."), "{screen}");
    assert!(
        screen.contains("approval"),
        "no row was flagged as needing approval:\n{screen}"
    );
}
