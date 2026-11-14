//! One project's page.
//!
//! A project is a folder devd looks after: its files, its git history, what can be run in it, a shell in it,
//! and what the team knows about it. This is that folder as a place to work, with the same tabs the old
//! interface had and every one of them wired to capabilities, so that nothing here is something only a
//! person can do. A button is a capability call (see `devd_app_core::surface`, which a test checks against
//! the real registry); what it changes goes through the same tiers, approvals and record as when the
//! assistant makes the same call; and every page can be handed to the assistant with what is on it.
//!
//! The page keeps very little of its own: which folders are open, which file or change is selected, and the
//! commit message being written. Everything else is read through the sync engine, which keeps it true as
//! events arrive, so a change the assistant makes appears here without a refresh.

use std::collections::HashSet;

use devd_app_core::nav::Route;
use devd_app_core::project::{Group, ProjectTab};
use devd_app_core::surface;
use devd_app_core::sync::Snapshot;
use gpui::{
    AnyElement, ClickEvent, Context, Div, ElementId, Entity, FontWeight, ScrollHandle, Stateful,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value, json};

use super::modal::{Field, FormSpec};
use super::pages::{Act, Row, act_button, form_button, list_or, read_call, text};
use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{Tone, mono, size, theme};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::{self, Button, ButtonKind, card, page_head, pill, rule};

/// How many lines of a file are read at first, and how many more each "Show more" adds.
const FILE_PAGE: usize = 400;

/// What a project's page remembers between draws.
#[derive(Default)]
pub struct ProjectUi {
    /// The project this state belongs to. Opening another starts clean.
    dir: String,
    /// Folders opened in the file tree, by path from the project's root.
    open: HashSet<String>,
    /// The file shown, by path from the root.
    file: Option<String>,
    /// How many of its lines are read.
    file_lines: usize,
    /// The change shown in the diff: a path, and whether it is the staged side.
    change: Option<(String, bool)>,
    /// The commit shown on the history tab.
    commit: Option<String>,
    /// The commit message being written.
    message: Option<Entity<TextInput>>,
    /// The file open in the editor.
    editor: Option<Editor>,
    /// A file asked for, waiting for its exact text to arrive.
    edit_request: Option<String>,
    /// Why a file could not be opened for editing.
    edit_error: Option<String>,
    /// A Markdown file shown as its source rather than drawn.
    source: bool,
    /// Whether the editor holds changes that are not saved. Worked out as the page is drawn.
    dirty: bool,
    /// A name being typed for a new file, or a new name for one.
    naming: Option<Naming>,
    /// What scrolls the editor, so it can follow the cursor.
    scroll: ScrollHandle,
    /// The shells shown on the Terminal tab.
    pub term: super::term::TermUi,
}

/// A file open for editing.
struct Editor {
    rel: String,
    input: Entity<TextInput>,
    /// The line endings the file had. The editor works in `\n` and puts them back on save.
    crlf: bool,
    _watch: Subscription,
}

/// What a typed name is for.
enum NamingKind {
    NewFile,
    Rename(String),
}

/// A name being typed.
struct Naming {
    kind: NamingKind,
    input: Entity<TextInput>,
    _watch: Subscription,
}

impl ProjectUi {
    /// The project on screen and what has been picked on its page: the folder, the file, and a sentence for the assistant.
    pub fn looking_at(&self, route: &Route) -> (Option<String>, Option<String>, Option<String>) {
        let Some(tab) = route.project_tab() else {
            return (None, None, None);
        };
        let Some(dir) = route.selection().map(str::to_string) else {
            return (None, None, None);
        };
        let (file, detail) = match tab {
            ProjectTab::Files if self.dir == dir => match &self.file {
                Some(rel) => (
                    Some(join(&dir, rel)),
                    Some(format!(
                        "reading the file {rel} in the project {}",
                        leaf(&dir)
                    )),
                ),
                None => (None, None),
            },
            ProjectTab::Changes if self.dir == dir => match &self.change {
                Some((path, staged)) => (
                    Some(join(&dir, path)),
                    Some(format!(
                        "looking at the {} changes to {path} in the project {}",
                        if *staged { "staged" } else { "unstaged" },
                        leaf(&dir)
                    )),
                ),
                None => (None, None),
            },
            ProjectTab::History if self.dir == dir => match &self.commit {
                Some(hash) => (
                    None,
                    Some(format!(
                        "looking at commit {} in the project {}",
                        hash.chars().take(7).collect::<String>(),
                        leaf(&dir)
                    )),
                ),
                None => (None, None),
            },
            _ => (None, None),
        };
        (Some(dir), file, detail)
    }
}

fn join(dir: &str, rel: &str) -> String {
    format!("{}/{}", dir.trim_end_matches(['/', '\\']), rel)
}

fn leaf(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

fn parent(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit_once(['/', '\\'])
        .map(|(p, _)| p)
        .unwrap_or("")
}

// -------------------------------------------------------------------------------------------- the frame

impl Shell {
    /// Put what is on this page in the message box and go to the assistant, so "fix this" has something to refer to.
    pub fn ask_about(&mut self, text: String, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.set_text(text, cx));
        self.go(Route::Assistant(None), cx);
    }
}

/// Draw one project's page on a tab.
pub fn render(
    shell: &mut Shell,
    dir: String,
    tab: ProjectTab,
    cx: &mut Context<Shell>,
) -> AnyElement {
    if shell.project_ui.dir != dir {
        shell.project_ui = ProjectUi {
            dir: dir.clone(),
            file_lines: FILE_PAGE,
            ..ProjectUi::default()
        };
    }
    if shell.project_ui.message.is_none() {
        shell.project_ui.message =
            Some(cx.new(|cx| TextInput::new("Commit message", cx).multiline(5)));
    }
    refresh_editor(shell, &dir, cx);
    let t = theme(cx).clone();
    let projects = devd(cx).sync.watch(&crate::state::q::projects());
    let entry = projects
        .list("")
        .iter()
        .find(|p| {
            p["path"]
                .as_str()
                .is_some_and(|p| p.replace('\\', "/") == dir.replace('\\', "/"))
        })
        .cloned();
    let name = entry
        .as_ref()
        .and_then(|e| e["name"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| leaf(&dir).to_string());
    let present = entry.as_ref().is_none_or(|e| e["exists"] != json!(false));
    let status = devd(cx).sync.watch(&surface::git::status(&dir).query());
    let is_repo = status.data.is_some();

    let detail = match status.data.as_deref() {
        Some(d) => {
            let (ahead, behind) = (
                d["ahead"].as_u64().unwrap_or(0),
                d["behind"].as_u64().unwrap_or(0),
            );
            let mut line = format!("On {}", d["branch"].as_str().unwrap_or("a detached head"));
            if ahead > 0 {
                line.push_str(&format!(", {ahead} to push"));
            }
            if behind > 0 {
                line.push_str(&format!(", {behind} to pull"));
            }
            line.push_str(if d["clean"] == json!(true) {
                ". Nothing to commit."
            } else {
                ". There are changes."
            });
            line
        }
        None if status.is_first_load() => "Reading the project…".to_string(),
        None => ui::leaf(&dir).to_string(),
    };

    let about = {
        let (n, d) = (name.clone(), dir.clone());
        move |this: &mut Shell, cx: &mut Context<Shell>| {
            this.ask_about(format!("About the project {n} ({d}): "), cx)
        }
    };
    let actions = div().flex().gap(px(8.0)).child(
        Button::new("project.ask", "Ask the assistant")
            .icon("assistant")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| about(this, cx))),
    );

    let mut column = div().flex().flex_col().gap(px(18.0)).child(
        div()
            .child(page_head(name.clone(), detail, actions, cx))
            .child(group_row(&dir, tab, cx)),
    );
    if tab.group() == Group::Git {
        column = column.child(git_row(&dir, tab, is_repo, cx));
    }
    column = column.children(ui::stale(&projects, cx));
    if !present {
        column = column.child(ui::empty("alert", "This folder is missing", "devd remembers the project but its folder is gone. Remove it from Projects, or put the folder back.", cx));
        return ui::page("project", 1180.0, column).into_any_element();
    }
    let ui_state = view_of(shell);
    let body = match tab {
        ProjectTab::Overview => overview(&dir, entry.as_ref(), &status, cx),
        ProjectTab::Files => files(&dir, &ui_state, cx),
        ProjectTab::Changes => changes(
            &dir,
            &ui_state,
            &status,
            shell.project_ui.message.clone(),
            cx,
        ),
        ProjectTab::History => history(&dir, &ui_state, cx),
        ProjectTab::Branches => branches(&dir, cx),
        ProjectTab::Stashes => stashes(&dir, cx),
        ProjectTab::Tags => tags(&dir, cx),
        ProjectTab::Remotes => remotes(&dir, cx),
        ProjectTab::Worktrees => worktrees(&dir, cx),
        ProjectTab::Recover => recover(&dir, cx),
        ProjectTab::Run => run(&dir, cx),
        ProjectTab::Terminal => {
            super::term::view(shell, super::term::Home::Project, &dir, 160, 480.0, cx)
        }
        ProjectTab::Agents => team(&dir, cx),
    };
    let _ = t;
    ui::page("project", 1180.0, column.child(body)).into_any_element()
}

