//! Mapping from the `devd git ...` command surface to capabilities.
//!
//! Nothing here performs work. Each arm shapes typed arguments and dispatches a
//! capability, so the CLI cannot do anything the desktop GUI or an agent cannot
//! do, and cannot skip an approval the registry would demand.

use std::path::PathBuf;

use serde_json::json;

use devd_core::{DevdError, Result};

use crate::app::App;
use crate::{
    BisectCommand, BranchCommand, GitCommand, RemoteCommand, RepoCommand, ResetArg, StashCommand,
    StrategyArg, TagCommand, VerdictArg, WorktreeCommand,
};

/// The repository to act on: an explicit path, or the current directory.
fn repo(explicit: &Option<String>) -> String {
    match explicit {
        Some(p) => p.clone(),
        None => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .to_string(),
    }
}

fn strategy(value: StrategyArg) -> &'static str {
    match value {
        StrategyArg::Merge => "merge",
        StrategyArg::Rebase => "rebase",
        StrategyArg::FastForwardOnly => "fast-forward-only",
    }
}

fn reset_mode(value: ResetArg) -> &'static str {
    match value {
        ResetArg::Soft => "soft",
        ResetArg::Mixed => "mixed",
        ResetArg::Hard => "hard",
    }
}

fn verdict_name(value: VerdictArg) -> &'static str {
    match value {
        VerdictArg::Good => "good",
        VerdictArg::Bad => "bad",
        VerdictArg::Skip => "skip",
    }
}

/// Refuse an empty path list rather than silently doing nothing.
fn require_paths(paths: Vec<String>) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Err(DevdError::invalid(
            "at least one path is required (use `.` for everything)",
        ));
    }
    Ok(paths)
}

