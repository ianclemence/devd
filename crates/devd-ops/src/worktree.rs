//! A site for every git worktree.
//!
//! A worktree is a second checkout of the same repository on its own branch. That is how a
//! person reviews a branch without stashing, and how several agents can work on one project at
//! once without treading on each other. Each of them needs somewhere to *look at* what it
//! built, so each worktree is served as a site of its own: `<branch>.<site>.test`, with its own
//! `.env` pointing at that address and, when asked, its own copy of the database.
//!
//! There is no new record. A worktree site is an ordinary site whose folder is the worktree and
//! whose name is `<site>-<branch>`; which sites belong to which parent is read from the folders,
//! the way every other relation in devd is. Removing the site is `site.remove`; removing the
//! checkout is `git.worktree.remove`, kept separate because it deletes files.
//!
//! What this does not do: install dependencies. A fresh checkout has no `vendor/` or
//! `node_modules/`, and installing them runs a package manager for minutes; the result lists
//! what is missing so the person (or the agent that asked) runs it where it can be watched.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::state::Site;
use devd_core::util;
use devd_core::{DevdError, Result};

/// What a worktree gets for its database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// Use the parent site's database, which is the default: nothing to set up, nothing to drift.
    Share,
    /// A copy of it under a new name, so a migration on this branch cannot touch the parent's.
    Clone,
    /// Leave `DB_DATABASE` as the `.env` has it.
    Leave,
}

impl Database {
    pub fn parse(s: Option<&str>) -> Result<Self> {
        match s.map(str::trim).filter(|v| !v.is_empty()) {
            None | Some("share") => Ok(Self::Share),
            Some("clone") => Ok(Self::Clone),
            Some("none") | Some("leave") => Ok(Self::Leave),
            Some(other) => Err(DevdError::invalid(format!("`{other}` is not a database choice; use share, clone or none"))),
        }
    }
}

/// A branch as a subdomain label: `feature/Auth_v2` becomes `feature-auth-v2`.
pub fn slug(branch: &str) -> String {
    let mut out = String::new();
    for c in branch.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if matches!(c, '/' | '_' | '.' | '-') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    let out: String = out.chars().take(40).collect();
    out.trim_end_matches('-').to_string()
}

/// A name for a worktree made without one, like `brave-otter`: a branch name and a subdomain
/// at once. `taken` says which are already in use.
pub fn made_up_name(seed: u64, taken: &dyn Fn(&str) -> bool) -> String {
    const ADJECTIVES: [&str; 16] = [
        "brave", "calm", "eager", "gentle", "bright", "quick", "quiet", "witty", "bold", "keen", "lucky", "merry", "noble", "sunny", "swift", "wise",
    ];
    const ANIMALS: [&str; 16] = [
        "otter", "heron", "lynx", "finch", "badger", "marmot", "falcon", "gecko", "ibis", "koala", "lemur", "newt", "osprey", "panda", "quail", "raven",
    ];
    for i in 0..256u64 {
        let n = seed.wrapping_add(i.wrapping_mul(0x9E37_79B9));
        let name = format!("{}-{}", ADJECTIVES[(n % 16) as usize], ANIMALS[((n / 16) % 16) as usize]);
        if !taken(&name) {
            return name;
        }
    }
    format!("worktree-{seed}")
}

/// `slug` made unique among the names and hosts already in use: `feature-auth`, then
/// `feature-auth-2`.
fn unique_slug(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..1000).map(|n| format!("{base}-{n}")).find(|c| !taken(c)).unwrap_or_else(|| format!("{base}-x"))
}

/// The `.env` text for a worktree: the parent's, pointed at the worktree's address and, when it
/// has its own database, at that. Only keys that already say the parent's host are rewritten,
/// so a value the project chose for another reason is left alone.
pub fn env_for(parent_env: &str, parent_host: &str, new_url: &str, new_host: &str, database: Option<&str>) -> Result<String> {
    use std::collections::BTreeMap;
    let current = crate::env::variables(parent_env);
    let mut set: BTreeMap<String, String> = BTreeMap::new();
    set.insert("APP_URL".into(), new_url.to_string());
    for key in ["SESSION_DOMAIN", "SANCTUM_STATEFUL_DOMAINS", "ASSET_URL", "VITE_APP_URL"] {
        if let Some(v) = current.get(key) {
            if v.contains(parent_host) {
                set.insert(key.to_string(), v.replace(parent_host, new_host));
            }
        }
    }
    if let Some(db) = database {
        set.insert("DB_DATABASE".into(), db.to_string());
    }
    crate::env::with_set(parent_env, &set)
}