/// A copy of what the page remembers, so drawing does not hold the shell.
fn view_of(shell: &Shell) -> ProjectView {
    let u = &shell.project_ui;
    ProjectView {
        open: u.open.clone(),
        file: u.file.clone(),
        file_lines: u.file_lines,
        change: u.change.clone(),
        commit: u.commit.clone(),
        source: u.source,
        dirty: u.dirty,
        editor: u.editor.as_ref().map(|e| (e.rel.clone(), e.input.clone())),
        naming: u
            .naming
            .as_ref()
            .map(|n| (matches!(n.kind, NamingKind::NewFile), n.input.clone())),
        edit_error: u.edit_error.clone(),
        scroll: u.scroll.clone(),
    }
}

/// What the page remembers, copied for drawing.
#[derive(Clone)]
struct ProjectView {
    open: HashSet<String>,
    file: Option<String>,
    file_lines: usize,
    change: Option<(String, bool)>,
    commit: Option<String>,
    source: bool,
    dirty: bool,
    /// The file in the editor, and its field.
    editor: Option<(String, Entity<TextInput>)>,
    /// Whether the name being typed is for a new file (else a rename), and its field.
    naming: Option<(bool, Entity<TextInput>)>,
    edit_error: Option<String>,
    scroll: ScrollHandle,
}

/// The first row: the six places of a project.
fn group_row(dir: &str, tab: ProjectTab, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let mut row = div()
        .flex()
        .items_end()
        .gap(px(2.0))
        .mb(px(0.0))
        .border_b_1()
        .border_color(t.line);
    for (n, group) in Group::ALL.into_iter().enumerate() {
        let on = tab.group() == group;
        let (dir, target) = (dir.to_string(), group.first());
        let hover = t.text;
        let mut el = div()
            .id(ui::nid("project.group", n))
            .px(px(12.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .text_size(px(size::TEXT))
            .cursor_pointer()
            .border_b_2()
            .child(group.label());
        el = if on {
            el.text_color(t.text)
                .font_weight(FontWeight::MEDIUM)
                .border_color(t.accent)
        } else {
            el.text_color(t.muted)
                .border_color(gpui::transparent_black())
                .hover(move |s| s.text_color(hover))
        };
        row = row.child(el.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.go(Route::project(dir.clone(), target), cx)
        })));
    }
    row
}

/// The second row, under Git: its eight views.
fn git_row(dir: &str, tab: ProjectTab, is_repo: bool, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let mut row = div().flex().flex_wrap().gap(px(4.0));
    if !is_repo {
        return row.child(
            div()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child("This folder is not a git repository yet."),
        );
    }
    for (n, target) in ProjectTab::GIT.into_iter().enumerate() {
        let on = tab == target;
        let (dir, hover, active) = (dir.to_string(), t.hover, t.active);
        let mut el = div()
            .id(ui::nid("project.git", n))
            .px(px(10.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .rounded(px(size::RADIUS))
            .text_size(px(size::TEXT_SM))
            .cursor_pointer()
            .child(target.label());
        el = if on {
            el.bg(active)
                .text_color(t.text)
                .font_weight(FontWeight::MEDIUM)
        } else {
            el.text_color(t.muted).hover(move |s| s.bg(hover))
        };
        row = row.child(el.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.go(Route::project(dir.clone(), target), cx)
        })));
    }
    row
}

// ------------------------------------------------------------------------------------------ small parts

/// A list entry that can be selected, with actions at its right.
#[allow(clippy::too_many_arguments)]
fn pick_row(
    id: ElementId,
    word: Option<(Tone, String)>,
    title: String,
    sub: String,
    selected: bool,
    acts: Vec<(ElementId, Act)>,
    on_pick: impl Fn(&mut Shell, &mut Context<Shell>) + 'static,
    cx: &mut Context<Shell>,
) -> Stateful<Div> {
    let t = theme(cx).clone();
    let (hover, active) = (t.hover, t.active);
    let mut line = div()
        .id(id)
        .px(px(12.0))
        .min_h(px(40.0))
        .py(px(5.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .cursor_pointer();
    line = if selected {
        line.bg(active)
    } else {
        line.hover(move |s| s.bg(hover))
    };
    if let Some((tone, w)) = word {
        line = line.child(
            div()
                .w(px(18.0))
                .flex_none()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .text_color(t.tone(tone))
                .child(w),
        );
    }
    let text = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .min_w_0()
                .text_size(px(size::TEXT))
                .child(ui::Clip::new(title)),
        )
        .child(
            div()
                .min_w_0()
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child(ui::Clip::new(sub)),
        );
    line = line.child(text);
    for (aid, act) in acts {
        line = line.child(act_button(aid, act, ButtonKind::Ghost, cx));
    }
    line.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_pick(this, cx)))
}

/// A heading line over a list, with something at its right.
fn titled(title: &'static str, right: Option<AnyElement>, cx: &App_) -> Div {
    let t = theme(cx);
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(28.0))
        .child(
            div()
                .text_size(px(size::TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.muted)
                .child(title),
        );
    match right {
        Some(r) => head.child(r),
        None => head,
    }
}

type App_ = gpui::App;

/// A unified diff as the window's own diff view, or what to say when there is none.
fn diff_view(
    id: &str,
    snap: &Snapshot,
    empty_title: &'static str,
    empty_detail: &'static str,
    cx: &App_,
) -> Div {
    let t = theme(cx);
    if snap.is_first_load() {
        return div().p(px(16.0)).child(ui::skeleton(
            ElementId::Name(format!("{id}.sk").into()),
            280.0,
            cx,
        ));
    }
    let patch = snap
        .data
        .as_deref()
        .and_then(|d| d["patch"].as_str())
        .unwrap_or_default();
    // A commit's patch starts with its message; the diffs begin at the first `diff --git`.
    let (head, body) = patch
        .split_once("\ndiff --git")
        .map(|(h, b)| (h.to_string(), format!("diff --git{b}")))
        .unwrap_or_else(|| (String::new(), patch.to_string()));
    let files = devd_app_core::diff::parse(&body);
    if files.is_empty() {
        return div()
            .p(px(16.0))
            .child(ui::empty("diff", empty_title, empty_detail, cx));
    }
    let mut column = div().flex().flex_col().gap(px(10.0));
    if !head.trim().is_empty() {
        column = column.child(
            div()
                .px(px(2.0))
                .font_family(mono())
                .text_size(px(size::TEXT_XS))
                .text_color(t.muted)
                .line_height(px(17.0))
                .child(head.trim().to_string()),
        );
    }
    for (n, file) in files.iter().enumerate() {
        column = column.child(ui::diff::file(&format!("{id}.f{n}"), file, cx));
    }
    column
}

/// The two panes of the Files, Changes and History tabs: a list on the left, what it selected on the right.
fn split(left: impl IntoElement, right: impl IntoElement, cx: &App_) -> Div {
    let t = theme(cx);
    div()
        .flex()
        .gap(px(14.0))
        .items_start()
        .child(
            div()
                .w(px(360.0))
                .flex_none()
                .child(card(cx).flex().flex_col().overflow_hidden().child(left)),
        )
        .child(
            div().flex_1().min_w_0().child(
                card(cx)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .min_h(px(260.0))
                    .bg(t.surface)
                    .child(right),
            ),
        )
}

// ---------------------------------------------------------------------------------------------- overview

