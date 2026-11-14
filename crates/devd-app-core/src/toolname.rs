//! What an agent did, in the words a person would use.
//!
//! An agent's tool calls are capability ids: `git.stash.push`, `browser.open`,
//! `fs.edit`. Those are right for a log and wrong for a conversation. Nobody
//! reading along thinks "fs.search"; they think "it searched the code".
//!
//! [`describe`] turns a call into a short phrase and the thing it was done to:
//! *Edited* `BookingController.php`, *Ran* `php artisan migrate`, *Opened*
//! `https://shop.test`. The phrase changes with the call's state, because
//! "Reading" and "Read" tell a person different things.
//!
//! Every id has an answer. The common ones are written out; anything else is
//! read from the id itself (`git.branch.create` becomes "Create branch" under
//! Git), so a capability added tomorrow is never shown as a dotted id.

use serde_json::Value;

/// Where a call stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Asked for, not answered yet.
    Running,
    /// Answered.
    Done,
    /// Refused or failed.
    Failed,
}

/// A call, described.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    /// What was done: "Edited", "Running tests", "Could not read".
    pub action: String,
    /// What it was done to: a file name, a command, an address. May be empty.
    pub object: String,
    /// The kind of work, for choosing an icon: `file`, `search`, `terminal`,
    /// `git`, `browser`, `team`, `plan`, `server`, `lookup`, `other`.
    pub kind: &'static str,
}

