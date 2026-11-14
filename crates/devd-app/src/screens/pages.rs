//! The pages behind the rail's screens: a site's hosts, certificates and mail; the server's runtimes, databases and
//! tools; the team's schedules, skills and outside tools; and the settings pages.
//!
//! Every page is the same four things, so they look and behave alike:
//!
//! 1. **A head** with what the page is and what it is for in one line, and the page's own actions at its right.
//! 2. **The area's tabs**, so any page in an area is one click from any other.
//! 3. **Lists** drawn by one function: a state dot, a title, a line of detail, the state in words, and the actions
//!    that apply to that row. A row that is loading, empty or failed says so in words, never as a blank.
//! 4. **Dialogs** for what needs asking: anything that removes something confirms first and says it cannot be undone
//!    when that is true; anything that needs a name asks for it.
//!
//! Nothing here decides anything. A button runs a capability through the same path as every other button, so a
//! capability that needs the person's approval still asks for it, and what it did is in Activity.

use devd_app_core::nav::Route;
use devd_app_core::pages as table;
use devd_app_core::sync::{Mutation, QueryKey, Snapshot};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, FontWeight, SharedString, Window, div,
    prelude::*, px,
};
use serde_json::{Value, json};

use crate::screens::modal::{Detail, Field, FormSpec};
use crate::shell::Shell;
use crate::state::devd;
use crate::theme::{Tone, mono, size, theme};
use crate::ui::{self, Button, ButtonKind, card, eyebrow, icon, page_head, pill, rule};

// ------------------------------------------------------------------------------------------- actions

/// What a button does when it is pressed.
#[derive(Clone)]
pub struct Act {
    /// What it says.
    pub label: &'static str,
    /// A small picture before the words.
    pub icon: Option<&'static str>,
    /// The capability it runs.
    pub cap: &'static str,
    /// Its arguments.
    pub args: Value,
    /// Whether a confirmation comes first, and what it says.
    pub ask: Option<Ask>,
    /// What to say while it runs, when it can take long enough to notice.
    pub busy: Option<String>,
}

/// A confirmation before an act that removes or replaces something.
#[derive(Clone)]
pub struct Ask {
    /// The dialog's heading.
    pub title: String,
    /// What will happen, and that it cannot be undone when that is true.
    pub body: String,
    /// The button, naming the act.
    pub verb: &'static str,
}

impl Act {
    /// A button that runs `cap` at once.
    pub fn new(label: &'static str, cap: &'static str, args: Value) -> Self {
        Self {
            label,
            icon: None,
            cap,
            args,
            ask: None,
            busy: None,
        }
    }

    /// Say what it is doing while it runs.
    pub fn busy(mut self, text: impl Into<String>) -> Self {
        self.busy = Some(text.into());
        self
    }

    /// With a small picture.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Ask first.
    pub fn ask(
        mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        verb: &'static str,
    ) -> Self {
        self.ask = Some(Ask {
            title: title.into(),
            body: body.into(),
            verb,
        });
        self
    }
}

/// The part of what devd shows that a capability changes, so the right lists are read again.
pub fn domain_of(capability: &str) -> &'static str {
    match capability.split('.').next().unwrap_or("") {
        "runtime" => "runtime",
        "db" => "db",
        "backup" => "backup",
        "tls" => "tls",
        "schedule" => "schedule",
        "cron" => "cron",
        "automation" => "automation",
        "skills" => "skills",
        "mcp" => "mcp",
        "share" => "share",
        "hosts" => "hosts",
        "mail" => "mail",
        "memory" => "memory",
        "agent_client" => "agent_client",
        "flags" => "flags",
        "toolchain" => "toolchain",
        "autostart" => "autostart",
        "update" => "update",
        "project" => "project",
        "service" => "service",
        "git" => "git",
        "fs" => "fs",
        "terminal" => "terminal",
        "todo" => "todo",
        "review" => "review",
        "stacks" | "stack" => "stacks",
        _ => "site",
    }
}

impl Shell {
    /// Run what a button stands for, asking first when it removes something.
    pub fn run_act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let domain = domain_of(act.cap);
        let mut mutation = Mutation::new(act.cap, act.args).invalidating(domain);
        if matches!(act.cap, "project.run" | "project.run_destructive") {
            // Starting a background task changes the list of what is running.
            mutation = mutation.invalidating("task");
        }
        match act.ask {
            Some(ask) => self.confirm(ask.title, ask.body, ask.verb, true, mutation, window, cx),
            None => {
                crate::state::start_write(cx, mutation, act.busy, None);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------- rows

/// One line of a list.
#[derive(Default)]
pub struct Row {
    /// What it is.
    pub title: String,
    /// A short fact beside the title, in the code font: a version, an address.
    pub tag: String,
    /// A line under it.
    pub sub: String,
    /// Whether `sub` is a place on disk, told apart by its end.
    pub sub_is_path: bool,
    /// How it is, in words, and the colour of those words.
    pub state: Option<(Tone, String)>,
    /// A number or a time to the right.
    pub side: String,
    /// What can be done to it.
    pub acts: Vec<Act>,
    /// The id of the tool, runtime, service or stack the row is about, whose own mark is drawn before its name.
    pub mark: Option<String>,
    /// A form this row opens, shown as a button.
    pub form: Option<FormSpec>,
    /// What that button says. Empty means "Start one".
    pub form_label: &'static str,
    /// Reading it in full, when there is more than a line.
    pub open: Option<(Detail, QueryKey, String)>,
}

/// The rows as one card, or nothing when there are none.
fn rows_card(rows: Vec<Row>, key: &'static str, cx: &mut Context<Shell>) -> Div {
    let t = theme(cx).clone();
    let mut list = card(cx).flex().flex_col().overflow_hidden();
    for (n, row) in rows.into_iter().enumerate() {
        if n > 0 {
            list = list.child(rule(cx));
        }
        let tone = row.state.as_ref().map(|s| s.0).unwrap_or(Tone::Neutral);
        let name = div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .font_weight(FontWeight::MEDIUM)
            .child(row.title.clone());
        let mut title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w_0()
            .child(name);
        if !row.tag.is_empty() {
            title = title.child(
                div()
                    .flex_none()
                    .font_family(mono())
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(row.tag.clone()),
            );
        }
        let sub = if row.sub_is_path {
            ui::Clip::tail(row.sub.clone()).into_any_element()
        } else {
            ui::Clip::new(row.sub.clone()).into_any_element()
        };
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(title)
            .child(
                div()
                    .min_w_0()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(sub),
            );
        let hover = t.hover;
        let mut line = div()
            .id(ui::nid(key, n))
            .px(px(14.0))
            .min_h(px(52.0))
            .py(px(7.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .hover(move |s| s.bg(hover))
            .child(ui::dot(tone, cx));
        if let Some(mark) = &row.mark {
            line = line.child(ui::tool_mark(mark, &row.title, 28.0, cx));
        }
        let mut line = line.child(text);
        if let Some((kind, query, heading)) = row.open.clone() {
            line = line.cursor_pointer().on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    this.open_detail(kind, query.clone(), heading.clone(), window, cx)
                },
            ));
        }
        if let Some((tone, word)) = &row.state {
            line = line.child(
                div()
                    .flex_none()
                    .text_size(px(size::TEXT_SM))
                    .text_color(t.tone(*tone))
                    .child(word.clone()),
            );
        }
        if !row.side.is_empty() {
            line = line.child(
                div()
                    .flex_none()
                    .min_w(px(64.0))
                    .flex()
                    .justify_end()
                    .font_family(mono())
                    .text_size(px(size::TEXT_SM))
                    .text_color(t.muted)
                    .child(row.side.clone()),
            );
        }
        if let Some(spec) = row.form.clone() {
            let label = if row.form_label.is_empty() {
                "Start one"
            } else {
                row.form_label
            };
            line = line.child(
                Button::new(ui::nid("row.form", n), label)
                    .icon("plus")
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_form(spec.clone(), window, cx)
                    })),
            );
        }
        for (a, act) in row.acts.into_iter().enumerate() {
            let id = ElementId::NamedInteger(format!("{key}.{n}.act").into(), a as u64);
            line = line.child(act_button(id, act, ButtonKind::Secondary, cx));
        }
        if row.open.is_some() {
            // A row that opens says so, instead of relying on the pointer alone.
            line = line.child(icon("chevron-right", 13.0, t.faint));
        }
        list = list.child(line);
    }
    list
}

/// A button for an act.
pub(super) fn act_button(
    id: ElementId,
    act: Act,
    kind: ButtonKind,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let mut button = Button::new(id, act.label).kind(kind);
    if let Some(name) = act.icon {
        button = button.icon(name);
    }
    button
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.run_act(act.clone(), window, cx)
        }))
        .into_any_element()
}

/// A button that opens a form.
pub(super) fn form_button(
    id: &'static str,
    label: &'static str,
    name: &'static str,
    spec: FormSpec,
    kind: ButtonKind,
    cx: &mut Context<Shell>,
) -> AnyElement {
    Button::new(id, label)
        .icon(name)
        .kind(kind)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.open_form(spec.clone(), window, cx)
        }))
        .into_any_element()
}

/// What to show instead of a list that has not arrived, failed, or has nothing in it.
fn waiting(
    snap: &Snapshot,
    rows: usize,
    icon_name: &'static str,
    title: &'static str,
    detail: &'static str,
    cx: &App,
) -> Option<Div> {
    if snap.is_first_load() {
        let mut column = div().flex().flex_col().gap(px(18.0)).py(px(12.0));
        for n in 0..5 {
            column = column.child(ui::skeleton(
                ui::nid("pages.skeleton", n),
                260.0 + ((n * 67) % 180) as f32,
                cx,
            ));
        }
        return Some(column);
    }
    if let (Some(failure), true) = (&snap.error, snap.data.is_none()) {
        return Some(ui::empty(
            "alert",
            "devd could not answer",
            devd_app_core::status::failure(&failure.message),
            cx,
        ));
    }
    (rows == 0).then(|| ui::empty(icon_name, title, detail, cx))
}

/// A titled section.
pub(super) fn section(title: impl Into<SharedString>, body: Div, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(eyebrow(title, cx))
        .child(body)
}

