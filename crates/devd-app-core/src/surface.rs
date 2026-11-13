//! Every capability call the window's project and site pages make, written once.
//!
//! A page is a view of what devd can do. Each button and each list on it is a capability call, the same call
//! an agent or a script would make, and the only authority behind it is the capability's own tier and approval.
//! To keep the pages honest, no page builds a call by hand: it asks this module, and this module is checked
//! against the real registry by a test in `devd-cli`.
//!
//! That test exists because of a mistake that was made once: a page asked for `stack.list`, which has never
//! existed, and showed an error where a list should have been. The test takes every call here (see
//! [`samples`]), looks up its capability by id, and fails if the capability is missing, if an argument it
//! sends is not in the capability's schema, or if a required argument is not sent. A page that names a
//! capability or an argument that is wrong cannot be merged.
//!
//! Calls are plain data ([`Call`]), so the window turns one into a query (a read, cached and kept current by
//! events) or a mutation (a write, which invalidates what it changed) without this module knowing about either.

use serde_json::{json, Map, Value};

use crate::sync::{Mutation, QueryKey};

/// One capability call: which, and with what.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The capability's id.
    pub cap: &'static str,
    /// Its arguments, an object.
    pub args: Value,
}

impl Call {
    fn new(cap: &'static str, args: Value) -> Self {
        Self { cap, args }
    }

    /// The call as a cached read.
    pub fn query(&self) -> QueryKey {
        QueryKey::new(self.cap, self.args.clone())
    }

    /// The call as a write.
    pub fn mutation(&self) -> Mutation {
        Mutation::new(self.cap, self.args.clone())
    }
}

/// An object with the entries whose value is present, so an argument left out is left out of the call.
fn args(pairs: &[(&str, Option<Value>)]) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        if let Some(value) = value {
            map.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(map)
}

fn some<T: Into<Value>>(v: T) -> Option<Value> {
    Some(v.into())
}

/// Files: browsing, reading, searching and changing a project's folder.
pub mod files {
    use super::*;

    /// One folder's entries, for a tree that opens a folder when it is asked to.
    pub fn tree(path: &str) -> Call {
        Call::new("fs.tree", args(&[("path", some(path))]))
    }

    /// A file's text, optionally a window of its lines.
    pub fn read(path: &str, offset: Option<usize>, limit: Option<usize>) -> Call {
        Call::new("fs.read", args(&[("path", some(path)), ("offset", offset.map(Value::from)), ("limit", limit.map(Value::from))]))
    }

    /// A file's text exactly as it is on disk, with its line endings: what an editor opens.
    pub fn read_exact(path: &str) -> Call {
        Call::new("fs.read", args(&[("path", some(path)), ("raw", some(true))]))
    }

    /// Make an empty file, and any folders on the way to it.
    pub fn create(path: &str) -> Call {
        Call::new("fs.write", args(&[("path", some(path)), ("content", some("")), ("create_dirs", some(true))]))
    }

    /// Replace a file's whole text.
    pub fn write(path: &str, content: &str) -> Call {
        Call::new("fs.write", args(&[("path", some(path)), ("content", some(content))]))
    }

    /// Search file contents under a folder.
    pub fn search(path: &str, pattern: &str, limit: usize) -> Call {
        Call::new("fs.search", args(&[("path", some(path)), ("pattern", some(pattern)), ("limit", some(limit))]))
    }

    /// Move or rename.
    pub fn rename(from: &str, to: &str) -> Call {
        Call::new("fs.move", args(&[("from", some(from)), ("to", some(to))]))
    }

    /// Delete a file or folder. The capability is destructive and asks first.
    pub fn delete(path: &str) -> Call {
        Call::new("fs.delete", args(&[("path", some(path))]))
    }
}

/// Git, for one repository by its folder.
pub mod git {
    use super::*;

    /// Branch, upstream, ahead and behind, and the changed files.
    pub fn status(repo: &str) -> Call {
        Call::new("git.status", args(&[("repo", some(repo))]))
    }

