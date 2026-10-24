//! Deploys: a script per site, the run, its history, rollback, the repository link and the
//! push-to-deploy hook.
//!
//! The script is a list of command lines, run one after another in the site's folder with the
//! site's PHP first on PATH, stopping at the first one that fails (what `set -e` does on a server).
//! Lines that only make sense on a server - `cd` into the site folder, `sudo`, reloading PHP-FPM -
//! are skipped and the log says so, so a script copied from Ploi runs here unchanged.

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::store_ops::OpsItem;
use devd_core::{util, DevdError, Result};

/// Kinds under which things are stored.
pub const SCRIPT: &str = "deploy_script";
/// A deployment.
pub const DEPLOY: &str = "deploy";
/// A site's repository link.
pub const REPO: &str = "repo";

/// A script for a project that has none, from what the folder contains.
pub fn default_script(root: &std::path::Path) -> String {
    let laravel = root.join("artisan").is_file();
    let composer = root.join("composer.json").is_file();
    let node = root.join("package.json").is_file();
    let mut s = String::from("git pull origin {BRANCH}\n");
    if composer {
        s.push_str("composer install --no-interaction --prefer-dist --optimize-autoloader\n");
    }
    if laravel {
        s.push_str("php artisan migrate --force\n");
    }
    if node {
        s.push_str(if root.join("package-lock.json").is_file() { "npm ci\n" } else { "npm install\n" });
        s.push_str("npm run build\n");
    }
    if laravel {
        s.push_str("php artisan optimize\n");
    }
    s.push_str("echo \"Application deployed!\"\n");
    s
}

/// The saved script, or the default one for this project.
pub fn script(ctx: &Ctx, site: &str) -> Result<(String, bool)> {
    let (_, root) = crate::site_root(ctx, site)?;
    Ok(match ctx.store.ops_get(SCRIPT, site)? {
        Some(i) => (i.data["script"].as_str().unwrap_or("").to_string(), true),
        None => (default_script(&root), false),
    })
}

/// Save a script.
pub fn set_script(ctx: &Ctx, site: &str, script: &str) -> Result<()> {
    crate::site_named(ctx, site)?;
    if script.len() > 20_000 || script.contains('\0') {
        return Err(DevdError::invalid("a deploy script is text of at most 20000 characters"));
    }
    ctx.store.ops_put(SCRIPT, site, site, &json!({ "script": script.replace("\r\n", "\n") }))
}

/// Why a script line is not run here, or None when it is.
fn skipped(line: &str) -> Option<&'static str> {
    let l = line.trim();
    let first = l.split_whitespace().next().unwrap_or("");
    if first == "cd" {
        Some("it runs in the site folder already")
    } else if l.contains("sudo ") || first == "sudo" {
        Some("it is for a server")
    } else if l.contains("service php") && l.contains("reload") {
        Some("devd restarts PHP itself")
    } else {
        None
    }
}

/// The command lines of a script with its variables filled in.
pub fn lines(script: &str, root: &std::path::Path, branch: &str, skip_git_sync: bool) -> Vec<(String, Option<&'static str>)> {
    script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let filled = l.replace("{SITE_DIRECTORY}", &root.display().to_string()).replace("{BRANCH}", branch);
            let why = skipped(&filled).or_else(|| {
                (skip_git_sync && (filled.starts_with("git pull") || filled.starts_with("git fetch") || filled.starts_with("git stash"))).then_some("the code is already at the commit being restored")
            });
            (filled, why)
        })
        .collect()
}

pub(crate) fn git(ctx: &Ctx, root: &std::path::Path, args: &[&str]) -> Result<String> {
    let exe = ctx.settings.git_executable.clone().unwrap_or_else(|| "git".to_string());
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let out = util::run(std::path::Path::new(&exe), &args, Some(root), &[], Some(std::time::Duration::from_secs(60)))?;
    if out.code != 0 {
        return Err(DevdError::unsupported(format!("git {} failed: {}", args.join(" "), out.stderr.trim())));
    }
    Ok(out.stdout.trim().to_string())
}

pub(crate) fn head(ctx: &Ctx, root: &std::path::Path) -> Option<String> {
    git(ctx, root, &["rev-parse", "HEAD"]).ok().filter(|s| s.len() >= 7)
}

