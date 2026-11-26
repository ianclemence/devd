//! What GitHub says about a project, as facts an intake rule can act on.
//!
//! devd watches a repository only when a project has a rule that asks for it: a new issue, a pull request
//! waiting for review, or the default branch's CI turning red. The facts come from the GitHub CLI (`gh`, signed
//! in by the person), are read at most every few minutes per project, and are what devd saw, never a model's
//! reading of it, so a rule cannot be talked into firing.
//!
//! Each issue, pull request or failed CI run is offered once: when a rule acts on it, it is recorded as seen. The
//! first time a rule looks at a project, what is already open is recorded as the starting point, so writing a rule
//! does not turn a year of open issues into a year of runs; those can still be handed to the team by hand.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::util;

/// The triggers this module answers, with what each means in a sentence.
pub const TRIGGERS: &[(&str, &str)] = &[
    ("issue.opened", "a new issue is opened on the project's GitHub repository"),
    ("pr.waiting", "a pull request on the project's repository is waiting for review"),
    ("ci.failed", "CI failed on the project's default branch"),
];

/// How often one project's repository is asked, at most: GitHub limits how often a person's token may ask.
const POLL: Duration = Duration::from_secs(300);
/// How long `gh` may take to answer.
const GH_TIMEOUT: Duration = Duration::from_secs(30);
/// How much of an issue's body travels with it into the goal and the proposal.
const BODY: usize = 1500;

/// One thing GitHub said that a rule may act on.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    /// The project folder it belongs to.
    pub project: String,
    /// What it is, stable across polls: `issue#12`, `pr#7`, `ci#45016268483`.
    pub key: String,
    /// The fields a rule's goal may name: `{number}`, `{title}`, `{url}`, `{body}`.
    pub fields: Value,
    /// What happened, in a few lines, for the run's goal and the proposal.
    pub detail: String,
}

/// Whether `trigger` is one of GitHub's.
pub fn is_github(trigger: &str) -> bool {
    TRIGGERS.iter().any(|(t, _)| *t == trigger)
}

/// The unseen facts for `trigger` in each of `projects`.
pub fn facts(ctx: &Ctx, trigger: &str, projects: &[String]) -> Vec<Fact> {
    let mut out = Vec::new();
    for project in projects {
        let Some(found) = poll(trigger, project) else {
            continue;
        };
        let baseline_id = format!("{project}|{trigger}");
        if ctx.store.ops_get("github.baseline", &baseline_id).ok().flatten().is_none() {
            // The first look: what is open now is where watching starts.
            for fact in &found {
                mark_seen(ctx, fact);
            }
            let _ = ctx.store.ops_put(
                "github.baseline",
                &baseline_id,
                project,
                &json!({ "at": util::now(), "items": found.len() }),
            );
            continue;
        }
        out.extend(found.into_iter().filter(|f| !seen(ctx, f)));
    }
    out
}

/// Record that a rule acted on `fact`, so it is not offered again.
pub fn mark_seen(ctx: &Ctx, fact: &Fact) {
    let _ = ctx.store.ops_put(
        "github.seen",
        &format!("{}|{}", fact.project, fact.key),
        &fact.project,
        &json!({ "at": util::now(), "title": fact.fields["title"] }),
    );
}

fn seen(ctx: &Ctx, fact: &Fact) -> bool {
    ctx.store
        .ops_get("github.seen", &format!("{}|{}", fact.project, fact.key))
        .ok()
        .flatten()
        .is_some()
}

/// A rule's goal with the fact's own words put in: `{title}`, `{number}`, `{url}`, `{body}`, `{detail}`. A goal
/// that names none of them gets the fact under it, so a run always knows which issue it is for.
pub fn fill(goal: &str, fact: &Fact) -> String {
    let field = |name: &str| match &fact.fields[name] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let named = ["{title}", "{number}", "{url}", "{body}", "{detail}"]
        .iter()
        .any(|p| goal.contains(p));
    let filled = goal
        .replace("{title}", &field("title"))
        .replace("{number}", &field("number"))
        .replace("{url}", &field("url"))
        .replace("{body}", &field("body"))
        .replace("{detail}", &fact.detail);
    if named {
        filled
    } else {
        format!("{filled}\n\n{}", fact.detail)
    }
}

/// What `gh` says now for one project, read at most every five minutes; `None` when it was asked too recently or
/// could not answer.
fn poll(trigger: &str, project: &str) -> Option<Vec<Fact>> {
    static LAST: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let map = last.get_or_insert_with(HashMap::new);
        let id = format!("{project}|{trigger}");
        if map.get(&id).is_some_and(|at| at.elapsed() < POLL) {
            return None;
        }
        map.insert(id, Instant::now());
    }
    let dir = Path::new(project);
    if !dir.join(".git").exists() {
        return None;
    }
    match trigger {
        "issue.opened" => {
            let list = gh(dir, &["issue", "list", "--state", "open", "--limit", "30", "--json", "number,title,url,body"])?;
            Some(issues(project, &list))
        }
        "pr.waiting" => {
            let list = gh(
                dir,
                &["pr", "list", "--state", "open", "--limit", "30", "--json", "number,title,url,body,isDraft,reviewDecision"],
            )?;
            Some(pulls(project, &list))
        }
        "ci.failed" => {
            let repo = gh(dir, &["repo", "view", "--json", "defaultBranchRef"])?;
            let branch = repo["defaultBranchRef"]["name"].as_str()?.to_string();
            let runs = gh(
                dir,
                &[
                    "run", "list", "--branch", &branch, "--limit", "1", "--json",
                    "databaseId,conclusion,status,workflowName,url,headSha,displayTitle",
                ],
            )?;
            Some(ci(project, &branch, &runs))
        }
        _ => None,
    }
}