/// A list's card, or its waiting or empty state.
pub(super) fn list_or(
    snap: &Snapshot,
    rows: Vec<Row>,
    key: &'static str,
    icon_name: &'static str,
    title: &'static str,
    detail: &'static str,
    cx: &mut Context<Shell>,
) -> Div {
    match waiting(snap, rows.len(), icon_name, title, detail, cx) {
        Some(state) => card(cx).child(div().px(px(16.0)).child(state)),
        None => rows_card(rows, key, cx),
    }
}

/// When a screen cannot read because the service behind it is down, say that once, in words, with the one thing to do:
/// start it, or install it. A client's own error ("Can't connect to server on 127.0.0.1 (10061)") is not shown twice.
fn service_down(
    snaps: &[&Snapshot],
    service: &'static str,
    label: &'static str,
    icon_name: &'static str,
    purpose: &'static str,
    port: u64,
    cx: &mut Context<Shell>,
) -> Option<AnyElement> {
    let down = snaps.iter().any(|snap| {
        snap.data.is_none()
            && snap.error.as_ref().is_some_and(|f| {
                let m = f.message.to_lowercase();
                ["can't connect", "2002", "10061", "not running", "connection refused", "actively refused", "not installed", "connection failed"]
                    .iter()
                    .any(|w| m.contains(w))
            })
    });
    if !down {
        return None;
    }
    let services = devd(cx).sync.watch(&crate::state::q::services());
    let row = services.list("").iter().find(|s| s["name"] == json!(service)).cloned();
    let installed = row.as_ref().is_some_and(|s| s["installed"] == json!(true));
    let answering = row.as_ref().is_some_and(|s| s["ready"] == json!(true));
    let running = answering || row.as_ref().is_some_and(|s| s["running"] == json!(true));
    // It answers, yet it cannot be read: that is not a service to start, so say what devd ran into.
    if answering {
        let why = snaps
            .iter()
            .find_map(|s| s.error.as_ref())
            .map(|f| devd_app_core::status::failure(&f.message))
            .unwrap_or_default();
        let state = ui::empty(icon_name, format!("devd cannot read {label}"), format!("It is answering on port {port}, but: {why}"), cx);
        return Some(card(cx).child(div().px(px(16.0)).child(state)).into_any_element());
    }
    let (title, said) = match (installed, running) {
        (false, _) => (format!("{label} is not installed"), format!("Install it from the Server screen {purpose}.")),
        (true, false) => (format!("{label} is not running"), format!("Start it {purpose}.")),
        (true, true) => (
            format!("{label} is not answering yet"),
            format!("It is running but not accepting connections on port {port}. If it has just started, give it a moment; otherwise restart it from the Server screen."),
        ),
    };
    let mut state = ui::empty(icon_name, title, said, cx);
    if installed && !running {
        state = state.child(
            div().pt(px(6.0)).child(
                Button::new(ui::nid("service.down.start", port as usize), format!("Start {}", if service == "mailpit" { "it" } else { label }))
                    .icon("play")
                    .kind(ButtonKind::Primary)
                    .on_click(move |_, _, cx| {
                        devd(cx).sync.mutate(
                            Mutation::new("service.start", json!({ "name": service })).invalidating("db").invalidating("mail").invalidating("service"),
                        );
                    }),
            ),
        );
    } else {
        state = state.child(
            div().pt(px(6.0)).child(
                Button::new(ui::nid("service.down.open", port as usize), "Open Server")
                    .kind(ButtonKind::Secondary)
                    .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.go(Route::Server, cx))),
            ),
        );
    }
    Some(card(cx).child(div().px(px(16.0)).child(state)).into_any_element())
}

fn read(cx: &App, capability: &str) -> Snapshot {
    devd(cx).sync.watch(&QueryKey::bare(capability))
}

fn read_with(cx: &App, capability: &str, args: Value) -> Snapshot {
    devd(cx).sync.watch(&QueryKey::new(capability, args))
}

/// A call, read through the sync engine.
pub(super) fn read_call(cx: &App, call: &devd_app_core::surface::Call) -> Snapshot {
    devd(cx).sync.watch(&call.query())
}

pub(super) fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

// --------------------------------------------------------------------------------------------- frame

/// The row of tabs every screen of an area carries. `None` for a screen with no pages behind it.
pub fn tab_strip(current: &Route, cx: &mut Context<Shell>) -> Option<Div> {
    let area = table::area_of(current)?;
    let t = theme(cx).clone();
    let mut strip = div()
        .flex()
        .items_end()
        .gap(px(2.0))
        .mb(px(18.0))
        .border_b_1()
        .border_color(t.line);
    for (n, tab) in table::tabs(area).into_iter().enumerate() {
        let on = table::is_current(&tab, current);
        let route = tab.route.clone();
        let hover = t.text;
        let mut el = div()
            .id(ui::nid("tab", n))
            .px(px(12.0))
            .h(px(34.0))
            .flex()
            .items_center()
            .text_size(px(size::TEXT))
            .cursor_pointer()
            .border_b_2()
            .child(tab.label);
        el = if on {
            el.text_color(t.text)
                .font_weight(FontWeight::MEDIUM)
                .border_color(t.accent)
        } else {
            el.text_color(t.muted)
                .border_color(gpui::transparent_black())
                .hover(move |s| s.text_color(hover))
        };
        strip = strip.child(
            el.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(route.clone(), cx))),
        );
    }
    Some(strip)
}