fn overview(
    dir: &str,
    entry: Option<&Value>,
    status: &Snapshot,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let detect = read_call(cx, &surface::project::detect(dir));
    let tasks = read_call(cx, &surface::project::tasks(dir));
    let log = read_call(cx, &surface::git::log(dir, 6, None));
    let todos = read_call(cx, &surface::team::todos(dir));
    let d = status.data.as_deref();
    let count = |key: &str| d.and_then(|d| d[key].as_array()).map(Vec::len).unwrap_or(0);
    let (staged, unstaged, untracked, conflicted) = (
        count("staged"),
        count("unstaged"),
        count("untracked"),
        count("conflicted"),
    );
    let stack = detect
        .data
        .as_deref()
        .and_then(|d| d["primary"].as_str())
        .map(str::to_string)
        .or_else(|| entry.and_then(|e| e["stack"].as_str()).map(str::to_string));

    // The facts.
    let mut facts = Vec::new();
    facts.push(("Folder", dir.to_string(), true));
    if let Some(stack) = &stack {
        let manager = entry
            .and_then(|e| e["package_manager"].as_str())
            .map(|m| format!(" with {m}"))
            .unwrap_or_default();
        let mut shown = stack.clone();
        if let Some(first) = shown.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        facts.push(("Built with", format!("{shown}{manager}"), false));
    }
    if let Some(d) = d {
        facts.push((
            "Branch",
            d["branch"].as_str().unwrap_or("detached").to_string()
                + &d["upstream"]
                    .as_str()
                    .map(|u| format!("  ›  {u}"))
                    .unwrap_or_default(),
            false,
        ));
    }
    if let Some(sites) = entry
        .and_then(|e| e["sites"].as_array())
        .filter(|s| !s.is_empty())
    {
        facts.push((
            "Sites",
            sites
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            false,
        ));
    }
    let mut facts_card = card(cx).flex().flex_col().overflow_hidden();
    for (n, (label, value, path)) in facts.into_iter().enumerate() {
        if n > 0 {
            facts_card = facts_card.child(rule(cx));
        }
        let shown = if path {
            ui::Clip::tail(value).into_any_element()
        } else {
            ui::Clip::new(value).into_any_element()
        };
        facts_card = facts_card.child(
            div()
                .px(px(14.0))
                .py(px(9.0))
                .flex()
                .gap(px(12.0))
                .text_size(px(size::TEXT))
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(shown)),
        );
    }

    // What needs doing.
    let mut needs = Vec::new();
    let go =
        |label: &'static str, tab: ProjectTab, dir: &str, cx: &mut Context<Shell>, n: usize| {
            let dir = dir.to_string();
            Button::new(ui::nid("overview.go", n), label)
                .kind(ButtonKind::Ghost)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(Route::project(dir.clone(), tab), cx)
                }))
                .into_any_element()
        };
    if conflicted > 0 {
        needs.push(Row {
            title: format!(
                "{conflicted} file{} in conflict",
                if conflicted == 1 { "" } else { "s" }
            ),
            sub: "A merge or rebase stopped here and needs you.".into(),
            state: Some((Tone::Danger, "stopped".into())),
            ..Row::default()
        });
    }
    if staged + unstaged + untracked > 0 {
        needs.push(Row {
            title: format!(
                "{} file{} changed",
                staged + unstaged + untracked,
                if staged + unstaged + untracked == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            sub: format!("{staged} staged, {unstaged} not staged, {untracked} new."),
            state: Some((Tone::Warn, "uncommitted".into())),
            ..Row::default()
        });
    }
    if let Some(d) = d {
        if d["ahead"].as_u64().unwrap_or(0) > 0 {
            needs.push(Row {
                title: format!(
                    "{} commit{} not pushed",
                    d["ahead"],
                    if d["ahead"] == json!(1) { "" } else { "s" }
                ),
                sub: "They are only on this machine.".into(),
                state: Some((Tone::Info, "to push".into())),
                ..Row::default()
            });
        }
        if d["behind"].as_u64().unwrap_or(0) > 0 {
            needs.push(Row {
                title: format!(
                    "{} commit{} to pull",
                    d["behind"],
                    if d["behind"] == json!(1) { "" } else { "s" }
                ),
                sub: "The remote has work this machine does not.".into(),
                state: Some((Tone::Info, "behind".into())),
                ..Row::default()
            });
        }
    }
    let open_todos = todos
        .list("")
        .iter()
        .filter(|t| {
            !matches!(
                t["status"].as_str(),
                Some("done" | "completed" | "cancelled")
            )
        })
        .count();
    if open_todos > 0 {
        needs.push(Row {
            title: format!(
                "{open_todos} open to-do{}",
                if open_todos == 1 { "" } else { "s" }
            ),
            sub: "On the team's list for this project.".into(),
            state: Some((Tone::Neutral, "open".into())),
            ..Row::default()
        });
    }
    let needs_card = if needs.is_empty() && !status.is_first_load() {
        card(cx).p(px(14.0)).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(ui::dot(Tone::Ok, cx))
                .child(
                    div()
                        .text_size(px(size::TEXT))
                        .child("Nothing needs you here. Everything is committed and pushed."),
                ),
        )
    } else {
        list_or(
            status,
            needs,
            "overview.needs",
            "check",
            "Reading the project…",
            "",
            cx,
        )
    };

    // Recent work.
    let recent: Vec<Row> = log
        .list("")
        .iter()
        .map(|c| Row {
            title: text(c, "subject"),
            tag: text(c, "short"),
            sub: format!(
                "{} · {}",
                text(c, "author"),
                ui::ago(&iso(c["timestamp"].as_i64().unwrap_or(0)))
            ),
            ..Row::default()
        })
        .collect();
    let recent_card = list_or(
        &log,
        recent,
        "overview.recent",
        "branch",
        "No commits yet",
        "The first commit will be listed here.",
        cx,
    );

    // Tasks to run.
    let mut task_rows = Vec::new();
    for task in tasks
        .list("")
        .iter()
        .filter(|t| t["available"] != json!(false))
        .take(6)
    {
        let (id, title) = (text(task, "id"), text(task, "title"));
        let long = task["long_running"] == json!(true);
        task_rows.push(Row {
            title: if title.is_empty() { id.clone() } else { title },
            sub: text(task, "command"),
            state: long.then(|| (Tone::Info, "keeps running".to_string())),
            acts: vec![
                Act::new(
                    "Run",
                    "project.run",
                    surface::project::run(dir, &id, long).args,
                )
                .icon("play")
                .busy("Running…"),
            ],
            ..Row::default()
        });
    }
    let tasks_card = list_or(
        &tasks,
        task_rows,
        "overview.tasks",
        "play",
        "No tasks found",
        "devd looks for scripts in package.json, composer.json, a Makefile and the stack's own commands.",
        cx,
    );

    let all_tasks = go("All tasks", ProjectTab::Run, dir, cx, 1);
    let all_changes = go("Changes", ProjectTab::Changes, dir, cx, 2);
    let all_history = go("History", ProjectTab::History, dir, cx, 3);
    div()
        .flex()
        .flex_col()
        .gap(px(20.0))
        .child(facts_card)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("WHAT NEEDS YOU", Some(all_changes), cx))
                .child(needs_card),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("RECENT WORK", Some(all_history), cx))
                .child(recent_card),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("RUN", Some(all_tasks), cx))
                .child(tasks_card),
        )
        .into_any_element()
}

/// A Unix time as the ISO text `ui::ago` reads.
fn iso(seconds: i64) -> String {
    chrono_free_iso(seconds)
}

fn chrono_free_iso(seconds: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm), so no date library is needed here.
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let (y, doy) = (yoe + era * 400, doe - (365 * yoe + yoe / 4 - yoe / 100));
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// ------------------------------------------------------------------------------------------------ files

fn files(dir: &str, view: &ProjectView, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let root = read_call(cx, &surface::files::tree(dir));
    let mut rows: Vec<AnyElement> = Vec::new();
    tree_rows(dir, "", 0, view, &mut rows, cx);
    let tree = if rows.is_empty() {
        if root.is_first_load() {
            div().p(px(16.0)).child(ui::skeleton("files.sk", 200.0, cx))
        } else {
            div().p(px(16.0)).child(ui::empty(
                "folder",
                "This folder is empty",
                "Files you add are listed here.",
                cx,
            ))
        }
    } else {
        div().flex().flex_col().py(px(4.0)).children(rows)
    };
    let tree = div()
        .id("files.tree")
        .max_h(px(560.0))
        .overflow_y_scroll()
        .child(tree);
    let new_file = Button::new("files.new", "New file")
        .icon("plus")
        .kind(ButtonKind::Ghost)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.start_naming(NamingKind::NewFile, String::new(), window, cx)
        }));
    let mut head = div().flex().flex_col().child(
        div()
            .px(px(12.0))
            .h(px(38.0))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .text_size(px(size::TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.muted)
                    .child(leaf(dir).to_string()),
            )
            .child(new_file),
    );
    if let Some((new, input)) = &view.naming {
        head = head.child(naming_box(*new, input.clone(), cx));
    }
    let tree = div().flex().flex_col().child(head).child(tree);
    let viewer = match (&view.editor, &view.file) {
        (Some((rel, _)), Some(file)) if rel == file => editor_view(dir, view, cx),
        (_, Some(rel)) => file_view(dir, rel, view, cx),
        _ => div().p(px(24.0)).child(ui::empty(
            "file",
            "Pick a file",
            "Its text is shown here. Edit it right here, or ask the assistant to.",
            cx,
        )),
    };
    split(tree, viewer, cx).into_any_element()
}

/// One folder's rows, and the rows of every folder opened inside it.
fn tree_rows(
    dir: &str,
    rel: &str,
    depth: usize,
    view: &ProjectView,
    out: &mut Vec<AnyElement>,
    cx: &mut Context<Shell>,
) {
    let t = theme(cx).clone();
    let here = if rel.is_empty() {
        dir.to_string()
    } else {
        join(dir, rel)
    };
    let snap = read_call(cx, &surface::files::tree(&here));
    let mut entries: Vec<Value> = snap
        .data
        .as_deref()
        .and_then(|d| d["entries"].as_array())
        .cloned()
        .unwrap_or_default();
    entries.sort_by(|a, b| {
        let dir_first = |e: &Value| e["kind"] != json!("dir");
        (dir_first(a), text(a, "name").to_lowercase())
            .cmp(&(dir_first(b), text(b, "name").to_lowercase()))
    });
    for entry in entries.iter().take(400) {
        let name = text(entry, "name");
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let is_dir = entry["kind"] == json!("dir");
        let skipped = entry["skipped"] == json!(true);
        let secret = entry["secret"] == json!(true);
        let open = view.open.contains(&path);
        let selected = view.file.as_deref() == Some(path.as_str());
        let (hover, active) = (t.hover, t.active);
        let mut line = div()
            .id(ElementId::Name(format!("tree:{path}").into()))
            .h(px(26.0))
            .pl(px(10.0 + depth as f32 * 14.0))
            .pr(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(size::TEXT_SM));
        line = if selected {
            line.bg(active)
        } else {
            line.hover(move |s| s.bg(hover))
        };
        let glyph = if is_dir {
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            }
        } else {
            "file"
        };
        line = line.child(ui::icon(
            glyph,
            12.0,
            if is_dir { t.muted } else { t.faint },
        ));
        if is_dir {
            line = line.child(ui::icon("folder", 13.0, t.muted));
        }
        line = line.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(if skipped { t.faint } else { t.text })
                .child(name),
        );
        if secret {
            line = line.child(ui::icon("lock", 12.0, t.warn));
        }
        if skipped {
            line = line.child(
                div()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child("not opened"),
            );
        }
        let key = path.clone();
        let line = if skipped {
            line
        } else if is_dir {
            line.cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !this.project_ui.open.remove(&key) {
                        this.project_ui.open.insert(key.clone());
                    }
                    cx.notify();
                }))
        } else {
            line.cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.project_ui.dirty {
                        let open = this
                            .project_ui
                            .editor
                            .as_ref()
                            .map(|e| e.rel.clone())
                            .unwrap_or_default();
                        cx.global_mut::<crate::state::Devd>().toasts.push(
                            crate::state::Toast::new(
                                format!("Save or discard your changes to {} first.", leaf(&open)),
                                true,
                            ),
                        );
                        return cx.notify();
                    }
                    this.project_ui.editor = None;
                    this.project_ui.edit_error = None;
                    this.project_ui.file = Some(key.clone());
                    this.project_ui.file_lines = FILE_PAGE;
                    this.present(cx);
                    cx.notify();
                }))
        };
        out.push(line.into_any_element());
        if is_dir && open && !skipped && depth < 12 {
            tree_rows(dir, &path, depth + 1, view, out, cx);
        }
    }
    if entries.len() > 400 {
        out.push(
            div()
                .pl(px(10.0 + depth as f32 * 14.0))
                .h(px(24.0))
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child(format!("and {} more", entries.len() - 400))
                .into_any_element(),
        );
    }
}