    /// The most recent commits, optionally of one path.
    pub fn log(repo: &str, limit: usize, path: Option<&str>) -> Call {
        Call::new("git.log", args(&[("repo", some(repo)), ("limit", some(limit)), ("path", path.map(Value::from))]))
    }

    /// A diff: the working tree, the staged changes, or a revision; optionally of one path.
    pub fn diff(repo: &str, path: Option<&str>, staged: bool) -> Call {
        Call::new("git.diff", args(&[("repo", some(repo)), ("path", path.map(Value::from)), ("staged", staged.then(|| json!(true)))]))
    }

    /// One commit with its changes.
    pub fn show(repo: &str, rev: &str) -> Call {
        Call::new("git.show", args(&[("repo", some(repo)), ("rev", some(rev))]))
    }

    /// Stage files.
    pub fn stage(repo: &str, paths: &[String]) -> Call {
        Call::new("git.stage", args(&[("repo", some(repo)), ("paths", some(paths.to_vec()))]))
    }

    /// Unstage files.
    pub fn unstage(repo: &str, paths: &[String]) -> Call {
        Call::new("git.unstage", args(&[("repo", some(repo)), ("paths", some(paths.to_vec()))]))
    }

    /// Throw away changes to files. Destructive; asks first.
    pub fn discard(repo: &str, paths: &[String]) -> Call {
        Call::new("git.discard", args(&[("repo", some(repo)), ("paths", some(paths.to_vec()))]))
    }

    /// Commit what is staged.
    pub fn commit(repo: &str, message: &str) -> Call {
        Call::new("git.commit", args(&[("repo", some(repo)), ("message", some(message))]))
    }

    /// Branches, with the current one marked.
    pub fn branches(repo: &str) -> Call {
        Call::new("git.branch.list", args(&[("repo", some(repo))]))
    }

    /// Move to a branch.
    pub fn switch(repo: &str, name: &str) -> Call {
        Call::new("git.switch", args(&[("repo", some(repo)), ("name", some(name))]))
    }

    /// Make a branch, and move to it.
    pub fn branch_create(repo: &str, name: &str) -> Call {
        Call::new("git.branch.create", args(&[("repo", some(repo)), ("name", some(name)), ("checkout", some(true))]))
    }

    /// Delete a branch.
    pub fn branch_delete(repo: &str, name: &str) -> Call {
        Call::new("git.branch.delete", args(&[("repo", some(repo)), ("name", some(name))]))
    }

    /// Fetch from the remote.
    pub fn fetch(repo: &str) -> Call {
        Call::new("git.fetch", args(&[("repo", some(repo))]))
    }

    /// Pull the current branch.
    pub fn pull(repo: &str) -> Call {
        Call::new("git.pull", args(&[("repo", some(repo))]))
    }

    /// Push the current branch.
    pub fn push(repo: &str) -> Call {
        Call::new("git.push", args(&[("repo", some(repo))]))
    }

    /// Stashes.
    pub fn stashes(repo: &str) -> Call {
        Call::new("git.stash.list", args(&[("repo", some(repo))]))
    }

    /// Put the working changes aside.
    pub fn stash_push(repo: &str, message: Option<&str>) -> Call {
        Call::new("git.stash.push", args(&[("repo", some(repo)), ("message", message.map(Value::from)), ("include_untracked", some(true))]))
    }

    /// Bring a stash back, and drop it.
    pub fn stash_pop(repo: &str, reference: &str) -> Call {
        Call::new("git.stash.apply", args(&[("repo", some(repo)), ("reference", some(reference)), ("pop", some(true))]))
    }

    /// Drop a stash.
    pub fn stash_drop(repo: &str, reference: &str) -> Call {
        Call::new("git.stash.drop", args(&[("repo", some(repo)), ("reference", some(reference))]))
    }

    /// Tags.
    pub fn tags(repo: &str) -> Call {
        Call::new("git.tag.list", args(&[("repo", some(repo))]))
    }