fn frame(
    id: &'static str,
    detail: impl Into<SharedString>,
    actions: Div,
    snaps: &[&Snapshot],
    body: Vec<AnyElement>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let page = table::find(id).map(|p| p.title).unwrap_or("Page");
    frame_at(
        id,
        page,
        Route::Page(id.to_string()),
        detail,
        actions,
        snaps,
        body,
        cx,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn frame_at(
    id: &'static str,
    page: &'static str,
    route: Route,
    detail: impl Into<SharedString>,
    actions: Div,
    snaps: &[&Snapshot],
    body: Vec<AnyElement>,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let mut column = div().flex().flex_col().gap(px(22.0)).child(
        div()
            .child(page_head(page, detail, actions, cx))
            .children(tab_strip(&route, cx)),
    );
    for snap in snaps {
        column = column.children(ui::stale(snap, cx));
    }
    for part in body {
        column = column.child(part);
    }
    ui::page(id, 980.0, column).into_any_element()
}

/// Draw the page with this id.
pub fn render(_shell: &mut Shell, id: &str, cx: &mut Context<Shell>) -> AnyElement {
    match id {
        "runtimes" => runtimes(cx),
        "databases" => databases(cx),
        "tools" => tools(cx),
        "remote" => remote(cx),
        "hosts" => hosts(cx),
        "certificates" => certificates(cx),
        "mail" => mail(cx),
        "schedules" => schedules(cx),
        "automations" => automations(cx),
        "skills" => skills(cx),
        "mcp" => mcp(cx),
        "notes" => notes(cx),
        "new-project" => new_project(cx),
        "stacks" => stacks(cx),
        "diagnostics" => diagnostics(cx),
        "capabilities" => capabilities(cx),
        "connect" => connect(cx),
        "flags" => flags(cx),
        _ => ui::page(
            "page",
            980.0,
            ui::empty(
                "alert",
                "There is no such page",
                "It may have been renamed. Use the tabs above, or Ctrl+K to search.",
                cx,
            ),
        )
        .into_any_element(),
    }
}

// -------------------------------------------------------------------------------------------- server

fn kind_name(kind: &str) -> String {
    match kind {
        "php" => "PHP".to_string(),
        "node" | "nodejs" => "Node.js".to_string(),
        "go" => "Go".to_string(),
        "composer" => "Composer".to_string(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn runtimes(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "runtime.list");
    let mut rows = Vec::new();
    for item in snap.list("") {
        let r = &item["runtime"];
        let (kind, version) = (text(r, "kind"), text(r, "version"));
        let primary = item["primary"] == json!(true);
        let present = item["present"] != json!(false);
        let managed = item["managed"] == json!(true);
        let name = format!("{} {}", kind_name(&kind), version);
        let mut acts = Vec::new();
        // Only PHP has a "default version" that sites inherit; Node.js and Go
        // are chosen per project, so offering it for them would write a PHP
        // setting with a Node version in it.
        if present && !primary && kind == "php" {
            acts.push(Act::new(
                "Use this one",
                "runtime.use",
                json!({ "version": version }),
            ));
        }
        if managed {
            acts.push(Act::new("Remove", "runtime.remove", json!({ "kind": kind, "version": version })).ask(
                format!("Remove {name}?"),
                format!("devd deletes its own copy of {name}. Sites that use it will need another version. This cannot be undone, but you can install it again."),
                "Remove runtime",
            ));
        }
        rows.push(Row {
            mark: Some(kind.clone()),
            title: name,
            sub: item["executable"]
                .as_str()
                .unwrap_or(r["path"].as_str().unwrap_or_default())
                .to_string(),
            sub_is_path: true,
            state: Some(match (primary, present) {
                (true, _) => (Tone::Ok, "in use".to_string()),
                (false, true) => (Tone::Neutral, "installed".to_string()),
                (false, false) => (Tone::Warn, "missing".to_string()),
            }),
            side: if managed {
                "managed by devd".to_string()
            } else {
                "from this machine".to_string()
            },
            acts,
            ..Row::default()
        });
    }
    let detail = if snap.is_first_load() {
        "Reading what is installed…".to_string()
    } else {
        format!("{} installed on this machine.", rows.len())
    };
    let actions = div().child(form_button(
        "runtimes.install",
        "Install a runtime",
        "plus",
        FormSpec {
            title: "Install a runtime",
            intro: "devd downloads it from its publisher and checks it before it is used. PHP, Node.js and Go are supported.",
            verb: "Install",
            capability: "runtime.install",
            domain: "runtime",
            fixed: json!({}),
            fields: vec![Field::text("kind", "Runtime", "php, node or go"), Field::text("version", "Version", "8.4")],
            busy: Some("Installing {name}…"),
        },
        ButtonKind::Primary,
        cx,
    ));
    let body = vec![
        list_or(
            &snap,
            rows,
            "runtimes.row",
            "machine",
            "No runtimes yet",
            "Install PHP, Node.js or Go and it is listed here, with the one your sites use.",
            cx,
        )
        .into_any_element(),
    ];
    frame("runtimes", detail, actions, &[&snap], body, cx)
}

fn databases(cx: &mut Context<Shell>) -> AnyElement {
    let dbs = read(cx, "db.list");
    let users = read(cx, "db.users");
    let backups = read(cx, "backup.list");
    let port = dbs
        .data
        .as_deref()
        .and_then(|d| d["port"].as_u64())
        .unwrap_or(3306);
    let mut rows = Vec::new();
    for name in dbs
        .data
        .as_deref()
        .and_then(|d| d["databases"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        let name = name.as_str().unwrap_or_default().to_string();
        rows.push(Row {
            mark: Some("mariadb".into()),
            title: name.clone(),
            sub: format!("MariaDB, port {port}"),
            side: String::new(),
            acts: vec![
                Act::new("Back up", "db.dump", json!({ "database": name })).icon("database").busy(format!("Writing a backup of {name}…")),
                Act::new("Remove", "db.drop", json!({ "name": name })).ask(
                    format!("Remove the database {name}?"),
                    format!("Every table and row in {name} is deleted. devd takes a snapshot first unless you turn that off, but treat this as permanent."),
                    "Remove database",
                ),
            ],
            ..Row::default()
        });
    }
    let user_rows: Vec<Row> = users
        .data
        .as_deref()
        .and_then(|d| d["users"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|u| Row {
            title: text(u, "user"),
            sub: if text(u, "host").is_empty() {
                "any host".to_string()
            } else {
                format!("from {}", text(u, "host"))
            },
            ..Row::default()
        })
        .collect();
    let schedule_rows: Vec<Row> = backups
        .data
        .as_deref()
        .and_then(|d| d["schedules"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|b| {
            let id = text(b, "id");
            Row {
                title: text(b, "database"),
                sub: format!("{}, keeps the last {}", text(b, "schedule"), b["keep"].as_u64().unwrap_or(7)),
                state: Some(if b["enabled"] == json!(false) { (Tone::Neutral, "paused".to_string()) } else { (Tone::Ok, "scheduled".to_string()) }),
                acts: vec![
                    Act::new("Back up now", "backup.run", json!({ "id": id })).busy("Taking a backup…"),
                    Act::new("Remove", "backup.remove", json!({ "id": id })).ask("Stop this backup schedule?", "No more backups are taken on this schedule. Backups already taken are kept.", "Remove schedule"),
                ],
                ..Row::default()
            }
        })
        .collect();
    let detail = if dbs.is_first_load() {
        "Reading the databases…".to_string()
    } else if dbs.error.is_some() && dbs.data.is_none() {
        "The database server is not available.".to_string()
    } else {
        format!(
            "{} on MariaDB, port {port}.",
            if rows.len() == 1 {
                "1 database".to_string()
            } else {
                format!("{} databases", rows.len())
            }
        )
    };
    let actions = div().flex().gap(px(8.0)).child(form_button(
        "databases.create",
        "Create a database",
        "plus",
        FormSpec { title: "Create a database", intro: "A new, empty MariaDB database. Give it a name with letters, numbers and underscores.", verb: "Create database", capability: "db.create", domain: "db", fixed: json!({}), fields: vec![Field::text("name", "Name", "shop")], busy: Some("Creating {name}…") },
        ButtonKind::Primary,
        cx,
    ));
    let dbs_list = list_or(
        &dbs,
        rows,
        "databases.row",
        "database",
        "No databases",
        "Create one here, or let a site create its own when you add it.",
        cx,
    );
    let users_list = list_or(
        &users,
        user_rows,
        "databases.user",
        "lock",
        "No database users",
        "A site's own user is made when its database is.",
        cx,
    );
    let backups_list = list_or(
        &backups,
        schedule_rows,
        "databases.backup",
        "database",
        "No backups scheduled",
        "Schedule backups and the copies are listed here.",
        cx,
    );
    let mut body = if let Some(down) = service_down(&[&dbs, &users], "mariadb", "MariaDB", "database", "to see its databases and users, create new ones and take backups", port, cx) {
        vec![down]
    } else {
        vec![
            section("DATABASES", dbs_list, cx).into_any_element(),
            section("USERS", users_list, cx).into_any_element(),
        ]
    };
    let add_backup = form_button(
        "databases.schedule",
        "Schedule backups",
        "plus",
        FormSpec {
            title: "Schedule backups",
            intro: "devd dumps the database on a timetable and keeps the most recent copies. The schedule is a cron time such as 0 3 * * * for every night at three.",
            verb: "Schedule backups",
            capability: "backup.add",
            domain: "backup",
            fixed: json!({}),
            fields: vec![
                Field::text("database", "Database", "shop"),
                Field::text("schedule", "When", "0 3 * * *"),
                Field::number("keep", "Copies to keep", "7"),
            ],
            busy: None,
        },
        ButtonKind::Secondary,
        cx,
    );
    body.push(
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(eyebrow("BACKUPS", cx))
                    .child(add_backup),
            )
            .child(backups_list)
            .into_any_element(),
    );
    frame("databases", detail, actions, &[&dbs], body, cx)
}

fn tools(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "toolchain.list");
    let mut rows = Vec::new();
    let tools = snap
        .data
        .as_deref()
        .and_then(|d| d["tools"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let installed = tools
        .iter()
        .filter(|x| x["status"]["installed"] == json!(true))
        .count();
    for entry in tools {
        let (tool, status) = (&entry["tool"], &entry["status"]);
        let id = text(tool, "id");
        let have = status["installed"] == json!(true);
        let name = text(tool, "name");
        let mut acts = Vec::new();
        if have {
            acts.push(Act::new("Remove", "toolchain.remove", json!({ "tool": id })).ask(
                format!("Remove {name}?"),
                format!("devd asks your package manager to remove {name}. Anything that needs it stops working until it is installed again."),
                "Remove tool",
            ));
        } else {
            acts.push(Act::new("Install", "toolchain.install", json!({ "tool": id })).ask(
                format!("Install {name}?"),
                format!("devd asks your package manager to install {name}. {} Your system may ask for your permission.", text(tool, "purpose")),
                "Install tool",
            ));
        }
        rows.push(Row {
            mark: Some(id.clone()),
            title: name,
            tag: text(status, "version"),
            sub: text(tool, "purpose"),
            state: Some(if status["outdated"] == json!(true) {
                (Tone::Warn, "out of date".to_string())
            } else if have {
                (Tone::Ok, "installed".to_string())
            } else {
                (Tone::Neutral, "not installed".to_string())
            }),
            acts,
            ..Row::default()
        });
    }
    let detail = if snap.is_first_load() {
        "Looking for your tools…".to_string()
    } else {
        format!("{installed} of {} installed.", tools.len())
    };
    let body = vec![
        list_or(
            &snap,
            rows,
            "tools.row",
            "wrench",
            "No tools listed",
            "devd lists the tools it knows how to install here.",
            cx,
        )
        .into_any_element(),
    ];
    frame("tools", detail, div(), &[&snap], body, cx)
}

fn remote(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "share.list");
    let shares = snap
        .data
        .as_deref()
        .and_then(|d| d["shares"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let rows: Vec<Row> = shares
        .iter()
        .map(|s| {
            let name = text(s, "site");
            Row {
                title: name.clone(),
                sub: s["url"]
                    .as_str()
                    .or(s["public_url"].as_str())
                    .unwrap_or("Waiting for a public link…")
                    .to_string(),
                state: Some((Tone::Ok, "shared".to_string())),
                side: s["expires_at"]
                    .as_str()
                    .map(|e| format!("closes in {}", ui::until(e)))
                    .unwrap_or_default(),
                acts: vec![Act::new(
                    "Stop sharing",
                    "share.stop",
                    json!({ "name": text(s, "site") }),
                )],
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Reading what is shared…".to_string()
    } else if rows.is_empty() {
        "Nothing is shared with the internet.".to_string()
    } else if rows.len() == 1 {
        "1 site is shared with the internet right now.".to_string()
    } else {
        format!(
            "{} sites are shared with the internet right now.",
            rows.len()
        )
    };
    let actions = div().child(form_button(
        "remote.share",
        "Share a site",
        "external",
        FormSpec {
            title: "Share a site",
            intro: "Anyone with the public link can open the site while it is shared. It stops by itself after the time you give, and you can stop it sooner.",
            verb: "Share site",
            capability: "share.start",
            domain: "share",
            fixed: json!({}),
            fields: vec![Field::text("name", "Site", "shop"), Field::number("minutes", "Minutes to share for", "60")],
            busy: None,
        },
        ButtonKind::Primary,
        cx,
    ));
    let body = vec![
        list_or(
            &snap,
            rows,
            "remote.row",
            "external",
            "Nothing is shared",
            "Share a site to give someone a public link for a while.",
            cx,
        )
        .into_any_element(),
    ];
    frame("remote", detail, actions, &[&snap], body, cx)
}

// --------------------------------------------------------------------------------------------- sites

fn hosts(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "hosts.show");
    let data = snap.data.as_deref();
    let entries = data
        .and_then(|d| d["entries"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let adding = data
        .and_then(|d| d["adding"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let removing = data
        .and_then(|d| d["removing"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut rows: Vec<Row> = adding
        .iter()
        .map(|e| Row {
            title: text(e, "host"),
            sub: format!("will point to {}", text(e, "address")),
            state: Some((Tone::Warn, "not published".to_string())),
            ..Row::default()
        })
        .collect();
    rows.extend(removing.iter().map(|e| Row {
        title: text(e, "host"),
        sub: format!("points to {} now", text(e, "address")),
        state: Some((Tone::Warn, "will be removed".to_string())),
        ..Row::default()
    }));
    rows.extend(entries.iter().map(|e| Row {
        title: text(e, "host"),
        sub: format!("points to {}", text(e, "address")),
        state: Some((Tone::Ok, "published".to_string())),
        ..Row::default()
    }));
    let pending = adding.len() + removing.len();
    let file = data.map(|d| text(d, "file")).unwrap_or_default();
    let detail = if snap.is_first_load() {
        "Reading the hosts file…".to_string()
    } else if pending == 0 {
        "Every site name is in the hosts file.".to_string()
    } else if pending == 1 {
        "1 change is not written to the hosts file yet. Windows asks your permission once to write it.".to_string()
    } else {
        format!(
            "{pending} changes are not written to the hosts file yet. Windows asks your permission once to write them."
        )
    };
    let actions = div().child(
        Button::new("hosts.sync", "Publish site names")
            .icon("globe")
            .kind(if pending > 0 {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            })
            .disabled(pending == 0)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.run_act(
                    Act::new("Publish", "hosts.sync", json!({ "elevate": true })),
                    window,
                    cx,
                )
            })),
    );
    let t = theme(cx).clone();
    let mut body = vec![
        list_or(
            &snap,
            rows,
            "hosts.row",
            "globe",
            "No site names",
            "Add a site and its name appears here.",
            cx,
        )
        .into_any_element(),
    ];
    if !file.is_empty() {
        body.push(
            div()
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap(px(6.0))
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child("Hosts file")
                .child(div().font_family(mono()).text_color(t.muted).child(file))
                .into_any_element(),
        );
    }
    frame("hosts", detail, actions, &[&snap], body, cx)
}

fn certificates(cx: &mut Context<Shell>) -> AnyElement {
    let list = read(cx, "tls.list");
    let ca = read(cx, "tls.ca.status");
    let t = theme(cx).clone();
    let authority = ca.data.as_deref();
    let trusted =
        authority.is_some_and(|a| a["user"] == json!(true) || a["machine"] == json!(true));
    let exists = authority.is_some_and(|a| a["exists"] == json!(true));
    let (tone, word, sub) = match (exists, trusted) {
        (false, _) => (
            Tone::Neutral,
            "not created yet",
            "devd makes its certificate authority the first time a site needs HTTPS.",
        ),
        (true, true) => (
            Tone::Ok,
            "trusted",
            "Browsers on this machine accept every certificate devd issues.",
        ),
        (true, false) => (
            Tone::Warn,
            "not trusted",
            "Browsers will warn about every devd site until this machine trusts the authority.",
        ),
    };
    let ca_row = Row {
        title: "devd certificate authority".to_string(),
        sub: if let Some(p) = authority.and_then(|a| a["certificate"].as_str()) {
            format!("{sub} {p}")
        } else {
            sub.to_string()
        },
        state: Some((tone, word.to_string())),
        acts: if trusted {
            vec![Act::new("Stop trusting", "tls.untrust", json!({})).ask("Stop trusting devd's authority?", "Browsers on this machine will warn about every site devd serves over HTTPS. You can trust it again later. Windows asks your permission.", "Stop trusting")]
        } else if exists {
            vec![Act::new("Trust it", "tls.trust", json!({})).ask("Trust devd's certificate authority?", "This machine will accept every certificate devd issues, so local HTTPS sites open without a warning. Only devd holds the key. Windows asks your permission.", "Trust authority")]
        } else {
            vec![]
        },
        ..Row::default()
    };
    let rows: Vec<Row> = list
        .list("")
        .iter()
        .map(|c| {
            let host = text(c, "host");
            let ready = c["key_present"] != json!(false);
            Row {
                title: host.clone(),
                sub: text(c, "cert"),
                sub_is_path: true,
                state: Some(if ready {
                    (Tone::Ok, "ready".to_string())
                } else {
                    (Tone::Warn, "key is missing".to_string())
                }),
                acts: vec![
                    Act::new("Issue again", "tls.issue", json!({ "host": host }))
                        .busy(format!("Issuing a certificate for {host}…")),
                ],
                ..Row::default()
            }
        })
        .collect();
    let detail = if list.is_first_load() {
        "Reading the certificates…".to_string()
    } else {
        format!(
            "{} issued. {}",
            if rows.len() == 1 {
                "1 certificate".to_string()
            } else {
                format!("{} certificates", rows.len())
            },
            if trusted {
                "This machine trusts them."
            } else {
                "This machine does not trust them yet."
            }
        )
    };
    let actions = div().child(form_button(
        "certificates.issue",
        "Issue a certificate",
        "plus",
        FormSpec { title: "Issue a certificate", intro: "devd signs a certificate for this name with its own authority. Sites get theirs when HTTPS is turned on.", verb: "Issue certificate", capability: "tls.issue", domain: "tls", fixed: json!({}), fields: vec![Field::text("host", "Name", "shop.test")], busy: None },
        ButtonKind::Primary,
        cx,
    ));
    let _ = t;
    let authority_card = rows_card(vec![ca_row], "certificates.authority", cx);
    let certs_list = list_or(
        &list,
        rows,
        "certificates.row",
        "lock",
        "No certificates",
        "A site gets a certificate when you turn HTTPS on for it.",
        cx,
    );
    let body = vec![
        section("AUTHORITY", authority_card, cx).into_any_element(),
        section("CERTIFICATES", certs_list, cx).into_any_element(),
    ];
    frame("certificates", detail, actions, &[&list], body, cx)
}

fn mail(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "mail.inbox");
    let data = snap.data.as_deref();
    let messages = data
        .and_then(|d| d["messages"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let total = data
        .and_then(|d| d["total"].as_u64())
        .unwrap_or(messages.len() as u64);
    let unread = data.and_then(|d| d["unread"].as_u64()).unwrap_or(0);
    let rows: Vec<Row> = messages
        .iter()
        .map(|m| {
            let id = text(m, "id");
            let subject = text(m, "subject");
            let site = text(m, "site");
            Row {
                title: if subject.is_empty() {
                    "No subject".to_string()
                } else {
                    subject.clone()
                },
                sub: format!(
                    "from {}{}",
                    text(m, "from"),
                    if site.is_empty() {
                        String::new()
                    } else {
                        format!(", sent by {site}")
                    }
                ),
                state: (m["read"] == json!(false) || m["unread"] == json!(true))
                    .then(|| (Tone::Info, "unread".to_string())),
                side: ui::ago(&text(m, "created")),
                acts: vec![Act::new("Delete", "mail.delete", json!({ "id": id })).ask(
                    "Delete this message?",
                    "It is removed from the mailbox. This cannot be undone.",
                    "Delete message",
                )],
                open: Some((
                    Detail::Mail,
                    QueryKey::new("mail.read", json!({ "id": id })),
                    subject,
                )),
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Opening the mailbox…".to_string()
    } else if total == 0 {
        "No mail has been caught.".to_string()
    } else {
        format!(
            "{} caught, {} unread. Nothing here was really sent.",
            if total == 1 {
                "1 message".to_string()
            } else {
                format!("{total} messages")
            },
            unread
        )
    };
    let actions = div().child(
        Button::new("mail.clear", "Delete all")
            .icon("x")
            .disabled(total == 0)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.run_act(
                    Act::new("Delete all", "mail.clear", json!({})).ask(
                        "Delete all mail?",
                        "Every caught message is removed. This cannot be undone.",
                        "Delete all mail",
                    ),
                    window,
                    cx,
                )
            })),
    );
    if let Some(down) = service_down(&[&snap], "mailpit", "The mail catcher", "inbox", "to catch the mail your sites send", 8025, cx) {
        return frame("mail", "The mail catcher is not running.", actions, &[], vec![down], cx);
    }
    let body = vec![list_or(&snap, rows, "mail.row", "inbox", "No mail", "When a site sends an email, it is caught here instead of being delivered, so you can read what it would have sent.", cx).into_any_element()];
    frame("mail", detail, actions, &[&snap], body, cx)
}

// ---------------------------------------------------------------------------------------------- team

fn schedules(cx: &mut Context<Shell>) -> AnyElement {
    let work = read(cx, "schedule.list");
    let cron = read(cx, "cron.list");
    let work_rows: Vec<Row> = work
        .list("")
        .iter()
        .map(|s| {
            let name = text(s, "name");
            let on = s["enabled"] != json!(false);
            Row {
                title: name.clone(),
                tag: text(s, "spec"),
                sub: format!("{}{}", text(s, "project"), if text(s, "goal").is_empty() { String::new() } else { format!(": {}", text(s, "goal")) }),
                state: Some(if on { (Tone::Ok, "scheduled".to_string()) } else { (Tone::Neutral, "paused".to_string()) }),
                side: s["next_run_at"].as_str().map(|n| format!("next {}", ui::in_time(n))).unwrap_or_default(),
                acts: vec![
                    Act::new("Run now", "schedule.run_now", json!({ "name": name })).icon("play").busy(format!("Starting {name}…")),
                    Act::new(if on { "Pause" } else { "Resume" }, "schedule.enable", json!({ "name": name, "enabled": !on })),
                    Act::new("Delete", "schedule.delete", json!({ "name": name })).ask(format!("Delete the schedule {name}?"), "It stops running on its timetable. Work it already did is kept. This cannot be undone.", "Delete schedule"),
                ],
                ..Row::default()
            }
        })
        .collect();
    let jobs = cron
        .data
        .as_deref()
        .and_then(|d| d["jobs"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let cron_rows: Vec<Row> = jobs
        .iter()
        .map(|j| {
            let id = text(j, "id");
            let on = j["enabled"] != json!(false);
            let ran = j["last_run_at"].as_str().is_some();
            let ok = j["last_exit"].as_i64() == Some(0);
            Row {
                title: text(j, "command"),
                tag: text(j, "schedule"),
                sub: format!(
                    "{}{}",
                    text(j, "site"),
                    j["next_run_at"]
                        .as_str()
                        .map(|n| format!(", next {}", ui::in_time(n)))
                        .unwrap_or_default()
                ),
                state: Some(if !on {
                    (Tone::Neutral, "paused".to_string())
                } else if !ran {
                    (Tone::Info, "waiting for its first run".to_string())
                } else if ok {
                    (Tone::Ok, "last run worked".to_string())
                } else {
                    (Tone::Danger, "last run failed".to_string())
                }),
                acts: vec![
                    Act::new("Run now", "cron.run", json!({ "id": id }))
                        .icon("play")
                        .busy("Running the cron job…"),
                    Act::new(
                        if on { "Pause" } else { "Resume" },
                        "cron.enable",
                        json!({ "id": id, "enabled": !on }),
                    ),
                    Act::new("Remove", "cron.remove", json!({ "id": id })).ask(
                        "Remove this cron job?",
                        "It stops running. Its log is kept. This cannot be undone.",
                        "Remove job",
                    ),
                ],
                ..Row::default()
            }
        })
        .collect();
    let detail = if work.is_first_load() {
        "Reading the timetables…".to_string()
    } else {
        format!(
            "{} for the team, {} for sites.",
            if work_rows.len() == 1 {
                "1 schedule".to_string()
            } else {
                format!("{} schedules", work_rows.len())
            },
            if cron_rows.len() == 1 {
                "1 cron job".to_string()
            } else {
                format!("{} cron jobs", cron_rows.len())
            }
        )
    };
    let add_work = form_button(
        "schedules.add",
        "Schedule team work",
        "plus",
        FormSpec {
            title: "Schedule team work",
            intro: "The team starts this goal on the timetable and reports what it did. The time is a cron expression such as 0 9 * * 1 for every Monday at nine.",
            verb: "Schedule work",
            capability: "schedule.create",
            domain: "schedule",
            fixed: json!({}),
            fields: vec![
                Field::text("name", "Name", "weekly-dependency-check"),
                Field::text("project", "Project folder", "D:\\devd\\www\\shop"),
                Field::text("spec", "When", "0 9 * * 1"),
                Field::optional(
                    "goal",
                    "What the team should do",
                    "Check for outdated dependencies and report",
                ),
            ],
            busy: None,
        },
        ButtonKind::Primary,
        cx,
    );
    let add_cron = form_button(
        "schedules.cron",
        "Add a cron job",
        "plus",
        FormSpec {
            title: "Add a cron job",
            intro: "devd runs this command in the site's folder on the timetable, and keeps its log. The time is a cron expression such as */5 * * * * for every five minutes.",
            verb: "Add job",
            capability: "cron.add",
            domain: "cron",
            fixed: json!({}),
            fields: vec![
                Field::text("site", "Site", "shop"),
                Field::text("schedule", "When", "*/5 * * * *"),
                Field::text("command", "Command", "php artisan schedule:run"),
            ],
            busy: None,
        },
        ButtonKind::Secondary,
        cx,
    );
    let work_list = list_or(
        &work,
        work_rows,
        "schedules.work",
        "clock",
        "No scheduled work",
        "Schedule a goal and the team starts it on the timetable.",
        cx,
    );
    let cron_list = list_or(
        &cron,
        cron_rows,
        "schedules.cron.row",
        "clock",
        "No cron jobs",
        "A cron job runs a command in a site's folder on a timetable.",
        cx,
    );
    let body = vec![
        section("TEAM WORK", work_list, cx).into_any_element(),
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(eyebrow("CRON JOBS", cx))
                    .child(add_cron),
            )
            .child(cron_list)
            .into_any_element(),
    ];
    frame(
        "schedules",
        detail,
        div().child(add_work),
        &[&work, &cron],
        body,
        cx,
    )
}

fn automations(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "automation.list");
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|a| {
            let name = text(a, "name");
            let action = match a["action"]["type"].as_str() {
                Some("team") => "Starts a team run".to_string(),
                Some("chat") => "Messages the assistant".to_string(),
                Some("inbox") => "Posts to the inbox".to_string(),
                Some(other) => title_case(other),
                None => "Does something".to_string(),
            };
            Row {
                title: name.clone(),
                tag: text(a, "event"),
                sub: action,
                state: Some(if a["enabled"] == json!(false) {
                    (Tone::Neutral, "off".to_string())
                } else {
                    (Tone::Ok, "on".to_string())
                }),
                acts: vec![
                    Act::new("Delete", "automation.delete", json!({ "name": name })).ask(
                        format!("Delete the automation {name}?"),
                        "It stops reacting to its event. This cannot be undone.",
                        "Delete automation",
                    ),
                ],
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Reading the automations…".to_string()
    } else {
        format!(
            "{} reacting to events.",
            if rows.len() == 1 {
                "1 automation".to_string()
            } else {
                format!("{} automations", rows.len())
            }
        )
    };
    let body = vec![list_or(&snap, rows, "automations.row", "bolt", "No automations", "An automation does something when an event happens, such as a deploy finishing or a site going down. Ask the assistant to set one up.", cx).into_any_element()];
    frame("automations", detail, div(), &[&snap], body, cx)
}

fn skills(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "skills.list");
    let all = snap.list("");
    let mut rows: Vec<Row> = all
        .iter()
        .map(|s| {
            let name = text(s, "name");
            let own = text(s, "source") != "builtin";
            Row {
                title: name.clone(),
                tag: text(s, "kind"),
                sub: text(s, "description"),
                state: Some(if own {
                    (Tone::Info, "yours".to_string())
                } else {
                    (Tone::Neutral, "built in".to_string())
                }),
                acts: if own {
                    vec![
                        Act::new("Delete", "skills.delete", json!({ "name": name })).ask(
                            format!("Delete the skill {name}?"),
                            "Agents can no longer load it. This cannot be undone.",
                            "Delete skill",
                        ),
                    ]
                } else {
                    vec![]
                },
                open: Some((
                    Detail::Skill,
                    QueryKey::new("skills.get", json!({ "name": name })),
                    name.clone(),
                )),
                ..Row::default()
            }
        })
        .collect();
    // Your own skills first: they are the ones you can edit or delete.
    rows.sort_by(|a, b| {
        let own = |row: &Row| {
            row.state
                .as_ref()
                .is_some_and(|(tone, _)| *tone == Tone::Info)
        };
        own(b).cmp(&own(a)).then_with(|| a.title.cmp(&b.title))
    });
    let mine = all
        .iter()
        .filter(|s| text(s, "source") != "builtin")
        .count();
    let detail = if snap.is_first_load() {
        "Reading the skills…".to_string()
    } else {
        format!("{} agents can load, {} of them yours.", all.len(), mine)
    };
    let actions = div().flex().gap(px(8.0)).child(form_button(
        "skills.install",
        "Install from GitHub",
        "plus",
        FormSpec { title: "Install skills from GitHub", intro: "devd reads the skill files in a repository and saves them as your own. Nothing in a skill is run: it is text agents read.", verb: "Install skills", capability: "skills.install", domain: "skills", fixed: json!({}), fields: vec![Field::text("source", "Repository", "owner/name")], busy: None },
        ButtonKind::Primary,
        cx,
    ));
    let body = vec![
        list_or(
            &snap,
            rows,
            "skills.row",
            "bolt",
            "No skills",
            "Skills are instructions agents load before a kind of work.",
            cx,
        )
        .into_any_element(),
    ];
    frame("skills", detail, actions, &[&snap], body, cx)
}

fn mcp(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "mcp.list");
    let servers = snap
        .data
        .as_deref()
        .and_then(|d| d["servers"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let rows: Vec<Row> = servers
        .iter()
        .map(|s| {
            let name = text(s, "name");
            let on = s["enabled"] == json!(true);
            let trust = text(s, "trust");
            let how = s["command"].as_str().or(s["url"].as_str()).unwrap_or_default().to_string();
            let tools = s["tools"].as_array().map(Vec::len).unwrap_or(0);
            let failed = s["last_error"].as_str().filter(|e| !e.is_empty());
            Row {
                title: name.clone(),
                tag: text(s, "transport"),
                sub: match failed {
                    Some(e) => e.to_string(),
                    None => format!("{how} · {} · {}", if tools == 1 { "1 tool".to_string() } else { format!("{tools} tools") }, match trust.as_str() { "allow" => "runs without asking", "deny" => "agents may not use it", _ => "asks before each call" }),
                },
                state: Some(match (on, failed.is_some()) {
                    (_, true) => (Tone::Danger, "not answering".to_string()),
                    (true, false) => (Tone::Ok, "on".to_string()),
                    (false, false) => (Tone::Neutral, "off".to_string()),
                }),
                acts: vec![
                    Act::new("Test", "mcp.test", json!({ "name": name })).busy(format!("Testing {name}…")),
                    Act::new(if on { "Turn off" } else { "Turn on" }, "mcp.enable", json!({ "name": name, "enabled": !on })),
                    Act::new("Remove", "mcp.remove", json!({ "name": name })).ask(format!("Remove {name}?"), "The assistant can no longer use its tools. This cannot be undone, but you can add it again.", "Remove server"),
                ],
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Reading the tool servers…".to_string()
    } else {
        format!(
            "{} the assistant may use.",
            if rows.len() == 1 {
                "1 outside tool server".to_string()
            } else {
                format!("{} outside tool servers", rows.len())
            }
        )
    };
    let actions = div().child(form_button(
        "mcp.add",
        "Add a server",
        "plus",
        FormSpec {
            title: "Add an MCP server",
            intro: "An outside program or service whose tools the assistant may use. It starts turned off and asks before every call until you say otherwise. Give a command for a program on this machine, or an address for a remote one.",
            verb: "Add server",
            capability: "mcp.add",
            domain: "mcp",
            fixed: json!({ "enabled": false }),
            fields: vec![Field::text("name", "Name", "github"), Field::optional("command", "Command", "npx"), Field::optional("url", "Address", "https://example.com/mcp")],
            busy: None,
        },
        ButtonKind::Primary,
        cx,
    ));
    let body = vec![
        list_or(
            &snap,
            rows,
            "mcp.row",
            "bolt",
            "No tool servers",
            "Add one and the assistant can use its tools, each call asking first.",
            cx,
        )
        .into_any_element(),
    ];
    frame("mcp", detail, actions, &[&snap], body, cx)
}

fn notes(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "memory.list");
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|n| {
            let id = text(n, "id");
            let project = text(n, "project");
            let evidence = text(n, "evidence");
            let sub = if evidence.is_empty() {
                if project.is_empty() { String::new() } else { format!("From {}", ui::leaf(&project)) }
            } else {
                evidence
            };
            Row {
                title: text(n, "lesson"),
                tag: text(n, "target"),
                sub,
                state: Some((Tone::Warn, "waiting for you".to_string())),
                acts: vec![Act::new("Keep it", "memory.apply", json!({ "id": id })).ask("Keep this note?", "devd writes it into the project's AGENTS.md under Project memory. Every agent working in that project then reads it.", "Keep the note")],
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Reading the notes…".to_string()
    } else if rows.is_empty() {
        "The team has nothing waiting to be remembered.".to_string()
    } else {
        format!(
            "{} the team would like to remember.",
            if rows.len() == 1 {
                "1 note".to_string()
            } else {
                format!("{} notes", rows.len())
            }
        )
    };
    let body = vec![list_or(&snap, rows, "notes.row", "file", "Nothing to review", "When the team learns something worth keeping about a project, it proposes it here and waits for you.", cx).into_any_element()];
    frame("notes", detail, div(), &[&snap], body, cx)
}

// ----------------------------------------------------------------------------------------- projects

/// The form that starts one stack. Kept in one place so New project and Stacks
/// cannot offer different fields for the same scaffolder.
fn stack_form(stack: &Value) -> Option<FormSpec> {
    (stack["creatable"] != json!(false)).then(|| FormSpec {
        title: "Start a new project",
        intro: "devd runs the stack's own scaffolder in a new folder, then checks what it made. Leave the folder empty to use your projects folder.",
        verb: "Create project",
        capability: "project.init",
        domain: "project",
        fixed: json!({ "stack": text(stack, "id"), "git": true, "install": true }),
        fields: vec![Field::text("name", "Project name", "shop"), Field::optional("destination", "Folder to create it in", "D:\\devd\\www")],
        busy: Some("Creating {name}…"),
    })
}

/// One way to begin, as a card with the button that starts it.
fn path_card(
    title: &'static str,
    detail: &'static str,
    action: AnyElement,
    cx: &mut Context<Shell>,
) -> AnyElement {
    let t = theme(cx).clone();
    card(cx)
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(16.0))
        .py(px(14.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(detail),
                ),
        )
        .child(action)
        .into_any_element()
}

fn new_project(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "stacks.list");
    let history = read_with(cx, "stacks.runs", json!({ "limit": 8 }));
    let mut stacks: Vec<Value> = snap.list("").to_vec();
    stacks.sort_by_key(|s| (text(s, "category"), text(s, "name")));

    let browse = Button::new("new-project.browse", "Browse stacks")
        .icon("folder")
        .on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| this.go(Route::Page("stacks".into()), cx)),
        );
    let empty = form_button(
        "new-project.empty",
        "Create an empty project",
        "plus",
        FormSpec {
            title: "Start empty",
            intro: "A new folder with a README and its own git repository. Nothing else is added, so you can choose every tool yourself.",
            verb: "Create project",
            capability: "project.create",
            domain: "project",
            fixed: json!({ "git": true }),
            fields: vec![
                Field::text("name", "Project name", "shop"),
                Field::optional("destination", "Folder to create it in", "D:\\devd\\www"),
            ],
            busy: Some("Creating {name}…"),
        },
        ButtonKind::Primary,
        cx,
    );
    let existing = form_button(
        "new-project.existing",
        "Add a folder",
        "folder",
        FormSpec {
            title: "Add an existing project",
            intro: "devd reads the folder, detects its stack and tasks, and registers it. No file in the folder is changed.",
            verb: "Add project",
            capability: "project.import",
            domain: "project",
            fixed: json!({}),
            fields: vec![Field::text("path", "Project folder", "D:\\devd\\www\\shop")],
            busy: None,
        },
        ButtonKind::Secondary,
        cx,
    );

    let mut body = vec![
        section(
            "HOW WOULD YOU LIKE TO BEGIN?",
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(path_card("Start from a stack", "A working starting point with its dependencies installed: Laravel, Next.js, React and more.", browse.into_any_element(), cx))
                .child(path_card("Start empty", "A new folder with a README and git, ready for anything you want to build.", empty, cx))
                .child(path_card("Add a folder you already have", "Register an existing project. devd detects what it is and leaves its files alone.", existing, cx)),
            cx,
        )
        .into_any_element(),
    ];

    // Six common starting points, so the page helps without becoming a catalogue.
    let popular_ids = ["laravel", "nextjs", "react-vite", "vue", "django", "go"];
    let mut popular: Vec<&Value> = popular_ids
        .iter()
        .filter_map(|id| stacks.iter().find(|s| text(s, "id") == *id))
        .collect();
    if popular.is_empty() {
        popular = stacks
            .iter()
            .filter(|s| s["creatable"] != json!(false))
            .take(6)
            .collect();
    }
    if !popular.is_empty() {
        let rows: Vec<Row> = popular
            .iter()
            .map(|s| Row {
                mark: Some(text(s, "id")),
                title: text(s, "name"),
                tag: text(s, "language"),
                sub: text(s, "summary"),
                side: version_count(s),
                form: stack_form(s),
                ..Row::default()
            })
            .collect();
        body.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(eyebrow("POPULAR STACKS", cx))
                        .child(Button::new("new-project.all", "See all stacks").on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.go(Route::Page("stacks".into()), cx)
                            }),
                        )),
                )
                .child(rows_card(rows, "new-project.popular", cx))
                .into_any_element(),
        );
    }

    // What was started from this page, including one still running. A scaffold
    // outlives the dialog that started it, so its state belongs on the page too.
    let active: Vec<String> = devd(cx)
        .running
        .iter()
        .filter_map(|run| run.run_id.clone())
        .collect();
    let run_rows: Vec<Row> = history
        .list("")
        .iter()
        .filter(|run| text(run, "kind") == "init")
        .take(6)
        .map(|run| {
            let id = text(run, "id");
            let running = active.iter().any(|known| known == &id);
            let finished = run["finished_at"].as_str().is_some();
            let ok = run["ok"].as_bool();
            let state = if running {
                (Tone::Info, "creating".to_string())
            } else if !finished {
                (Tone::Warn, "stopped before it finished".to_string())
            } else if ok == Some(true) {
                (Tone::Ok, "finished".to_string())
            } else {
                (Tone::Danger, "did not finish".to_string())
            };
            let side = run["finished_at"]
                .as_str()
                .map(|_| match run["duration_ms"].as_u64() {
                    Some(ms) if ms >= 1000 => format!("{}s", ms / 1000),
                    Some(ms) => format!("{ms}ms"),
                    None => String::new(),
                })
                .unwrap_or_else(|| run["started_at"].as_str().map(|s| format!("started {}", ui::ago(s))).unwrap_or_default());
            let acts = if running {
                vec![Act::new("Stop", "stacks.cancel", json!({ "run_id": id }))
                    .busy("Stopping the scaffold…")
                    .ask("Stop creating this project?", "The files it has made so far stay in the folder for you to inspect. This cannot be undone.", "Stop creating")]
            } else {
                Vec::new()
            };
            Row {
                title: text(run, "title"),
                sub: text(run, "subject"),
                sub_is_path: true,
                state: Some(state),
                side,
                acts,
                ..Row::default()
            }
        })
        .collect();
    if !run_rows.is_empty() {
        body.push(
            section(
                "RECENT RUNS",
                rows_card(run_rows, "new-project.run", cx),
                cx,
            )
            .into_any_element(),
        );
    }
    frame(
        "new-project",
        "Three ways to begin. Pick the one that fits what you already have.",
        div(),
        &[&snap, &history],
        body,
        cx,
    )
}

fn version_count(stack: &Value) -> String {
    let versions = stack["versions"].as_array().map(Vec::len).unwrap_or(0);
    if versions > 1 {
        format!("{versions} versions")
    } else {
        String::new()
    }
}

fn title_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The catalogue: what devd can build, not what you already built.
fn stacks(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "stacks.list");
    let mut stacks: Vec<Value> = snap.list("").to_vec();
    stacks.sort_by_key(|s| (text(s, "category"), text(s, "name")));
    let mut body = Vec::new();
    let mut category = String::new();
    let mut rows: Vec<Row> = Vec::new();
    let flush =
        |category: &str, rows: Vec<Row>, body: &mut Vec<AnyElement>, cx: &mut Context<Shell>| {
            if !rows.is_empty() {
                body.push(
                    section(
                        // Section labels are small capitals on every screen.
                        title_case(category).to_uppercase(),
                        rows_card(rows, "stacks.catalogue", cx),
                        cx,
                    )
                    .into_any_element(),
                );
            }
        };
    for stack in &stacks {
        let now = text(stack, "category");
        if now != category {
            flush(&category, std::mem::take(&mut rows), &mut body, cx);
            category = now.clone();
        }
        let creatable = stack["creatable"] != json!(false);
        rows.push(Row {
            title: text(stack, "name"),
            tag: text(stack, "language"),
            sub: text(stack, "summary"),
            side: version_count(stack),
            state: (!creatable).then(|| (Tone::Warn, "detect only".to_string())),
            form: stack_form(stack),
            ..Row::default()
        });
    }
    flush(&category, rows, &mut body, cx);
    let detail = if snap.is_first_load() {
        "Reading the stacks…".to_string()
    } else {
        format!(
            "{} templates devd can create, and what each one makes.",
            stacks.len()
        )
    };
    frame("stacks", detail, div(), &[&snap], body, cx)
}

/// The Projects screen: every project, what it is built with, and the tasks it can run.
pub fn projects_hub(cx: &mut Context<Shell>) -> AnyElement {
    let projects = devd(cx).sync.watch(&crate::state::q::projects());
    let catalogue = read(cx, "stacks.list");
    let background = read(cx, "task.list");
    let running_processes: Vec<String> = background
        .list("")
        .iter()
        .filter(|p| p["running"] == json!(true))
        .filter_map(|p| p["name"].as_str().map(str::to_string))
        .collect();
    let stack_name = |id: &str| {
        catalogue
            .list("")
            .iter()
            .find(|s| text(s, "id") == id)
            .map(|s| text(s, "name"))
            .unwrap_or_else(|| title_case(id))
    };
    let t = theme(cx).clone();
    let all: Vec<Value> = projects.list("").to_vec();
    let missing = all.iter().filter(|p| p["exists"] == json!(false)).count();
    let mut body = Vec::new();
    for (n, project) in all.iter().enumerate() {
        let path = text(project, "path");
        let present = project["exists"] != json!(false);
        let name = project["name"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| ui::leaf(&path).to_string());
        let stack = project["stack"].as_str().map(stack_name);
        let manager = project["package_manager"].as_str().map(str::to_string);
        let sites = project["sites"].as_array().map(Vec::len).unwrap_or(0);
        let open = devd(cx).open_projects.contains(&path);
        let tasks = if open && stack.is_some() && present {
            Some(read_with(cx, "project.tasks", json!({ "path": path })))
        } else {
            None
        };
        let title = div()
            .id(ui::nid("projects.name", n))
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .font_weight(FontWeight::MEDIUM)
            .child(name);
        // The name opens the project.
        let title = if present {
            let go = path.clone();
            let accent = t.accent;
            title
                .cursor_pointer()
                .hover(move |s| s.text_color(accent))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(Route::Projects(Some(go.clone())), cx)
                }))
        } else {
            title
        };
        let mut head = div().flex().items_center().gap(px(10.0)).child(title);
        head = if !present {
            head.child(pill("folder missing", Tone::Warn, cx))
        } else {
            match (&stack, &manager) {
                (Some(s), Some(m)) => head.child(pill(s.clone(), Tone::Info, cx)).child(
                    div()
                        .font_family(mono())
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(m.clone()),
                ),
                (Some(s), None) => head.child(pill(s.clone(), Tone::Info, cx)),
                _ => head.child(pill("not set up yet", Tone::Neutral, cx)),
            }
        };
        let mut head_line = div().flex().items_center().gap(px(12.0)).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(head)
                .child(
                    div()
                        .text_size(px(size::TEXT_XS))
                        .text_color(t.faint)
                        .child(ui::Clip::tail(path.clone())),
                ),
        );
        if sites > 0 {
            head_line = head_line.child(pill(
                if sites == 1 {
                    "1 site".to_string()
                } else {
                    format!("{sites} sites")
                },
                Tone::Neutral,
                cx,
            ));
        }
        if present && stack.is_none() {
            let p = path.clone();
            head_line = head_line.child(
                Button::new(ui::nid("projects.import", n), "Set up in devd")
                    .icon("plus")
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_act(
                            Act::new("Set up", "project.import", json!({ "path": p }))
                                .busy("Adding the project…"),
                            window,
                            cx,
                        )
                    })),
            );
        }
        if present && stack.is_some() {
            let key = path.clone();
            let label = if open { "Hide tasks" } else { "Tasks" };
            head_line = head_line.child(
                Button::new(ui::nid("projects.expand", n), label)
                    .icon(if open {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    })
                    .kind(ButtonKind::Ghost)
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        let state = cx.global_mut::<crate::state::Devd>();
                        if !state.open_projects.remove(&key) {
                            state.open_projects.insert(key.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        if !present {
            let p = path.clone();
            // A folder that is listed only because a site serves it (a worktree that was deleted, say) was never
            // registered as a project, so there is nothing to forget: what keeps it listed is the site.
            let sites_here: Vec<String> = project["sites"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let only_a_site = project["registered"] != json!(true) && !sites_here.is_empty();
            head_line = head_line.child(Button::new(ui::nid("projects.forget", n), if only_a_site { "Remove site" } else { "Remove" }).kind(ButtonKind::Ghost).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                let act = if only_a_site {
                    let first = sites_here[0].clone();
                    Act::new("Remove site", "site.remove", json!({ "name": first })).ask(
                        format!("Remove the site {first}?"),
                        "This folder is listed only because that site serves it, and the folder is gone. Removing the site deletes its record, certificate and web server entry. No files are touched.",
                        "Remove site",
                    )
                } else {
                    Act::new("Remove", "project.forget", json!({ "path": p })).ask("Remove this project from devd?", "Its folder is missing. This forgets devd's record only; no files are touched.", "Remove project")
                };
                this.run_act(act, window, cx)
            })));
        }
        let mut card_el = card(cx)
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(div().px(px(14.0)).py(px(10.0)).child(head_line));
        if let Some(tasks) = tasks {
            let items = tasks.list("");
            if items.is_empty() {
                let words = if tasks.is_first_load() {
                    "Looking for tasks…"
                } else {
                    "This project has no tasks devd knows how to run."
                };
                card_el = card_el.child(rule(cx)).child(
                    div()
                        .px(px(14.0))
                        .py(px(10.0))
                        .text_size(px(size::TEXT_SM))
                        .text_color(t.muted)
                        .child(words),
                );
            }
            for (k, task) in items.iter().enumerate() {
                let (id, title, command) =
                    (text(task, "id"), text(task, "title"), text(task, "command"));
                let available = task["available"] != json!(false);
                let risk = text(task, "risk");
                let cap = if risk == "destructive" {
                    "project.run_destructive"
                } else {
                    "project.run"
                };
                let long = task["long_running"] == json!(true);
                let args = if long {
                    json!({ "path": path, "task": id, "background": true })
                } else {
                    json!({ "path": path, "task": id })
                };
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
                let process_name = format!("task:{}:{}", ui::leaf(&path), id);
                let running_now =
                    long && running_processes.iter().any(|known| known == &process_name);
                let act = if running_now {
                    Act::new("Stop", "task.stop", json!({ "name": process_name }))
                        .icon("stop")
                        .busy(format!("Stopping {title}…"))
                } else {
                    let act = Act::new("Run", cap, args)
                        .icon("play")
                        .busy(format!("Running {title}…"));
                    if risk == "destructive" {
                        act.ask(format!("Run {title}?"), format!("This runs {command} in {}. It can destroy data and may not be reversible.", ui::leaf(&path)), "Run it")
                    } else {
                        act
                    }
                };
                let state = if running_now {
                    Some((Tone::Ok, "running in the background".to_string()))
                } else {
                    state
                };
                let mut line = div()
                    .px(px(14.0))
                    .min_h(px(44.0))
                    .py(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(ui::dot(
                        if running_now || available {
                            Tone::Ok
                        } else {
                            Tone::Neutral
                        },
                        cx,
                    ));
                line = line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(
                            div()
                                .font_family(mono())
                                .text_size(px(size::TEXT_XS))
                                .text_color(t.faint)
                                .child(ui::Clip::new(command)),
                        ),
                );
                if let Some((tone, word)) = state {
                    line = line.child(
                        div()
                            .flex_none()
                            .text_size(px(size::TEXT_SM))
                            .text_color(t.tone(tone))
                            .child(word),
                    );
                }
                if available || running_now {
                    line = line.child(act_button(
                        ElementId::NamedInteger(format!("projects.run.{n}").into(), k as u64),
                        act,
                        ButtonKind::Secondary,
                        cx,
                    ));
                }
                card_el = card_el.child(rule(cx)).child(line);
            }
        }
        body.push(card_el.into_any_element());
    }
    let detail = if projects.is_first_load() {
        "Reading your projects…".to_string()
    } else if all.is_empty() {
        "No projects yet. Start one, or add a folder you already have.".to_string()
    } else if missing > 0 {
        format!(
            "{} project{}, and {missing} whose folder is missing.",
            all.len(),
            if all.len() == 1 { "" } else { "s" }
        )
    } else if all.len() == 1 {
        "What your project is built with, and the tasks devd can run for it.".to_string()
    } else {
        format!(
            "What each of your {} projects is built with, and the tasks devd can run for them.",
            all.len()
        )
    };
    let actions = div()
        .flex()
        .gap(px(8.0))
        .child(form_button(
            "projects.add",
            "Add a folder",
            "folder",
            FormSpec {
                title: "Add an existing project",
                intro: "devd reads the folder, detects its stack and tasks, and registers it. No file in the folder is changed.",
                verb: "Add project",
                capability: "project.import",
                domain: "project",
                fixed: json!({}),
                fields: vec![Field::text("path", "Project folder", "D:\\devd\\www\\shop")],
                busy: None,
            },
            ButtonKind::Secondary,
            cx,
        ))
        .child(Button::new("projects.new", "New project").icon("plus").kind(ButtonKind::Primary).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Route::Page("new-project".into()), cx))));
    if body.is_empty() {
        body.push(
            ui::empty(
                "folder",
                "No projects yet",
                "Start from a stack, start empty, or add a folder you already have.",
                cx,
            )
            .into_any_element(),
        );
    }
    frame_at(
        "projects",
        "Projects",
        Route::Projects(None),
        detail,
        actions,
        &[&projects],
        body,
        cx,
    )
}

