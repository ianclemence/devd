//! Home: what needs you, then what you were doing, then the server.
//!
//! In that order, because that is the order of the questions a person arrives
//! with. Anything waiting on a decision is first and is never below the fold;
//! the message box is second because most visits end in asking for something.

use devd_app_core::nav::Route;
use gpui::{div, prelude::*, px, AnyElement, ClickEvent, Context, Div, FontWeight, Window};
use serde_json::{json, Value};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{mono, size, theme, Tone};
use crate::ui::{self, card, eyebrow, icon, pill, rule};

/// The screen.
pub fn render(shell: &mut Shell, _window: &mut Window, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let sync = devd(cx).sync.clone();
    let health = sync.watch(&q::health());
    let threads = sync.watch(&q::threads());
    let services = sync.watch(&q::services());
    let sites = sync.watch(&q::sites());
    let inbox = sync.watch(&q::inbox());

    let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
    let greeting = match hour {
        5..=11 => "Good morning.",
        12..=17 => "Good afternoon.",
        _ => "Good evening.",
    };
    let open: Vec<&Value> = inbox.list("").iter().filter(|m| m["state"] == json!("open")).collect();
    let headline = match (open.len(), health.data.as_deref()) {
        (0, Some(data)) => data["headline"].as_str().unwrap_or("Everything is in order.").to_string(),
        (0, None) => "Reading the machine…".to_string(),
        (1, _) => "One thing is waiting for you.".to_string(),
        (n, _) => format!("{n} things are waiting for you."),
    };

    let mut column = div().flex().flex_col().gap(px(26.0));
    column = column.child(
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(div().text_size(px(size::TEXT_DISPLAY)).font_weight(FontWeight::SEMIBOLD).child(greeting))
            .child(div().text_size(px(size::TEXT_LG)).text_color(t.muted).child(headline)),
    );

    // What needs you.
    if !open.is_empty() {
        let mut list = card(cx).flex().flex_col().overflow_hidden();
        for (n, message) in open.iter().take(4).enumerate() {
            if n > 0 {
                list = list.child(rule(cx));
            }
            let run = message["run_id"].as_str().map(str::to_string);
            let kind = message["kind"].as_str().unwrap_or("message");
            // A finished run is news, not a problem; only what blocks work is a warning.
            let (tone, mark) = match kind {
                "finished" | "done" => (Tone::Ok, "check"),
                "paused" | "blocked" | "approval" | "question" => (Tone::Warn, "alert"),
                _ => (Tone::Info, "info"),
            };
            let hover = t.hover;
            list = list.child(
                div()
                    .id(ui::nid("home.needs", n))
                    .px(px(14.0))
                    .py(px(11.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(icon(mark, 15.0, t.tone(tone)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(div().truncate().font_weight(FontWeight::MEDIUM).child(message["subject"].as_str().unwrap_or_default().to_string()))
                            .child(div().truncate().text_size(px(size::TEXT_SM)).text_color(t.muted).child(message["body"].as_str().unwrap_or_default().lines().next().unwrap_or_default().to_string())),
                    )
                    .child(pill(kind.to_string(), tone, cx))
                    .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::ago(message["created_at"].as_str().unwrap_or_default())))
                    .child(icon("chevron-right", 13.0, t.faint))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| match &run {
                        Some(run) => this.go(Route::Runs(Some(run.clone())), cx),
                        None => this.go(Route::Inbox, cx),
                    })),
            );
        }
        column = column.child(div().flex().flex_col().gap(px(8.0)).child(eyebrow("NEEDS YOU", cx)).child(list));
    }

    // Ask.
    column = column.child(
        div()
            .min_h(px(48.0))
            .px(px(14.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(size::RADIUS_L))
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .text_size(px(size::TEXT_MD))
            .line_height(px(22.0))
            .child(icon("assistant", 16.0, t.accent))
            .child(shell.composer.clone())
            .child(ui::kbd("enter", cx)),
    );

    // Recent work and the machine, side by side.
    let mut recent = div().flex().flex_col().gap(px(1.0));
    let own: Vec<&Value> = threads.list("").iter().filter(|th| !th["team"]["role"].is_string()).take(6).collect();
    if threads.is_first_load() {
        for n in 0..4 {
            recent = recent.child(div().h(px(size::ROW)).flex().items_center().child(ui::skeleton(ui::nid("home.recent", n), 240.0, cx)));
        }
    } else if own.is_empty() {
        recent = recent.child(div().py(px(10.0)).text_color(t.muted).child("No conversations yet. Ask something above."));
    }
    for thread in own {
        let id = thread["id"].as_str().unwrap_or_default().to_string();
        let said = devd_app_core::status::of(thread["state"].as_str().unwrap_or_default());
        let (tone, word): (Tone, String) = (said.level.into(), said.label.clone());
        let go = id.clone();
        let mut line = ui::row(gpui::ElementId::Name(format!("home.thread.{id}").into()), false, cx)
            .child(ui::dot(tone, cx))
            .child(div().flex_1().min_w_0().truncate().child(thread["title"].as_str().unwrap_or("Conversation").to_string()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(Route::Assistant(Some(go.clone())), cx)));
        if !word.is_empty() {
            line = line.child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(tone)).child(word));
        }
        recent = recent.child(
            line.child(div().w(px(110.0)).flex_none().truncate().text_size(px(size::TEXT_SM)).text_color(t.faint).child(thread["project"].as_str().map(|p| ui::leaf(p).to_string()).unwrap_or_default()))
                .child(div().w(px(30.0)).flex_none().flex().justify_end().text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::ago(thread["updated_at"].as_str().unwrap_or_default()))),
        );
    }

    let installed: Vec<&Value> = services.list("").iter().filter(|s| s["installed"] == json!(true)).collect();
    let ready = installed.iter().filter(|s| s["ready"] == json!(true)).count();
    let serving = sites.list("").iter().filter(|s| s["server_running"] == json!(true) && s["present"] == json!(true)).count();
    let unpublished = sites.list("").iter().filter(|s| s["in_hosts"] == json!(false)).count();
    let reading = |label: &'static str, value: String, detail: String, tone: Tone| -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(div().flex().items_center().gap(px(6.0)).child(ui::dot(tone, cx)).child(eyebrow(label, cx)))
            .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::MEDIUM).child(value))
            .child(div().text_size(px(size::TEXT_SM)).text_color(t.muted).child(detail))
    };
    let machine = card(cx)
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(reading(
            "SERVICES",
            if services.is_first_load() { "…".into() } else { format!("{ready} of {} answering", installed.len()) },
            installed.iter().filter(|s| s["ready"] == json!(true)).filter_map(|s| s["name"].as_str()).take(4).collect::<Vec<_>>().join(", "),
            if installed.is_empty() { Tone::Neutral } else if ready == installed.len() { Tone::Ok } else { Tone::Warn },
        ))
        .child(rule(cx))
        .child(reading(
            "SITES",
            if sites.is_first_load() { "…".into() } else { format!("{serving} of {} serving", sites.list("").len()) },
            if unpublished > 0 { format!("{unpublished} hostname{} not published yet", if unpublished == 1 { "" } else { "s" }) } else { "every hostname resolves".to_string() },
            if unpublished > 0 { Tone::Warn } else { Tone::Ok },
        ))
        .child(rule(cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                // The folder can be long; it gives way to the button, keeping its end.
                .child(div().flex_1().min_w_0().truncate().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(ui::tail(&devd(cx).conn.home, 24)))
                .child(ui::Button::new("home.server", "Open Server").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Route::Server, cx)))),
        );

    column = column.child(
        div()
            .flex()
            .items_start()
            .gap(px(28.0))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(8.0)).child(eyebrow("RECENT WORK", cx)).child(recent))
            .child(div().w(px(320.0)).flex_none().flex().flex_col().gap(px(8.0)).child(eyebrow("THE SERVER", cx)).child(machine)),
    );

    ui::page("home", 980.0, column).into_any_element()
}