    /// Remotes.
    pub fn remotes(repo: &str) -> Call {
        Call::new("git.remote.list", args(&[("repo", some(repo))]))
    }

    /// Worktrees.
    pub fn worktrees(repo: &str) -> Call {
        Call::new("git.worktree.list", args(&[("repo", some(repo))]))
    }

    /// Where HEAD has been: the way back from a mistake.
    pub fn reflog(repo: &str, limit: usize) -> Call {
        Call::new("git.reflog", args(&[("repo", some(repo)), ("limit", some(limit))]))
    }

    /// Undo a commit with a new commit.
    pub fn revert(repo: &str, rev: &str) -> Call {
        Call::new("git.revert", args(&[("repo", some(repo)), ("rev", some(rev))]))
    }

    /// Who last changed each line of a file.
    pub fn blame(repo: &str, path: &str) -> Call {
        Call::new("git.blame", args(&[("repo", some(repo)), ("path", some(path))]))
    }
}

/// The project as devd knows it: what it can run, and what is running.
pub mod project {
    use super::*;

    /// The tasks devd knows how to run.
    pub fn tasks(path: &str) -> Call {
        Call::new("project.tasks", args(&[("path", some(path))]))
    }

    /// Run a task by id.
    pub fn run(path: &str, task: &str, background: bool) -> Call {
        Call::new("project.run", args(&[("path", some(path)), ("task", some(task)), ("background", background.then(|| json!(true)))]))
    }

    /// What devd detected: stack, package manager, scripts.
    pub fn detect(path: &str) -> Call {
        Call::new("project.detect", args(&[("path", some(path))]))
    }

    /// The processes the project's `devd.yml` declares, and whether they run.
    pub fn status(project: &str) -> Call {
        Call::new("project.status", args(&[("project", some(project))]))
    }

    /// Start a declared process.
    pub fn start(project: &str, name: &str) -> Call {
        Call::new("project.start", args(&[("project", some(project)), ("name", some(name))]))
    }

    /// Stop a declared process.
    pub fn stop(project: &str, name: &str) -> Call {
        Call::new("project.stop", args(&[("project", some(project)), ("name", some(name))]))
    }

    /// A process's output.
    pub fn logs(project: &str, name: &str, lines: usize) -> Call {
        Call::new("project.logs", args(&[("project", some(project)), ("name", some(name)), ("lines", some(lines))]))
    }
}

/// The team's view of a project: its to-dos, its reviews, and what it remembers.
pub mod team {
    use super::*;

    /// The project's to-dos.
    pub fn todos(project: &str) -> Call {
        Call::new("todo.list", args(&[("project", some(project))]))
    }

    /// Add a to-do.
    pub fn todo_create(project: &str, title: &str) -> Call {
        Call::new("todo.create", args(&[("project", some(project)), ("title", some(title))]))
    }

    /// Mark a to-do done. `revision` is the one the list showed, so a change made meanwhile is not overwritten.
    pub fn todo_complete(id: &str, revision: u64) -> Call {
        Call::new("todo.complete", args(&[("id", some(id)), ("expected_revision", some(revision))]))
    }

    /// Reviews of the project's work.
    pub fn reviews() -> Call {
        Call::new("review.list", args(&[]))
    }

    /// Start a review of the working changes.
    pub fn review(project: &str) -> Call {
        Call::new("review.start", args(&[("project", some(project))]))
    }

    /// What devd remembers about the project.
    pub fn memory(project: &str) -> Call {
        Call::new("memory.get", args(&[("project", some(project))]))
    }
}

/// A shell in the project's folder.
pub mod terminal {
    use super::*;

    /// Open the system's own terminal in a folder.
    pub fn open_external(path: &str) -> Call {
        Call::new("project.terminal_external", args(&[("path", some(path))]))
    }

    /// Open a shell in a folder, on the runtimes the project is pinned to.
    pub fn open(path: &str, cols: u16, rows: u16) -> Call {
        Call::new("project.terminal", args(&[("path", some(path)), ("cols", some(cols)), ("rows", some(rows))]))
    }

