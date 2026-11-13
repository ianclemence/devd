//! The screens that are, at heart, a list of things with a state: the inbox,
//! runs, projects, sites, the server and the team.
//!
//! They share one row: a state dot, a name, one supporting line, the facts that
//! matter on the right, and actions that appear under the pointer. A stopped
//! service and a paused run look like relatives because they are.

use devd_app_core::nav::Route;
use devd_app_core::status;
use devd_app_core::sync::{Mutation, Patch, Snapshot};
use gpui::{div, prelude::*, px, AnyElement, App, ClickEvent, Context, Div, FontWeight};
use serde_json::{json, Value};

use crate::screens::roles as screens_roles;
use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{mono, size, theme, Tone};
use crate::ui::{self, card, eyebrow, icon, live_dot, page_head, pill, rule, Button, ButtonKind};

/// What to show in place of a list that has not arrived, failed, or is empty.
fn placeholder(snap: &Snapshot, name: &'static str, empty_icon: &'static str, empty_title: &'static str, empty_detail: &'static str, cx: &App) -> Option<Div> {
    if snap.is_first_load() {
        let mut column = div().flex().flex_col().gap(px(18.0)).py(px(12.0));
        for n in 0..6 {
            column = column.child(ui::skeleton(ui::nid(name, n), 260.0 + ((n * 67) % 180) as f32, cx));
        }
        return Some(column);
    }
    if let (Some(failure), true) = (&snap.error, snap.data.is_none()) {
        return Some(ui::empty("alert", "devd could not answer", failure.message.clone(), cx));
    }
    snap.list("").is_empty().then(|| ui::empty(empty_icon, empty_title, empty_detail, cx))
}

/// Approvals, questions and messages waiting for the person.
pub fn inbox(_shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::inbox());
    let open = snap.list("").iter().filter(|m| m["state"] == json!("open")).count();
    // A decision and a piece of news are different things to be told about.
    let decisions = snap.list("").iter().filter(|m| status::inbox(m["kind"].as_str().unwrap_or_default(), m["state"] == json!("open")).needs_you).count();
    let column = div().flex().flex_col().child(page_head(
        "Inbox",
        match (decisions, open - decisions) {
            (0, 0) => "Nothing is waiting for you.".to_string(),
            (0, news) => format!("Nothing needs a decision. {news} to read."),
            (n, 0) => format!("{n} waiting for a decision."),
            (n, news) => format!("{n} waiting for a decision · {news} to read."),
        },
        div(),
        cx,
    ));
    if let Some(shown) = placeholder(&snap, "inbox", "inbox", "Nothing here", "When an agent needs a decision, has a question, or proposes work, it arrives here.", cx) {
        return ui::page("inbox", 860.0, column.child(shown)).into_any_element();
    }
    let mut list = card(cx).flex().flex_col().overflow_hidden();
    let mut primary_given = false;
    for (n, message) in snap.list("").iter().enumerate() {
        if n > 0 {
            list = list.child(rule(cx));
        }
        let is_open = message["state"] == json!("open");
        let said = status::inbox(message["kind"].as_str().unwrap_or_default(), is_open);
        let tone: Tone = said.level.into();
        // One thing on a screen is the thing to do: the first message that is
        // waiting on the person. News about finished work does not compete with it.
        let asked = primary_given;
        primary_given |= said.needs_you;
        let run = message["run_id"].as_str().map(str::to_string);
        let id = message["id"].as_str().unwrap_or_default().to_string();
        let mut actions = div().flex().flex_none().gap(px(6.0));
        if let Some(run) = run {
            actions = actions.child(Button::new(ui::nid("inbox.open", n), "Open the run").kind(if said.needs_you && !asked { ButtonKind::Primary } else { ButtonKind::Secondary }).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Runs(Some(run.clone())), cx))));
        }
        if is_open {
            let dismiss = id.clone();
            actions = actions.child(Button::new(ui::nid("inbox.done", n), "Mark answered").kind(ButtonKind::Ghost).on_click(move |_, _, cx| {
                // The row leaves the open list at once; devd confirms after.
                devd(cx).sync.mutate(
                    Mutation::new("inbox.reply", json!({ "id": dismiss })).optimistic(Patch::update_row(q::inbox(), "", "id", json!(dismiss), json!({ "state": "answered" }))),
                );
            }));
        }
        list = list.child(
            div()
                .px(px(16.0))
                .py(px(13.0))
                .flex()
                .items_start()
                .gap(px(12.0))
                .opacity(if is_open { 1.0 } else { 0.55 })
                .child(div().pt(px(5.0)).child(ui::dot(tone, cx)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(div().min_w_0().font_weight(FontWeight::MEDIUM).child(ui::Clip::new(message["subject"].as_str().unwrap_or_default().to_string())))
                                .child(pill(said.label.clone(), tone, cx)),
                        )
                        // The opening paragraph says what happened; the rest is on the run.
                        .child(div().line_clamp(3).text_color(t.muted).child(message["body"].as_str().unwrap_or_default().split("

").find(|p| !p.trim().is_empty()).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")))
                        .child(
                            div()
                                .flex()
                                .gap(px(10.0))
                                .text_size(px(size::TEXT_XS))
                                .text_color(t.faint)
                                .child(message["project"].as_str().map(|p| ui::leaf(p).to_string()).unwrap_or_default())
                                .child(message["from_actor"].as_str().unwrap_or_default().to_string())
                                .child(ui::ago(message["created_at"].as_str().unwrap_or_default())),
                        ),
                )
                .child(actions),
        );
    }
    ui::page("inbox", 860.0, column.child(list)).into_any_element()
}

