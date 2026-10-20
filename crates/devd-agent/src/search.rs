//! Developer search: one query across what a developer actually looks for.
//!
//! Nothing here is an index. Each source is read at query time and every
//! response says which sources were searched, how, and what was cut off, so a
//! caller never mistakes "no result" for "searched everything and found
//! nothing".
//!
//! * projects, conversations, models, tasks and activity come from devd's own
//!   state and match on names and titles;
//! * files are matched by path on a bounded live walk of the project roots;
//! * content is a literal text search and only runs inside a named project.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use walkdir::WalkDir;

use devd_core::capability::Ctx;
use devd_core::Result;

use crate::fs;

/// Everything scanned across all roots before a file search stops.
const FILE_SCAN_CAP: usize = 30_000;

/// Where a query may look.
pub const SCOPES: &[&str] = &["projects", "files", "content", "conversations", "models", "tasks", "activity"];

fn tokens(q: &str) -> Vec<String> {
    q.split_whitespace().map(|t| t.to_lowercase()).collect()
}

/// Score `text` against every token; None when any token is missing.
fn score(text: &str, name: &str, toks: &[String]) -> Option<i32> {
    let hay = text.to_lowercase();
    let name = name.to_lowercase();
    let mut total = 0;
    for t in toks {
        if name == *t {
            total += 100;
        } else if name.starts_with(t.as_str()) {
            total += 80;
        } else if name.contains(t.as_str()) {
            total += 60;
        } else if hay.contains(t.as_str()) {
            total += 30;
        } else {
            return None;
        }
    }
    Some(total)
}

fn project_roots(ctx: &Ctx) -> Result<Vec<(String, String)>> {
    let mut roots: Vec<(String, String)> = Vec::new();
    for r in ctx.store.list_repos()? {
        roots.push((r.name, r.path));
    }
    for p in ctx.store.list_stack_projects()? {
        if !roots.iter().any(|(_, path)| *path == p.path) {
            roots.push((p.name, p.path));
        }
    }
    Ok(roots)
}

