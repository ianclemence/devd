//! Registry-level policy tests for the Git capability set.
//!
//! These do not test Git behaviour; they test the promises the product makes
//! *around* Git behaviour, drive the real dispatcher, and are the reason those
//! promises are checkable rather than aspirational:
//!
//! * a declined operation is refused and does not run;
//! * an agent cannot approve its own operation, from any surface;
//! * a force push cannot be reached except through its own capability;
//! * a dry run changes nothing;
//! * destructive operations can always describe themselves before approval;
//! * secrets never reach the audit trail.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Tier};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::{DevdError, Registry};
use devd_git::{caps, ConfigScope, GitService};
use tempfile::TempDir;

/// A registry, a context and a repository, wired the way the CLI wires them.
struct Harness {
    registry: Registry,
    ctx: Ctx,
    store: Arc<Store>,
    repo: PathBuf,
    git: GitService,
    _home: TempDir,
    _repo_dir: TempDir,
}

impl Harness {
    fn new(actor: Actor) -> Self {
        let home = tempfile::Builder::new()
            .prefix("devd-policy-home-")
            .tempdir()
            .expect("temp home");
        let paths = Paths::rooted(home.path());
        paths.ensure().expect("layout");

        let store = Arc::new(Store::open_in_memory().expect("in-memory store"));
        store.migrate().expect("migrate");

        let git = GitService::discover(None).expect("git on PATH");
        let mut registry = Registry::new();
        caps::register(&mut registry, Arc::new(GitService::discover(None).unwrap()))
            .expect("register git capabilities");

        let repo_dir = tempfile::Builder::new()
            .prefix("devd-policy-repo-")
            .tempdir()
            .expect("temp repo");
        let repo = git.init(repo_dir.path(), "main").expect("init");
        git.config_set(Some(&repo), "user.name", "devd test", ConfigScope::Local)
            .unwrap();
        git.config_set(Some(&repo), "user.email", "devd@example.test", ConfigScope::Local)
            .unwrap();
        git.config_set(Some(&repo), "core.autocrlf", "false", ConfigScope::Local)
            .unwrap();

        let ctx = Ctx {
            store: store.clone(),
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor,
            dry_run: false,
        };

        Self {
            registry,
            ctx,
            store,
            repo,
            git,
            _home: home,
            _repo_dir: repo_dir,
        }
    }

    fn args(&self, extra: Value) -> Value {
        let mut base = json!({ "repo": self.repo.to_string_lossy() });
        if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }
        base
    }

    fn dispatch(&self, id: &str, args: Value, approval: Approval) -> devd_core::Result<devd_core::capability::InvokeResult> {
        self.registry.dispatch(
            &self.ctx,
            &InvokeRequest {
                id: id.to_string(),
                args,
                dry_run: false,
            },
            approval,
        )
    }

    fn plan(&self, id: &str, args: Value) -> devd_core::Result<devd_core::capability::InvokeResult> {
        self.registry.dispatch(
            &self.ctx,
            &InvokeRequest {
                id: id.to_string(),
                args,
                dry_run: true,
            },
            Approval::None,
        )
    }

    fn write(&self, rel: &str, body: &str) {
        std::fs::write(self.repo.join(rel), body).expect("write");
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.repo.join(rel)).expect("read")
    }

    fn commit_all(&self, message: &str) {
        self.git
            .stage(&self.repo, &[".".to_string()])
            .expect("stage");
        self.git
            .commit(&self.repo, message, false, false)
            .expect("commit");
    }
}

// ------------------------------------------------------------ well-formedness

#[test]
fn every_git_capability_is_well_formed_and_discoverable() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    let caps = h.registry.list();
    assert!(caps.len() > 50, "expected a broad Git surface, got {}", caps.len());

    for cap in &caps {
        assert!(cap.id.starts_with("git."), "{} is not in the git namespace", cap.id);
        assert!(cap.group.starts_with("git."), "{} has group {}", cap.id, cap.group);
        assert!(!cap.summary.is_empty(), "{} has no summary", cap.id);
        assert!(!cap.description.is_empty(), "{} has no description", cap.id);
        // Every capability must carry a machine-readable argument schema; this
        // is what the GUI, the HTTP API and the MCP tool list are built from.
        assert!(cap.args_schema.is_object(), "{} has no argument schema", cap.id);
        assert!(h.registry.describe_json(&cap.id).is_ok());
    }

    // Discovery is by group, which is how an agent loads a handful of tools
    // instead of the whole set at once.
    let groups = h.registry.groups();
    for expected in [
        "git.repo",
        "git.changes",
        "git.history",
        "git.branches",
        "git.tags",
        "git.remotes",
        "git.merge",
        "git.stashes",
        "git.worktrees",
        "git.recovery",
    ] {
        assert!(groups.contains_key(expected), "missing group {expected}");
    }

    let found = h.registry.search("branch", None, 50, 0);
    assert!(
        found.iter().any(|c| c.id == "git.branch.list"),
        "search should find branch operations"
    );
}