/// Team runs, and the one that is open.
pub fn runs(_shell: &mut Shell, selected: Option<String>, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::runs());
    let mut side = div().flex().flex_col().gap(px(1.0)).p(px(8.0));
    if let Some(shown) = placeholder(&snap, "runs", "runs", "No runs yet", "Give the team a goal from a conversation and it appears here with its plan.", cx) {
        side = side.child(div().px(px(8.0)).child(shown));
    }
    for run in snap.list("") {
        let id = run["id"].as_str().unwrap_or_default().to_string();
        let said = status::of(run["state"].as_str().unwrap_or_default());
        let marker = ui::state_mark(gpui::ElementId::Name(format!("run.live.{id}").into()), &said, cx);
        let go = id.clone();
        side = side.child(
            ui::row(gpui::ElementId::Name(format!("run.{id}").into()), selected.as_deref() == Some(id.as_str()), cx)
                .h(px(48.0))
                .child(marker)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().child(ui::Clip::new(run["title"].as_str().unwrap_or("Run").to_string())))
                        .child(
                            div().flex().gap(px(8.0)).text_size(px(size::TEXT_XS)).text_color(t.faint).child(run["project"].as_str().map(|p| ui::leaf(p).to_string()).unwrap_or_default()).child(said.label.clone()).child(ui::ago(run["updated_at"].as_str().unwrap_or_default())),
                        ),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Runs(Some(go.clone())), cx))),
        );
    }
    let sidebar = div()
        .w(px(size::SIDEBAR + 32.0))
        .h_full()
        .flex()
        .flex_col()
        .flex_none()
        .border_r_1()
        .border_color(t.line)
        .child(div().h(px(44.0)).px(px(14.0)).flex().flex_none().items_center().font_weight(FontWeight::MEDIUM).child("Runs"))
        .child(div().id("runs.list").flex_1().overflow_y_scroll().child(side));

    let detail: AnyElement = match selected {
        None => div().flex_1().child(ui::empty("runs", "Choose a run", "Its plan, each task's owner and state, and what is blocking it.", cx)).into_any_element(),
        Some(id) => run_detail(&id, cx),
    };
    div().size_full().flex().child(sidebar).child(detail).into_any_element()
}