// --------------------------------------------------------------------------------------------- settings

fn diagnostics(cx: &mut Context<Shell>) -> AnyElement {
    let stats = read(cx, "monitor.stats");
    let dns = read(cx, "dns.status");
    let auto = read(cx, "autostart.status");
    let update = read(cx, "update.check");
    let needs = read(cx, "setup.requirements");
    let t = theme(cx).clone();
    let mut machine = Vec::new();
    if let Some(d) = stats.data.as_deref() {
        let mem = &d["memory"];
        machine.push(Row {
            title: "Processor".to_string(),
            sub: format!("{}% busy", d["cpu_percent"].as_u64().unwrap_or(0)),
            state: Some((
                if d["cpu_percent"].as_u64().unwrap_or(0) > 90 {
                    Tone::Warn
                } else {
                    Tone::Ok
                },
                if d["cpu_percent"].as_u64().unwrap_or(0) > 90 {
                    "busy".to_string()
                } else {
                    "fine".to_string()
                },
            )),
            ..Row::default()
        });
        let (used, total) = (
            mem["used_mb"].as_u64().unwrap_or(0),
            mem["total_mb"].as_u64().unwrap_or(0),
        );
        let pct = (used * 100).checked_div(total).unwrap_or(0);
        machine.push(Row {
            title: "Memory".to_string(),
            sub: format!(
                "{:.1} of {:.1} GB in use",
                used as f64 / 1024.0,
                total as f64 / 1024.0
            ),
            state: Some(if pct > 90 {
                (Tone::Warn, "nearly full".to_string())
            } else {
                (Tone::Ok, "fine".to_string())
            }),
            ..Row::default()
        });
        for disk in d["disks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let (free, size_gb) = (
                disk["free_gb"].as_f64().unwrap_or(0.0),
                disk["total_gb"].as_f64().unwrap_or(0.0),
            );
            machine.push(Row {
                title: format!("Disk {}", text(disk, "mount")),
                sub: format!("{free:.1} GB free of {size_gb:.1} GB"),
                state: Some(if size_gb > 0.0 && free / size_gb < 0.1 {
                    (Tone::Warn, "nearly full".to_string())
                } else {
                    (Tone::Ok, "fine".to_string())
                }),
                ..Row::default()
            });
        }
        machine.push(Row {
            title: "This machine has been on".to_string(),
            sub: format!(
                "{:.1} hours, with {} of {} devd services running",
                d["uptime_hours"].as_f64().unwrap_or(0.0),
                d["devd"]["running"].as_u64().unwrap_or(0),
                d["devd"]["services"].as_u64().unwrap_or(0)
            ),
            ..Row::default()
        });
    }
    let current = text(update.data.as_deref().unwrap_or(&Value::Null), "current");
    let newer = update
        .data
        .as_deref()
        .is_some_and(|u| u["newer"] == json!(true));
    let signed = update
        .data
        .as_deref()
        .is_some_and(|u| u["signed"] == json!(true));
    let outstanding = needs
        .data
        .as_deref()
        .and_then(|d| d["missing"].as_u64())
        .unwrap_or(0);
    let autostart_on = auto
        .data
        .as_deref()
        .is_some_and(|a| a["enabled"] == json!(true));
    let dns_ok = dns
        .data
        .as_deref()
        .is_some_and(|d| d["system_resolves"] == json!(true));
    let setup = vec![
        Row {
            title: "devd version".to_string(),
            sub: if newer {
                format!(
                    "{} is available.{}",
                    text(update.data.as_deref().unwrap_or(&Value::Null), "latest"),
                    if signed {
                        ""
                    } else {
                        " Its signature could not be checked, so devd will not install it for you."
                    }
                )
            } else {
                format!("You have {current}, the newest release.")
            },
            state: Some(if newer {
                (Tone::Info, "update available".to_string())
            } else {
                (Tone::Ok, "up to date".to_string())
            }),
            acts: if newer && signed {
                vec![Act::new("Update", "update.apply", json!({})).ask("Update devd?", "devd downloads the new version, checks its signature, and restarts itself. Your sites and data are kept.", "Update devd")]
            } else {
                vec![]
            },
            ..Row::default()
        },
        Row {
            title: "Start when I sign in".to_string(),
            sub: auto
                .data
                .as_deref()
                .map(|a| text(a, "command"))
                .unwrap_or_default(),
            sub_is_path: true,
            state: Some(if autostart_on {
                (Tone::Ok, "on".to_string())
            } else {
                (Tone::Neutral, "off".to_string())
            }),
            acts: vec![if autostart_on {
                Act::new("Turn off", "autostart.disable", json!({}))
            } else {
                Act::new("Turn on", "autostart.enable", json!({}))
            }],
            ..Row::default()
        },
        Row {
            title: "Site names".to_string(),
            sub: if dns_ok {
                "devd answers for your .test names.".to_string()
            } else {
                "devd's name server is not running, so sites are reached through the hosts file."
                    .to_string()
            },
            state: Some(if dns_ok {
                (Tone::Ok, "answering".to_string())
            } else {
                (Tone::Neutral, "using the hosts file".to_string())
            }),
            ..Row::default()
        },
        Row {
            title: "What devd needs".to_string(),
            sub: if outstanding == 0 {
                "Everything devd needs is in place.".to_string()
            } else {
                format!(
                    "{outstanding} thing{} still to set up.",
                    if outstanding == 1 { " is" } else { "s are" }
                )
            },
            state: Some(if outstanding == 0 {
                (Tone::Ok, "complete".to_string())
            } else {
                (Tone::Warn, "incomplete".to_string())
            }),
            ..Row::default()
        },
    ];
    let _ = t;
    let machine_list = list_or(
        &stats,
        machine,
        "diagnostics.machine",
        "machine",
        "No readings yet",
        "devd reads this machine's load and disks here.",
        cx,
    );
    let setup_card = rows_card(setup, "diagnostics.devd", cx);
    let body = vec![
        section("THIS MACHINE", machine_list, cx).into_any_element(),
        section("DEVD", setup_card, cx).into_any_element(),
    ];
    frame(
        "diagnostics",
        "What this machine is doing, and what devd needs from it.",
        div(),
        &[&stats],
        body,
        cx,
    )
}