/// (capability, while running, when done, kind). `{}` is not used: the object
/// is always shown after the phrase.
const KNOWN: &[(&str, &str, &str, &str)] = &[
    ("fs.read", "Reading", "Read", "file"),
    ("fs.list", "Listing", "Listed", "file"),
    ("fs.tree", "Looking through", "Looked through", "file"),
    ("fs.search", "Searching for", "Searched for", "search"),
    ("fs.write", "Writing", "Wrote", "file"),
    ("fs.edit", "Editing", "Edited", "file"),
    ("fs.move", "Moving", "Moved", "file"),
    ("fs.delete", "Deleting", "Deleted", "file"),
    (
        "project.detect",
        "Reading the project",
        "Read the project",
        "lookup",
    ),
    (
        "project.tasks",
        "Checking the project's tasks",
        "Checked the project's tasks",
        "lookup",
    ),
    ("project.run", "Running", "Ran", "terminal"),
    ("project.exec", "Running", "Ran", "terminal"),
    ("project.exec_destructive", "Running", "Ran", "terminal"),
    ("project.start", "Starting", "Started", "terminal"),
    ("project.stop", "Stopping", "Stopped", "terminal"),
    (
        "project.logs",
        "Reading the logs of",
        "Read the logs of",
        "terminal",
    ),
    (
        "task.logs",
        "Reading the output of",
        "Read the output of",
        "terminal",
    ),
    (
        "task.list",
        "Checking running tasks",
        "Checked running tasks",
        "terminal",
    ),
    (
        "terminal.open",
        "Opening a terminal",
        "Opened a terminal",
        "terminal",
    ),
    (
        "terminal.write",
        "Typing in the terminal",
        "Typed in the terminal",
        "terminal",
    ),
    (
        "git.status",
        "Checking what changed",
        "Checked what changed",
        "git",
    ),
    ("git.diff", "Reading the changes", "Read the changes", "git"),
    ("git.log", "Reading the history", "Read the history", "git"),
    ("git.show", "Reading commit", "Read commit", "git"),
    ("git.stage", "Staging", "Staged", "git"),
    ("git.unstage", "Unstaging", "Unstaged", "git"),
    ("git.commit", "Committing", "Committed", "git"),
    ("git.push", "Pushing", "Pushed", "git"),
    ("git.pull", "Pulling", "Pulled", "git"),
    ("git.fetch", "Fetching", "Fetched", "git"),
    ("git.switch", "Switching to", "Switched to", "git"),
    ("git.merge", "Merging", "Merged", "git"),
    (
        "git.branch.list",
        "Listing branches",
        "Listed branches",
        "git",
    ),
    (
        "git.branch.create",
        "Creating branch",
        "Created branch",
        "git",
    ),
    (
        "git.branch.delete",
        "Deleting branch",
        "Deleted branch",
        "git",
    ),
    (
        "git.stash.push",
        "Stashing changes",
        "Stashed changes",
        "git",
    ),
    (
        "git.stash.apply",
        "Restoring stashed changes",
        "Restored stashed changes",
        "git",
    ),
    ("git.stash.list", "Listing stashes", "Listed stashes", "git"),
    (
        "git.discard",
        "Discarding changes to",
        "Discarded changes to",
        "git",
    ),
    ("git.reset", "Resetting to", "Reset to", "git"),
    ("browser.open", "Opening", "Opened", "browser"),
    (
        "browser.snapshot",
        "Looking at the page",
        "Looked at the page",
        "browser",
    ),
    ("browser.click", "Clicking", "Clicked", "browser"),
    (
        "browser.type",
        "Typing into the page",
        "Typed into the page",
        "browser",
    ),
    (
        "browser.screenshot",
        "Taking a screenshot",
        "Took a screenshot",
        "browser",
    ),
    (
        "browser.console",
        "Reading the page's console",
        "Read the page's console",
        "browser",
    ),
    ("browser.navigate", "Loading", "Loaded", "browser"),
    (
        "browser.back",
        "Going back a page",
        "Went back a page",
        "browser",
    ),
    (
        "browser.forward",
        "Going forward a page",
        "Went forward a page",
        "browser",
    ),
    (
        "browser.reload",
        "Reloading the page",
        "Reloaded the page",
        "browser",
    ),
    (
        "browser.key",
        "Pressing a key in the page",
        "Pressed a key in the page",
        "browser",
    ),
    (
        "browser.scroll",
        "Scrolling the page",
        "Scrolled the page",
        "browser",
    ),
    (
        "browser.mouse",
        "Clicking in the page",
        "Clicked in the page",
        "browser",
    ),
    (
        "browser.evaluate",
        "Reading a value from the page",
        "Read a value from the page",
        "browser",
    ),
    (
        "browser.tabs.list",
        "Listing the browser's tabs",
        "Listed the browser's tabs",
        "browser",
    ),
    (
        "browser.tabs.open",
        "Opening a tab",
        "Opened a tab",
        "browser",
    ),
    (
        "browser.tabs.focus",
        "Switching tabs",
        "Switched tabs",
        "browser",
    ),
    (
        "browser.tabs.close",
        "Closing a tab",
        "Closed a tab",
        "browser",
    ),
    (
        "browser.close",
        "Closing the browser",
        "Closed the browser",
        "browser",
    ),
    ("docs.fetch", "Reading", "Read", "browser"),
    ("chat.explore", "Exploring", "Explored", "search"),
    ("web.open", "Reading", "Read", "browser"),
    ("search.query", "Searching for", "Searched for", "search"),
    ("todo.list", "Checking the plan", "Checked the plan", "plan"),
    (
        "todo.create",
        "Adding to the plan",
        "Added to the plan",
        "plan",
    ),
    ("todo.complete", "Ticking off", "Ticked off", "plan"),
    (
        "todo.update",
        "Updating the plan",
        "Updated the plan",
        "plan",
    ),
    (
        "skills.get",
        "Loading the playbook",
        "Loaded the playbook",
        "lookup",
    ),
    (
        "skills.discover",
        "Looking for a playbook",
        "Looked for a playbook",
        "lookup",
    ),
    ("team.start", "Handing the team", "Handed the team", "team"),
    ("team.assign", "Assigning", "Assigned", "team"),
    (
        "team.get",
        "Checking on the run",
        "Checked on the run",
        "team",
    ),
    (
        "team.list",
        "Checking the team's runs",
        "Checked the team's runs",
        "team",
    ),
    (
        "team.overview",
        "Checking on the team",
        "Checked on the team",
        "team",
    ),
    (
        "team.roles",
        "Checking the roster",
        "Checked the roster",
        "team",
    ),
    ("team.report", "Reporting back", "Reported back", "team"),
    (
        "team.plan_submit",
        "Proposing a plan",
        "Proposed a plan",
        "team",
    ),
    ("team.retry", "Retrying a task", "Retried a task", "team"),
    ("team.resume", "Resuming the run", "Resumed the run", "team"),
    (
        "team.cancel",
        "Cancelling the run",
        "Cancelled the run",
        "team",
    ),
    ("team.suggest", "Suggesting", "Suggested", "team"),
    (
        "inbox.post",
        "Sending you a message",
        "Sent you a message",
        "team",
    ),
    ("chat.send", "Messaging", "Messaged", "team"),
    ("service.start", "Starting", "Started", "server"),
    ("service.stop", "Stopping", "Stopped", "server"),
    ("service.restart", "Restarting", "Restarted", "server"),
    (
        "service.list",
        "Checking the services",
        "Checked the services",
        "server",
    ),
    (
        "health.status",
        "Checking the server's health",
        "Checked the server's health",
        "server",
    ),
    ("site.list", "Listing sites", "Listed sites", "server"),
    ("site.create", "Creating site", "Created site", "server"),
    ("runtime.install", "Installing", "Installed", "server"),
    ("toolchain.install", "Installing", "Installed", "server"),
    ("toolchain.service", "Managing", "Managed", "server"),
    (
        "php.pool",
        "Managing PHP workers",
        "Managed PHP workers",
        "server",
    ),
    ("app.go", "Showing you", "Showed you", "other"),
    (
        "team.trace",
        "Reading what the team did",
        "Read what the team did",
        "team",
    ),
    ("roles.save", "Saving a role", "Saved a role", "team"),
    ("roles.delete", "Removing a role", "Removed a role", "team"),
    (
        "team.flow",
        "Checking how the team hands work on",
        "Checked how the team hands work on",
        "team",
    ),
    (
        "skills.install",
        "Installing skills from",
        "Installed skills from",
        "lookup",
    ),
    (
        "motion.new",
        "Starting a composition",
        "Started a composition",
        "browser",
    ),
    ("motion.render", "Rendering", "Rendered", "browser"),
    (
        "app.where",
        "Checking what you are looking at",
        "Checked what you are looking at",
        "lookup",
    ),
    (
        "activity.list",
        "Checking the activity record",
        "Checked the activity record",
        "lookup",
    ),
    (
        "audit.verify",
        "Checking the record was not changed",
        "Checked the record was not changed",
        "lookup",
    ),
    (
        "devd_groups",
        "Looking up what devd can do",
        "Looked up what devd can do",
        "lookup",
    ),
    (
        "devd_capabilities",
        "Looking up how to",
        "Looked up how to",
        "lookup",
    ),
    (
        "devd_describe",
        "Checking how to use",
        "Checked how to use",
        "lookup",
    ),
    (
        "devd_doctor",
        "Checking the server",
        "Checked the server",
        "server",
    ),
];