fn is_markdown(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx")
}

/// The buttons every view of a file has: ask about it, rename it, delete it, copy its path, open it elsewhere.
fn file_actions(dir: &str, rel: &str, cx: &mut Context<Shell>) -> Div {
    let abs = join(dir, rel);
    let (outside, copy, ask, renamed) = (
        abs.clone(),
        abs.clone(),
        format!(
            "About the file {rel} in the project {} ({dir}): ",
            leaf(dir)
        ),
        rel.to_string(),
    );
    div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(
            Button::new("file.ask", "Ask the assistant")
                .icon("assistant")
                .kind(ButtonKind::Ghost)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_about(ask.clone(), cx)),
                ),
        )
        .child(
            Button::new("file.rename", "Rename")
                .kind(ButtonKind::Ghost)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.start_naming(
                        NamingKind::Rename(renamed.clone()),
                        renamed.clone(),
                        window,
                        cx,
                    )
                })),
        )
        .child(act_button(
            ElementId::Name("file.delete".into()),
            Act::new("Delete", "fs.delete", surface::files::delete(&abs).args).ask(
                format!("Delete {}?", leaf(rel)),
                "devd keeps a copy of the file first, so it can be restored from Activity.",
                "Delete file",
            ),
            ButtonKind::Ghost,
            cx,
        ))
        .child(
            Button::icon_only("file.copy", "copy").on_click(move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()))
            }),
        )
        .child(
            Button::icon_only("file.outside", "external").on_click(move |_, _, _| {
                let _ = open::that(&outside);
            }),
        )
}