    /// The shells that are open.
    pub fn list() -> Call {
        Call::new("terminal.list", args(&[]))
    }

    /// Tell a shell how big its window is.
    pub fn resize(id: &str, cols: u16, rows: u16) -> Call {
        Call::new("terminal.resize", args(&[("id", some(id)), ("cols", some(cols)), ("rows", some(rows))]))
    }

    /// What a shell has printed lately.
    pub fn read(id: &str, lines: usize) -> Call {
        Call::new("terminal.read", args(&[("id", some(id)), ("lines", some(lines))]))
    }

    /// Type into a shell.
    pub fn write(id: &str, data: &str) -> Call {
        Call::new("terminal.write", args(&[("id", some(id)), ("data", some(data))]))
    }

    /// Close a shell.
    pub fn close(id: &str) -> Call {
        Call::new("terminal.close", args(&[("id", some(id))]))
    }
}

/// The browser the assistant and the person share.
pub mod browser {
    use super::*;

    /// Whether it is running and what it holds. Starts nothing.
    pub fn status() -> Call {
        Call::new("browser.status", args(&[]))
    }

    /// Start it, on a page when given one.
    pub fn open(url: Option<&str>) -> Call {
        Call::new("browser.open", args(&[("url", url.map(|u| json!(u)))]))
    }

    /// Load a page in a tab.
    pub fn navigate(url: &str) -> Call {
        Call::new("browser.navigate", args(&[("url", some(url))]))
    }

    /// A picture of the page as it is.
    pub fn screenshot() -> Call {
        Call::new("browser.screenshot", args(&[]))
    }

    /// The page as the assistant reads it: a tree with a name on everything that can be acted on.
    pub fn snapshot() -> Call {
        Call::new("browser.snapshot", args(&[]))
    }

    /// What the page logged and what it asked for.
    pub fn console() -> Call {
        Call::new("browser.console", args(&[]))
    }

    /// Back, forward or reload, by the capability's name.
    pub fn history(which: &str) -> Call {
        Call::new(match which {
            "back" => "browser.back",
            "forward" => "browser.forward",
            _ => "browser.reload",
        }, args(&[]))
    }

    /// Click a point, in the page's own pixels.
    pub fn mouse(x: f64, y: f64) -> Call {
        Call::new("browser.mouse", args(&[("x", some(x)), ("y", some(y))]))
    }

    /// Scroll the page at a point.
    pub fn scroll(x: f64, y: f64, delta_y: f64) -> Call {
        Call::new("browser.scroll", args(&[("x", some(x)), ("y", some(y)), ("delta_y", some(delta_y))]))
    }

    /// Type text into whatever has focus.
    pub fn text(text: &str) -> Call {
        Call::new("browser.key", args(&[("text", some(text))]))
    }

    /// Press a named key.
    pub fn key(name: &str) -> Call {
        Call::new("browser.key", args(&[("key", some(name))]))
    }

    /// A new tab, on a page when given one.
    pub fn tab_open(url: Option<&str>) -> Call {
        Call::new("browser.tabs.open", args(&[("url", url.map(|u| json!(u)))]))
    }

    /// Bring a tab forward.
    pub fn tab_focus(tab: &str) -> Call {
        Call::new("browser.tabs.focus", args(&[("tab", some(tab))]))
    }

    /// Close a tab.
    pub fn tab_close(tab: &str) -> Call {
        Call::new("browser.tabs.close", args(&[("tab", some(tab))]))
    }

    /// Close the browser.
    pub fn close() -> Call {
        Call::new("browser.close", args(&[]))
    }
}