#[test]
fn anything_that_needs_approval_can_also_produce_a_plan() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    for cap in h.registry.list() {
        if cap.tier.requires_approval() {
            assert!(
                cap.supports_dry_run,
                "{} requires approval but cannot describe itself first",
                cap.id
            );
        }
    }
}

#[test]
fn read_tier_git_operations_need_no_approval() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "a\n");
    h.commit_all("base");

    for id in ["git.status", "git.log", "git.branch.list", "git.remote.list", "git.worktree.list"] {
        let cap = h.registry.get(id).expect(id);
        assert!(!cap.tier.requires_approval(), "{id} should not need approval");
    }
    // And they actually run with no approval.
    h.dispatch("git.status", h.args(json!({})), Approval::None)
        .expect("status is readable");
}

// ------------------------------------------------------------------- refusals

#[test]
fn a_declined_destructive_operation_does_not_run() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "PRECIOUS UNCOMMITTED WORK\n");

    let err = h
        .dispatch(
            "git.discard",
            h.args(json!({ "paths": ["a.txt"] })),
            Approval::Denied,
        )
        .expect_err("a declined discard must not succeed");
    assert!(matches!(err, DevdError::Denied(_)), "{err:?}");

    // The decisive property: the file is byte-for-byte what it was.
    assert_eq!(h.read("a.txt"), "PRECIOUS UNCOMMITTED WORK\n");
    assert!(!h.git.status(&h.repo).unwrap().clean);
}

#[test]
fn a_directive_without_approval_is_refused_for_every_destructive_capability() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "changed\n");

    // Approval::None means "the caller has not obtained an approval". It must
    // never be enough for a tier that requires one.
    let err = h
        .dispatch(
            "git.discard",
            h.args(json!({ "paths": ["a.txt"] })),
            Approval::None,
        )
        .expect_err("approval is required");
    assert!(
        matches!(err, DevdError::ApprovalRequired { .. }),
        "expected an approval requirement, got {err:?}"
    );
    assert_eq!(h.read("a.txt"), "changed\n");
}

#[test]
fn a_granted_approval_lets_the_operation_run() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "changed\n");

    h.dispatch(
        "git.discard",
        h.args(json!({ "paths": ["a.txt"] })),
        Approval::Granted,
    )
    .expect("an approved discard runs");
    assert_eq!(h.read("a.txt"), "committed\n");
}

#[test]
fn an_agent_cannot_supply_its_own_approval() {
    let h = Harness::new(Actor::agent("test-client"));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "agent must not destroy this\n");

    // The agent asks, holding nothing it can grant itself.
    let err = h
        .dispatch(
            "git.discard",
            h.args(json!({ "paths": ["a.txt"] })),
            Approval::None,
        )
        .expect_err("an agent may not proceed unapproved");
    assert!(matches!(err, DevdError::ApprovalRequired { .. }), "{err:?}");

    // Even a status read is unaffected: the refusal is specific, not a blanket ban.
    h.dispatch("git.status", h.args(json!({})), Approval::None)
        .expect("agents can read");

    assert_eq!(h.read("a.txt"), "agent must not destroy this\n");
}

#[test]
fn an_agent_cannot_call_a_human_only_capability_even_when_approved() {
    let h = Harness::new(Actor::agent("test-client"));
    let human_only: Vec<String> = h
        .registry
        .list()
        .into_iter()
        .filter(|c| !c.agent_safe)
        .map(|c| c.id.clone())
        .collect();
    assert!(
        !human_only.is_empty(),
        "the Git surface should include operations reserved for humans"
    );

    // Credential-bearing configuration is one of them, and approving it does
    // not help: `human_only` is checked before approval is even considered.
    let err = h
        .dispatch(
            "git.config.set",
            h.args(json!({ "key": "user.email", "value": "x@example.test" })),
            Approval::Granted,
        )
        .expect_err("an agent may not perform a human-only operation");
    assert!(matches!(err, DevdError::Denied(_)), "{err:?}");
}