/// Run a search. `scopes` empty means the default set (content is opt-in).
pub fn query(ctx: &Ctx, query: &str, scopes: &[String], project: Option<&str>, limit: usize) -> Result<Value> {
    let q = query.trim();
    let toks = tokens(q);
    let want = |s: &str| {
        if scopes.is_empty() {
            s != "content" && s != "activity"
        } else {
            scopes.iter().any(|x| x == s)
        }
    };
    let mut results: Vec<Value> = Vec::new();
    let mut sources: Vec<Value> = Vec::new();
    if toks.is_empty() {
        return Ok(json!({ "results": results, "sources": sources, "query": q }));
    }
    let roots = project_roots(ctx)?;

    if want("projects") {
        let mut n = 0;
        for (name, path) in &roots {
            if let Some(s) = score(&format!("{name} {path}"), name, &toks) {
                n += 1;
                results.push(json!({ "kind": "project", "title": name, "detail": path, "path": path, "score": s + 20 }));
            }
        }
        sources.push(json!({ "source": "projects", "searched": true, "how": "names and paths of projects devd knows", "count": n }));
    }

    if want("files") {
        let scan: Vec<PathBuf> = match project {
            Some(p) => vec![fs::authorize(ctx, p, false)?],
            None => roots.iter().map(|(_, p)| PathBuf::from(p)).filter(|p| p.is_dir()).collect(),
        };
        let mut scanned = 0usize;
        let mut capped = false;
        let mut found = 0usize;
        'roots: for root in &scan {
            for e in WalkDir::new(root)
                .into_iter()
                .filter_entry(|e| !(e.file_type().is_dir() && fs::SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref())))
                .filter_map(|e| e.ok())
            {
                scanned += 1;
                if scanned > FILE_SCAN_CAP {
                    capped = true;
                    break 'roots;
                }
                if !e.file_type().is_file() || fs::is_secret_file(e.path()) {
                    continue;
                }
                let rel = e.path().strip_prefix(root).unwrap_or(e.path()).to_string_lossy().replace('\\', "/");
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(s) = score(&rel, &name, &toks) {
                    found += 1;
                    results.push(json!({ "kind": "file", "title": name, "detail": rel, "path": e.path().display().to_string(), "project": root.display().to_string(), "score": s }));
                }
            }
        }
        sources.push(json!({
            "source": "files", "searched": true, "how": "file paths, walked live (nothing is indexed)",
            "roots": scan.len(), "scanned": scanned.min(FILE_SCAN_CAP), "complete": !capped,
            "note": if capped { Some("stopped after 30,000 entries; name a project to search it alone") } else { None },
            "count": found,
        }));
    }

    if want("content") {
        match project {
            None => sources.push(json!({ "source": "content", "searched": false, "note": "text search needs a project; pick one" })),
            Some(p) => {
                let r = fs::search(ctx, p, &regex::escape(q), None, limit, 0)?;
                let hits = r["matches"].as_array().cloned().unwrap_or_default();
                for h in &hits {
                    results.push(json!({
                        "kind": "text", "title": h["text"], "detail": format!("{}:{}", h["file"].as_str().unwrap_or(""), h["line"]),
                        "path": Path::new(p).join(h["file"].as_str().unwrap_or("")).display().to_string(), "line": h["line"], "project": p, "score": 10,
                    }));
                }
                sources.push(json!({
                    "source": "content", "searched": true, "how": "literal text in files up to 1 MB, secrets files and binaries skipped",
                    "scanned": r["files_scanned"], "complete": !r["truncated"].as_bool().unwrap_or(false), "count": hits.len(),
                }));
            }
        }
    }

    if want("conversations") {
        let mut n = 0;
        for t in ctx.store.list_threads(200)? {
            if let Some(s) = score(&format!("{} {}", t.title, t.project.clone().unwrap_or_default()), &t.title, &toks) {
                n += 1;
                results.push(json!({ "kind": "conversation", "title": t.title, "detail": t.state, "id": t.id, "project": t.project, "score": s }));
            }
        }
        sources.push(json!({ "source": "conversations", "searched": true, "how": "titles of the last 200 conversations", "count": n }));
    }

    if want("models") {
        let mut n = 0;
        for m in ctx.store.list_catalogue(None)? {
            if let Some(s) = score(&format!("{} {}", m.model_id, m.display), &m.model_id, &toks) {
                n += 1;
                results.push(json!({ "kind": "model", "title": m.model_id, "detail": m.source, "score": s }));
            }
        }
        for p in ctx.store.list_providers()? {
            if let Some(s) = score(&format!("{} {}", p.id, p.label), &p.label, &toks) {
                n += 1;
                results.push(json!({ "kind": "provider", "title": p.label, "detail": p.kind, "id": p.id, "score": s + 10 }));
            }
        }
        sources.push(json!({ "source": "models", "searched": true, "how": "providers and the cached model catalogue (refresh it on the Models page)", "count": n }));
    }

    if want("tasks") {
        let mut n = 0;
        for t in ctx.store.list_tasks(100)? {
            if let Some(s) = score(&format!("{} {}", t.capability, t.message), &t.capability, &toks) {
                n += 1;
                results.push(json!({ "kind": "task", "title": t.capability, "detail": format!("{} · {}", t.state, t.message), "id": t.id, "score": s }));
            }
        }
        sources.push(json!({ "source": "tasks", "searched": true, "how": "the last 100 background tasks", "count": n }));
    }

    if want("activity") {
        let mut n = 0;
        for a in ctx.store.list_audit(300)? {
            if a.dry_run {
                continue;
            }
            if let Some(s) = score(&format!("{} {} {}", a.capability, a.actor, a.args), &a.capability, &toks) {
                n += 1;
                results.push(json!({ "kind": "activity", "title": a.capability, "detail": format!("{} · {}", a.actor, a.at), "score": s }));
            }
        }
        sources.push(json!({ "source": "activity", "searched": true, "how": "the last 300 audit entries (arguments are already redacted)", "count": n }));
    }

    results.sort_by(|a, b| b["score"].as_i64().cmp(&a["score"].as_i64()));
    results.truncate(limit.max(1) * 3);
    Ok(json!({ "query": q, "results": results, "sources": sources }))
}