/// Every call above, once with its arguments filled in and once with the optional ones left out where there
/// are any, so a test can check each against the real capabilities.
pub fn samples() -> Vec<Call> {
    let (p, f, rev, who) = ("D:/work/shop", "src/main.rs", "abc1234", "main");
    let paths = vec![f.to_string()];
    vec![
        terminal::open(p, 100, 30),
        terminal::list(),
        terminal::resize("t1", 100, 30),
        browser::status(),
        browser::open(None),
        browser::open(Some("http://localhost")),
        browser::navigate("http://localhost"),
        browser::screenshot(),
        browser::snapshot(),
        browser::console(),
        browser::history("back"),
        browser::history("forward"),
        browser::history("reload"),
        browser::mouse(10.0, 20.0),
        browser::scroll(10.0, 20.0, 120.0),
        browser::text("hello"),
        browser::key("Enter"),
        browser::tab_open(None),
        browser::tab_open(Some("http://localhost")),
        browser::tab_focus("t1"),
        browser::tab_close("t1"),
        browser::close(),
        files::tree(p),
        files::read(f, None, None),
        files::read(f, Some(1), Some(200)),
        files::read_exact(f),
        files::write(f, "text"),
        files::create(f),
        files::search(p, "todo", 50),
        files::rename(f, "src/app.rs"),
        files::delete(f),
        git::status(p),
        git::log(p, 50, None),
        git::log(p, 50, Some(f)),
        git::diff(p, None, false),
        git::diff(p, Some(f), true),
        git::show(p, rev),
        git::stage(p, &paths),
        git::unstage(p, &paths),
        git::discard(p, &paths),
        git::commit(p, "message"),
        git::branches(p),
        git::switch(p, who),
        git::branch_create(p, "feature"),
        git::branch_delete(p, "feature"),
        git::fetch(p),
        git::pull(p),
        git::push(p),
        git::stashes(p),
        git::stash_push(p, None),
        git::stash_push(p, Some("wip")),
        git::stash_pop(p, "stash@{0}"),
        git::stash_drop(p, "stash@{0}"),
        git::tags(p),
        git::remotes(p),
        git::worktrees(p),
        git::reflog(p, 50),
        git::blame(p, f),
        git::revert(p, rev),
        project::tasks(p),
        project::run(p, "build", false),
        project::run(p, "dev", true),
        project::detect(p),
        project::status(p),
        project::start(p, "web"),
        project::stop(p, "web"),
        project::logs(p, "web", 200),
        team::todos(p),
        team::todo_create(p, "title"),
        team::todo_complete("t1", 1),
        team::reviews(),
        team::review(p),
        team::memory(p),
        terminal::open(p, 100, 30),
        terminal::open_external(p),
        terminal::read("t1", 200),
        terminal::write("t1", "ls\n"),
        terminal::close("t1"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_argument_left_out_is_not_sent_as_null() {
        let call = git::log("D:/x", 10, None);
        assert_eq!(call.args, json!({ "repo": "D:/x", "limit": 10 }));
        let with = git::log("D:/x", 10, Some("a.rs"));
        assert_eq!(with.args["path"], "a.rs");
        assert_eq!(git::diff("D:/x", None, false).args, json!({ "repo": "D:/x" }));
        assert_eq!(git::diff("D:/x", None, true).args["staged"], true);
    }

    #[test]
    fn a_call_is_a_read_or_a_write_with_the_same_arguments() {
        let call = git::status("D:/x");
        assert_eq!(call.query().capability, "git.status");
        assert_eq!(call.query().domain(), "git");
        assert_eq!(call.mutation().args, call.args);
    }

    #[test]
    fn every_sample_names_a_capability_and_sends_an_object() {
        let samples = samples();
        assert!(samples.len() > 40);
        for call in &samples {
            assert!(call.cap.contains('.'), "{}", call.cap);
            assert!(call.args.is_object(), "{}", call.cap);
        }
        // Every builder is sampled: the capability ids sampled are the ones the module can make.
        let ids: std::collections::BTreeSet<&str> = samples.iter().map(|c| c.cap).collect();
        for expected in ["fs.tree", "fs.read", "git.status", "git.commit", "git.stash.apply", "project.run", "todo.list", "project.terminal"] {
            assert!(ids.contains(expected), "{expected} is not sampled");
        }
    }
}