fn parent_site(ctx: &Ctx, name: &str) -> Result<Site> {
    ctx.store
        .list_sites()?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DevdError::not_found(format!("no site named `{name}`")))
}

fn same(a: &Path, b: &Path) -> bool {
    util::same_folder(a, b)
}

/// The folder a site is served from, relative to its project (`public`, or nothing).
fn docroot_rel(site: &Site) -> PathBuf {
    let project = site.project_dir();
    Path::new(&site.root).strip_prefix(&project).map(|p| p.to_path_buf()).unwrap_or_default()
}

fn git(ctx: &Ctx) -> Result<devd_git::GitService> {
    devd_git::GitService::discover(ctx.settings.git_executable.as_deref())
}

/// The worktrees of a site's repository, and the site (if any) each is served as.
pub fn list(ctx: &Ctx, name: &str) -> Result<Value> {
    let parent = parent_site(ctx, name)?;
    let project = parent.project_dir();
    let git = git(ctx)?;
    let trees = git.worktrees(&project).map_err(|e| DevdError::conflict(format!("`{name}` is not in a git repository devd can read: {e}")))?;
    let sites = devd_sites::list(ctx)?;
    let rows: Vec<Value> = trees
        .iter()
        .map(|t| {
            let path = PathBuf::from(&t.path);
            let main = t.main || same(&path, &project);
            let served = sites.iter().find(|r| !main && same(&r.site.project_dir(), &path));
            let missing: Vec<&str> = ["vendor", "node_modules"]
                .into_iter()
                .filter(|d| project.join(d).is_dir() && !path.join(d).is_dir())
                .collect();
            json!({
                "path": t.path,
                "branch": t.branch,
                "head": t.head,
                "main": main,
                "slug": t.branch.as_deref().map(slug),
                "site": served.map(|r| r.site.name.clone()),
                "host": served.map(|r| r.site.host.clone()),
                "url": served.map(|r| r.url.clone()),
                "has_env": path.join(".env").is_file(),
                "missing_dependencies": missing,
            })
        })
        .collect();
    Ok(json!({ "site": name, "project": project.display().to_string(), "worktrees": rows }))
}