/// Derive a clone destination from a URL when the user did not give one.
fn destination_for(url: &str, destination: &Option<String>) -> String {
    if let Some(d) = destination {
        return d.clone();
    }
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let tail = trimmed
        .rsplit(['/', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("repository");
    tail.to_string()
}

/// Report the operation currently in progress, or `None`.
fn in_progress(app: &App, path: &str) -> Result<Option<String>> {
    let root = app
        .git
        .repo_root(&PathBuf::from(path))?
        .ok_or_else(|| DevdError::invalid(format!("not a Git repository: {path}")))?;
    Ok(app.git.status(&root)?.in_progress)
}

pub fn run(app: &App, cmd: GitCommand) -> Result<()> {
    match cmd {
        // ------------------------------------------------------------- inspect
        GitCommand::Status { repo: r } => app.invoke("git.status", json!({ "repo": repo(&r) })),
        GitCommand::Diff {
            staged,
            stat,
            rev,
            path,
            repo: r,
        } => app.invoke(
            "git.diff",
            json!({ "repo": repo(&r), "staged": staged, "stat": stat, "rev": rev, "path": path }),
        ),
        GitCommand::Log { limit, repo: r } => {
            app.invoke("git.log", json!({ "repo": repo(&r), "limit": limit }))
        }
        GitCommand::Reflog { limit, repo: r } => {
            app.invoke("git.reflog", json!({ "repo": repo(&r), "limit": limit }))
        }
        GitCommand::Conflicts { repo: r } => {
            app.invoke("git.conflicts.list", json!({ "repo": repo(&r) }))
        }

        // ------------------------------------------------------------- changes
        GitCommand::Stage { paths, repo: r } => {
            app.invoke("git.stage", json!({ "repo": repo(&r), "paths": require_paths(paths)? }))
        }
        GitCommand::Unstage { paths, repo: r } => app.invoke(
            "git.unstage",
            json!({ "repo": repo(&r), "paths": require_paths(paths)? }),
        ),
        GitCommand::Discard { paths, repo: r } => app.invoke(
            "git.discard",
            json!({ "repo": repo(&r), "paths": require_paths(paths)? }),
        ),
        GitCommand::Commit {
            message,
            amend,
            allow_empty,
            repo: r,
        } => app.invoke(
            "git.commit",
            json!({
                "repo": repo(&r),
                "message": message,
                "amend": amend,
                "allow_empty": allow_empty,
            }),
        ),

        // ------------------------------------------------------------ branches
        GitCommand::Branch { cmd } => match cmd {
            BranchCommand::List { all, repo: r } => {
                app.invoke("git.branch.list", json!({ "repo": repo(&r), "all": all }))
            }
            BranchCommand::Create {
                name,
                start_point,
                checkout,
                repo: r,
            } => app.invoke(
                "git.branch.create",
                json!({ "repo": repo(&r), "name": name, "start_point": start_point, "checkout": checkout }),
            ),
            BranchCommand::Delete {
                name,
                force,
                remote,
                repo: r,
            } => app.invoke(
                "git.branch.delete",
                json!({ "repo": repo(&r), "name": name, "force": force, "remote": remote }),
            ),
            BranchCommand::Rename { from, to, repo: r } => app.invoke(
                "git.branch.rename",
                json!({ "repo": repo(&r), "from": from, "to": to }),
            ),
            BranchCommand::Check { target, repo: r } => app.invoke(
                "git.switch.check",
                json!({ "repo": repo(&r), "target": target }),
            ),
            BranchCommand::Prune {
                base,
                dry_run,
                repo: r,
            } => app.invoke(
                "git.repo.prune",
                json!({ "repo": repo(&r), "base": base, "dry_run": dry_run }),
            ),
        },
        GitCommand::Switch {
            name,
            create,
            force,
            repo: r,
        } => app.invoke(
            "git.switch",
            json!({ "repo": repo(&r), "name": name, "create": create, "force": force }),
        ),

        // -------------------------------------------------------------- remotes
        GitCommand::Fetch {
            remote,
            prune,
            repo: r,
        } => app.invoke(
            "git.fetch",
            json!({ "repo": repo(&r), "remote": remote, "prune": prune }),
        ),
        GitCommand::Pull {
            remote,
            branch,
            strategy: s,
            repo: r,
        } => app.invoke(
            "git.pull",
            json!({
                "repo": repo(&r),
                "remote": remote,
                "branch": branch,
                "strategy": s.map(strategy),
            }),
        ),
        GitCommand::Push {
            remote,
            branch,
            set_upstream,
            tags,
            force,
            repo: r,
        } => app.invoke(
            // Force push is a different capability on purpose: it cannot be
            // reached as a flag on the ordinary push path.
            if force { "git.push.force" } else { "git.push" },
            json!({
                "repo": repo(&r),
                "remote": remote,
                "branch": branch,
                "set_upstream": set_upstream,
                "tags": tags,
            }),
        ),
        GitCommand::Clone {
            url,
            destination,
            depth,
            branch,
            recurse_submodules,
        } => app.invoke(
            "git.clone",
            json!({
                "url": url,
                "destination": destination_for(&url, &destination),
                "depth": depth,
                "branch": branch,
                "recurse_submodules": recurse_submodules,
            }),
        ),
        GitCommand::Remote { cmd } => match cmd {
            RemoteCommand::List { repo: r } => {
                app.invoke("git.remote.list", json!({ "repo": repo(&r) }))
            }
            RemoteCommand::Add { name, url, repo: r } => {
                app.invoke("git.remote.add", json!({ "repo": repo(&r), "name": name, "url": url }))
            }
            RemoteCommand::Rename { from, to, repo: r } => app.invoke(
                "git.remote.rename",
                json!({ "repo": repo(&r), "from": from, "to": to }),
            ),
            RemoteCommand::Remove { name, repo: r } => app.invoke(
                "git.remote.remove",
                json!({ "repo": repo(&r), "name": name }),
            ),
        },

        // ---------------------------------------------------------------- tags
        GitCommand::Tag { cmd } => match cmd {
            TagCommand::List { repo: r } => app.invoke("git.tag.list", json!({ "repo": repo(&r) })),
            TagCommand::Create {
                name,
                message,
                rev,
                force,
                repo: r,
            } => app.invoke(
                "git.tag.create",
                json!({ "repo": repo(&r), "name": name, "message": message, "rev": rev, "force": force }),
            ),
            TagCommand::Delete {
                name,
                remote,
                repo: r,
            } => app.invoke(
                "git.tag.delete",
                json!({ "repo": repo(&r), "name": name, "remote": remote }),
            ),
        },

        // --------------------------------------------------------------- merge
        GitCommand::Merge {
            branch,
            no_ff,
            squash,
            repo: r,
        } => app.invoke(
            "git.merge",
            json!({ "repo": repo(&r), "branch": branch, "no_ff": no_ff, "squash": squash }),
        ),
        GitCommand::Continue { repo: r } => {
            let path = repo(&r);
            let op = in_progress(app, &path)?.ok_or_else(|| {
                DevdError::invalid("no merge, rebase, cherry-pick or revert is in progress")
            })?;
            app.invoke("git.op.continue", json!({ "repo": path, "op": op }))
        }
        GitCommand::Abort { repo: r } => {
            let path = repo(&r);
            let op = in_progress(app, &path)?.ok_or_else(|| {
                DevdError::invalid("no merge, rebase, cherry-pick or revert is in progress")
            })?;
            if op == "bisect" {
                return app.invoke("git.bisect.reset", json!({ "repo": path }));
            }
            app.invoke("git.op.abort", json!({ "repo": path, "op": op }))
        }

        // ------------------------------------------------------------- stashes
        GitCommand::Stash { cmd } => match cmd {
            StashCommand::List { repo: r } => {
                app.invoke("git.stash.list", json!({ "repo": repo(&r) }))
            }
            StashCommand::Push {
                message,
                include_untracked,
                repo: r,
            } => app.invoke(
                "git.stash.push",
                json!({ "repo": repo(&r), "message": message, "include_untracked": include_untracked }),
            ),
            StashCommand::Apply {
                reference,
                index,
                repo: r,
            } => app.invoke(
                "git.stash.apply",
                json!({ "repo": repo(&r), "reference": reference, "index": index, "pop": false }),
            ),
            StashCommand::Pop { reference, repo: r } => app.invoke(
                "git.stash.apply",
                json!({ "repo": repo(&r), "reference": reference, "pop": true }),
            ),
            StashCommand::Drop { reference, repo: r } => app.invoke(
                "git.stash.drop",
                json!({ "repo": repo(&r), "reference": reference }),
            ),
            StashCommand::Show { reference, repo: r } => app.invoke(
                "git.stash.show",
                json!({ "repo": repo(&r), "reference": reference }),
            ),
        },

        // ----------------------------------------------------------- worktrees
        GitCommand::Worktree { cmd } => match cmd {
            WorktreeCommand::List { repo: r } => {
                app.invoke("git.worktree.list", json!({ "repo": repo(&r) }))
            }
            WorktreeCommand::Add {
                path,
                branch,
                create_branch,
                repo: r,
            } => app.invoke(
                "git.worktree.add",
                json!({ "repo": repo(&r), "path": path, "branch": branch, "create_branch": create_branch }),
            ),
            WorktreeCommand::Remove {
                path,
                force,
                repo: r,
            } => app.invoke(
                "git.worktree.remove",
                json!({ "repo": repo(&r), "path": path, "force": force }),
            ),
            WorktreeCommand::Prune { repo: r } => {
                app.invoke("git.worktree.prune", json!({ "repo": repo(&r) }))
            }
        },

        // ------------------------------------------------------- history edits
        GitCommand::Revert {
            rev,
            no_commit,
            repo: r,
        } => app.invoke(
            "git.revert",
            json!({ "repo": repo(&r), "rev": rev, "no_commit": no_commit }),
        ),
        GitCommand::CherryPick { rev, repo: r } => {
            app.invoke("git.cherry.pick", json!({ "repo": repo(&r), "rev": rev }))
        }
        GitCommand::Reset { rev, mode, repo: r } => app.invoke(
            "git.reset",
            json!({ "repo": repo(&r), "rev": rev, "mode": reset_mode(mode) }),
        ),
        GitCommand::Rebase {
            onto,
            root,
            autosquash,
            repo: r,
        } => {
            let target = if root {
                "--root".to_string()
            } else {
                onto.ok_or_else(|| {
                    DevdError::invalid("give a revision with --onto, or use --root for the whole history")
                })?
            };
            app.invoke(
                // Autosquash implies an interactive rebase, which the Git
                // backend drives with a no-op sequence editor.
                "git.rebase",
                json!({ "repo": repo(&r), "onto": target, "autosquash": autosquash }),
            )
        }
        GitCommand::Squash {
            count,
            message,
            repo: r,
        } => app.invoke(
            "git.squash",
            json!({ "repo": repo(&r), "count": count, "message": message }),
        ),
        GitCommand::Bisect { cmd } => match cmd {
            BisectCommand::Start {
                bad,
                good,
                repo: r,
            } => app.invoke(
                "git.bisect.start",
                json!({ "repo": repo(&r), "bad": bad, "good": good }),
            ),
            BisectCommand::Mark { verdict, repo: r } => app.invoke(
                "git.bisect.mark",
                json!({ "repo": repo(&r), "verdict": verdict_name(verdict) }),
            ),
            BisectCommand::Reset { repo: r } => {
                app.invoke("git.bisect.reset", json!({ "repo": repo(&r) }))
            }
        },
        GitCommand::Repo { cmd } => match cmd {
            RepoCommand::List => app.invoke("git.repo.list", json!({})),
            RepoCommand::Open { path } => app.invoke("git.repo.open", json!({ "repo": path })),
            RepoCommand::Init {
                path,
                initial_branch,
            } => app.invoke(
                "git.repo.init",
                json!({ "path": path, "initial_branch": initial_branch }),
            ),
            RepoCommand::Track { path } => app.invoke("git.repo.track", json!({ "repo": path })),
            RepoCommand::Untrack { path } => app.invoke("git.repo.untrack", json!({ "repo": path })),
        },

        // ---------------------------------------------------------------- undo
        GitCommand::Undo { repo: r } => {
            let path = repo(&r);
            match in_progress(app, &path)?.as_deref() {
                Some("bisect") => app.invoke("git.bisect.reset", json!({ "repo": path })),
                Some(op) => app.invoke("git.op.abort", json!({ "repo": path, "op": op })),
                None => {
                    if !app.json {
                        println!("Nothing is in progress, so there is no operation to abort.");
                        println!(
                            "Below is the reflog, newest first. To move the branch back to an \
                             earlier state:"
                        );
                        println!("  devd git reset <sha> --mode soft    # keep every change staged");
                        println!("  devd git reset <sha> --mode mixed   # keep the files, unstage them");
                        println!("  devd git reset <sha> --mode hard    # discard the changes (destructive)");
                        println!();
                    }
                    app.invoke("git.reflog", json!({ "repo": path, "limit": 20 }))
                }
            }
        }
    }
}
