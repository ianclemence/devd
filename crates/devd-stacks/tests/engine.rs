//! End-to-end tests of the engine against controlled stack definitions and
//! project fixtures. Nothing here downloads or installs anything.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use devd_core::capability::{Actor, Approval, Ctx, InvokeRequest, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;
use devd_core::Registry;
use devd_stacks::{caps, detect, init, StackRegistry};

fn ctx_with(root: &Path, actor: Actor) -> Ctx {
    let paths = Paths::rooted(root.join("home"));
    paths.ensure().unwrap();
    let mut settings = Settings::default();
    settings.www_root = Some(root.join("www"));
    let store = Store::open_in_memory().unwrap();
    store.migrate().unwrap();
    Ctx { store: Arc::new(store), paths: Arc::new(paths), settings: Arc::new(settings), events: EventBus::new(), actor, dry_run: false }
}

const FIXTURE_STACK: &str = r#"
id: fixture
name: Fixture
language: shell
category: language
summary: A stack made of file writes, for tests.
docs: [{title: Docs, url: "https://example.com/docs"}]
verified: {method: docs, against: test, date: "2026-10-20"}
versions: [{id: one, label: One, value: "1", default: true}]
options:
  - {id: flavour, label: Flavour, kind: select, default: plain, choices: [{value: plain, label: Plain}, {value: spicy, label: Spicy}]}
  - {id: tag, label: Tag, kind: text, default: "v-{name}", pattern: '^[a-z0-9-]+$'}
init:
  creates_dir: true
  steps:
    - {id: hello, title: Write hello.txt, kind: write, cwd: project, path: hello.txt, content: "hello {name} {opt.flavour} {opt.tag}\n"}
verify:
  files: [hello.txt]
tasks:
  - {id: say, title: Print, kind: other, run: [rustc, --version]}
  - {id: wipe, title: Wipe, kind: other, risk: destructive, run: [rustc, --version]}
detect:
  rules: [{file: hello.txt, weight: 5, evidence: hello.txt}]
exec:
  programs: [rustc]
  destructive_args: [--nuke]
"#;

fn registry_with_fixture(root: &Path) -> StackRegistry {
    let dir = root.join("home").join("stacks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fixture.yaml"), FIXTURE_STACK).unwrap();
    std::fs::write(dir.join("broken.yaml"), FIXTURE_STACK.replace("id: fixture", "id: broken").replace("kind: write, cwd: project", "kind: exec, cwd: project")).unwrap();
    std::fs::write(dir.join("shadow.yaml"), FIXTURE_STACK.replace("id: fixture", "id: laravel")).unwrap();
    StackRegistry::with_extensions(&dir)
}

#[test]
fn a_new_stack_is_a_yaml_file_and_invalid_ones_are_rejected_with_reasons() {
    let d = tempfile::tempdir().unwrap();
    let reg = registry_with_fixture(d.path());
    assert!(reg.stack("fixture").is_some(), "{:?}", reg.rejected);
    assert!(reg.stack("broken").is_none());
    assert_eq!(reg.stacks.iter().filter(|s| s.id == "laravel").count(), 1, "an extension cannot replace a built-in");
    assert!(reg.rejected.iter().any(|r| r.contains("broken.yaml")));
    assert!(reg.rejected.iter().any(|r| r.contains("already exists")));
}

#[test]
fn the_plan_reports_every_problem_with_a_field_and_a_code() {
    let d = tempfile::tempdir().unwrap();
    let reg = registry_with_fixture(d.path());
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    let mut req = init::InitRequest { stack: "fixture".into(), name: "Bad Name!".into(), ..Default::default() };
    req.options.insert("flavour".into(), json!("sweet"));
    req.options.insert("nope".into(), json!(1));
    req.options.insert("tag".into(), json!("UPPER"));
    let plan = init::plan(&ctx, &reg, &req).unwrap();
    assert!(!plan.valid && !plan.ready);
    let codes: Vec<(&str, &str)> = plan.issues.iter().map(|i| (i.field.as_str(), i.code.as_str())).collect();
    assert!(codes.contains(&("name", "invalid_name")), "{codes:?}");
    assert!(codes.contains(&("options.flavour", "invalid_option")));
    assert!(codes.contains(&("options.nope", "unknown_option")));
    assert!(codes.contains(&("options.tag", "invalid_option")));
}