/// Serve an existing worktree as a site and set it up.
pub fn serve(ctx: &Ctx, parent_name: &str, worktree: &Path, database: Database) -> Result<Value> {
    let parent = parent_site(ctx, parent_name)?;
    let project = parent.project_dir();
    if !worktree.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a folder", worktree.display())));
    }
    if same(worktree, &project) {
        return Err(DevdError::invalid("that is the site's own folder, which the site already serves"));
    }
    let sites = ctx.store.list_sites()?;
    if let Some(existing) = sites.iter().find(|s| same(&s.project_dir(), worktree)) {
        return Err(DevdError::conflict(format!("{} is already served by the site `{}`", worktree.display(), existing.name)));
    }

    // The branch the worktree is on names it; a detached one is named after its folder.
    let trees = git(ctx)?.worktrees(&project).unwrap_or_default();
    let branch = trees.iter().find(|t| same(Path::new(&t.path), worktree)).and_then(|t| t.branch.clone());
    let label = branch.as_deref().map(slug).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        slug(&worktree.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
    });
    if label.is_empty() {
        return Err(DevdError::invalid("could not make a hostname from that branch or folder name"));
    }
    let taken = |c: &str| sites.iter().any(|s| s.name == format!("{}-{c}", parent.name) || s.host.eq_ignore_ascii_case(&format!("{c}.{}", parent.host)));
    let label = unique_slug(&label, &taken);
    let name = format!("{}-{label}", parent.name);
    let host = format!("{label}.{}", parent.host);

    let rel = docroot_rel(&parent);
    let root = if rel.as_os_str().is_empty() || !worktree.join(&rel).is_dir() { worktree.to_path_buf() } else { worktree.join(&rel) };

    let created = devd_sites::create_for_existing_folder(
        ctx,
        &devd_sites::CreateRequest {
            name: name.clone(),
            host: Some(host.clone()),
            root: Some(root.display().to_string()),
            tls: parent.tls_mode != "off",
            db: false,
            upstream: None,
            serving: parent.serving.clone(),
        },
    )?;

    // The worktree runs on the same PHP the parent has chosen for itself on this machine; what
    // the project pins is read from its own files, which the worktree carries.
    if parent.php.is_some() {
        let _ = devd_sites::set_php(ctx, &name, parent.php.as_deref());
    }

    let row = devd_sites::list(ctx)?.into_iter().find(|r| r.site.name == name);
    let url = row.as_ref().map(|r| r.url.clone()).unwrap_or_else(|| format!("http://{host}"));

    let mut steps: Vec<String> = vec![format!("served at {url}")];
    let mut warnings: Vec<String> = Vec::new();

    // The database first, so the .env can name it.
    let mut db_name: Option<String> = None;
    if database == Database::Clone {
        match parent.db_name.as_deref() {
            Some(db) => match clone_database(ctx, db, &label) {
                Ok(new) => {
                    steps.push(format!("copied database `{db}` to `{new}`"));
                    db_name = Some(new);
                }
                Err(e) => warnings.push(format!("the database was not copied, so the worktree uses the parent's: {e}")),
            },
            None => warnings.push("the parent site has no database managed by devd, so there is nothing to copy".to_string()),
        }
    }

    // The .env: the worktree's own, if the branch has one; else the parent's; else the example.
    let target = worktree.join(".env");
    if !target.exists() {
        let source = [project.join(".env"), worktree.join(".env.example")].into_iter().find(|p| p.is_file());
        match source {
            Some(src) => match std::fs::read_to_string(&src) {
                Ok(text) => match env_for(&text, &parent.host, &url, &host, db_name.as_deref()) {
                    Ok(updated) => {
                        std::fs::write(&target, updated)?;
                        steps.push(format!("wrote .env from {} pointing at {url}", src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                    }
                    Err(e) => warnings.push(format!("the .env was not written: {e}")),
                },
                Err(e) => warnings.push(format!("could not read {}: {e}", src.display())),
            },
            None => steps.push("no .env to copy: the project has none".to_string()),
        }
    } else if let Some(db) = &db_name {
        warnings.push(format!("the worktree already has a .env, so it was not changed; set DB_DATABASE={db} in it to use the copy"));
    }

    let missing: Vec<&str> = ["vendor", "node_modules"]
        .into_iter()
        .filter(|d| project.join(d).is_dir() && !worktree.join(d).is_dir())
        .collect();
    let mut todo: Vec<String> = Vec::new();
    if missing.contains(&"vendor") {
        todo.push("composer install".to_string());
    }
    if missing.contains(&"node_modules") {
        todo.push("npm install".to_string());
    }
    if created.certificate.is_none() && parent.tls_mode != "off" {
        warnings.push("no certificate was issued".to_string());
    }

    Ok(json!({
        "site": name,
        "host": host,
        "url": url,
        "folder": worktree.display().to_string(),
        "branch": branch,
        "database": db_name.or_else(|| parent.db_name.clone()),
        "steps": steps,
        "todo": todo,
        "warnings": warnings,
        "note": "`devd hosts sync` adds the new name to the hosts file; it asks first",
    }))
}

/// Check out a branch as a new worktree and serve it.
pub fn create(ctx: &Ctx, parent_name: &str, branch: Option<&str>, from: Option<&str>, database: Database) -> Result<Value> {
    let parent = parent_site(ctx, parent_name)?;
    let project = parent.project_dir();
    let git = git(ctx)?;
    let trees = git.worktrees(&project).map_err(|e| DevdError::conflict(format!("`{parent_name}` is not in a git repository devd can read: {e}")))?;
    let used = |c: &str| {
        trees.iter().any(|t| t.branch.as_deref().map(slug).as_deref() == Some(c))
            || ctx.store.list_sites().map(|s| s.iter().any(|x| x.name == format!("{}-{c}", parent.name))).unwrap_or(false)
    };

    let (branch, create_branch) = match branch.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => {
            // A branch that exists is checked out; a new name is created from `from` (or HEAD).
            let exists = git.branches(&project, true).map(|bs| bs.iter().any(|x| x.name == b || x.name.ends_with(&format!("/{b}")))).unwrap_or(false);
            (b.to_string(), !exists)
        }
        None => {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            (made_up_name(seed, &used), true)
        }
    };
    let label = slug(&branch);
    if label.is_empty() {
        return Err(DevdError::invalid(format!("`{branch}` cannot make a hostname")));
    }
    // Worktrees live under devd's own folder, outside any parked folder: a parked folder turns
    // every subfolder into a site, and these are served deliberately, under their own names.
    let path = ctx.paths.root().join("worktrees").join(&parent.name).join(&label);
    if path.exists() {
        return Err(DevdError::conflict(format!("{} already exists", path.display())));
    }
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    git.worktree_add_from(&project, &path, &branch, create_branch, from)?;
    let mut result = serve(ctx, parent_name, &path, database)?;
    if let Some(o) = result.as_object_mut() {
        o.insert("created_branch".into(), json!(create_branch));
    }
    Ok(result)
}