fn run_detail(id: &str, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::run(id));
    let Some(data) = snap.data.clone() else {
        let body = match &snap.error {
            Some(failure) => ui::empty("alert", "This run could not be read", failure.message.clone(), cx),
            None => div().p(px(32.0)).flex().flex_col().gap(px(16.0)).children((0..5).map(|n| ui::skeleton(ui::nid("run.skeleton", n), 240.0 + n as f32 * 50.0, cx))),
        };
        return div().flex_1().child(body).into_any_element();
    };
    // `team.get` nests the run under `run` in some versions and returns it flat in others.
    let run = if data["run"].is_object() { &data["run"] } else { &data };
    let tasks = data["tasks"].as_array().or(run["tasks"].as_array()).map(Vec::as_slice).unwrap_or(&[]);
    let said = status::of(run["state"].as_str().unwrap_or_default());
    let (tone, word): (Tone, String) = (said.level.into(), said.label.clone());
    let done = tasks.iter().filter(|task| matches!(task["state"].as_str(), Some("done" | "verified" | "accepted"))).count();

    let mut actions = div().flex().gap(px(8.0));
    let state = run["state"].as_str().unwrap_or_default().to_string();
    if state == "paused" {
        let run_id = id.to_string();
        actions = actions.child(Button::new("run.resume", "Resume").icon("play").kind(ButtonKind::Primary).on_click(move |_, _, cx| {
            devd(cx).sync.mutate(Mutation::new("team.resume", json!({ "run": run_id })).optimistic(Patch::set(q::run(&run_id), "/run/state", json!("running"))));
        }));
    }
    if let Some(thread) = run["thread_id"].as_str().map(str::to_string) {
        actions = actions.child(Button::new("run.thread", "Open the conversation").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Assistant(Some(thread.clone())), cx))));
    }

    let mut column = div().flex().flex_col().gap(px(20.0)).child(page_head(
        run["title"].as_str().unwrap_or("Run").to_string(),
        format!("{} · {done} of {} tasks done · {}", run["project"].as_str().map(ui::leaf).unwrap_or("no project"), tasks.len(), ui::ago(run["updated_at"].as_str().unwrap_or_default())),
        actions,
        cx,
    ));
    let mut facts = div().flex().items_center().gap(px(10.0)).child(pill(word, tone, cx));
    if let Some(verdict) = run["verdict"].as_str() {
        facts = facts.child(pill(format!("verdict: {verdict}"), if verdict == "approve" { Tone::Ok } else { Tone::Warn }, cx));
    }
    if let Some(commit) = run["reviewed_commit"].as_str() {
        facts = facts.child(div().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(format!("reviewed {}", &commit[..commit.len().min(10)])));
    }
    column = column.child(facts);
    if let Some(error) = run["error"].as_str().filter(|e| !e.is_empty()) {
        column = column.child(
            div().p(px(12.0)).flex().items_start().gap(px(9.0)).rounded(px(size::RADIUS_L)).border_1().border_color(ui::tint(t.warn, 0.35)).bg(ui::tint(t.warn, 0.07)).child(icon("alert", 15.0, t.warn)).child(div().flex_1().min_w_0().child(error.to_string())),
        );
    }
    column = column.child(div().flex().flex_col().gap(px(6.0)).child(eyebrow("GOAL", cx)).child(div().line_clamp(6).text_color(t.muted).child(run["goal"].as_str().unwrap_or_default().to_string())));

    let mut list = card(cx).flex().flex_col().overflow_hidden();
    for (n, task) in tasks.iter().enumerate() {
        if n > 0 {
            list = list.child(rule(cx));
        }
        let said = status::of(task["state"].as_str().unwrap_or_default());
        let (tone, word): (Tone, String) = (said.level.into(), said.label.clone());
        let marker = ui::state_mark(ui::nid("task.live", n), &said, cx);
        let thread = task["thread_id"].as_str().map(str::to_string);
        let mut line = div()
            .id(ui::nid("task", n))
            .px(px(14.0))
            .py(px(10.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(marker)
            .child(div().w(px(104.0)).flex_none().text_size(px(size::TEXT_SM)).text_color(t.muted).child(ui::Clip::new(task["role"].as_str().unwrap_or_default().to_string())))
            .child(div().flex_1().min_w_0().child(ui::Clip::new(task["title"].as_str().unwrap_or_default().to_string())))
            .child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(tone)).child(word));
        if let Some(thread) = thread {
            let hover = t.hover;
            line = line.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Assistant(Some(thread.clone())), cx)));
        }
        list = list.child(line);
    }
    if !tasks.is_empty() {
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("TASKS", cx)).child(list));
    }
    if let Some(summary) = run["summary"].as_str().filter(|s| !s.is_empty()) {
        let blocks = devd_app_core::markdown::parse(summary);
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("REPORT", cx)).child(div().text_size(px(size::TEXT_MD)).line_height(px(22.0)).child(ui::markdown::document("run.summary", &blocks, cx))));
    }
    div().flex_1().min_w_0().h_full().child(ui::page("run.detail", 860.0, column)).into_any_element()
}

/// Every project devd knows.
pub fn projects(_shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    // The project hub lives with the other pages so it can share their rows,
    // dialogs and task buttons. This screen is the area's first tab.
    super::pages::projects_hub(cx)
}