fn capabilities(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "api.capabilities");
    let t = theme(cx).clone();
    let all = snap.list("/items");
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for c in all {
        let g = text(c, "group");
        match groups.iter_mut().find(|(name, _)| *name == g) {
            Some((_, list)) => list.push(c),
            None => groups.push((g, vec![c])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let mut body = Vec::new();
    for (g, list) in groups {
        let open = devd(cx).open_groups.contains(&g);
        let label = g.to_uppercase();
        let key = g.clone();
        let toggle = Button::new(
            ui::nid("capabilities.toggle", body.len()),
            list.len().to_string(),
        )
        .icon(if open {
            "chevron-down"
        } else {
            "chevron-right"
        })
        .kind(ButtonKind::Ghost)
        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
            let state = cx.global_mut::<crate::state::Devd>();
            if !state.open_groups.remove(&key) {
                state.open_groups.insert(key.clone());
            }
            cx.notify();
        }));
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(eyebrow(label, cx))
            .child(toggle);
        if !open {
            body.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(header)
                    .into_any_element(),
            );
            continue;
        }
        let mut card_el = card(cx).flex().flex_col().overflow_hidden();
        for (n, c) in list.iter().enumerate() {
            if n > 0 {
                card_el = card_el.child(rule(cx));
            }
            let tier = text(c, "tier");
            let tone = match tier.as_str() {
                "read" => Tone::Neutral,
                "write" => Tone::Info,
                "exec" => Tone::Warn,
                _ => Tone::Danger,
            };
            let who = if c["human_only"] == json!(true) {
                "you only"
            } else if c["agent_safe"] == json!(true) {
                "you and agents"
            } else {
                "agents ask first"
            };
            card_el = card_el.child(
                div()
                    .px(px(14.0))
                    .min_h(px(44.0))
                    .py(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .w(px(230.0))
                            .flex_none()
                            .font_family(mono())
                            .text_size(px(size::TEXT_SM))
                            .child(ui::Clip::new(text(c, "id"))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(size::TEXT_SM))
                            .text_color(t.muted)
                            .child(ui::Clip::new(text(c, "summary"))),
                    )
                    .child(pill(who, Tone::Neutral, cx))
                    .child(
                        div()
                            .w(px(70.0))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .child(pill(tier, tone, cx)),
                    ),
            );
        }
        body.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(header)
                .child(card_el)
                .into_any_element(),
        );
    }
    if body.is_empty() {
        body.push(
            waiting(
                &snap,
                0,
                "bolt",
                "Nothing listed",
                "devd lists everything it can do here.",
                cx,
            )
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| div().into_any_element()),
        );
    }
    let total = snap
        .data
        .as_deref()
        .and_then(|d| d["total"].as_u64())
        .unwrap_or(all.len() as u64);
    let detail = if snap.is_first_load() {
        "Reading what devd can do…".to_string()
    } else {
        format!(
            "{total} things devd can do, grouped. The assistant, the command line and other programs reach them by these names."
        )
    };
    frame("capabilities", detail, div(), &[&snap], body, cx)
}