fn file_view(dir: &str, rel: &str, view: &ProjectView, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let lines = view.file_lines;
    let markdown = is_markdown(rel);
    let abs = join(dir, rel);
    let snap = read_call(
        cx,
        &surface::files::read(
            &abs,
            None,
            Some(if markdown && !view.source {
                4000
            } else {
                lines
            }),
        ),
    );
    let toggle = markdown.then(|| {
        Button::new(
            "file.source",
            if view.source { "Preview" } else { "Source" },
        )
        .kind(ButtonKind::Ghost)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.project_ui.source = !this.project_ui.source;
            cx.notify();
        }))
    });
    let edit = {
        let key = rel.to_string();
        Button::new("file.edit", "Edit")
            .icon("file")
            .kind(ButtonKind::Secondary)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.project_ui.edit_error = None;
                this.project_ui.edit_request = Some(key.clone());
                cx.notify();
            }))
    };
    let head = div()
        .px(px(14.0))
        .h(px(44.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .border_b_1()
        .border_color(t.line)
        .child(ui::icon("file", 14.0, t.faint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .child(ui::Clip::tail(rel.to_string())),
        )
        .children(toggle)
        .child(edit)
        .child(file_actions(dir, rel, cx));
    let head = match &view.edit_error {
        Some(why) => div()
            .child(head)
            .child(div().px(px(14.0)).pt(px(10.0)).child(ui::Banner::new(Tone::Warn).text(why.clone()).render(cx))),
        None => div().child(head),
    };
    if snap.is_first_load() {
        return div()
            .child(head)
            .child(div().p(px(16.0)).child(ui::skeleton("file.sk", 300.0, cx)));
    }
    let Some(data) = snap.data.as_deref() else {
        let why = snap
            .error
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| "devd could not read it.".into());
        return div()
            .child(head)
            .child(div().p(px(16.0)).text_color(t.warn).child(why));
    };
    // `fs.read` returns each line as its number, a tab, then the text.
    let content = data["content"].as_str().unwrap_or_default();
    if markdown && !view.source {
        let source: String = content
            .lines()
            .map(|l| l.split_once('\t').map(|(_, t)| t).unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n");
        let blocks = devd_app_core::markdown::parse(&source);
        let drawn = ui::markdown::document("file.md", &blocks, cx);
        let note = (data["truncated"] == json!(true)).then(|| div().pt(px(12.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child("This preview stops where the file's first 4000 lines end. Switch to Source to read on."));
        return div().child(head).child(
            div()
                .id("file.preview")
                .max_h(px(560.0))
                .overflow_y_scroll()
                .px(px(28.0))
                .py(px(20.0))
                .child(
                    div()
                        .max_w(px(760.0))
                        .text_size(px(size::TEXT_MD))
                        .child(drawn)
                        .children(note),
                ),
        );
    }
    let mut body = div().flex().flex_col().py(px(8.0));
    for line in content.lines() {
        let (number, code) = line.split_once('\t').unwrap_or(("", line));
        body = body.child(
            div()
                .flex()
                .gap(px(12.0))
                .px(px(14.0))
                .child(
                    div()
                        .w(px(44.0))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .font_family(mono())
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(number.trim().to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono())
                        .text_size(px(size::TEXT_SM))
                        .line_height(px(18.0))
                        .whitespace_nowrap()
                        .child(if code.is_empty() {
                            " ".to_string()
                        } else {
                            code.replace('\t', "    ")
                        }),
                ),
        );
    }
    let total = data["total_lines"].as_u64().unwrap_or(0);
    let shown = data["to_line"].as_u64().unwrap_or(0);
    let mut view = div().child(head).child(
        div()
            .id("file.body")
            .max_h(px(560.0))
            .overflow_scroll()
            .child(body),
    );
    if data["truncated"] == json!(true) || shown < total {
        let more = Button::new("file.more", format!("Show {FILE_PAGE} more lines"))
            .kind(ButtonKind::Ghost)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.project_ui.file_lines += FILE_PAGE;
                cx.notify();
            }));
        view = view.child(
            div()
                .px(px(14.0))
                .h(px(40.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .border_t_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex_1()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(format!("Showing lines 1 to {shown} of {total}.")),
                )
                .child(more),
        );
    }
    view
}

// --------------------------------------------------------------------------------------------- changes

fn change_word(status: &str) -> (Tone, String) {
    match status {
        "A" => (Tone::Ok, "A".into()),
        "D" => (Tone::Danger, "D".into()),
        "R" => (Tone::Info, "R".into()),
        "?" => (Tone::Ok, "N".into()),
        _ => (Tone::Warn, "M".into()),
    }
}

fn changes(
    dir: &str,
    view: &ProjectView,
    status: &Snapshot,
    message: Option<Entity<TextInput>>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    let Some(d) = status.data.as_deref() else {
        return if status.is_first_load() {
            div()
                .child(ui::skeleton("changes.sk", 240.0, cx))
                .into_any_element()
        } else {
            ui::empty(
                "branch",
                "This folder is not a git repository",
                "Ask the assistant to start one, or run git init in the Terminal tab.",
                cx,
            )
            .into_any_element()
        };
    };
    let entries = |key: &str| -> Vec<(String, String)> {
        d[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| (text(e, "path"), text(e, "status")))
                    .collect()
            })
            .unwrap_or_default()
    };
    let staged = entries("staged");
    let unstaged = entries("unstaged");
    let untracked: Vec<(String, String)> = d["untracked"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|p| (p.to_string(), "?".to_string()))
                .collect()
        })
        .unwrap_or_default();
    let conflicted: Vec<(String, String)> = d["conflicted"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().or(e["path"].as_str()))
                .map(|p| (p.to_string(), "U".to_string()))
                .collect()
        })
        .unwrap_or_default();

    // The remote: fetch, pull, push.
    let ahead = d["ahead"].as_u64().unwrap_or(0);
    let behind = d["behind"].as_u64().unwrap_or(0);
    let branch = d["branch"].as_str().unwrap_or("detached head").to_string();
    let upstream = d["upstream"].as_str().map(str::to_string);
    let sync_bar = card(cx)
        .px(px(14.0))
        .py(px(10.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(ui::icon("branch", 14.0, t.muted))
        .child(div().font_weight(FontWeight::MEDIUM).child(branch.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child(match &upstream {
                    Some(u) => format!(
                        "{u}{}{}",
                        if ahead > 0 {
                            format!(", {ahead} to push")
                        } else {
                            String::new()
                        },
                        if behind > 0 {
                            format!(", {behind} to pull")
                        } else {
                            String::new()
                        }
                    ),
                    None => "No upstream set. Push to publish this branch.".to_string(),
                }),
        )
        .child(act_button(
            ElementId::Name("changes.fetch".into()),
            Act::new("Fetch", "git.fetch", surface::git::fetch(dir).args)
                .icon("refresh")
                .busy("Fetching…"),
            ButtonKind::Ghost,
            cx,
        ))
        .child(act_button(
            ElementId::Name("changes.pull".into()),
            Act::new("Pull", "git.pull", surface::git::pull(dir).args)
                .icon("arrow-down")
                .busy("Pulling…"),
            ButtonKind::Secondary,
            cx,
        ))
        .child(act_button(
            ElementId::Name("changes.push".into()),
            Act::new("Push", "git.push", surface::git::push(dir).args)
                .icon("arrow-up")
                .busy("Pushing…"),
            if ahead > 0 || upstream.is_none() {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            },
            cx,
        ));

    let group = |title: &'static str,
                 items: &[(String, String)],
                 is_staged: bool,
                 cx: &mut Context<Shell>|
     -> Option<Div> {
        if items.is_empty() {
            return None;
        }
        let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
        let all = if is_staged {
            Act::new(
                "Unstage all",
                "git.unstage",
                surface::git::unstage(dir, &paths).args,
            )
        } else {
            Act::new(
                "Stage all",
                "git.stage",
                surface::git::stage(dir, &paths).args,
            )
        };
        let mut list = div().flex().flex_col();
        for (n, (path, st)) in items.iter().enumerate() {
            let selected = view
                .change
                .as_ref()
                .is_some_and(|(p, s)| p == path && *s == is_staged);
            let one = vec![path.clone()];
            let mut acts = Vec::new();
            if is_staged {
                acts.push((
                    ui::nid("change.unstage", n + if is_staged { 0 } else { 1000 }),
                    Act::new(
                        "Unstage",
                        "git.unstage",
                        surface::git::unstage(dir, &one).args,
                    ),
                ));
            } else {
                acts.push((
                    ui::nid("change.stage", n),
                    Act::new("Stage", "git.stage", surface::git::stage(dir, &one).args),
                ));
                if st != "?" {
                    acts.push((ui::nid("change.discard", n), Act::new("Discard", "git.discard", surface::git::discard(dir, &one).args).ask(format!("Discard changes to {}?", leaf(path)), "The file goes back to how it was at the last commit. What you changed is lost and cannot be brought back.", "Discard changes")));
                }
            }
            let key = path.clone();
            let id = ElementId::Name(format!("change:{}:{path}", is_staged as u8).into());
            list = list.child(pick_row(
                id,
                Some(change_word(st)),
                leaf(path).to_string(),
                parent(path).to_string(),
                selected,
                acts,
                move |this, cx| {
                    this.project_ui.change = Some((key.clone(), is_staged));
                    this.present(cx);
                    cx.notify();
                },
                cx,
            ));
            if n + 1 < items.len() {
                list = list.child(rule(cx));
            }
        }
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .h(px(34.0))
            .bg(t.hover)
            .child(
                div()
                    .text_size(px(size::TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("{title}  {}", items.len())),
            )
            .child(act_button(
                ElementId::Name(format!("change.all.{}", is_staged as u8).into()),
                all,
                ButtonKind::Ghost,
                cx,
            ));
        Some(div().flex().flex_col().child(head).child(list))
    };
    let mut left = div().flex().flex_col();
    let mut first = true;
    for (title, items, is_staged) in [
        ("Conflicts", &conflicted, false),
        ("Staged", &staged, true),
        ("Changes", &unstaged, false),
        ("New files", &untracked, false),
    ] {
        if let Some(g) = group(title, items, is_staged, cx) {
            if !first {
                left = left.child(rule(cx));
            }
            left = left.child(g);
            first = false;
        }
    }
    if first {
        left = left.child(div().p(px(16.0)).child(ui::empty(
            "check",
            "Nothing has changed",
            "Edit a file and it is listed here, ready to commit.",
            cx,
        )));
    }

    // The commit box.
    let n_staged = staged.len();
    let msg = message.clone();
    let repo = dir.to_string();
    let commit = Button::new(
        "changes.commit",
        if n_staged == 0 {
            "Commit".to_string()
        } else {
            format!(
                "Commit {n_staged} file{}",
                if n_staged == 1 { "" } else { "s" }
            )
        },
    )
    .kind(ButtonKind::Primary)
    .disabled(n_staged == 0)
    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
        let Some(input) = msg.clone() else { return };
        let text = input.read(cx).text().trim().to_string();
        if text.is_empty() {
            cx.global_mut::<crate::state::Devd>()
                .toasts
                .push(crate::state::Toast::new(
                    "Write a message for the commit first.",
                    true,
                ));
            cx.notify();
            return;
        }
        crate::state::start_write(
            cx,
            surface::git::commit(&repo, &text).mutation(),
            Some("Committing…".to_string()),
            None,
        );
        input.update(cx, |i, cx| i.clear(cx));
        this.project_ui.change = None;
    }));
    let stash = act_button(
        ElementId::Name("changes.stash".into()),
        Act::new(
            "Stash",
            "git.stash.push",
            surface::git::stash_push(dir, None).args,
        )
        .busy("Putting changes aside…"),
        ButtonKind::Ghost,
        cx,
    );
    let box_ = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(12.0))
        .border_t_1()
        .border_color(t.line)
        .child(match message {
            Some(input) => div()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line_strong)
                .bg(t.raised)
                .px(px(10.0))
                .py(px(6.0))
                .child(input)
                .into_any_element(),
            None => div().into_any_element(),
        })
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(stash)
                .child(commit),
        );
    left = left.child(box_);

    // The diff of the chosen file.
    let right = match &view.change {
        Some((path, is_staged)) => {
            let snap = read_call(cx, &surface::git::diff(dir, Some(path), *is_staged));
            let new_file = untracked.iter().any(|(p, _)| p == path);
            let head = div()
                .px(px(14.0))
                .h(px(40.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .border_b_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono())
                        .text_size(px(size::TEXT_SM))
                        .child(ui::Clip::tail(path.clone())),
                )
                .child(pill(
                    if *is_staged {
                        "staged"
                    } else if new_file {
                        "new file"
                    } else {
                        "not staged"
                    },
                    Tone::Neutral,
                    cx,
                ));
            if new_file {
                // A new file has no diff yet: show its text.
                file_view(
                    dir,
                    path,
                    &ProjectView {
                        file_lines: 120,
                        source: true,
                        edit_error: None,
                        ..view.clone()
                    },
                    cx,
                )
                .into_any_element()
            } else {
                div()
                    .child(head)
                    .child(
                        div()
                            .p(px(12.0))
                            .id("change.diff")
                            .max_h(px(680.0))
                            .overflow_y_scroll()
                            .child(diff_view(
                                "change.d",
                                &snap,
                                "No changes to show",
                                "This file has no differences from the last commit.",
                                cx,
                            )),
                    )
                    .into_any_element()
            }
        }
        None => div()
            .p(px(24.0))
            .child(ui::empty(
                "diff",
                "Pick a file to see what changed",
                "Added lines are green and removed lines are red.",
                cx,
            ))
            .into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(sync_bar)
        .child(split(left, right, cx))
        .into_any_element()
}

// ---------------------------------------------------------------------------------------------- history

fn history(dir: &str, view: &ProjectView, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let log = read_call(cx, &surface::git::log(dir, 100, None));
    let mut list = div()
        .id("history.list")
        .max_h(px(680.0))
        .overflow_y_scroll()
        .flex()
        .flex_col();
    if log.list("").is_empty() {
        list = list.child(div().p(px(16.0)).child(if log.is_first_load() {
            ui::skeleton("history.sk", 220.0, cx).into_any_element()
        } else {
            ui::empty(
                "branch",
                "No commits yet",
                "Commit your changes and they are listed here.",
                cx,
            )
            .into_any_element()
        }));
    }
    for (n, c) in log.list("").iter().enumerate() {
        let hash = text(c, "hash");
        let selected = view.commit.as_deref() == Some(hash.as_str());
        let key = hash.clone();
        let when = ui::ago(&iso(c["timestamp"].as_i64().unwrap_or(0)));
        list = list.child(pick_row(
            ElementId::Name(format!("commit:{hash}").into()),
            None,
            text(c, "subject"),
            format!(
                "{}  ·  {}  ·  {}",
                text(c, "short"),
                text(c, "author"),
                when
            ),
            selected,
            vec![],
            move |this, cx| {
                this.project_ui.commit = Some(key.clone());
                this.present(cx);
                cx.notify();
            },
            cx,
        ));
        if n + 1 < log.list("").len() {
            list = list.child(rule(cx));
        }
    }
    let right = match &view.commit {
        Some(hash) => {
            let snap = read_call(cx, &surface::git::show(dir, hash));
            let short: String = hash.chars().take(7).collect();
            let branch_form = FormSpec {
                title: "Start a branch here",
                intro: "A new branch that begins at this commit. Your working files are not touched.",
                verb: "Create branch",
                capability: "git.branch.create",
                domain: "git",
                fixed: {
                    let mut a = surface::git::branch_create(dir, "").args;
                    a.as_object_mut().map(|m| m.remove("name"));
                    a["start_point"] = json!(hash);
                    a["checkout"] = json!(false);
                    a
                },
                fields: vec![Field::text("name", "Branch name", "fix-login")],
                busy: Some("Creating {name}…"),
            };
            let head = div().px(px(14.0)).h(px(40.0)).flex().items_center().gap(px(10.0)).border_b_1().border_color(t.line).child(div().flex_1().font_family(mono()).text_size(px(size::TEXT_SM)).child(short.clone())).child(form_button("history.branch", "Branch from here", "branch", branch_form, ButtonKind::Ghost, cx)).child(act_button(ElementId::Name("history.revert".into()), Act::new("Revert", "git.revert", surface::git::revert(dir, hash).args).busy("Reverting…").ask(format!("Revert {short}?"), "devd makes a new commit that undoes this one. The original stays in the history.", "Revert commit"), ButtonKind::Ghost, cx));
            div().child(head).child(
                div()
                    .p(px(12.0))
                    .id("history.diff")
                    .max_h(px(680.0))
                    .overflow_y_scroll()
                    .child(diff_view(
                        "history.d",
                        &snap,
                        "Nothing to show",
                        "This commit changes no files.",
                        cx,
                    )),
            )
        }
        None => div().p(px(24.0)).child(ui::empty(
            "branch",
            "Pick a commit",
            "Its message and every change in it are shown here.",
            cx,
        )),
    };
    split(list, right, cx).into_any_element()
}