/// Addresses, and what answers at each.
pub fn sites(_shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::sites());
    let all = snap.list("");
    let unpublished = all.iter().filter(|s| s["in_hosts"] == json!(false)).count();
    let actions = if unpublished > 0 {
        div().child(Button::new("sites.publish", format!("Publish {unpublished} hostname{}", if unpublished == 1 { "" } else { "s" })).kind(ButtonKind::Primary).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.run("server.hosts", window, cx))))
    } else {
        div()
    };
    let column = div().flex().flex_col().child(page_head("Sites", format!("{} addresses on this machine.", all.len()), actions, cx)).children(super::pages::tab_strip(&Route::Sites(None), cx)).children(ui::stale(&snap, cx));
    if let Some(shown) = placeholder(&snap, "sites", "globe", "No sites yet", "Create a site and it gets a .test name and a certificate.", cx) {
        return ui::page("sites", 980.0, column.child(shown)).into_any_element();
    }
    let mut list = div().flex().flex_col().gap(px(1.0));
    for site in all {
        let name = site["site"]["name"].as_str().unwrap_or_default().to_string();
        let url = site["url"].as_str().unwrap_or_default().to_string();
        let present = site["present"] == json!(true);
        let up = site["server_running"] == json!(true) && present;
        let serving = site["serving"].as_str().unwrap_or("nothing");
        let mut line = ui::row(gpui::ElementId::Name(format!("site.{name}").into()), false, cx)
            .h(px(46.0))
            .child(ui::dot(if up { Tone::Ok } else if present { Tone::Neutral } else { Tone::Warn }, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().child(ui::Clip::new(site["site"]["host"].as_str().unwrap_or(&name).to_string())))
                    .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::Clip::new(site["detected"].as_str().unwrap_or_default().to_string()))),
            )
            .child(pill(match serving { "no_entry" => "no entry point".to_string(), other => other.replace('_', " ") }, Tone::Neutral, cx));
        if let Some(php) = site["php_effective"].as_str() {
            line = line.child(pill(format!("PHP {php}"), Tone::Neutral, cx));
        }
        if site["in_hosts"] == json!(false) {
            line = line.child(pill("not in hosts", Tone::Warn, cx));
        }
        if site["certificate"] == json!(false) {
            // Every .test site has this until HTTPS is set up, so it is said quietly.
            line = line.child(pill("no certificate", Tone::Neutral, cx));
        }
        if !present {
            line = line.child(pill("folder missing", Tone::Warn, cx));
        }
        line = line.child(Button::icon_only(gpui::ElementId::Name(format!("site.open.{name}").into()), "external").on_click(move |_, _, cx| cx.open_url(&url)));
        list = list.child(line);
    }
    ui::page("sites", 980.0, column.child(list)).into_any_element()
}