fn connect(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "agent_client.list");
    let clients = snap
        .data
        .as_deref()
        .and_then(|d| d["clients"].as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let connected = clients
        .iter()
        .filter(|c| c["added"] == json!(true) && c["current"] == json!(true))
        .count();
    let rows: Vec<Row> = clients
        .iter()
        .map(|c| {
            let id = text(c, "id");
            let name = text(c, "name");
            let (found, added, current) = (c["found"] == json!(true), c["added"] == json!(true), c["current"] == json!(true));
            let (state, acts) = if added && current {
                ((Tone::Ok, "connected".to_string()), vec![Act::new("Disconnect", "agent_client.remove", json!({ "client": id })).ask(format!("Disconnect {name}?"), format!("{name} can no longer use devd. Its other settings are untouched."), "Disconnect")])
            } else if added {
                ((Tone::Warn, "points at an old devd".to_string()), vec![Act::new("Reconnect", "agent_client.add", json!({ "client": id })).ask(format!("Reconnect {name} to devd?"), format!("devd updates only its own entry in {name}'s configuration and keeps a copy of the old file. Restart {name} to pick up the change."), "Reconnect")])
            } else if found {
                ((Tone::Neutral, "not connected".to_string()), vec![Act::new("Connect", "agent_client.add", json!({ "client": id })).ask(format!("Let {name} use devd?"), format!("devd adds one entry to {name}'s configuration and keeps a copy of the old file. Everything else is untouched. Restart {name} to pick up the change."), "Connect")])
            } else {
                ((Tone::Neutral, "not installed".to_string()), vec![])
            };
            Row { mark: Some(id.clone()), title: name, sub: c["problem"].as_str().map(str::to_string).unwrap_or_else(|| text(c, "config")), sub_is_path: c["problem"].is_null(), state: Some(state), acts, ..Row::default() }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Looking for coding agents…".to_string()
    } else {
        format!(
            "devd is connected to {connected} of {} coding agents on this machine.",
            clients.len()
        )
    };
    let body = vec![
        list_or(
            &snap,
            rows,
            "connect.row",
            "team",
            "No coding agents found",
            "Install Claude Code, Cursor or another coding agent and it is listed here.",
            cx,
        )
        .into_any_element(),
    ];
    frame("connect", detail, div(), &[&snap], body, cx)
}

fn flags(cx: &mut Context<Shell>) -> AnyElement {
    let snap = read(cx, "flags.list");
    let rows: Vec<Row> = snap
        .list("")
        .iter()
        .map(|f| {
            let key = text(f, "key");
            let source = text(f, "source");
            let is_bool = f["value"].is_boolean();
            let on = f["value"] == json!(true);
            let value = if is_bool {
                if on {
                    "on".to_string()
                } else {
                    "off".to_string()
                }
            } else {
                f["value"].to_string()
            };
            let mut acts = Vec::new();
            if is_bool {
                acts.push(Act::new(
                    if on { "Turn off" } else { "Turn on" },
                    "flags.set",
                    json!({ "key": key, "value": if on { "off" } else { "on" } }),
                ));
            }
            if source == "setting" {
                acts.push(Act::new(
                    "Use the default",
                    "flags.set",
                    json!({ "key": key }),
                ));
            }
            let ignored = f["ignored"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .map(str::to_string);
            // A number cannot be toggled, so it gets a small form instead of
            // being the one setting on this page a person cannot change.
            let form = (!is_bool).then(|| FormSpec {
                title: "Set a flag",
                intro: "Type a whole number. Leave it empty to go back to the default.",
                verb: "Save",
                capability: "flags.set",
                domain: "flags",
                fixed: json!({ "key": key }),
                fields: vec![Field::optional("value", "Value", "3")],
                busy: None,
            });
            Row {
                title: key,
                tag: value,
                sub: ignored.unwrap_or_else(|| text(f, "summary")),
                state: Some(match source.as_str() {
                    "environment" => (Tone::Info, "set in the environment".to_string()),
                    "setting" => (Tone::Info, "changed on this machine".to_string()),
                    _ => (Tone::Neutral, "the default".to_string()),
                }),
                acts,
                form,
                form_label: "Set value",
                ..Row::default()
            }
        })
        .collect();
    let detail = if snap.is_first_load() {
        "Reading the flags…".to_string()
    } else {
        "Switches for new behaviour. A value set in the environment wins over one changed here."
            .to_string()
    };
    let body = vec![
        list_or(
            &snap,
            rows,
            "flags.row",
            "settings",
            "No flags",
            "Switches for new behaviour are listed here.",
            cx,
        )
        .into_any_element(),
    ];
    frame("flags", detail, div(), &[&snap], body, cx)
}