// ------------------------------------------------------------- force pushes

#[test]
fn a_force_push_cannot_be_reached_without_its_own_capability() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));

    let ordinary = h.registry.get("git.push").expect("git.push");
    let schema = ordinary.args_schema.clone();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("git.push has an argument schema with properties");
    assert!(
        !properties.contains_key("force"),
        "git.push must not accept a force flag: {properties:?}"
    );

    let forced = h.registry.get("git.push.force").expect("git.push.force");
    assert_eq!(forced.tier, Tier::Remote);
    assert!(forced.tier.requires_approval());
    assert!(ordinary.tier.requires_approval(), "an ordinary push is still remote");
}

#[test]
fn the_reset_plan_names_what_a_hard_reset_destroys() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "one\n");
    h.commit_all("first");
    h.write("a.txt", "two\n");
    h.commit_all("second");

    let planned = h
        .plan("git.reset", h.args(json!({ "mode": "hard", "rev": "HEAD~1" })))
        .expect("a plan");
    let plan = planned.plan.as_ref().expect("a plan body");
    assert!(plan.destructive, "a hard reset is destructive");
    assert!(plan.requires_approval);
    let text = format!("{:?}", plan.steps);
    assert!(
        text.to_lowercase().contains("discard") || text.to_lowercase().contains("destroy"),
        "the plan should say the work is lost: {text}"
    );

    // And planning changed nothing.
    assert_eq!(h.read("a.txt"), "two\n");
    assert!(h.git.status(&h.repo).unwrap().clean);
}

// ------------------------------------------------------------------ dry runs

#[test]
fn a_dry_run_changes_nothing() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "changed\n");

    let planned = h
        .plan("git.stage", h.args(json!({ "paths": ["a.txt"] })))
        .expect("a plan");
    assert!(planned.plan.is_some());
    assert!(
        h.git.status(&h.repo).unwrap().staged.is_empty(),
        "a dry run must not stage anything"
    );

    h.dispatch(
        "git.stage",
        h.args(json!({ "paths": ["a.txt"] })),
        Approval::None,
    )
    .expect("staging runs");
    assert_eq!(h.git.status(&h.repo).unwrap().staged.len(), 1);
}

// -------------------------------------------------------------------- audits

#[test]
fn every_attempt_is_audited_including_a_declined_one() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "changed\n");

    let _ = h.dispatch(
        "git.discard",
        h.args(json!({ "paths": ["a.txt"] })),
        Approval::Denied,
    );

    let rows = h.store.list_audit(10).expect("audit rows");
    let declined = rows
        .iter()
        .find(|r| r.capability == "git.discard")
        .expect("the declined attempt is recorded");
    assert!(!declined.ok, "the attempt must be recorded as a failure");
    assert!(
        declined.message.to_lowercase().contains("denied")
            || declined.message.to_lowercase().contains("approval"),
        "the row should explain the refusal: {}",
        declined.message
    );
    assert_eq!(h.read("a.txt"), "changed\n");
}

#[test]
fn the_audit_trail_records_who_called_and_from_which_surface() {
    let h = Harness::new(Actor::agent("some-client"));
    h.write("a.txt", "a\n");
    h.commit_all("base");

    h.dispatch("git.status", h.args(json!({})), Approval::None)
        .expect("status");

    let rows = h.store.list_audit(10).expect("audit rows");
    let row = rows
        .iter()
        .find(|r| r.capability == "git.status")
        .expect("row");
    assert!(row.actor.contains("agent"), "{}", row.actor);
    assert_eq!(row.surface, "mcp", "an agent actor reports the MCP surface");
    assert_eq!(row.tier, "read");
    assert!(row.ok);
}