/// The direct tool names an agent may use, and the capability each stands for.
const DIRECT: &[(&str, &str)] = &[
    ("read_file", "fs.read"),
    ("write_file", "fs.write"),
    ("edit_file", "fs.edit"),
    ("delete_file", "fs.delete"),
    ("list_files", "fs.list"),
    ("search_files", "fs.search"),
    ("project_info", "project.detect"),
    ("project_tasks", "project.tasks"),
    ("run_task", "project.run"),
    ("git_status", "git.status"),
    ("git_diff", "git.diff"),
    ("run_command", "project.exec"),
    ("git_stage", "git.stage"),
    ("git_commit", "git.commit"),
    ("git_log", "git.log"),
    ("fetch_page", "docs.fetch"),
    ("explore", "chat.explore"),
    ("todo_list", "todo.list"),
    ("todo_create", "todo.create"),
    ("todo_complete", "todo.complete"),
];

/// The written-out phrases as data, for a surface that is not Rust.
///
/// devd's browser interface draws conversations too. It reads this table
/// instead of keeping its own, so the two cannot describe the same call in
/// different words: `known` maps a capability to its phrases and kind, and
/// `direct` maps an agent's direct tool names to the capability each stands for.
pub fn table() -> Value {
    let known: serde_json::Map<String, Value> = KNOWN
        .iter()
        .map(|(id, doing, done, kind)| {
            (
                id.to_string(),
                serde_json::json!({ "doing": doing, "done": done, "kind": kind }),
            )
        })
        .collect();
    let direct: serde_json::Map<String, Value> = DIRECT
        .iter()
        .map(|(tool, cap)| (tool.to_string(), Value::String(cap.to_string())))
        .collect();
    serde_json::json!({ "known": known, "direct": direct })
}