#[test]
fn running_creates_verifies_registers_and_a_second_run_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let reg = registry_with_fixture(d.path());
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    let req = init::InitRequest { stack: "fixture".into(), name: "demo".into(), ..Default::default() };
    let r = init::run(&ctx, &reg, &req).unwrap();
    assert_eq!(r.status, "ok", "{:?}", r);
    let dir = d.path().join("www").join("demo");
    assert_eq!(std::fs::read_to_string(dir.join("hello.txt")).unwrap(), "hello demo plain v-demo\n");
    assert!(dir.join(".git").exists(), "git was requested and must exist");
    assert!(r.checks.iter().any(|c| c.title.contains("hello.txt") && c.status == "ok"));
    let known = ctx.store.list_stack_projects().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].stack_id.as_deref(), Some("fixture"));
    assert!(!ctx.store.list_stack_runs(None, 10).unwrap().is_empty());
    let again = init::run(&ctx, &reg, &req).unwrap_err();
    assert_eq!(again.code(), "invalid_input");
}

#[test]
fn a_failing_step_is_reported_as_failed_never_as_success() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("home").join("stacks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.yaml"), FIXTURE_STACK.replace("id: fixture", "id: failing").replace("echo, \"{name}\"", "echo, x").replace(
        "    - {id: hello,",
        "    - {id: boom, title: Always fails, kind: exec, cwd: project, run: [git, definitely-not-a-git-command]}\n    - {id: hello,",
    )).unwrap();
    let reg = StackRegistry::with_extensions(&dir);
    assert!(reg.stack("failing").is_some(), "{:?}", reg.rejected);
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    let r = init::run(&ctx, &reg, &init::InitRequest { stack: "failing".into(), name: "x".into(), git: Some(false), ..Default::default() }).unwrap();
    assert_eq!(r.status, "failed");
    assert!(r.error.is_some());
    assert!(r.steps.iter().any(|s| s.id == "boom" && s.status == "failed"));
    assert!(!r.steps.iter().any(|s| s.id == "hello"), "steps after a failure must not run");
    assert!(ctx.store.list_stack_projects().unwrap().is_empty(), "a failed creation is not registered");
}

#[test]
fn agents_are_confined_to_the_projects_root() {
    let d = tempfile::tempdir().unwrap();
    let reg = registry_with_fixture(d.path());
    let agent = ctx_with(d.path(), Actor::agent("claude-code"));
    let mut req = init::InitRequest { stack: "fixture".into(), name: "demo".into(), ..Default::default() };
    req.destination = Some(d.path().join("elsewhere").display().to_string());
    let plan = init::plan(&agent, &reg, &req).unwrap();
    assert!(plan.issues.iter().any(|i| i.code == "destination_not_allowed"));
    req.destination = Some("/etc".into());
    assert!(init::plan(&agent, &reg, &req).unwrap().issues.iter().any(|i| i.code == "destination_not_allowed"));
    req.destination = Some(d.path().join("www").join("sub").display().to_string());
    assert!(init::plan(&agent, &reg, &req).unwrap().valid);
}