/// The server: its services, and the checks behind the health line.
pub fn server(_shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let services = devd(cx).sync.watch(&q::services());
    let health = devd(cx).sync.watch(&q::health());
    let headline = health.data.as_deref().and_then(|d| d["headline"].as_str()).unwrap_or("Reading the machine…").to_string();
    let actions = div().child(Button::new("server.start", "Start what is needed").icon("play").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.run("server.start", window, cx))));
    let mut column = div().flex().flex_col().gap(px(22.0)).child(div().child(page_head("Server", headline, actions, cx)).children(super::pages::tab_strip(&Route::Server, cx))).children(ui::stale(&services, cx)).children(ui::stale(&health, cx));

    let mut list = card(cx).flex().flex_col().overflow_hidden();
    if let Some(shown) = placeholder(&services, "services", "machine", "No services", "Install a web server or a database and it is listed here.", cx) {
        list = list.child(div().px(px(16.0)).child(shown));
    }
    // A PHP version runs as several identical workers. They are one thing to a
    // person, so they are one row: the first worker stands for the pool.
    let pool = |name: &str| name.strip_prefix("php-cgi-").map(|rest| rest.split('-').next().unwrap_or(rest).to_string());
    let all: Vec<&Value> = services.list("").iter().filter(|s| s["installed"] == json!(true)).collect();
    let workers = |version: &str| all.iter().filter(|s| pool(s["name"].as_str().unwrap_or_default()).as_deref() == Some(version)).count();
    let answering = |version: &str| all.iter().filter(|s| pool(s["name"].as_str().unwrap_or_default()).as_deref() == Some(version) && s["ready"] == json!(true)).count();
    let installed: Vec<&Value> = all
        .iter()
        .copied()
        .filter(|s| {
            let name = s["name"].as_str().unwrap_or_default();
            // Keep `php-cgi-8.4`, drop `php-cgi-8.4-2` and the rest.
            pool(name).is_none_or(|version| name == format!("php-cgi-{version}"))
        })
        .collect();
    for (n, service) in installed.iter().enumerate() {
        if n > 0 {
            list = list.child(rule(cx));
        }
        let name = service["name"].as_str().unwrap_or_default().to_string();
        let version = pool(&name);
        let running = service["running"] == json!(true);
        let ready = match &version {
            Some(version) => answering(version) == workers(version),
            None => service["ready"] == json!(true),
        };
        let title = version.as_ref().map(|v| format!("PHP {v}")).unwrap_or_else(|| name.clone());
        let detail = match &version {
            Some(version) => format!("{} of {} workers answering FastCGI", answering(version), workers(version)),
            None => service["ready_detail"].as_str().or(service["message"].as_str()).unwrap_or_default().replace('`', ""),
        };
        let starting = service["starting"] == json!(true);
        let (tone, word) = match (running, ready, starting) {
            (_, _, true) => (Tone::Info, "starting"),
            (true, true, _) => (Tone::Ok, "answering"),
            (true, false, _) => (Tone::Warn, "running, not answering"),
            (false, _, _) => (Tone::Neutral, "stopped"),
        };
        let marker: AnyElement = if starting { live_dot(ui::nid("service.live", n), tone, cx).into_any_element() } else { ui::dot(tone, cx).into_any_element() };
        let (capability, label, button_icon) = if running { ("service.stop", "Stop", "stop") } else { ("service.start", "Start", "play") };
        let target = name.clone();
        list = list.child(
            div()
                .px(px(14.0))
                .h(px(52.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(marker)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().flex().items_center().gap(px(8.0)).child(div().font_weight(FontWeight::MEDIUM).child(title)).child(div().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(service["version"].as_str().unwrap_or_default().to_string())))
                        .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::Clip::new(detail))),
                )
                .child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(tone)).child(word))
                .child(div().w(px(64.0)).flex_none().flex().justify_end().font_family(mono()).text_size(px(size::TEXT_SM)).text_color(t.muted).child(service["port"].as_u64().map(|p| format!(":{p}")).unwrap_or_default()))
                .child(Button::new(gpui::ElementId::Name(format!("service.toggle.{name}").into()), label).icon(button_icon).disabled(starting).on_click(move |_, _, cx| {
                    // The row says "starting" or "stopped" at once; the health
                    // probe decides what it says next.
                    let shown = if capability == "service.start" { json!({ "starting": true }) } else { json!({ "running": false, "ready": false }) };
                    devd(cx).sync.mutate(
                        Mutation::new(capability, json!({ "name": target })).optimistic(Patch::update_row(q::services(), "", "name", json!(target), shown)),
                    );
                })),
        );
    }
    column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("SERVICES", cx)).child(list));

    if let Some(data) = health.data.as_deref() {
        let mut checks = card(cx).flex().flex_col().overflow_hidden();
        for (n, check) in data["checks"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
            if n > 0 {
                checks = checks.child(rule(cx));
            }
            let tone: Tone = status::check(check["level"].as_str().unwrap_or_default()).into();
            checks = checks.child(
                div()
                    .px(px(14.0))
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(ui::dot(tone, cx))
                    .child(div().w(px(200.0)).flex_none().child(ui::Clip::new(check["label"].as_str().unwrap_or_default().to_string())))
                    .child(div().flex_1().min_w_0().text_size(px(size::TEXT_SM)).text_color(t.muted).child({
                        let detail = check["detail"].as_str().unwrap_or_default().to_string();
                        // A line about a folder is told apart by its end; a sentence by its start.
                        if detail.contains(":\\") || detail.starts_with('/') { ui::Clip::tail(detail) } else { ui::Clip::new(detail) }
                    })),
            );
        }
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("CHECKS", cx)).child(checks));
    }
    ui::page("server", 980.0, column).into_any_element()
}