/// Describe a tool call as the model made it: its name, its arguments, its state.
pub fn describe(name: &str, input: &Value, phase: Phase) -> Described {
    // `devd_invoke` is a wrapper; the capability inside it is what happened.
    let (id, args) = match name {
        "devd_invoke" => (
            input["id"].as_str().unwrap_or("devd_invoke"),
            &input["args"],
        ),
        other => (other, input),
    };
    let id = DIRECT
        .iter()
        .find(|(tool, _)| *tool == id)
        .map(|(_, cap)| *cap)
        .unwrap_or(id);
    let (running, done, kind) = match KNOWN.iter().find(|(known, ..)| *known == id) {
        Some((_, running, done, kind)) => (running.to_string(), done.to_string(), *kind),
        None => from_id(id),
    };
    let object = object_of(id, args);
    let action = match phase {
        Phase::Running => running,
        Phase::Done => done,
        // "Could not read", from "Reading": the plain verb is the one that reads right.
        Phase::Failed => format!("Could not {}", infinitive(&running)),
    };
    Described {
        action,
        object,
        kind,
    }
}

/// Whether a call only looked at things: it read, listed, searched or checked, and changed nothing.
///
/// A feed is read to decide whether to step in, so what changed something is loud and what only
/// looked is quiet. This goes by the capability's own name, which says it: a call that cannot be
/// told is not quiet, because a quiet change is the one that would be missed.
pub fn only_looks(name: &str, input: &Value) -> bool {
    let id = match name {
        "devd_invoke" => input["id"].as_str().unwrap_or(name),
        other => other,
    };
    let id = DIRECT
        .iter()
        .find(|(tool, _)| *tool == id)
        .map(|(_, cap)| *cap)
        .unwrap_or(id);
    const WORDS: &[&str] = &[
        "get",
        "list",
        "read",
        "search",
        "tree",
        "status",
        "diff",
        "log",
        "describe",
        "verify",
        "snapshot",
        "inventory",
        "catalogue",
        "overview",
        "roles",
        "flow",
        "capabilities",
    ];
    let last = id.rsplit(['.', '_']).next().unwrap_or(id);
    !id.starts_with("mcp__") && WORDS.contains(&last)
}

/// A phrase for a capability nobody wrote one for, read from its id.
///
/// `mcp__github__create_issue` is "Create issue" with GitHub; `tls.trust` is
/// "Trust" under TLS. The words are the id's own, so it is never wrong, only
/// plain.
fn from_id(id: &str) -> (String, String, &'static str) {
    if let Some(rest) = id.strip_prefix("mcp__") {
        let (server, tool) = rest.split_once("__").unwrap_or(("", rest));
        let phrase = format!(
            "{} ({})",
            sentence(&words(tool)),
            server.replace(['_', '-'], " ")
        );
        return (phrase.clone(), phrase, "other");
    }
    let mut parts: Vec<&str> = id.split('.').collect();
    let domain = if parts.len() > 1 { parts.remove(0) } else { "" };
    // `git.branch.create` reads as "create branch", verb first.
    parts.reverse();
    let phrase = sentence(&parts.iter().map(|p| words(p)).collect::<Vec<_>>().join(" "));
    let label = match domain {
        "" => phrase,
        "tls" | "php" | "dns" | "mcp" | "ssh" => format!("{}: {phrase}", domain.to_uppercase()),
        other => format!("{}: {phrase}", sentence(other)),
    };
    let kind = match domain {
        "git" | "review" => "git",
        "site" | "hosts" | "tls" | "service" | "runtime" | "toolchain" | "environment"
        | "stack" | "db" => "server",
        "team" | "inbox" | "approval" | "chat" => "team",
        "browser" | "web" | "docs" => "browser",
        "fs" => "file",
        _ => "other",
    };
    (label.clone(), label, kind)
}

fn words(part: &str) -> String {
    part.replace(['_', '-'], " ")
}