// --------------------------------------------------------------------------------- the other git views

fn branches(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::branches(dir));
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|b| {
            let name = text(b, "name");
            let current = b["current"] == json!(true);
            let mut acts = Vec::new();
            if !current {
                acts.push(Act::new("Switch", "git.switch", surface::git::switch(dir, &name).args).busy("Switching…"));
                acts.push(Act::new("Delete", "git.branch.delete", surface::git::branch_delete(dir, &name).args).ask(format!("Delete the branch {name}?"), "Commits only on this branch become hard to find. Git refuses if it has unmerged work.", "Delete branch"));
            }
            Row { title: name, sub: text(b, "subject"), tag: text(b, "upstream"), state: Some(if current { (Tone::Ok, "current".into()) } else { (Tone::Neutral, String::new()) }), acts, ..Row::default() }
        })
        .collect();
    let mut fixed = surface::git::branch_create(dir, "").args;
    fixed.as_object_mut().map(|m| m.remove("name"));
    let new = form_button(
        "branches.new",
        "New branch",
        "plus",
        FormSpec {
            title: "New branch",
            intro: "Make a branch from where you are now, and move to it.",
            verb: "Create branch",
            capability: "git.branch.create",
            domain: "git",
            fixed,
            fields: vec![Field::text("name", "Branch name", "feature-login")],
            busy: Some("Creating {name}…"),
        },
        ButtonKind::Primary,
        cx,
    );
    let list = list_or(
        &snap,
        rows,
        "branches.row",
        "branch",
        "No branches",
        "Make a commit and its branch appears here.",
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("BRANCHES", Some(new), cx))
        .child(list)
        .into_any_element()
}

fn stashes(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::stashes(dir));
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|s| {
            let reference = s["reference"]
                .as_str()
                .or(s["name"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("stash@{{{}}}", s["index"].as_u64().unwrap_or(0)));
            // git words a stash "On main: work in progress" (or "WIP on main: 1a2b3c4 subject").
            let subject = s["message"]
                .as_str()
                .or(s["subject"].as_str())
                .unwrap_or_default();
            let (branch, message) = match subject.split_once(": ") {
                Some((head, rest))
                    if head.to_lowercase().contains(" on ")
                        || head.to_lowercase().starts_with("on ") =>
                {
                    (
                        head.rsplit(' ').next().map(str::to_string),
                        rest.to_string(),
                    )
                }
                _ => (
                    s["branch"].as_str().map(str::to_string),
                    subject.to_string(),
                ),
            };
            Row {
                title: if message.is_empty() {
                    "Changes put aside".to_string()
                } else {
                    message
                },
                tag: reference.clone(),
                sub: branch.map(|b| format!("from {b}")).unwrap_or_default(),
                acts: vec![
                    Act::new(
                        "Bring back",
                        "git.stash.apply",
                        surface::git::stash_pop(dir, &reference).args,
                    )
                    .busy("Bringing the changes back…"),
                    Act::new(
                        "Drop",
                        "git.stash.drop",
                        surface::git::stash_drop(dir, &reference).args,
                    )
                    .ask(
                        "Drop this stash?",
                        "The changes in it are deleted for good.",
                        "Drop stash",
                    ),
                ],
                ..Row::default()
            }
        })
        .collect();
    let new = act_button(
        ElementId::Name("stashes.new".into()),
        Act::new(
            "Stash changes",
            "git.stash.push",
            surface::git::stash_push(dir, None).args,
        )
        .icon("plus")
        .busy("Putting changes aside…"),
        ButtonKind::Primary,
        cx,
    );
    let list = list_or(
        &snap,
        rows,
        "stashes.row",
        "branch",
        "No stashes",
        "Stash your changes to put them aside and come back to them later.",
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("STASHES", Some(new), cx))
        .child(list)
        .into_any_element()
}

fn tags(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::tags(dir));
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|t| Row {
            title: text(t, "name"),
            tag: text(t, "target"),
            sub: t["message"].as_str().unwrap_or_default().to_string(),
            ..Row::default()
        })
        .collect();
    let new = form_button(
        "tags.new",
        "New tag",
        "plus",
        FormSpec {
            title: "New tag",
            intro: "A name for the commit you are on, such as a release.",
            verb: "Create tag",
            capability: "git.tag.create",
            domain: "git",
            fixed: json!({ "repo": dir }),
            fields: vec![
                Field::text("name", "Tag name", "v1.0.0"),
                Field::optional("message", "Message", "First release"),
            ],
            busy: Some("Tagging {name}…"),
        },
        ButtonKind::Primary,
        cx,
    );
    let list = list_or(
        &snap,
        rows,
        "tags.row",
        "branch",
        "No tags",
        "Tag a commit to mark a release.",
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("TAGS", Some(new), cx))
        .child(list)
        .into_any_element()
}

fn remotes(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::remotes(dir));
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|r| Row {
            title: text(r, "name"),
            sub: text(r, "fetch_url"),
            ..Row::default()
        })
        .collect();
    let list = list_or(
        &snap,
        rows,
        "remotes.row",
        "external",
        "No remotes",
        "This repository is not connected to a server such as GitHub. Ask the assistant to add one.",
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("REMOTES", None, cx))
        .child(list)
        .into_any_element()
}

fn worktrees(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::worktrees(dir));
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|w| Row {
            title: text(w, "path"),
            sub: format!("on {}", w["branch"].as_str().unwrap_or("a detached head")),
            tag: text(w, "head").chars().take(7).collect(),
            state: Some(if w["main"] == json!(true) {
                (Tone::Ok, "main".into())
            } else if w["locked"] == json!(true) {
                (Tone::Warn, "locked".into())
            } else {
                (Tone::Neutral, String::new())
            }),
            sub_is_path: false,
            ..Row::default()
        })
        .collect();
    let list = list_or(
        &snap,
        rows,
        "worktrees.row",
        "folder",
        "No other working folders",
        "A worktree lets you have another branch checked out in its own folder at the same time.",
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("WORKTREES", None, cx))
        .child(list)
        .into_any_element()
}

fn recover(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let snap = read_call(cx, &surface::git::reflog(dir, 60));
    let entries: Vec<Value> = snap
        .data
        .as_deref()
        .and_then(|d| d["entries"].as_array())
        .cloned()
        .unwrap_or_default();
    let rows: Vec<Row> = entries
        .iter()
        .map(|e| {
            let hash = text(e, "hash");
            let mut fixed = surface::git::branch_create(dir, "").args;
            fixed.as_object_mut().map(|m| m.remove("name"));
            fixed["start_point"] = json!(hash);
            fixed["checkout"] = json!(false);
            Row {
                title: text(e, "subject"),
                tag: hash,
                sub: format!("{}  ·  {}", text(e, "action"), text(e, "timestamp")),
                form: Some(FormSpec { title: "Bring this back as a branch", intro: "A new branch that starts exactly here, so work you thought was lost is safe again. Nothing in your folder changes.", verb: "Create branch", capability: "git.branch.create", domain: "git", fixed, fields: vec![Field::text("name", "Branch name", "recovered")], busy: Some("Creating {name}…") }),
                form_label: "Branch from here",
                ..Row::default()
            }
        })
        .collect();
    let list = list_or(
        &snap,
        rows,
        "recover.row",
        "refresh",
        "Nothing to recover",
        "Everywhere HEAD has been is listed here, even commits no branch points to.",
        cx,
    );
    let note = div().text_size(px(size::TEXT_SM)).text_color(theme(cx).muted).child("Everything HEAD has pointed at lately, newest first. If a commit seems lost, find it here and start a branch from it.");
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(titled("RECOVER", None, cx))
        .child(note)
        .child(list)
        .into_any_element()
}

// -------------------------------------------------------------------------------------------- run, shell, team