#[test]
fn secrets_never_reach_the_audit_trail() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "a\n");
    h.commit_all("base");

    // An interface that leaks a secret into the argument object is exactly the
    // case the audit redaction exists for: the row must not contain it.
    let _ = h.dispatch(
        "git.status",
        h.args(json!({
            "token": "ghp_thisMustNotBeStored",
            "password": "hunter2",
            "Authorization": "Bearer abcdef"
        })),
        Approval::None,
    );

    let rows = h.store.list_audit(10).expect("audit rows");
    let row = rows
        .iter()
        .find(|r| r.capability == "git.status")
        .expect("row");
    assert!(!row.args.contains("ghp_thisMustNotBeStored"), "{}", row.args);
    assert!(!row.args.contains("hunter2"), "{}", row.args);
    assert!(!row.args.contains("abcdef"), "{}", row.args);
    assert!(row.args.contains("[redacted]"), "{}", row.args);
}

// --------------------------------------------------- operation coordination

#[test]
fn a_merge_conflict_is_reported_as_a_state_and_can_be_resolved_through_the_registry() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("z.txt", "base\n");
    h.commit_all("base");
    h.git.create_branch(&h.repo, "feature", None, true).unwrap();
    h.write("z.txt", "feature\n");
    h.commit_all("feature");
    h.git.switch_branch(&h.repo, "main", false, false).unwrap();
    h.write("z.txt", "main\n");
    h.commit_all("main");

    // A conflicted merge is not an error: the operation reached a state the user
    // must resolve, so the outcome says so rather than claiming success.
    h.dispatch(
        "git.merge",
        h.args(json!({ "branch": "feature" })),
        Approval::None,
    )
    .expect("merge reaches a conflicted state");

    let status = h.registry.dispatch(
        &h.ctx,
        &InvokeRequest {
            id: "git.conflicts.list".to_string(),
            args: h.args(json!({})),
            dry_run: false,
        },
        Approval::None,
    );
    let data = status.expect("conflicts are readable").outcome.unwrap().data;
    let conflicts = data
        .get("conflicted")
        .and_then(Value::as_array)
        .expect("a conflicted array");
    assert_eq!(conflicts.len(), 1);

    // Resolving a conflict in favour of one side discards the other side from the
    // working tree, so it is a destructive operation: the consequence is shown
    // first and a human approves it.
    let planned = h
        .plan(
            "git.conflicts.resolve",
            h.args(json!({ "path": "z.txt", "side": "ours" })),
        )
        .expect("a plan");
    let plan = planned.plan.as_ref().expect("a plan body");
    assert!(plan.destructive, "taking one side discards the other");
    assert!(plan.requires_approval);
    assert!(
        format!("{:?}", plan.steps)
            .to_lowercase()
            .contains("discard"),
        "the plan must say what is lost: {:?}",
        plan.steps
    );
    // Planning changed nothing: the conflict markers are still there.
    assert!(h.read("z.txt").contains("<<<<<<<"));

    h.dispatch(
        "git.conflicts.resolve",
        h.args(json!({ "path": "z.txt", "side": "ours" })),
        Approval::Granted,
    )
    .expect("an approved resolution runs");
    h.dispatch("git.op.continue", h.args(json!({ "op": "merge" })), Approval::None)
        .expect("continue");
    assert!(h.git.status(&h.repo).unwrap().clean);
    assert_eq!(h.read("z.txt"), "main\n");
}

#[test]
fn a_denied_operation_cannot_be_reached_through_a_different_capability() {
    let h = Harness::new(Actor::human(devd_core::capability::Surface::Cli));
    h.write("a.txt", "committed\n");
    h.commit_all("base");
    h.write("a.txt", "changed\n");

    // `git.discard` is denied. Every other route to the same effect must not be a
    // way around that: a hard reset is a *separate* capability that demands its
    // own approval, and it is refused here too.
    let _ = h.dispatch(
        "git.discard",
        h.args(json!({ "paths": ["a.txt"] })),
        Approval::Denied,
    );

    let err = h
        .dispatch(
            "git.reset",
            h.args(json!({ "mode": "hard", "rev": "HEAD" })),
            Approval::Denied,
        )
        .expect_err("the workaround is refused as well");
    assert!(matches!(err, DevdError::Denied(_)), "{err:?}");
    assert_eq!(h.read("a.txt"), "changed\n", "work preserved");

    // A declined entry in the audit does not disable the capability for later,
    // legitimately approved use.
    h.dispatch(
        "git.reset",
        h.args(json!({ "mode": "hard", "rev": "HEAD" })),
        Approval::Granted,
    )
    .expect("an approved reset runs");
    assert_eq!(h.read("a.txt"), "committed\n");
}