/// Stop serving a worktree. The site goes; the checkout stays, because it holds work.
pub fn unserve(ctx: &Ctx, parent_name: &str, worktree_site: &str) -> Result<Value> {
    let parent = parent_site(ctx, parent_name)?;
    let site = parent_site(ctx, worktree_site)?;
    let belongs = site.name.starts_with(&format!("{}-", parent.name)) && site.host.ends_with(&format!(".{}", parent.host));
    if !belongs {
        return Err(DevdError::invalid(format!("`{worktree_site}` is not a worktree site of `{parent_name}`")));
    }
    let removed = devd_sites::remove(ctx, worktree_site, false)?;
    Ok(json!({ "removed": worktree_site, "folder_kept": site.project_dir().display().to_string(), "result": removed }))
}

/// A copy of a database under `<db>_<label>`, through the same dump and restore devd uses
/// everywhere else.
fn clone_database(ctx: &Ctx, db: &str, label: &str) -> Result<String> {
    let target = format!("{db}_{}", label.replace('-', "_"));
    devd_services::dump::valid_source_name(&target)?;
    let (port, running) = devd_services::mariadb_service(ctx).ok_or_else(|| DevdError::not_found("MariaDB is not installed, so there is no database to copy"))?;
    if !running {
        return Err(DevdError::conflict("MariaDB is not running; start it to copy the database"));
    }
    let file = devd_services::dump::default_file(ctx, db);
    devd_services::dump::dump(ctx, &devd_services::dump::Endpoint::devd(port), db, &file)?;
    devd_services::dump::restore(ctx, &devd_services::dump::Endpoint::devd(port), &target, &file)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_becomes_a_subdomain_label() {
        assert_eq!(slug("feature/Auth_v2"), "feature-auth-v2");
        assert_eq!(slug("fix.login--bug"), "fix-login-bug");
        assert_eq!(slug("/odd//"), "odd");
        assert_eq!(slug("ünï"), "n");
        assert!(slug(&"a".repeat(80)).len() <= 40);
    }

    #[test]
    fn a_clashing_label_gets_a_number() {
        let taken = |c: &str| c == "feature-auth" || c == "feature-auth-2";
        assert_eq!(unique_slug("feature-auth", &taken), "feature-auth-3");
        assert_eq!(unique_slug("bugfix", &taken), "bugfix");
    }

    #[test]
    fn a_made_up_name_is_two_words_and_avoids_what_is_taken() {
        let first = made_up_name(7, &|_| false);
        assert_eq!(first.split('-').count(), 2);
        let second = made_up_name(7, &|n| n == first);
        assert_ne!(first, second);
    }

    #[test]
    fn the_env_points_at_the_worktree_and_leaves_unrelated_values_alone() {
        let parent = "APP_NAME=Acme\nAPP_URL=https://acme.test\nSESSION_DOMAIN=.acme.test\nSANCTUM_STATEFUL_DOMAINS=acme.test\nMAIL_HOST=127.0.0.1\nDB_DATABASE=acme\n";
        let out = env_for(parent, "acme.test", "https://feature-auth.acme.test", "feature-auth.acme.test", Some("acme_feature_auth")).unwrap();
        let v = crate::env::variables(&out);
        assert_eq!(v["APP_URL"], "https://feature-auth.acme.test");
        assert_eq!(v["SESSION_DOMAIN"], ".feature-auth.acme.test");
        assert_eq!(v["SANCTUM_STATEFUL_DOMAINS"], "feature-auth.acme.test");
        assert_eq!(v["DB_DATABASE"], "acme_feature_auth");
        assert_eq!(v["MAIL_HOST"], "127.0.0.1");
        assert_eq!(v["APP_NAME"], "Acme");
    }

    #[test]
    fn a_session_domain_that_does_not_name_the_site_is_left_alone() {
        let out = env_for("SESSION_DOMAIN=.example.com\n", "acme.test", "http://x.acme.test", "x.acme.test", None).unwrap();
        assert_eq!(crate::env::variables(&out)["SESSION_DOMAIN"], ".example.com");
    }

    #[test]
    fn the_database_choice_is_parsed_strictly() {
        assert_eq!(Database::parse(None).unwrap(), Database::Share);
        assert_eq!(Database::parse(Some("clone")).unwrap(), Database::Clone);
        assert_eq!(Database::parse(Some("none")).unwrap(), Database::Leave);
        assert!(Database::parse(Some("copy")).is_err());
    }
}