fn run(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let tasks = read_call(cx, &surface::project::tasks(dir));
    let procs = read_call(cx, &surface::project::status(dir));
    let mut rows = Vec::new();
    for task in tasks.list("") {
        let (id, title, command) = (text(task, "id"), text(task, "title"), text(task, "command"));
        let available = task["available"] != json!(false);
        let risk = text(task, "risk");
        let long = task["long_running"] == json!(true);
        let state = if !available {
            Some((
                Tone::Neutral,
                task["unavailable_reason"]
                    .as_str()
                    .unwrap_or("not available here")
                    .to_string(),
            ))
        } else if risk == "destructive" {
            Some((Tone::Danger, "destroys data".to_string()))
        } else if long {
            Some((Tone::Info, "keeps running".to_string()))
        } else {
            None
        };
        let mut acts = Vec::new();
        if available {
            let (cap, args) = if risk == "destructive" {
                (
                    "project.run_destructive",
                    surface::project::run(dir, &id, false).args,
                )
            } else {
                ("project.run", surface::project::run(dir, &id, long).args)
            };
            let act = Act::new("Run", cap, args).icon("play").busy(format!(
                "Running {}…",
                if title.is_empty() { &id } else { &title }
            ));
            acts.push(if risk == "destructive" {
                act.ask(
                    format!("Run {title}?"),
                    format!(
                        "This runs {command} and can destroy data that cannot be brought back."
                    ),
                    "Run it",
                )
            } else {
                act
            });
        }
        rows.push(Row {
            title: if title.is_empty() { id } else { title },
            sub: command,
            state,
            acts,
            ..Row::default()
        });
    }
    let list = list_or(
        &tasks,
        rows,
        "run.task",
        "play",
        "No tasks found",
        "devd looks for scripts in package.json, composer.json, a Makefile and the stack's own commands.",
        cx,
    );

    // Processes the project declares in devd.yml.
    let process_body = match procs.data.as_deref() {
        Some(d) => {
            let items: Vec<Value> = d["processes"].as_array().or(d.as_array()).cloned().unwrap_or_default();
            let rows: Vec<Row> = items
                .iter()
                .map(|p| {
                    let name = text(p, "name");
                    let running = p["running"] == json!(true) || p["state"] == json!("running");
                    Row {
                        title: name.clone(),
                        sub: p["command"].as_str().unwrap_or_default().to_string(),
                        state: Some(if running { (Tone::Ok, "running".into()) } else { (Tone::Neutral, "stopped".into()) }),
                        acts: vec![if running { Act::new("Stop", "project.stop", surface::project::stop(dir, &name).args) } else { Act::new("Start", "project.start", surface::project::start(dir, &name).args).icon("play") }],
                        ..Row::default()
                    }
                })
                .collect();
            list_or(&procs, rows, "run.proc", "machine", "No processes declared", "Declare the processes this project needs in its devd.yml and devd keeps them going.", cx)
        }
        None if procs.is_first_load() => card(cx).p(px(16.0)).child(ui::skeleton("run.proc.sk", 220.0, cx)),
        None => card(cx).p(px(16.0)).flex().flex_col().gap(px(4.0)).child(div().text_size(px(size::TEXT)).font_weight(FontWeight::MEDIUM).child("This project has no devd.yml")).child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child("A devd.yml says which processes the project needs (a web server, a queue worker, a build watcher) so devd can start, stop and watch them. Ask the assistant to write one from what it finds in the project.")),
    };
    let ask = {
        let (n, d) = (leaf(dir).to_string(), dir.to_string());
        Button::new("run.ask", "Write a devd.yml").icon("assistant").kind(ButtonKind::Ghost).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_about(format!("Read the project {n} ({d}) and write a devd.yml that declares the processes it needs. Show me the plan before applying it."), cx))).into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap(px(20.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("TASKS", None, cx))
                .child(list),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("PROCESSES", Some(ask), cx))
                .child(process_body),
        )
        .into_any_element()
}