#[test]
fn detection_weighs_evidence_and_admits_ambiguity() {
    let d = tempfile::tempdir().unwrap();
    let reg = StackRegistry::builtin();
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));

    let laravel = d.path().join("laravel-app");
    std::fs::create_dir_all(laravel.join("bootstrap")).unwrap();
    std::fs::write(laravel.join("composer.json"), r#"{"require":{"php":"^8.2","laravel/framework":"^12.0"},"scripts":{"test":"php artisan test"}}"#).unwrap();
    std::fs::write(laravel.join("artisan"), "#!/usr/bin/env php\n").unwrap();
    std::fs::write(laravel.join("composer.lock"), "{}").unwrap();
    std::fs::write(laravel.join(".env.example"), "APP_KEY=\nDB_PASSWORD=\nDB_CONNECTION=mysql\n").unwrap();
    std::fs::write(laravel.join(".env"), "APP_KEY=base64:abc\nDB_PASSWORD=hunter2\nDB_CONNECTION=mysql\n").unwrap();
    let c = detect::detect(&ctx, reg, &laravel, None).unwrap();
    assert_eq!(c.primary.as_deref(), Some("laravel"));
    assert_eq!(c.package_manager.as_deref(), Some("composer"));
    assert!(c.runtimes.iter().any(|r| r.tool == "php" && r.constraint == "^8.2"));
    assert!(c.tasks.iter().any(|t| t.id == "test"));
    assert!(c.tasks.iter().any(|t| t.id == "composer:test"), "the project's own scripts are discovered");
    assert!(c.tasks.iter().any(|t| t.id == "migrate-fresh" && format!("{:?}", t.risk) == "Destructive"));
    assert!(c.services.iter().any(|s| s.tool == "mysql"), "DB_CONNECTION=mysql implies a MySQL service");
    assert!(c.env.secret_keys.contains(&"DB_PASSWORD".to_string()));
    let dumped = serde_json::to_string(&c).unwrap();
    assert!(!dumped.contains("hunter2") && !dumped.contains("base64:abc"), "secret values must never reach the context");

    let mono = d.path().join("mono");
    std::fs::create_dir_all(&mono).unwrap();
    std::fs::write(mono.join("composer.json"), r#"{"require":{"laravel/framework":"^12"}}"#).unwrap();
    std::fs::write(mono.join("artisan"), "").unwrap();
    std::fs::write(mono.join("manage.py"), "os.environ.setdefault('DJANGO_SETTINGS_MODULE', 'x.settings')").unwrap();
    std::fs::write(mono.join("requirements.txt"), "Django==5.2\n").unwrap();
    let c = detect::detect(&ctx, reg, &mono, None).unwrap();
    assert!(c.ambiguous, "two strong frameworks must be reported as ambiguous: {:?}", c.stacks);
    let forced = detect::detect(&ctx, reg, &mono, Some("django")).unwrap();
    assert_eq!(forced.primary.as_deref(), Some("django"));

    let empty = d.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let c = detect::detect(&ctx, reg, &empty, None).unwrap();
    assert!(c.primary.is_none() && c.ambiguous);
}

#[test]
fn package_manager_comes_from_evidence_not_a_default() {
    let d = tempfile::tempdir().unwrap();
    let reg = StackRegistry::builtin();
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    let p = d.path().join("web");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("package.json"), r#"{"dependencies":{"next":"16"},"scripts":{"dev":"next dev","deploy":"ssh prod ./go"},"packageManager":"pnpm@10.0.0"}"#).unwrap();
    std::fs::write(p.join("package-lock.json"), "{}").unwrap();
    let c = detect::detect(&ctx, reg, &p, None).unwrap();
    assert_eq!(c.primary.as_deref(), Some("nextjs"));
    assert_eq!(c.package_manager.as_deref(), Some("pnpm"), "the packageManager field outranks a stray lockfile");
    let dev = c.tasks.iter().find(|t| t.id == "dev").unwrap();
    assert_eq!(dev.argv, vec!["pnpm", "run", "dev"]);
    let deploy = c.tasks.iter().find(|t| t.id == "script:deploy").unwrap();
    assert_eq!(format!("{:?}", deploy.risk), "Destructive", "a deploy script needs a human");
}

fn dispatch(reg: &Registry, ctx: &Ctx, id: &str, args: Value, approval: Approval) -> devd_core::Result<Value> {
    let r = reg.dispatch(ctx, &InvokeRequest { id: id.into(), args, dry_run: false }, approval)?;
    Ok(r.outcome.map(|o| serde_json::to_value(o).unwrap()).unwrap_or(Value::Null))
}

#[test]
fn tasks_run_with_evidence_and_destructive_ones_need_a_human() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    registry_with_fixture(root);
    let mut reg = Registry::new();
    caps::register(&mut reg).unwrap();
    let human = ctx_with(root, Actor::human(Surface::Cli));
    // The fixture stack is an extension, so create the project through the capability.
    let out = dispatch(&reg, &human, "project.init", json!({"stack": "fixture", "name": "proj"}), Approval::None).unwrap();
    assert_eq!(out["data"]["status"], "ok");
    let path = root.join("www").join("proj").display().to_string();

    let ran = dispatch(&reg, &human, "project.run", json!({"path": path, "task": "say"}), Approval::None).unwrap();
    assert_eq!(ran["ok"], true);
    assert_eq!(ran["data"]["exit_code"], 0);
    assert!(ran["data"]["output_tail"].as_array().unwrap().iter().any(|l| l.as_str().is_some_and(|s| s.contains("rustc"))));

    // A destructive task is refused on the safe path and needs approval on its own.
    let err = dispatch(&reg, &human, "project.run", json!({"path": path, "task": "wipe"}), Approval::None).unwrap_err();
    assert_eq!(err.code(), "permission_denied");
    let err = dispatch(&reg, &human, "project.run_destructive", json!({"path": path, "task": "wipe"}), Approval::None).unwrap_err();
    assert_eq!(err.code(), "approval_required");
    let ok = dispatch(&reg, &human, "project.run_destructive", json!({"path": path, "task": "wipe"}), Approval::Granted).unwrap();
    assert_eq!(ok["ok"], true);

    // An agent can run safe tasks but can never reach the destructive capability, approval or not.
    let agent = ctx_with(root, Actor::agent("claude-code"));
    let _ = agent.store.upsert_stack_project(&devd_core::StackProject { path: path.clone(), name: "proj".into(), stack_id: None, package_manager: None, source: "import".into(), context: "{}".into(), created_at: "x".into(), updated_at: "x".into() });
    let err = dispatch(&reg, &agent, "project.run_destructive", json!({"path": path, "task": "wipe"}), Approval::Granted).unwrap_err();
    assert_eq!(err.code(), "permission_denied");
    let err = dispatch(&reg, &agent, "toolchain.install", json!({"tool": "git"}), Approval::None).unwrap_err();
    assert_eq!(err.code(), "approval_required", "installing software always waits for a person");

    // project.exec: allow-listed programs only, never a shell, destructive flags redirected.
    let e = dispatch(&reg, &human, "project.exec", json!({"path": path, "argv": ["sh", "-c", "echo hi"]}), Approval::None).unwrap_err();
    assert_eq!(e.code(), "permission_denied");
    let e = dispatch(&reg, &human, "project.exec", json!({"path": path, "argv": ["rustc", "--nuke"]}), Approval::None).unwrap_err();
    assert!(e.to_string().contains("project.exec_destructive"));
    let fine = dispatch(&reg, &human, "project.exec", json!({"path": path, "argv": ["rustc", "--version"]}), Approval::None).unwrap();
    assert_eq!(fine["data"]["exit_code"], 0);
}

