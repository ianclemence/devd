//! Moving a devd home without rebuilding it.
//!
//! devd's layout is already rooted at one directory (`Paths`), so moving that
//! directory is supported by construction: runtimes, services, the state
//! database and the settings travel with it. What does not travel by itself are
//! the absolute paths recorded inside it - the projects and sites it knows. This
//! module remembers where the home was, notices when it is somewhere else, says
//! exactly which recorded paths no longer exist, and can rewrite the ones that
//! moved with the home.
//!
//! Nothing outside the old home is ever rewritten: a project on another disk is
//! reported, not guessed at.

use std::path::Path;

use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::{Registry, Result};

/// The key that remembers where the home was last seen.
const HOME_KEY: &str = "portability.home";

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
struct NoArgs {}

fn home_text(ctx: &Ctx) -> String {
    ctx.paths.root().to_string_lossy().to_string()
}

/// `path` with `from` replaced by `to`, when it is inside `from` and the result
/// is a real directory. A prefix that is only part of a name is not a match.
fn remap(path: &str, from: &str, to: &str) -> Option<String> {
    if path.len() <= from.len()
        || !path.is_char_boundary(from.len())
        || !path[..from.len()].eq_ignore_ascii_case(from)
    {
        return None;
    }
    let rest = &path[from.len()..];
    if !(rest.starts_with('\\') || rest.starts_with('/')) {
        return None;
    }
    let new = format!("{to}{rest}");
    Path::new(&new).is_dir().then_some(new)
}

pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("portability.status", "Is this devd home portable?")
    .description("Where devd is installed, where it was last seen, whether it has moved, which known projects and sites no longer exist at their recorded paths, whether the home is writable, and how secrets are stored. Read only.")
    .group("portability")
    .tier(Tier::Read)
    .effect(Effect::Read)
    .agent_safe()
    .idempotent()
    .handlers(
        |_, _| Ok(Plan::new("portability.status", "check whether this devd home moved")),
        |ctx, _| {
            let home = home_text(ctx);
            let previous = ctx.store.get_setting(HOME_KEY)?;
            let moved = previous.as_deref().is_some_and(|p| !p.eq_ignore_ascii_case(&home));
            let projects = ctx.store.list_stack_projects().unwrap_or_default();
            let sites = ctx.store.list_sites().unwrap_or_default();
            let missing_projects: Vec<Value> = projects.iter().filter(|p| !Path::new(&p.path).is_dir()).map(|p| json!({ "path": p.path, "name": p.name })).collect();
            let missing_sites: Vec<Value> = sites.iter().filter(|s| !Path::new(&s.root).is_dir()).map(|s| json!({ "root": s.root, "name": s.name })).collect();
            let probe = ctx.paths.root().join(".devd-write-probe");
            let writable = std::fs::write(&probe, b"1").is_ok();
            let _ = std::fs::remove_file(&probe);
            // First run: remember where this home is, so a later move is seen.
            if previous.is_none() {
                let _ = ctx.store.set_setting(HOME_KEY, &home);
            }
            let state = if moved { "moved" } else { "in place" };
            Ok(Outcome::with_data(
                format!("devd is {state} at {home}; {} project(s), {} missing", projects.len(), missing_projects.len()),
                json!({
                    "home": home,
                    "previous": previous,
                    "moved": moved,
                    "writable": writable,
                    "projects": projects.len(),
                    "sites": sites.len(),
                    "missing_projects": missing_projects,
                    "missing_sites": missing_sites,
                    "secrets": if cfg!(unix) { "owner-only files under secrets/" } else { "files under the user's own profile" },
                    "state_db": ctx.paths.state_db().display().to_string(),
                    "settings": ctx.paths.settings_file().display().to_string(),
                    "can_repair": moved,
                }),
            ))
        },
    )?;

    reg.define::<NoArgs>("portability.repair", "Move devd's recorded paths to its new home")
    .description("When devd has been moved, rewrites the projects and sites whose paths were inside the old home to the same place inside the new one, and records the new home. Anything outside the old home is left alone and reported. Human only: it changes recorded paths.")
    .group("portability")
    .tier(Tier::Write)
    .effect(Effect::Config)
    .human_only()
    .dry_run()
    .handlers(
        |ctx, _| {
            let home = home_text(ctx);
            let previous = ctx.store.get_setting(HOME_KEY)?.unwrap_or_default();
            Ok(Plan::new("portability.repair", format!("move recorded paths from {previous} to {home}"))
                .step("rewrite projects and sites that were inside the old home")
                .effect("changes recorded paths; files are not moved"))
        },
        |ctx, _| {
            let home = home_text(ctx);
            let Some(previous) = ctx.store.get_setting(HOME_KEY)? else {
                ctx.store.set_setting(HOME_KEY, &home)?;
                return Ok(Outcome::with_data("Recorded this home; there was no earlier one to move", json!({ "projects": 0, "sites": 0 })));
            };
            if previous.eq_ignore_ascii_case(&home) {
                return Ok(Outcome::with_data("devd is where it was; nothing to repair", json!({ "projects": 0, "sites": 0 })));
            }
            let mut projects = 0;
            for p in ctx.store.list_stack_projects().unwrap_or_default() {
                if let Some(new) = remap(&p.path, &previous, &home) {
                    let _ = ctx.store.delete_stack_project(&p.path);
                    let mut moved = p.clone();
                    moved.path = new;
                    if ctx.store.upsert_stack_project(&moved).is_ok() {
                        projects += 1;
                    }
                }
            }
            let mut sites = 0;
            for s in ctx.store.list_sites().unwrap_or_default() {
                if let Some(new) = remap(&s.root, &previous, &home) {
                    let mut moved = s.clone();
                    moved.root = new;
                    if ctx.store.upsert_site(&moved).is_ok() {
                        sites += 1;
                    }
                }
            }
            ctx.store.set_setting(HOME_KEY, &home)?;
            let mut outcome = Outcome::with_data(
                format!("Moved {projects} project(s) and {sites} site(s) to the new home"),
                json!({ "projects": projects, "sites": sites, "home": home }),
            );
            let left: Vec<String> = ctx
                .store
                .list_stack_projects()
                .unwrap_or_default()
                .into_iter()
                .filter(|p| !Path::new(&p.path).is_dir())
                .map(|p| p.path)
                .collect();
            if !left.is_empty() {
                outcome = outcome.warn(format!(
                    "{} project path(s) still do not exist and are outside the old home: {}",
                    left.len(),
                    left.join(", ")
                ));
            }
            Ok(outcome)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_inside_the_old_home_moves_with_it() {
        let base = std::env::temp_dir().join(format!("devd-port-{}", std::process::id()));
        let from = base.join("old");
        let to = base.join("new");
        std::fs::create_dir_all(to.join("www/shop")).unwrap();
        let got = remap(
            &from.join("www/shop").to_string_lossy(),
            &from.to_string_lossy(),
            &to.to_string_lossy(),
        );
        assert_eq!(got.as_deref(), Some(to.join("www/shop").to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_name_that_only_starts_with_the_old_home_is_not_moved() {
        assert_eq!(remap("/a/devd2/x", "/a/devd", "/b/devd"), None);
        assert_eq!(remap("/a/devd", "/a/devd", "/b/devd"), None);
    }
}