fn team(dir: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let todos = read_call(cx, &surface::team::todos(dir));
    let reviews = read_call(cx, &surface::team::reviews());
    let memory = read_call(cx, &surface::team::memory(dir));
    let todo_rows: Vec<Row> = todos
        .list("")
        .iter()
        .map(|d| {
            let status = text(d, "status");
            let done = matches!(status.as_str(), "done" | "completed");
            Row {
                title: text(d, "title"),
                sub: d["description"].as_str().unwrap_or_default().to_string(),
                state: Some(match status.as_str() {
                    "in_progress" => (Tone::Info, "in progress".into()),
                    "done" | "completed" => (Tone::Ok, "done".into()),
                    "cancelled" => (Tone::Neutral, "cancelled".into()),
                    _ => (Tone::Warn, "open".into()),
                }),
                side: d["assignee"].as_str().unwrap_or_default().to_string(),
                acts: if done {
                    vec![]
                } else {
                    vec![Act::new(
                        "Done",
                        "todo.complete",
                        surface::team::todo_complete(
                            &text(d, "id"),
                            d["revision"].as_u64().unwrap_or(0),
                        )
                        .args,
                    )]
                },
                ..Row::default()
            }
        })
        .collect();
    let new_todo = form_button(
        "team.todo",
        "Add a to-do",
        "plus",
        FormSpec {
            title: "Add a to-do",
            intro: "A job for the team to pick up on this project.",
            verb: "Add to-do",
            capability: "todo.create",
            domain: "todo",
            fixed: json!({ "project": dir }),
            fields: vec![
                Field::text("title", "What needs doing", "Add a login rate limit"),
                Field::optional("description", "Details", ""),
            ],
            busy: None,
        },
        ButtonKind::Primary,
        cx,
    );
    let review_rows: Vec<Row> = reviews
        .list("")
        .iter()
        .filter(|r| {
            r["project"]
                .as_str()
                .is_none_or(|p| p.replace('\\', "/") == dir.replace('\\', "/"))
        })
        .map(|r| Row {
            title: r["title"]
                .as_str()
                .or(r["summary"].as_str())
                .unwrap_or("Review")
                .to_string(),
            sub: text(r, "state"),
            ..Row::default()
        })
        .collect();
    let start_review = act_button(
        ElementId::Name("team.review".into()),
        Act::new(
            "Review the changes",
            "review.start",
            surface::team::review(dir).args,
        )
        .icon("check")
        .busy("Starting a review…"),
        ButtonKind::Ghost,
        cx,
    );
    let facts = memory
        .data
        .as_deref()
        .map(|m| {
            ["facts", "notes", "digest"]
                .iter()
                .filter_map(|k| {
                    m[*k]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    let memory_card = if facts.trim().is_empty() {
        card(cx).p(px(14.0)).child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child("Nothing is remembered about this project yet. As the team works on it, what it learns is proposed here and kept once you agree."))
    } else {
        card(cx).p(px(14.0)).child(
            div()
                .text_size(px(size::TEXT_SM))
                .line_height(px(20.0))
                .child(facts),
        )
    };
    div()
        .flex()
        .flex_col()
        .gap(px(20.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("TO-DOS", Some(new_todo), cx))
                .child(list_or(
                    &todos,
                    todo_rows,
                    "team.todos",
                    "check",
                    "No to-dos",
                    "Add one, or let the team propose them as it works.",
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("REVIEWS", Some(start_review), cx))
                .child(list_or(
                    &reviews,
                    review_rows,
                    "team.reviews",
                    "check",
                    "No reviews yet",
                    "Ask for a review and the team reads your changes and reports what it finds.",
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(titled("WHAT THE TEAM REMEMBERS", None, cx))
                .child(memory_card),
        )
        .into_any_element()
}

// ------------------------------------------------------------------------------------------- the editor

/// A box for typing the name of a new file, or a file's new name.
fn naming_box(new: bool, input: Entity<TextInput>, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    div()
        .px(px(12.0))
        .py(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .border_b_1()
        .border_color(t.line)
        .bg(t.hover)
        .child(
            div()
                .text_size(px(size::TEXT_XS))
                .text_color(t.muted)
                .child(if new {
                    "A path from the project's folder. Folders on the way are made."
                } else {
                    "The new path from the project's folder."
                }),
        )
        .child(
            div()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line_strong)
                .bg(t.raised)
                .px(px(10.0))
                .py(px(5.0))
                .child(input),
        )
        .child(
            div()
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child(
                    "Enter to ".to_string()
                        + if new { "create" } else { "rename" }
                        + ", Escape to cancel.",
                ),
        )
}

/// The file in the editor: a header with Save, line numbers, and the text.
fn editor_view(dir: &str, view: &ProjectView, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let Some((rel, input)) = view.editor.clone() else {
        return div();
    };
    let lines = input.read(cx).line_count();
    let dirty = view.dirty;
    let save = Button::new("editor.save", "Save")
        .icon("check")
        .kind(ButtonKind::Primary)
        .disabled(!dirty)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_editor(cx)));
    let close = Button::new(
        "editor.close",
        if dirty { "Discard changes" } else { "Close" },
    )
    .kind(ButtonKind::Ghost)
    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
        this.project_ui.editor = None;
        this.project_ui.dirty = false;
        cx.notify();
    }));
    let head = div()
        .px(px(14.0))
        .h(px(44.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .border_b_1()
        .border_color(t.line)
        .child(ui::icon("file", 14.0, t.faint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .child(ui::Clip::tail(rel.clone())),
        )
        .child(pill(
            if dirty { "Unsaved changes" } else { "Saved" },
            if dirty { Tone::Warn } else { Tone::Ok },
            cx,
        ))
        .child(close)
        .child(save);
    let mut gutter = div()
        .flex()
        .flex_col()
        .items_end()
        .flex_none()
        .pl(px(10.0))
        .pr(px(10.0))
        .py(px(8.0))
        .border_r_1()
        .border_color(t.line)
        .text_color(t.faint);
    for n in 1..=lines {
        gutter = gutter.child(div().h(px(18.0)).child(n.to_string()));
    }
    let text = div()
        .flex_1()
        .py(px(8.0))
        .px(px(10.0))
        .text_color(t.text)
        .child(input);
    let body = div()
        .id("editor.scroll")
        .track_scroll(&view.scroll)
        .max_h(px(560.0))
        .overflow_scroll()
        .font_family(mono())
        .text_size(px(13.0))
        .line_height(px(18.0))
        .child(div().flex().child(gutter).child(text));
    let hint = div()
        .px(px(14.0))
        .h(px(30.0))
        .flex()
        .items_center()
        .gap(px(14.0))
        .border_t_1()
        .border_color(t.line)
        .text_size(px(size::TEXT_XS))
        .text_color(t.faint)
        .child(format!("{lines} line{}", if lines == 1 { "" } else { "s" }))
        .child("Ctrl+S saves")
        .child("Tab indents, Shift+Tab outdents");
    let _ = dir;
    div().child(head).child(body).child(hint)
}

impl Shell {
    /// Save the editor's text to its file, with the line endings the file had.
    pub fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.project_ui.editor else {
            return;
        };
        let abs = join(&self.project_ui.dir, &editor.rel);
        let mut text = editor.input.read(cx).text().to_string();
        if editor.crlf {
            text = text.replace('\n', "\r\n");
        }
        let name = leaf(&editor.rel).to_string();
        crate::state::start_write(
            cx,
            surface::files::write(&abs, &text).mutation(),
            Some(format!("Saving {name}…")),
            None,
        );
    }

    /// Ask for a name: for a new file, or for a file's new path.
    fn start_naming(
        &mut self,
        kind: NamingKind,
        initial: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let placeholder = match kind {
            NamingKind::NewFile => "src/new-file.rs",
            NamingKind::Rename(_) => "src/renamed.rs",
        };
        let input = cx.new(|cx| TextInput::new(placeholder, cx));
        if !initial.is_empty() {
            input.update(cx, |i, cx| i.set_text(initial, cx));
        }
        let watch = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _window, cx| match event {
                InputEvent::Submitted => this.finish_naming(cx),
                InputEvent::Cancelled => {
                    this.project_ui.naming = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        window.focus(&gpui::Focusable::focus_handle(&input, cx));
        self.project_ui.naming = Some(Naming {
            kind,
            input,
            _watch: watch,
        });
        cx.notify();
    }

    /// Carry out the name that was typed.
    fn finish_naming(&mut self, cx: &mut Context<Self>) {
        let Some(naming) = self.project_ui.naming.take() else {
            return;
        };
        let typed = naming.input.read(cx).text().trim().replace('\\', "/");
        let bad = typed.is_empty()
            || typed.starts_with('/')
            || typed.contains(':')
            || typed.split('/').any(|p| p == ".." || p.is_empty());
        if bad {
            cx.global_mut::<crate::state::Devd>()
                .toasts
                .push(crate::state::Toast::new(
                    "Give a path inside the project, such as src/main.rs.",
                    true,
                ));
            self.project_ui.naming = Some(naming);
            return cx.notify();
        }
        let dir = self.project_ui.dir.clone();
        let target = join(&dir, &typed);
        match naming.kind {
            NamingKind::NewFile => {
                crate::state::start_write(
                    cx,
                    surface::files::create(&target).mutation(),
                    Some(format!("Creating {typed}…")),
                    None,
                );
                // Open the folders on the way, and the file itself.
                let mut path = String::new();
                for part in typed.split('/').take(typed.split('/').count() - 1) {
                    path = if path.is_empty() {
                        part.to_string()
                    } else {
                        format!("{path}/{part}")
                    };
                    self.project_ui.open.insert(path.clone());
                }
                self.project_ui.editor = None;
                self.project_ui.file = Some(typed);
            }
            NamingKind::Rename(from) => {
                crate::state::start_write(
                    cx,
                    surface::files::rename(&join(&dir, &from), &target).mutation(),
                    Some(format!("Renaming {}…", leaf(&from))),
                    None,
                );
                self.project_ui.editor = None;
                self.project_ui.file = Some(typed);
            }
        }
        cx.notify();
    }
}

/// Open a file for editing once its exact text has arrived, and keep note of whether it has been changed.
fn refresh_editor(shell: &mut Shell, dir: &str, cx: &mut Context<Shell>) {
    if let Some(rel) = shell.project_ui.edit_request.clone() {
        let snap = read_call(cx, &surface::files::read_exact(&join(dir, &rel)));
        if let Some(data) = snap.data.as_deref() {
            shell.project_ui.edit_request = None;
            let eol = data["eol"].as_str().unwrap_or("lf");
            if eol == "mixed" {
                shell.project_ui.edit_error = Some("This file mixes Windows and Unix line endings. Saving it here would change some of them, so open it in your own editor, or ask the assistant to make the endings the same first.".to_string());
            } else {
                let text = data["text"]
                    .as_str()
                    .unwrap_or_default()
                    .replace("\r\n", "\n");
                let handle = shell.project_ui.scroll.clone();
                let input = cx.new(|cx| TextInput::new("", cx).code(handle));
                input.update(cx, |i, cx| i.load(text, cx));
                let watch = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Saved) {
                        this.save_editor(cx);
                    }
                });
                shell.project_ui.editor = Some(Editor {
                    rel,
                    input,
                    crlf: eol == "crlf",
                    _watch: watch,
                });
                shell.project_ui.edit_error = None;
            }
        } else if let Some(failure) = &snap.error {
            shell.project_ui.edit_request = None;
            shell.project_ui.edit_error = Some(failure.message.clone());
        }
    }
    shell.project_ui.dirty = match &shell.project_ui.editor {
        Some(editor) => {
            let disk = read_call(cx, &surface::files::read_exact(&join(dir, &editor.rel)));
            let disk = disk
                .data
                .as_deref()
                .and_then(|d| d["text"].as_str())
                .map(|t| t.replace("\r\n", "\n"));
            disk.is_some_and(|d| d != editor.input.read(cx).text())
        }
        None => false,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_joined_and_cut_the_way_a_tree_needs() {
        assert_eq!(
            join(r"D:\www\shop", "src/main.rs"),
            r"D:\www\shop/src/main.rs"
        );
        assert_eq!(join("D:/www/shop/", "a"), "D:/www/shop/a");
        assert_eq!(leaf(r"D:\www\shop\"), "shop");
        assert_eq!(
            parent("crates/devd-agent/src/lib.rs"),
            "crates/devd-agent/src"
        );
        assert_eq!(parent("Cargo.toml"), "");
    }

    #[test]
    fn a_unix_time_is_read_as_the_date_it_is() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_794_538_034), "2026-11-13T02:47:14Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z", "a leap day");
        assert_eq!(iso(-86_400), "1969-12-31T00:00:00Z");
    }

    #[test]
    fn a_status_letter_is_a_tone_and_a_mark() {
        assert_eq!(change_word("M"), (Tone::Warn, "M".to_string()));
        assert_eq!(change_word("A").1, "A");
        assert_eq!(change_word("D").0, Tone::Danger);
        assert_eq!(
            change_word("?").1,
            "N",
            "an untracked file is marked as new"
        );
    }

    #[test]
    fn the_assistant_is_told_which_file_change_or_commit_is_picked() {
        let dir = "D:/work/shop".to_string();
        let mut ui = ProjectUi {
            dir: dir.clone(),
            ..ProjectUi::default()
        };
        // Nothing picked: the project, and no more.
        let files = Route::project(&dir, ProjectTab::Files);
        assert_eq!(ui.looking_at(&files), (Some(dir.clone()), None, None));
        ui.file = Some("src/main.rs".into());
        let (project, file, detail) = ui.looking_at(&files);
        assert_eq!(
            (project.as_deref(), file.as_deref()),
            (Some("D:/work/shop"), Some("D:/work/shop/src/main.rs"))
        );
        assert!(
            detail
                .unwrap()
                .contains("reading the file src/main.rs in the project shop")
        );
        ui.change = Some(("a.rs".into(), true));
        let (_, file, detail) = ui.looking_at(&Route::project(&dir, ProjectTab::Changes));
        assert_eq!(file.as_deref(), Some("D:/work/shop/a.rs"));
        assert!(detail.unwrap().contains("staged changes to a.rs"));
        ui.commit = Some("0123456789abcdef".into());
        assert!(
            ui.looking_at(&Route::project(&dir, ProjectTab::History))
                .2
                .unwrap()
                .contains("commit 0123456 ")
        );
        // The state of another project is never reported as this one's, and a screen that is not a project says nothing.
        assert_eq!(
            ui.looking_at(&Route::project("D:/work/blog", ProjectTab::Files))
                .1,
            None
        );
        assert_eq!(ui.looking_at(&Route::Home), (None, None, None));
    }
}