/// Capitalise the first letter.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// "Reading the logs of" as "read the logs of".
fn infinitive(progressive: &str) -> String {
    let (verb, rest) = progressive.split_once(' ').unwrap_or((progressive, ""));
    let lower = verb.to_lowercase();
    let stem = lower.strip_suffix("ing").unwrap_or(&lower);
    // Verbs whose plain form is not the stem: writing, typing, running, stopping…
    let plain = match stem {
        "writ" => "write".into(),
        "typ" => "type".into(),
        "mov" => "move".into(),
        "delet" => "delete".into(),
        "merg" => "merge".into(),
        "stag" => "stage".into(),
        "unstag" => "unstage".into(),
        "creat" => "create".into(),
        "manag" => "manage".into(),
        "tak" => "take".into(),
        "restor" => "restore".into(),
        "updat" => "update".into(),
        "propos" => "propose".into(),
        "resum" => "resume".into(),
        "messag" => "message".into(),
        "hand" => "hand".into(),
        other => {
            let bytes = other.as_bytes();
            let doubled = bytes.len() >= 2
                && bytes[bytes.len() - 1] == bytes[bytes.len() - 2]
                && !other.ends_with("ll")
                && !other.ends_with("ss");
            if doubled {
                other[..other.len() - 1].to_string()
            } else {
                other.to_string()
            }
        }
    };
    if rest.is_empty() {
        plain
    } else {
        format!("{plain} {rest}")
    }
}

/// The thing a call was done to, as a person would name it.
fn object_of(id: &str, args: &Value) -> String {
    let leaf = |v: &Value| {
        v.as_str().map(|s| {
            s.trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(s)
                .to_string()
        })
    };
    let text = |key: &str| args[key].as_str().map(str::to_string);
    let found = match id {
        "project.exec" | "project.exec_destructive" => args["argv"].as_array().map(|argv| {
            argv.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        }),
        "project.run" => text("task").map(|task| match task.as_str() {
            "test" => "the tests".to_string(),
            "lint" => "the linter".to_string(),
            "build" => "the build".to_string(),
            "typecheck" => "the type check".to_string(),
            "install" => "the install".to_string(),
            other => other.to_string(),
        }),
        "fs.search" | "search.query" => text("pattern")
            .or_else(|| text("query"))
            .map(|p| format!("\u{201c}{p}\u{201d}")),
        "git.commit" => text("message").map(|m| m.lines().next().unwrap_or_default().to_string()),
        "git.switch" | "git.branch.create" | "git.branch.delete" | "git.merge" => {
            text("branch").or_else(|| text("name"))
        }
        "git.show" | "git.reset" => text("rev").map(|rev| rev.chars().take(10).collect()),
        "git.stage" | "git.unstage" | "git.discard" => {
            args["paths"].as_array().map(|paths| match paths.len() {
                1 => leaf(&paths[0]).unwrap_or_default(),
                n => format!("{n} files"),
            })
        }
        "browser.open" | "browser.navigate" | "browser.tabs.open" | "docs.fetch" | "web.open" => {
            text("url")
        }
        "browser.click" | "browser.type" => text("ref").or_else(|| text("text")),
        "skills.get" => text("name"),
        "app.go" => text("route"),
        "roles.save" | "roles.delete" => text("id"),
        "skills.install" => text("source"),
        "motion.new" | "motion.render" => leaf(&args["dir"]),
        "devd_describe" => text("id").map(|cap| {
            describe(&cap, &Value::Null, Phase::Running)
                .action
                .to_lowercase()
        }),
        "devd_capabilities" => text("query").or_else(|| text("group")),
        "team.start" => text("goal").or_else(|| text("title")).map(|g| {
            g.lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(80)
                .collect()
        }),
        "team.assign" => text("member"),
        "todo.create" => text("title"),
        "service.start" | "service.stop" | "service.restart" | "site.create"
        | "toolchain.install" | "runtime.install" => text("name")
            .or_else(|| text("tool"))
            .or_else(|| text("runtime")),
        "task.logs" | "project.logs" => text("name").or_else(|| leaf(&args["path"])),
        _ => None,
    };
    // For work on a whole repository or project, the object is where it happened.
    let place = matches!(
        id,
        "git.status"
            | "git.diff"
            | "git.log"
            | "git.stash.push"
            | "git.stash.apply"
            | "git.stash.list"
            | "git.branch.list"
            | "git.fetch"
            | "git.pull"
            | "git.push"
            | "project.detect"
            | "project.tasks"
    );
    if place && found.is_none() {
        return leaf(&args["repo"])
            .or_else(|| leaf(&args["path"]))
            .or_else(|| leaf(&args["project"]))
            .map(|name| format!("in {name}"))
            .unwrap_or_default();
    }
    found
        .or_else(|| leaf(&args["path"]))
        .or_else(|| leaf(&args["repo"]))
        .or_else(|| leaf(&args["project"]))
        .or_else(|| text("name"))
        .or_else(|| text("title"))
        .unwrap_or_default()
}