/// Run `gh` in a project's folder and read its JSON.
fn gh(dir: &Path, args: &[&str]) -> Option<Value> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let out = util::run(Path::new("gh"), &args, Some(dir), &[], Some(GH_TIMEOUT)).ok()?;
    if !out.ok() {
        return None;
    }
    serde_json::from_str(&out.stdout).ok()
}

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= BODY {
        return text.to_string();
    }
    let cut: String = text.chars().take(BODY).collect();
    format!("{cut}…")
}

/// Open issues, as facts.
fn issues(project: &str, list: &Value) -> Vec<Fact> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let number = i["number"].as_u64()?;
            let (title, url) = (i["title"].as_str()?.to_string(), i["url"].as_str().unwrap_or_default().to_string());
            let body = clip(i["body"].as_str().unwrap_or_default());
            let mut detail = format!("Issue #{number}: {title}\n{url}");
            if !body.is_empty() {
                detail.push_str(&format!("\n\n{body}"));
            }
            Some(Fact {
                project: project.to_string(),
                key: format!("issue#{number}"),
                fields: json!({ "number": number, "title": title, "url": url, "body": body }),
                detail,
            })
        })
        .collect()
}

/// Open pull requests that are not drafts and have no review decision yet, as facts.
fn pulls(project: &str, list: &Value) -> Vec<Fact> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["isDraft"] != json!(true))
        .filter(|p| matches!(p["reviewDecision"].as_str(), None | Some("") | Some("REVIEW_REQUIRED")))
        .filter_map(|p| {
            let number = p["number"].as_u64()?;
            let (title, url) = (p["title"].as_str()?.to_string(), p["url"].as_str().unwrap_or_default().to_string());
            let body = clip(p["body"].as_str().unwrap_or_default());
            Some(Fact {
                project: project.to_string(),
                key: format!("pr#{number}"),
                fields: json!({ "number": number, "title": title, "url": url, "body": body }),
                detail: format!("Pull request #{number} is waiting for review: {title}\n{url}"),
            })
        })
        .collect()
}

/// The latest finished CI run on the default branch, as a fact when it failed.
fn ci(project: &str, branch: &str, runs: &Value) -> Vec<Fact> {
    let Some(run) = runs.as_array().and_then(|r| r.first()) else {
        return Vec::new();
    };
    if run["status"] != json!("completed") || run["conclusion"] != json!("failure") {
        return Vec::new();
    }
    let id = run["databaseId"].as_u64().unwrap_or(0);
    let name = run["workflowName"].as_str().unwrap_or("CI");
    let title = format!("{name} failed on {branch}");
    let url = run["url"].as_str().unwrap_or_default().to_string();
    let commit = run["headSha"].as_str().unwrap_or_default().chars().take(8).collect::<String>();
    vec![Fact {
        project: project.to_string(),
        key: format!("ci#{id}"),
        fields: json!({ "number": id, "title": title, "url": url, "body": run["displayTitle"] }),
        detail: format!("{title} at {commit}\n{url}"),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_are_facts_with_their_own_words() {
        let list = json!([{ "number": 12, "title": "Login fails on Safari", "url": "https://x/12", "body": "Steps: open it." }]);
        let found = issues("D:/p", &list);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "issue#12");
        assert!(found[0].detail.starts_with("Issue #12: Login fails on Safari"));
        assert!(found[0].detail.contains("Steps: open it."));
    }

    #[test]
    fn only_pull_requests_that_wait_for_a_review_are_offered() {
        let list = json!([
            { "number": 1, "title": "ready", "url": "u", "isDraft": false, "reviewDecision": "" },
            { "number": 2, "title": "draft", "url": "u", "isDraft": true, "reviewDecision": "" },
            { "number": 3, "title": "approved", "url": "u", "isDraft": false, "reviewDecision": "APPROVED" },
            { "number": 4, "title": "asked", "url": "u", "isDraft": false, "reviewDecision": "REVIEW_REQUIRED" },
        ]);
        let keys: Vec<String> = pulls("p", &list).into_iter().map(|f| f.key).collect();
        assert_eq!(keys, vec!["pr#1", "pr#4"]);
    }

    #[test]
    fn a_red_default_branch_is_a_fact_and_a_green_one_is_not() {
        let red = json!([{ "databaseId": 9, "status": "completed", "conclusion": "failure", "workflowName": "ci", "url": "u", "headSha": "abcdef123456" }]);
        let green = json!([{ "databaseId": 9, "status": "completed", "conclusion": "success", "workflowName": "ci" }]);
        let running = json!([{ "databaseId": 9, "status": "in_progress", "conclusion": "" }]);
        assert_eq!(ci("p", "main", &red)[0].key, "ci#9");
        assert!(ci("p", "main", &green).is_empty());
        assert!(ci("p", "main", &running).is_empty());
    }

    #[test]
    fn a_goal_names_the_issue_or_carries_it() {
        let fact = &issues("p", &json!([{ "number": 7, "title": "Broken link", "url": "https://x/7", "body": "" }]))[0];
        assert_eq!(fill("Fix {title} ({url})", fact), "Fix Broken link (https://x/7)");
        let plain = fill("Triage the new issue", fact);
        assert!(plain.starts_with("Triage the new issue\n\nIssue #7: Broken link"), "{plain}");
    }
}