#[test]
fn every_surface_gets_the_same_answer() {
    let d = tempfile::tempdir().unwrap();
    let mut reg = Registry::new();
    caps::register(&mut reg).unwrap();
    let args = json!({"stack": "go", "name": "parity", "options": {"kind": "http"}});
    let mut seen: Vec<Value> = Vec::new();
    for surface in [Surface::Cli, Surface::Tui, Surface::Gui, Surface::Http] {
        let ctx = ctx_with(d.path(), Actor::human(surface));
        let out = dispatch(&reg, &ctx, "stacks.validate", args.clone(), Approval::None).unwrap();
        seen.push(out["data"].clone());
    }
    let mcp = ctx_with(d.path(), Actor::agent("claude-code"));
    seen.push(dispatch(&reg, &mcp, "stacks.validate", args, Approval::None).unwrap()["data"].clone());
    // The permission model differs per actor; the plan an interface shows must not.
    for s in &seen[1..] {
        assert_eq!(s["command"], seen[0]["command"]);
        assert_eq!(s["steps"], seen[0]["steps"]);
        assert_eq!(s["issues"], seen[0]["issues"]);
    }
}

#[test]
fn schemas_are_machine_readable_and_options_schema_matches_the_definition() {
    let d = tempfile::tempdir().unwrap();
    let mut reg = Registry::new();
    caps::register(&mut reg).unwrap();
    let ctx = ctx_with(d.path(), Actor::agent("a"));
    let s = dispatch(&reg, &ctx, "stacks.schema", json!({}), Approval::None).unwrap();
    for k in ["init_request", "init_plan", "validation_issue", "init_report", "project_context", "stack_definition", "tool_plan"] {
        assert!(s["data"][k].is_object(), "missing schema {k}");
    }
    let desc = dispatch(&reg, &ctx, "stacks.describe", json!({"stack": "laravel"}), Approval::None).unwrap();
    let props = &desc["data"]["options_schema"]["properties"]["database"];
    assert_eq!(props["enum"], json!(["sqlite", "mysql", "mariadb", "pgsql"]));
}