/// The branch a folder is on, else the configured default.
pub(crate) fn current_branch(ctx: &Ctx, root: &std::path::Path) -> String {
    branch_of(ctx, root)
}

fn branch_of(ctx: &Ctx, root: &std::path::Path) -> String {
    git(ctx, root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok().filter(|b| b != "HEAD").unwrap_or_else(|| ctx.settings.git_default_branch.clone())
}

/// A deployment as the capabilities show it.
#[derive(Debug, Clone, Serialize)]
pub struct Deployment {
    /// Id.
    pub id: String,
    /// The site.
    pub site: String,
    /// `running`, `success` or `failed`.
    pub state: String,
    /// `manual`, `hook`, `agent` or `rollback`.
    pub trigger: String,
    /// The commit before, when the folder is a Git repository.
    pub commit_before: Option<String>,
    /// The commit after.
    pub commit_after: Option<String>,
    /// The last commit's subject.
    pub message: Option<String>,
    /// When it started.
    pub started_at: String,
    /// When it ended.
    pub finished_at: Option<String>,
    /// How many seconds it took.
    pub seconds: Option<f64>,
    /// The script line that failed.
    pub failed_line: Option<String>,
    /// Where the output is.
    pub log: String,
    /// `<server>:<path>` when the deploy went to a server rather than this machine.
    pub target: Option<String>,
}

fn deployment_of(ctx: &Ctx, item: &OpsItem) -> Deployment {
    let d = &item.data;
    let s = |k: &str| d[k].as_str().map(str::to_string);
    Deployment {
        id: item.id.clone(),
        site: item.site.clone(),
        state: s("state").unwrap_or_default(),
        trigger: s("trigger").unwrap_or_default(),
        commit_before: s("commit_before"),
        commit_after: s("commit_after"),
        message: s("message"),
        started_at: s("started_at").unwrap_or_default(),
        finished_at: s("finished_at"),
        seconds: d["seconds"].as_f64(),
        failed_line: s("failed_line"),
        log: log_path(ctx, &item.id).display().to_string(),
        target: s("target"),
    }
}

pub(crate) fn log_path(ctx: &Ctx, id: &str) -> std::path::PathBuf {
    crate::log_dir(ctx).join(format!("deploy-{id}.log"))
}

/// A deployment that says it is running although the process that was running it is gone (devd was closed, or a
/// one-shot command exited) did not finish. Say so, instead of leaving it running for ever.
fn reconcile(ctx: &Ctx, item: OpsItem) -> OpsItem {
    if item.data["state"].as_str() != Some("running") {
        return item;
    }
    let owner_gone = match item.data["pid"].as_u64() {
        Some(pid) => !devd_core::process::pid_alive(pid as u32),
        // Written before the process was recorded: give it ten minutes.
        None => item.data["started_at"].as_str().is_some_and(|t| t < (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339().as_str()),
    };
    if !owner_gone {
        return item;
    }
    let mut data = item.data.clone();
    data["state"] = json!("failed");
    data["finished_at"] = json!(util::now());
    data["failed_line"] = json!("interrupted: devd closed before the deploy finished");
    let _ = ctx.store.ops_put(DEPLOY, &item.id, &item.site, &data);
    ctx.store.ops_get(DEPLOY, &item.id).ok().flatten().unwrap_or(item)
}

/// One deployment.
pub fn get(ctx: &Ctx, id: &str) -> Result<Deployment> {
    let item = ctx.store.ops_get(DEPLOY, id)?.ok_or_else(|| DevdError::not_found(format!("no deployment `{id}`")))?;
    Ok(deployment_of(ctx, &reconcile(ctx, item)))
}

/// A site's deployments, newest first.
pub fn history(ctx: &Ctx, site: &str, limit: usize) -> Result<Vec<Deployment>> {
    crate::site_named(ctx, site)?;
    Ok(ctx.store.ops_recent(DEPLOY, site, limit.clamp(1, 100))?.into_iter().map(|i| deployment_of(ctx, &reconcile(ctx, i))).collect())
}

/// The last `lines` lines of a deployment's output.
pub fn log(ctx: &Ctx, id: &str, lines: usize) -> Result<Vec<String>> {
    get(ctx, id)?;
    let text = crate::tail_bytes(&log_path(ctx, id), 400_000);
    let mut all: Vec<String> = text.lines().map(str::to_string).collect();
    let lines = lines.clamp(1, 2000);
    if all.len() > lines {
        all.drain(0..all.len() - lines);
    }
    Ok(all)
}

pub(crate) fn running_now(ctx: &Ctx, site: &str) -> Result<Option<String>> {
    Ok(ctx
        .store
        .ops_recent(DEPLOY, site, 5)?
        .into_iter()
        .map(|i| reconcile(ctx, i))
        .find(|i| i.data["state"].as_str() == Some("running"))
        .map(|i| i.id))
}

/// Start a deploy and return at once with the deployment; the script runs on its own thread.
///
/// `restore` is a commit to check out first (a rollback).
pub fn start(ctx: &Ctx, site: &str, trigger: &str, restore: Option<&str>) -> Result<Deployment> {
    let (s, root) = crate::site_root(ctx, site)?;
    if let Some(other) = running_now(ctx, site)? {
        return Err(DevdError::conflict(format!("deployment {other} of {site} is still running")));
    }
    let (script_text, _) = script(ctx, site)?;
    let vars = vars(ctx, site)?;
    let script_text = substitute(&script_text, &vars);
    let envs: Vec<(String, String)> = vars.into_iter().collect();
    let id = util::new_id()[..8].to_string();
    let before = head(ctx, &root);
    let started = util::now();
    let mut doc = json!({ "state": "running", "trigger": trigger, "commit_before": before, "started_at": started, "restore": restore, "pid": std::process::id() });
    ctx.store.ops_put(DEPLOY, &id, site, &doc)?;
    let ctx2 = ctx.clone();
    let id2 = id.clone();
    let site2 = s.clone();
    let restore = restore.map(str::to_string);
    std::thread::spawn(move || {
        let ctx = ctx2;
        let log = log_path(&ctx, &id2);
        let mut header = format!("deploy {id2} of {} started {started}\n", site2.name);
        for w in crate::limits::warnings(&ctx, &site2.name) {
            header.push_str(&format!("note: {w}\n"));
        }
        let _ = std::fs::write(&log, header);
        let t0 = std::time::Instant::now();
        let mut failed: Option<String> = None;
        if let Some(sha) = &restore {
            let ran = crate::run_line_with(&ctx, &site2, &root, &format!("git checkout --detach {sha}"), &log, &envs);
            if ran.exit != 0 {
                failed = Some(format!("git checkout --detach {sha}"));
            }
        }
        if failed.is_none() {
            let branch = ctx.store.ops_get(REPO, &site2.name).ok().flatten().and_then(|i| i.data["branch"].as_str().map(str::to_string)).unwrap_or_else(|| branch_of(&ctx, &root));
            for (line, why) in lines(&script_text, &root, &branch, restore.is_some()) {
                if let Some(why) = why {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&log) {
                        let _ = writeln!(f, "(skipped: {line} - {why})");
                    }
                    continue;
                }
                if crate::run_line_with(&ctx, &site2, &root, &line, &log, &envs).exit != 0 {
                    failed = Some(line);
                    break;
                }
            }
        }
        let after = head(&ctx, &root);
        let message = git(&ctx, &root, &["log", "-1", "--pretty=%s"]).ok();
        doc["state"] = json!(if failed.is_none() { "success" } else { "failed" });
        doc["commit_after"] = json!(after);
        doc["message"] = json!(message);
        doc["finished_at"] = json!(util::now());
        doc["seconds"] = json!(t0.elapsed().as_secs_f64());
        doc["failed_line"] = json!(failed);
        let _ = ctx.store.ops_put(DEPLOY, &id2, &site2.name, &doc);
        let (event, text) = match &failed {
            None => ("deploy.finished", format!("{} deployed ({}s){}", site2.name, t0.elapsed().as_secs(), message.map(|m| format!(": {m}")).unwrap_or_default())),
            Some(l) => ("deploy.failed", format!("Deploy of {} failed at `{l}`", site2.name)),
        };
        crate::notify::send(&ctx, event, &site2.name, &text, json!({ "deployment": id2, "failed_line": failed }));
    });
    get(ctx, &id)
}

// ------------------------------------------------------------------------------------ variables

/// The kind under which a site's deploy variables are stored.
pub const VARS: &str = "deploy_vars";

/// A variable name for a deploy: capital letters, digits and underscores, not starting with a digit.
pub fn valid_var(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with(|c: char| c.is_ascii_digit()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A site's deploy variables.
pub fn vars(ctx: &Ctx, site: &str) -> Result<std::collections::BTreeMap<String, String>> {
    crate::site_named(ctx, site)?;
    Ok(ctx
        .store
        .ops_get(VARS, site)?
        .map(|i| i.data.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default())
        .unwrap_or_default())
}

/// Set (or, with `None`, remove) deploy variables. The values are available as `{NAME}` in the script and as environment
/// variables to every command in it.
pub fn set_vars(ctx: &Ctx, site: &str, set: &std::collections::BTreeMap<String, Option<String>>) -> Result<std::collections::BTreeMap<String, String>> {
    let mut cur = vars(ctx, site)?;
    for (k, v) in set {
        if !valid_var(k) {
            return Err(DevdError::invalid(format!("`{k}` is not a variable name (capital letters, digits and underscores)")));
        }
        if matches!(k.as_str(), "PATH" | "DEVD_SITE" | "BRANCH" | "SITE_DIRECTORY") {
            return Err(DevdError::invalid(format!("`{k}` is used by devd itself")));
        }
        match v {
            Some(value) => {
                if value.len() > 2000 || value.contains('\0') {
                    return Err(DevdError::invalid(format!("the value of {k} is too long or has a NUL character")));
                }
                cur.insert(k.clone(), value.clone());
            }
            None => {
                cur.remove(k);
            }
        }
    }
    let doc: serde_json::Map<String, Value> = cur.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    ctx.store.ops_put(VARS, site, site, &Value::Object(doc))?;
    Ok(cur)
}

/// Fill `{NAME}` in a script from the variables.
pub fn substitute(script: &str, vars: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = script.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

// ----------------------------------------------------------------------------- scheduled deploys

/// The kind under which scheduled deployments are stored.
pub const SCHEDULE: &str = "deploy_schedule";

/// When a deploy should run: an RFC 3339 time, or `in 30m`, `in 2h`, `in 1d`.
pub fn parse_when(text: &str) -> Result<chrono::DateTime<chrono::Local>> {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("in ") {
        let rest = rest.trim();
        let split = rest.char_indices().find(|(_, c)| !c.is_ascii_digit()).map(|(i, _)| i).unwrap_or(rest.len());
        let (digits, unit) = rest.split_at(split);
        let n: i64 = digits.parse().map_err(|_| DevdError::invalid("write it like `in 30m`, `in 2h` or `in 1d`"))?;
        let d = match unit.trim() {
            "m" | "min" | "minutes" => chrono::Duration::minutes(n),
            "h" | "hours" => chrono::Duration::hours(n),
            "d" | "days" => chrono::Duration::days(n),
            _ => return Err(DevdError::invalid("write it like `in 30m`, `in 2h` or `in 1d`")),
        };
        if n <= 0 || d > chrono::Duration::days(60) {
            return Err(DevdError::invalid("a scheduled deploy is between a minute and 60 days away"));
        }
        return Ok(chrono::Local::now() + d);
    }
    chrono::DateTime::parse_from_rfc3339(t).map(|d| d.with_timezone(&chrono::Local)).map_err(|_| DevdError::invalid("write the time like `2026-10-24T09:00:00+07:00`, or `in 30m`"))
}

/// Schedule a deploy.
pub fn schedule(ctx: &Ctx, site: &str, when: &str, note: Option<&str>) -> Result<Value> {
    crate::site_named(ctx, site)?;
    let at = parse_when(when)?;
    if at < chrono::Local::now() {
        return Err(DevdError::invalid("that time has already passed"));
    }
    let id = util::new_id()[..8].to_string();
    ctx.store.ops_put(SCHEDULE, &id, site, &json!({ "at": at.to_rfc3339(), "state": "pending", "note": note.map(|n| n.chars().take(200).collect::<String>()) }))?;
    Ok(json!({ "id": id, "site": site, "at": at.to_rfc3339(), "state": "pending" }))
}

/// Scheduled deployments, optionally one site's.
pub fn scheduled(ctx: &Ctx, site: Option<&str>) -> Result<Vec<Value>> {
    Ok(ctx
        .store
        .ops_list(SCHEDULE, site)?
        .into_iter()
        .map(|i| {
            let mut v = i.data.clone();
            v["id"] = json!(i.id);
            v["site"] = json!(i.site);
            v
        })
        .collect())
}

/// Cancel a scheduled deployment that has not run.
pub fn unschedule(ctx: &Ctx, id: &str) -> Result<()> {
    let item = ctx.store.ops_get(SCHEDULE, id)?.ok_or_else(|| DevdError::not_found(format!("no scheduled deployment `{id}`")))?;
    if item.data["state"].as_str() != Some("pending") {
        return Err(DevdError::conflict("that deployment already ran or was missed"));
    }
    let mut d = item.data.clone();
    d["state"] = json!("cancelled");
    ctx.store.ops_put(SCHEDULE, id, &item.site, &d)
}

/// How late a scheduled deploy may start. Later than this devd was closed, and it is marked missed, not run at a
/// time nobody chose.
const LATE: chrono::Duration = chrono::Duration::minutes(10);

/// Start every scheduled deployment that is due. Returns how many started.
pub fn tick_scheduled(ctx: &Ctx) -> usize {
    let Ok(items) = ctx.store.ops_list(SCHEDULE, None) else { return 0 };
    let now = chrono::Local::now();
    let mut started = 0;
    for item in items {
        if item.data["state"].as_str() != Some("pending") {
            continue;
        }
        let Some(at) = item.data["at"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&chrono::Local)) else { continue };
        if at > now {
            continue;
        }
        let mut d = item.data.clone();
        if now.signed_duration_since(at) > LATE {
            d["state"] = json!("missed");
            let _ = ctx.store.ops_put(SCHEDULE, &item.id, &item.site, &d);
            crate::notify::send(ctx, "deploy.failed", &item.site, &format!("The deploy scheduled for {} did not run: devd was closed at the time", at.format("%Y-%m-%d %H:%M")), json!({ "schedule": item.id }));
            continue;
        }
        match start(ctx, &item.site, "scheduled", None) {
            Ok(dep) => {
                d["state"] = json!("started");
                d["deployment"] = json!(dep.id);
                started += 1;
            }
            Err(e) => {
                d["state"] = json!("failed");
                d["error"] = json!(e.to_string());
                crate::notify::send(ctx, "deploy.failed", &item.site, &format!("The scheduled deploy of {} could not start: {e}", item.site), json!({ "schedule": item.id }));
            }
        }
        let _ = ctx.store.ops_put(SCHEDULE, &item.id, &item.site, &d);
    }
    started
}

// --------------------------------------------------------------------------------------- staging

/// The kind under which staging sites are stored (keyed by the staging site's name).
pub const STAGING: &str = "staging";

fn staging_of(ctx: &Ctx, site: &str) -> Result<OpsItem> {
    let all = ctx.store.ops_list(STAGING, None)?;
    all.into_iter()
        .find(|i| i.id == site || i.data["of"].as_str() == Some(site))
        .ok_or_else(|| DevdError::not_found(format!("{site} has no staging site; `deploy.staging_create` makes one")))
}

/// Make a staging copy of a site: a second working folder of the same repository on its own branch, served as
/// `<site>-staging`, with the production `.env` adjusted. A git worktree, so the two share history and a promotion is a merge.
pub fn staging_create(ctx: &Ctx, site: &str, branch: Option<&str>) -> Result<Value> {
    let (s, root) = crate::site_root(ctx, site)?;
    if !root.join(".git").exists() {
        return Err(DevdError::invalid(format!("{site} is not a Git repository, so it has nothing to branch a staging site from")));
    }
    let name = format!("{site}-staging");
    if ctx.store.ops_get(STAGING, &name)?.is_some() || ctx.store.get_site(&name)?.is_some() {
        return Err(DevdError::conflict(format!("{name} already exists")));
    }
    let path = root.parent().ok_or_else(|| DevdError::internal("a project with no parent folder"))?.join(&name);
    if path.exists() {
        return Err(DevdError::conflict(format!("{} already exists", path.display())));
    }
    let branch = branch.unwrap_or("staging");
    if branch.starts_with('-') || branch.contains(char::is_whitespace) || branch.contains("..") {
        return Err(DevdError::invalid("that is not a branch name"));
    }
    let path_text = path.display().to_string();
    let exists = git(ctx, &root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    if exists {
        git(ctx, &root, &["worktree", "add", &path_text, branch])?;
    } else {
        git(ctx, &root, &["worktree", "add", "-b", branch, &path_text, "HEAD"])?;
    }
    let created = devd_sites::create(ctx, &devd_sites::CreateRequest { name: name.clone(), host: None, root: Some(path_text.clone()), tls: true, db: false, upstream: None, serving: s.serving });
    let created = match created {
        Ok(c) => c,
        Err(e) => {
            let _ = git(ctx, &root, &["worktree", "remove", &path_text]);
            return Err(e);
        }
    };
    // Staging runs on what production runs on: a PHP version someone pinned for production is pinned here too,
    // and a site that follows the default follows it here.
    if let (Some(pin), Ok(Some(mut staging_site))) = (s.php.clone(), ctx.store.get_site(&name)) {
        staging_site.php = Some(pin);
        let _ = ctx.store.upsert_site(&staging_site);
    }
    // The environment: the same file, told it is staging and where it lives.
    if let Ok(text) = std::fs::read_to_string(crate::env::path_of(&root)) {
        let host = ctx.store.get_site(&name)?.map(|x| x.host).unwrap_or_else(|| format!("{name}.{}", ctx.settings.tld));
        let set = std::collections::BTreeMap::from([("APP_ENV".to_string(), "staging".to_string()), ("APP_URL".to_string(), format!("https://{host}"))]);
        if let Ok(new) = crate::env::with_set(&text, &set) {
            let _ = std::fs::write(crate::env::path_of(&path), new);
        }
    }
    // Its own deploy script: the production one without the pull, because the folder already holds the branch.
    let (script_text, _) = script(ctx, site)?;
    let adapted: String = script_text.lines().filter(|l| !l.trim_start().starts_with("git pull") && !l.trim_start().starts_with("git fetch")).collect::<Vec<_>>().join("\n");
    let _ = set_script(ctx, &name, &(adapted + "\n"));
    ctx.store.ops_put(STAGING, &name, &name, &json!({ "of": site, "branch": branch, "path": path_text, "base_site": s.name }))?;
    Ok(json!({ "staging_site": name, "branch": branch, "path": path_text, "site": created, "next": "Run `deploy.run` on the staging site to install its dependencies and build it." }))
}

/// Merge the staging branch into the production site's branch (fast-forward only) and deploy production.
pub fn staging_promote(ctx: &Ctx, site: &str) -> Result<Deployment> {
    let st = staging_of(ctx, site)?;
    let prod = st.data["of"].as_str().unwrap_or("").to_string();
    let branch = st.data["branch"].as_str().unwrap_or("").to_string();
    let (_, root) = crate::site_root(ctx, &prod)?;
    let dirty = git(ctx, &root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        return Err(DevdError::conflict(format!("{prod} has uncommitted changes ({} file(s)); commit or stash them first", dirty.lines().count())));
    }
    git(ctx, &root, &["merge", "--ff-only", &branch])?;
    start(ctx, &prod, "promote", None)
}

/// Remove a staging site: its worktree (refused while it has uncommitted changes), its site, its record. The branch stays.
pub fn staging_remove(ctx: &Ctx, site: &str) -> Result<Value> {
    let st = staging_of(ctx, site)?;
    let prod = st.data["of"].as_str().unwrap_or("").to_string();
    let path = st.data["path"].as_str().unwrap_or("").to_string();
    let (_, root) = crate::site_root(ctx, &prod)?;
    // Tracked changes are work that would be lost; untracked files here are the copied .env, installed dependencies and build output.
    let staging_root = std::path::PathBuf::from(&path);
    if staging_root.exists() {
        let dirty = git(ctx, &staging_root, &["status", "--porcelain", "--untracked-files=no"])?;
        if !dirty.is_empty() {
            return Err(DevdError::conflict(format!("{} has uncommitted changes ({} file(s)); commit or stash them first", st.id, dirty.lines().count())));
        }
    }
    git(ctx, &root, &["worktree", "remove", "--force", &path])?;
    let _ = devd_sites::remove(ctx, &st.id, false);
    ctx.store.ops_delete(STAGING, &st.id)?;
    Ok(json!({ "removed": st.id, "branch_kept": st.data["branch"] }))
}

/// Staging sites.
pub fn staging_list(ctx: &Ctx) -> Result<Vec<Value>> {
    Ok(ctx
        .store
        .ops_list(STAGING, None)?
        .into_iter()
        .map(|i| {
            let mut v = i.data.clone();
            v["staging_site"] = json!(i.id);
            v
        })
        .collect())
}

/// Wait for a deployment to end, up to two hours, and return it as it ended.
pub fn wait(ctx: &Ctx, id: &str) -> Result<Deployment> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2 * 3600);
    loop {
        let d = get(ctx, id)?;
        if d.state != "running" || std::time::Instant::now() > until {
            return Ok(d);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Which commit a rollback goes to: the one the named deployment ended on, else the commit before the newest deployment.
pub fn rollback_target(ctx: &Ctx, site: &str, deployment: Option<&str>) -> Result<(String, String)> {
    let all = ctx.store.ops_recent(DEPLOY, site, 100)?;
    if let Some(id) = deployment {
        let d = all.iter().find(|i| i.id == id).ok_or_else(|| DevdError::not_found(format!("no deployment `{id}` of {site}")))?;
        let sha = d.data["commit_after"].as_str().ok_or_else(|| DevdError::invalid("that deployment did not record a commit"))?;
        return Ok((sha.to_string(), format!("the code of deployment {id}")));
    }
    let newest = all.iter().find(|i| i.data["state"].as_str() == Some("success")).ok_or_else(|| DevdError::not_found("there is no successful deployment to roll back from"))?;
    let sha = newest.data["commit_before"].as_str().ok_or_else(|| DevdError::invalid("the newest deployment did not record the commit before it"))?;
    Ok((sha.to_string(), format!("the commit before deployment {}", newest.id)))
}

/// Check out an earlier commit and run the script. Refuses when there are uncommitted changes.
pub fn rollback(ctx: &Ctx, site: &str, deployment: Option<&str>, trigger: &str) -> Result<Deployment> {
    let (_, root) = crate::site_root(ctx, site)?;
    let (sha, _) = rollback_target(ctx, site, deployment)?;
    let dirty = git(ctx, &root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.is_empty() {
        return Err(DevdError::conflict(format!("{site} has uncommitted changes ({} file(s)); commit or stash them before a rollback", dirty.lines().count())));
    }
    start(ctx, site, trigger, Some(&sha))
}

// ----------------------------------------------------------------------------- repository link

/// A site's repository link, with the live state of the folder.
pub fn repo_status(ctx: &Ctx, site: &str) -> Result<Value> {
    let (_, root) = crate::site_root(ctx, site)?;
    let link = ctx.store.ops_get(REPO, site)?;
    let is_repo = root.join(".git").exists();
    let (mut branch, mut sha, mut remote, mut dirty, mut ahead_behind) = (None, None, None, None, None);
    if is_repo {
        branch = Some(branch_of(ctx, &root));
        sha = head(ctx, &root);
        remote = git(ctx, &root, &["remote", "get-url", "origin"]).ok();
        dirty = git(ctx, &root, &["status", "--porcelain"]).ok().map(|s| s.lines().count());
        ahead_behind = git(ctx, &root, &["rev-list", "--left-right", "--count", "@{u}...HEAD"]).ok();
    }
    Ok(json!({
        "site": site,
        "is_repository": is_repo,
        "linked": link.is_some(),
        "link": link.as_ref().map(|l| json!({ "branch": l.data["branch"], "deploy_on_push": l.data["deploy_on_push"].as_bool().unwrap_or(false) })),
        "branch": branch, "head": sha, "origin": remote, "uncommitted_files": dirty,
        "behind_ahead_of_upstream": ahead_behind,
    }))
}

/// Link a site to a branch of its repository, adding `origin` when a remote is given and there is none.
pub fn repo_link(ctx: &Ctx, site: &str, remote: Option<&str>, branch: Option<&str>) -> Result<Value> {
    let (_, root) = crate::site_root(ctx, site)?;
    if !root.join(".git").exists() {
        return Err(DevdError::invalid(format!("{} is not a Git repository; `devd git repo.init` makes it one", root.display())));
    }
    if let Some(r) = remote {
        if r.starts_with('-') || r.contains(char::is_whitespace) || r.len() > 500 {
            return Err(DevdError::invalid("that is not a usable remote address"));
        }
        if git(ctx, &root, &["remote", "get-url", "origin"]).is_err() {
            git(ctx, &root, &["remote", "add", "origin", r])?;
        }
    }
    let branch = branch.map(str::to_string).unwrap_or_else(|| branch_of(ctx, &root));
    if branch.starts_with('-') || branch.contains(char::is_whitespace) {
        return Err(DevdError::invalid("that is not a branch name"));
    }
    let old = ctx.store.ops_get(REPO, site)?;
    let token = old.as_ref().and_then(|o| o.data["hook_token"].as_str().map(str::to_string));
    let on = old.as_ref().and_then(|o| o.data["deploy_on_push"].as_bool()).unwrap_or(false);
    ctx.store.ops_put(REPO, site, site, &json!({ "branch": branch, "deploy_on_push": on, "hook_token": token }))?;
    repo_status(ctx, site)
}

/// Turn the push-to-deploy hook on or off. Turning it on makes a fresh secret; the hook address is
/// `/hooks/deploy/<site>/<secret>` on the devd server.
pub fn set_hook(ctx: &Ctx, site: &str, enabled: bool) -> Result<Value> {
    let item = ctx.store.ops_get(REPO, site)?.ok_or_else(|| DevdError::invalid(format!("link {site} to its repository first (`repo.link`)")))?;
    let mut d = item.data.clone();
    d["deploy_on_push"] = json!(enabled);
    if enabled {
        d["hook_token"] = json!(format!("{}{}", util::new_id().replace('-', ""), util::new_id().replace('-', "")));
    } else {
        d["hook_token"] = Value::Null;
    }
    ctx.store.ops_put(REPO, site, site, &d)?;
    Ok(json!({
        "site": site, "enabled": enabled,
        "path": d["hook_token"].as_str().map(|t| format!("/hooks/deploy/{site}/{t}")),
        "note": "POST to this path on the devd server to deploy. It is a secret: anyone with the address can start a deploy. To receive GitHub's push events, open a public link to the devd server (`devd site share`) and give GitHub that address."
    }))
}

/// Whether a hook secret is right for a site. The comparison does not stop at the first different byte.
pub fn hook_ok(ctx: &Ctx, site: &str, token: &str) -> bool {
    let Ok(Some(item)) = ctx.store.ops_get(REPO, site) else { return false };
    if !item.data["deploy_on_push"].as_bool().unwrap_or(false) {
        return false;
    }
    let Some(want) = item.data["hook_token"].as_str() else { return false };
    let (a, b) = (want.as_bytes(), token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_script_follows_the_project() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("artisan"), "").unwrap();
        std::fs::write(d.path().join("composer.json"), "{}").unwrap();
        std::fs::write(d.path().join("package.json"), "{}").unwrap();
        std::fs::write(d.path().join("package-lock.json"), "{}").unwrap();
        let s = default_script(d.path());
        for want in ["git pull origin {BRANCH}", "composer install", "php artisan migrate --force", "npm ci", "npm run build", "php artisan optimize"] {
            assert!(s.contains(want), "{want} in {s}");
        }
        let bare = tempfile::tempdir().unwrap();
        assert!(!default_script(bare.path()).contains("composer"));
    }

    #[test]
    fn server_only_lines_are_skipped_and_variables_filled() {
        let script = "# deploy\ncd /home/ploi/shop.com\ngit pull origin {BRANCH}\ncomposer install\necho \"\" | sudo -S service php8.4-fpm reload\nphp artisan migrate --force\n";
        let l = lines(script, std::path::Path::new("D:/www/shop"), "main", false);
        assert_eq!(l.len(), 5);
        assert!(l[0].1.is_some(), "cd is skipped");
        assert_eq!(l[1], ("git pull origin main".to_string(), None));
        assert!(l[3].1.is_some(), "sudo is skipped");
        let restoring = lines(script, std::path::Path::new("D:/www/shop"), "main", true);
        assert!(restoring[1].1.is_some(), "a rollback does not pull");
    }
}