/// The team: every specialist, what it is for, what it must prove, and
/// whether it is busy right now.
pub fn team(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let roles = devd(cx).sync.watch(&q::roles());
    let overview = devd(cx).sync.watch(&q::overview());
    let busy = overview.data.as_deref().map(|d| d["roles"].clone()).unwrap_or(Value::Null);
    let working: i64 = busy.as_object().map(|m| m.values().map(|r| r["working"].as_i64().unwrap_or(0)).sum()).unwrap_or(0);
    let detail = match (roles.list("").len(), working) {
        (0, _) => "Reading the roster…".to_string(),
        (n, 0) => format!("{n} specialists. Nobody is working right now."),
        (n, w) => format!("{n} specialists · {w} working now"),
    };
    let add = div().child(Button::new("team.add", "Add a member").icon("plus").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
        this.role_form.open = !this.role_form.open;
        cx.notify();
    })));
    let mut column = div().flex().flex_col().gap(px(22.0)).child(div().child(page_head("Team", detail, add, cx)).children(super::pages::tab_strip(&Route::Team, cx))).children(ui::stale(&roles, cx));
    if let Some(form) = screens_roles::render(shell, cx) {
        column = column.child(form);
    }
    if let Some(shown) = placeholder(&roles, "team", "team", "No roles", "The team's roles could not be read.", cx) {
        return ui::page("team", 980.0, column.child(shown)).into_any_element();
    }
    // Grouped the way devd groups them: by the part of the work they own.
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for role in roles.list("") {
        let group = role["team_title"].as_str().unwrap_or("Team").to_string();
        match groups.iter_mut().find(|(name, _)| *name == group) {
            Some((_, members)) => members.push(role),
            None => groups.push((group, vec![role])),
        }
    }
    let mut column = column;
    let mut n = 0;
    for (group, members) in groups {
        let mut list = card(cx).flex().flex_col().overflow_hidden();
        for (i, role) in members.iter().enumerate() {
            if i > 0 {
                list = list.child(rule(cx));
            }
            n += 1;
            let id = role["id"].as_str().unwrap_or_default();
            let title = role["title"].as_str().unwrap_or(id).to_string();
            let load = &busy[id];
            let (now_working, now_waiting, now_blocked) = (load["working"].as_i64().unwrap_or(0), load["waiting"].as_i64().unwrap_or(0), load["blocked"].as_i64().unwrap_or(0));
            let state = if now_blocked > 0 {
                status::of("blocked")
            } else if now_waiting > 0 {
                status::of("waiting_approval")
            } else if now_working > 0 {
                status::of("working")
            } else {
                status::of("idle")
            };
            let initials: String = title.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
            // What must be on record before this role's work is accepted.
            let proofs: Vec<&str> = [("change", "a change"), ("verify", "a passing check"), ("read", "what it read"), ("command", "a command run")]
                .iter()
                .filter(|(key, _)| role["must_prove"][*key] == json!(true))
                .map(|(_, word)| *word)
                .collect();
            let model = role["models"].as_array().and_then(|m| m.first()).and_then(|m| m["model"].as_str()).map(str::to_string);
            let mut facts = div().flex().flex_none().items_center().gap(px(6.0));
            // The most it may do, in words. Nearly every role changes files, so that is not worth a pill; a role that cannot, or can run commands, is.
            let may = match role["max_tier"].as_str().unwrap_or_default() {
                "read" => Some("reads only"),
                "exec" => Some("runs commands"),
                t if t.starts_with("admin") => Some("asks first"),
                _ => None,
            };
            if let Some(may) = may {
                facts = facts.child(pill(may, Tone::Neutral, cx));
            }
            if role["works_in_own_copy"] == json!(true) {
                facts = facts.child(pill("own copy", Tone::Neutral, cx));
            }
            if role["built_in"] == json!(false) {
                facts = facts.child(screens_roles::yours(id, n, cx));
            }
            let mut second = div().flex().gap(px(10.0)).text_size(px(size::TEXT_XS)).text_color(t.faint);
            if let Some(model) = model {
                second = second.child(div().font_family(mono()).child(model));
            }
            let next: Vec<&str> = role["hands_to"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if !next.is_empty() {
                second = second.child(format!("hands to {}", next.join(", ")));
            }
            if !proofs.is_empty() {
                second = second.child(format!("must show {}", proofs.join(", ")));
            }
            list = list.child(
                div()
                    .px(px(14.0))
                    .py(px(11.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(ui::face(initials, state.level.into(), state.live, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(div().font_weight(FontWeight::MEDIUM).child(title))
                            .child(div().line_clamp(2).text_size(px(size::TEXT_SM)).text_color(t.muted).child(role["summary"].as_str().unwrap_or_default().to_string()))
                            .child(second),
                    )
                    .child(facts)
                    .child(div().w(px(150.0)).flex_none().flex().items_center().justify_end().gap(px(7.0)).child(ui::state_mark(ui::nid("team.state", n), &state, cx)).child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(state.level.into())).child(if state.label.is_empty() { "Idle".to_string() } else { state.label.clone() }))),
            );
        }
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow(group.to_uppercase(), cx)).child(list));
    }
    ui::page("team", 980.0, column).into_any_element()
}