#[test]
fn the_compatibility_matrix_document_is_current() {
    let reg = StackRegistry::builtin();
    let generated = devd_stacks::matrix::markdown(reg);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/stacks.md");
    if std::env::var_os("DEVD_UPDATE_DOCS").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(on_disk, generated, "docs/stacks.md is stale; run: DEVD_UPDATE_DOCS=1 cargo test -p devd-stacks");
}

#[test]
fn toolchain_plans_never_pretend() {
    let d = tempfile::tempdir().unwrap();
    let mut reg = Registry::new();
    caps::register(&mut reg).unwrap();
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    // Expo is a per-project dependency: there is nothing to install on the machine.
    let p = dispatch(&reg, &ctx, "toolchain.plan", json!({"tool": "expo"}), Approval::None).unwrap();
    assert!(!p["data"]["blockers"].as_array().unwrap().is_empty());
    assert!(dispatch(&reg, &ctx, "toolchain.plan", json!({"tool": "no-such-tool"}), Approval::None).is_err());
    let l = dispatch(&reg, &ctx, "toolchain.list", json!({"category": "database"}), Approval::None).unwrap();
    let tools = l["data"]["tools"].as_array().unwrap();
    assert!(tools.iter().all(|t| t["tool"]["category"] == "database") && tools.len() >= 4);
}

#[test]
fn services_found_on_the_machine_are_never_called_healthy_without_an_answer() {
    let d = tempfile::tempdir().unwrap();
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    ctx.store
        .upsert_stack_project(&devd_core::store_stacks::StackProject {
            path: d.path().join("blog").display().to_string(),
            name: "blog".into(),
            stack_id: Some("laravel".into()),
            package_manager: None,
            source: "import".into(),
            context: json!({ "services": [{ "tool": "redis", "why": "queue driver" }] }).to_string(),
            created_at: devd_core::util::now(),
            updated_at: devd_core::util::now(),
        })
        .unwrap();
    let inv = devd_stacks::reconcile::inventory(&ctx, StackRegistry::builtin()).unwrap();
    let rows = inv["services"].as_array().unwrap();
    assert!(rows.len() >= 4, "databases and docker are inspected");
    for r in rows {
        let status = r["status"].as_str().unwrap();
        if status == "ready" {
            assert!(r["installed"] == true && r["healthy"] == true && r["running"] == true, "{r}");
        }
        if r["healthy"] == true {
            assert_eq!(r["running"], true, "healthy needs an answering port: {r}");
        }
        assert!(!r["options"].as_array().unwrap().is_empty());
        // Replacement is never the recommendation.
        assert_ne!(r["recommendation"]["action"], "remove");
        if r["installed"] == false {
            assert_eq!(status, "missing");
        }
    }
    let redis = rows.iter().find(|r| r["tool"] == "redis").unwrap();
    assert_eq!(redis["required"], true);
    assert_eq!(redis["required_by"][0]["project"], "blog");
}

#[test]
fn a_decision_to_only_observe_stops_devd_controlling_the_service() {
    let d = tempfile::tempdir().unwrap();
    let ctx = ctx_with(d.path(), Actor::human(Surface::Cli));
    assert!(devd_stacks::reconcile::control_allowed(&ctx, "postgresql").is_ok());
    ctx.store.set_service_decision("postgresql", "observe", None, None, "human:cli").unwrap();
    let why = devd_stacks::reconcile::control_allowed(&ctx, "postgresql").unwrap_err();
    assert!(why.contains("only watch"), "{why}");
    ctx.store.set_service_decision("postgresql", "adopt", None, None, "human:cli").unwrap();
    assert!(devd_stacks::reconcile::control_allowed(&ctx, "postgresql").is_ok());
    ctx.store.set_service_decision("redis", "external", Some(6380), None, "human:cli").unwrap();
    assert!(devd_stacks::reconcile::control_allowed(&ctx, "redis").is_err());
}