/// Finished steps of a turn, said as one line: "Edited a file, read 3 files and ran 2 commands". Each step is
/// a tool's name, what it was given, and whether it worked. A reply folds a run of finished steps into this
/// line, the way a person would report them, and opens them again when asked.
pub fn summary(steps: &[(&str, &Value, bool)]) -> String {
    let (mut read, mut edited, mut ran, mut searched, mut browsed, mut git, mut other, mut failed) =
        (0, 0, 0, 0, 0, 0, 0, 0);
    for (name, input, ok) in steps {
        if !ok {
            failed += 1;
        }
        let said = describe(name, input, Phase::Done);
        match said.kind {
            "file" if only_looks(name, input) => read += 1,
            "file" => edited += 1,
            "terminal" => ran += 1,
            "search" | "lookup" => searched += 1,
            "browser" => browsed += 1,
            "git" => git += 1,
            _ => other += 1,
        }
    }
    let count = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some(one.to_string()),
        n => Some(many.replace('#', &n.to_string())),
    };
    let parts: Vec<String> = [
        count(edited, "edited a file", "edited # files"),
        count(read, "read a file", "read # files"),
        count(ran, "ran a command", "ran # commands"),
        count(searched, "searched", "searched # times"),
        count(browsed, "used the browser", "took # steps in the browser"),
        count(git, "checked git", "took # git steps"),
        count(other, "took one more step", "took # more steps"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut line = match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    };
    if let Some(first) = line.get(..1) {
        line = format!("{}{}", first.to_uppercase(), &line[1..]);
    }
    match failed {
        0 => line,
        n => format!("{line} · {n} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_steps_read_as_one_sentence() {
        let read = serde_json::json!({ "path": "src/main.rs" });
        let cmd = serde_json::json!({ "command": "go test ./..." });
        let steps = [("read_file", &read, true), ("read_file", &read, true), ("write_file", &read, true), ("project_exec", &cmd, false)];
        let line = summary(&steps);
        assert!(line.starts_with("Edited a file, read 2 files"), "{line}");
        assert!(line.ends_with("· 1 failed"), "{line}");
        assert_eq!(summary(&[("read_file", &read, true)]), "Read a file");
    }

    #[test]
    fn only_a_call_that_looks_is_quiet() {
        assert!(only_looks("devd_invoke", &json!({ "id": "fs.read" })));
        assert!(only_looks("git_status", &json!({})));
        assert!(only_looks("devd_invoke", &json!({ "id": "audit.verify" })));
        assert!(!only_looks("devd_invoke", &json!({ "id": "fs.write" })));
        assert!(!only_looks(
            "devd_invoke",
            &json!({ "id": "service.start" })
        ));
        assert!(
            !only_looks("mcp__github__list_issues", &json!({})),
            "an outside tool is never assumed harmless"
        );
        assert!(
            !only_looks("devd_invoke", &json!({})),
            "a call that cannot be told is loud"
        );
    }
    use serde_json::json;

    fn said(name: &str, input: Value, phase: Phase) -> String {
        let d = describe(name, &input, phase);
        format!("{} {}", d.action, d.object).trim().to_string()
    }

    #[test]
    fn common_calls_read_as_sentences() {
        assert_eq!(
            said(
                "read_file",
                json!({"path": "D:\\www\\shop\\app\\BookingController.php"}),
                Phase::Done
            ),
            "Read BookingController.php"
        );
        assert_eq!(
            said("edit_file", json!({"path": "src/main.rs"}), Phase::Running),
            "Editing main.rs"
        );
        assert_eq!(
            said(
                "search_files",
                json!({"path": ".", "pattern": "fn main"}),
                Phase::Done
            ),
            "Searched for \u{201c}fn main\u{201d}"
        );
        assert_eq!(
            said("run_task", json!({"task": "test"}), Phase::Running),
            "Running the tests"
        );
        assert_eq!(
            said("git_status", json!({"repo": "D:\\www\\shop"}), Phase::Done),
            "Checked what changed in shop"
        );
    }

    #[test]
    fn a_wrapped_call_is_described_by_what_is_inside_it() {
        let call = json!({"id": "project.exec", "args": {"path": ".", "argv": ["php", "artisan", "migrate"]}});
        assert_eq!(
            said("devd_invoke", call, Phase::Done),
            "Ran php artisan migrate"
        );
        let stash = json!({"id": "git.stash.push", "args": {"repo": "D:\\www\\shop"}});
        assert_eq!(
            said("devd_invoke", stash, Phase::Done),
            "Stashed changes in shop"
        );
        let open = json!({"id": "browser.open", "args": {"url": "https://shop.test"}});
        assert_eq!(
            said("devd_invoke", open, Phase::Running),
            "Opening https://shop.test"
        );
    }

    #[test]
    fn a_failure_says_what_could_not_be_done() {
        assert_eq!(
            said("read_file", json!({"path": "gone.rs"}), Phase::Failed),
            "Could not read gone.rs"
        );
        assert_eq!(
            said("write_file", json!({"path": "a.txt"}), Phase::Failed),
            "Could not write a.txt"
        );
        assert_eq!(
            said("run_task", json!({"task": "lint"}), Phase::Failed),
            "Could not run the linter"
        );
        assert_eq!(
            said(
                "devd_invoke",
                json!({"id": "git.stash.push", "args": {}}),
                Phase::Failed
            ),
            "Could not stash changes"
        );
        assert_eq!(
            said(
                "devd_invoke",
                json!({"id": "service.stop", "args": {"name": "nginx"}}),
                Phase::Failed
            ),
            "Could not stop nginx"
        );
    }

    #[test]
    fn no_dotted_id_ever_reaches_a_person() {
        for (id, ..) in KNOWN {
            for phase in [Phase::Running, Phase::Done, Phase::Failed] {
                let d = describe(id, &Value::Null, phase);
                assert!(
                    !d.action.contains('.') && !d.action.contains('_'),
                    "{id} -> {}",
                    d.action
                );
                assert!(
                    d.action.chars().next().is_some_and(char::is_uppercase),
                    "{id} -> {}",
                    d.action
                );
            }
        }
        // And one nobody wrote a phrase for.
        let unknown = describe(
            "devd_invoke",
            &json!({"id": "git.worktree.prune", "args": {}}),
            Phase::Done,
        );
        assert_eq!(unknown.action, "Git: Prune worktree");
        assert_eq!(unknown.kind, "git");
        assert_eq!(
            describe("tls.trust", &Value::Null, Phase::Done).action,
            "TLS: Trust"
        );
    }

    #[test]
    fn another_program_s_tool_is_named_with_its_server() {
        let d = describe(
            "mcp__github__create_issue",
            &json!({"title": "Crash on save"}),
            Phase::Done,
        );
        assert_eq!(d.action, "Create issue (github)");
        assert_eq!(d.object, "Crash on save");
    }

    #[test]
    fn looking_something_up_says_what_was_looked_up() {
        assert_eq!(
            said("devd_describe", json!({"id": "fs.edit"}), Phase::Done),
            "Checked how to use editing"
        );
        assert_eq!(
            said(
                "devd_invoke",
                json!({"id": "skills.get", "args": {"name": "fix-bug"}}),
                Phase::Done
            ),
            "Loaded the playbook fix-bug"
        );
    }

    #[test]
    fn the_table_for_other_surfaces_carries_every_phrase() {
        let table = table();
        assert_eq!(table["known"]["git.stash.push"]["done"], "Stashed changes");
        assert_eq!(table["known"]["browser.open"]["doing"], "Opening");
        assert_eq!(table["known"]["browser.open"]["kind"], "browser");
        assert_eq!(table["direct"]["read_file"], "fs.read");
        assert_eq!(
            table["known"].as_object().unwrap().len(),
            KNOWN.len(),
            "no id is listed twice"
        );
        // No phrase is an id: nothing a person reads has a dot or an underscore in it.
        for (id, entry) in table["known"].as_object().unwrap() {
            for form in ["doing", "done"] {
                let phrase = entry[form].as_str().unwrap();
                assert!(
                    !phrase.contains('.') && !phrase.contains('_'),
                    "{id}: {phrase}"
                );
            }
        }
    }
}
