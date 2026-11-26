//! Activity and Settings: what was done on this machine and by whom, and how
//! this window is set up.
//!
//! Activity is the audit trail, read as sentences. Each row says who did what
//! to what, in the same words a conversation uses for the same call, and the
//! ones that failed are marked. Settings is deliberately short: appearance,
//! which devd this window talks to, and the policy that bounds what agents may
//! spend and where they may send a project.

use devd_app_core::toolname::{self, Phase};
use gpui::{AnyElement, Context, Div, FontWeight, div, prelude::*, px};
use serde_json::{Value, json};

use crate::shell::Shell;
use crate::state::{devd, q};
use crate::theme::{Tone, mono, size, theme};
use crate::ui::{self, Button, card, eyebrow, page_head, pill, rule};

/// Who made a call, as a person would say it.
pub fn who(actor: &str) -> (String, Tone) {
    let (kind, name) = actor.split_once(':').unwrap_or((actor, ""));
    match kind {
        // Who did it is not a link and not a selection: it is said in the muted text colour like every other fact.
        "human" => (
            match name {
                "cli" => "You, in the terminal".to_string(),
                "http" | "ui" | "app" | "" => "You".to_string(),
                other => format!("You ({other})"),
            },
            Tone::Neutral,
        ),
        "agent" => (
            format!("Agent {}", name.trim_start_matches("devd-")),
            Tone::Neutral,
        ),
        "system" => ("devd".to_string(), Tone::Neutral),
        _ => (actor.to_string(), Tone::Neutral),
    }
}

/// The audit trail.
pub fn activity(shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let snap = devd(cx).sync.watch(&q::activity());
    let everything = snap.list("/items");
    // What changed something is the record people come here for. Looking at things
    // (a screen reading its own data, a window saying where it is) would bury it.
    let shows_reads = shell.activity_reads;
    let rows: Vec<&Value> = everything
        .iter()
        .filter(|r| {
            shows_reads
                || (r["effect"].as_str() != Some("read")
                    && !r["capability"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("app."))
        })
        .collect();
    let hidden = everything.len() - rows.len();
    let failed = rows.iter().filter(|r| r["ok"] == json!(false)).count();
    let detail = if snap.is_first_load() {
        "Reading the record…".to_string()
    } else if rows.is_empty() {
        "Nothing has been changed yet.".to_string()
    } else if failed == 0 {
        format!("The last {} things done on this machine.", rows.len())
    } else {
        format!(
            "The last {} things done, {failed} that did not work.",
            rows.len()
        )
    };
    let toggle = if hidden > 0 || shows_reads {
        div().child(
            Button::new(
                "activity.reads",
                if shows_reads {
                    "Hide read-only actions".to_string()
                } else {
                    format!("Show {hidden} read-only actions")
                },
            )
            .kind(crate::ui::ButtonKind::Ghost)
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                this.activity_reads = !this.activity_reads;
                cx.notify();
            })),
        )
    } else {
        div()
    };
    // Whether the record can vouch for itself. Quiet when it can; loud, and specific, when it cannot.
    let audit = devd(cx).sync.watch(&q::audit());
    let seal = match audit.data.as_deref() {
        Some(v) if v["intact"] == json!(true) && v["checked"].as_u64().unwrap_or(0) > 0 => {
            Some(pill(
                format!("record intact · {} checked", v["checked"]),
                Tone::Ok,
                cx,
            ))
        }
        Some(v) if v["intact"] == json!(false) => Some(pill(
            format!("record changed at entry {}", v["first_break"]["id"]),
            Tone::Danger,
            cx,
        )),
        _ => None,
    };
    let actions = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .children(seal)
        .child(toggle);
    let column = div()
        .flex()
        .flex_col()
        .child(page_head("Activity", detail, actions, cx));
    if snap.is_first_load() {
        let mut waiting = div().flex().flex_col().gap(px(18.0)).py(px(12.0));
        for n in 0..6 {
            waiting = waiting.child(ui::skeleton(
                ui::nid("activity.skeleton", n),
                300.0 + ((n * 53) % 200) as f32,
                cx,
            ));
        }
        return ui::page("activity", size::PAGE, column.child(waiting)).into_any_element();
    }
    if rows.is_empty() {
        return ui::page(
            "activity",
            980.0,
            column.child(ui::empty(
                "activity",
                "Nothing changed yet",
                "Everything devd changes for you or an agent is recorded here, with who asked.",
                cx,
            )),
        )
        .into_any_element();
    }
    let mut list = card(cx).flex().flex_col().overflow_hidden();
    // The same actor on ten lines in a row is one fact. It is said once, as a heading, with the lines under it;
    // "You" repeated ten times is a column that carries nothing.
    let mut previous: Option<String> = None;
    for (n, row) in rows.iter().enumerate() {
        let ok = row["ok"] != json!(false);
        let capability = row["capability"].as_str().unwrap_or_default();
        let args: Value = row["args"]
            .as_str()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or(Value::Null);
        let said = toolname::describe(
            capability,
            &args,
            if ok { Phase::Done } else { Phase::Failed },
        );
        let (by, _tone) = who(row["actor"].as_str().unwrap_or_default());
        if previous.as_deref() != Some(by.as_str()) {
            if n > 0 {
                list = list.child(rule(cx));
            }
            list = list.child(
                div()
                    .px(px(14.0))
                    .pt(px(12.0))
                    .pb(px(4.0))
                    .child(eyebrow(by.to_uppercase(), cx)),
            );
            previous = Some(by.clone());
        }
        let mut line = div()
            .px(px(14.0))
            .py(px(7.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(ui::dot(if ok { Tone::Ok } else { Tone::Danger }, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .child(
                        div()
                            .flex_none()
                            .text_color(if ok { t.muted } else { t.danger })
                            .child(said.action),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .font_family(mono())
                            .text_size(px(size::TEXT_SM))
                            .child(ui::Clip::new(said.object)),
                    ),
            );
        if row["dry_run"] == json!(true) {
            line = line.child(pill("plan only", Tone::Neutral, cx));
        }
        if !ok {
            line = line.child(
                div()
                    .max_w(px(280.0))
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.faint)
                    .child(ui::Clip::new(
                        row["message"].as_str().unwrap_or_default().to_string(),
                    )),
            );
        }
        line = line.child(
            div()
                .w(px(36.0))
                .flex_none()
                .flex()
                .justify_end()
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child(ui::ago(row["at"].as_str().unwrap_or_default())),
        );
        list = list.child(line);
    }
    ui::page("activity", size::PAGE, column.child(list)).into_any_element()
}

/// The appearance section: an eyebrow, a heading with one line under it, then the theme cards.
///
/// It reads the same scale as the rest of Settings (an eyebrow, a title, spacing of 12 and 22), so the
/// section does not sit on 14 px gaps beside a page on 22 and look like two designs. The chosen theme says
/// so once, in the card's heading; the border and the tick that repeated it are gone.
fn theme_section(cx: &mut Context<Shell>) -> Div {
    use super::appearance::CHARACTER;
    use crate::theme::Theme as Shown;
    let current = theme(cx).clone();
    let mut grid = div().flex().flex_col().gap(px(12.0));
    for row in CHARACTER.chunks(3) {
        let mut line = div().flex().gap(px(12.0));
        for (id, feel, scene) in row {
            line = line.child(super::appearance::theme_card(
                Shown::by_id(id),
                (*feel).into(),
                (*scene).into(),
                None,
                &current,
                cx,
            ));
        }
        for _ in row.len()..3 {
            line = line.child(div().flex_1());
        }
        grid = grid.child(line);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(eyebrow("APPEARANCE", cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).child("Theme"))
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(current.muted)
                        .child("Each theme is a whole look: its own colours and its own picture behind every screen."),
                ),
        )
        .child(grid)
        .child(super::appearance::yours(&current, cx))
}

/// How this window is set up.
pub fn settings(_shell: &mut Shell, cx: &mut Context<Shell>) -> AnyElement {
    let t = theme(cx).clone();
    let base = devd(cx).conn.base.clone();
    let policy = devd(cx).sync.watch(&q::policy());
    let p = policy.data.as_deref().cloned().unwrap_or(Value::Null);
    let row = |label: &'static str, value: AnyElement| {
        div()
            .px(px(14.0))
            .h(px(46.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .w(px(180.0))
                    .flex_none()
                    .text_color(t.muted)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(value))
    };
    let text = |s: String| div().child(ui::Clip::new(s)).into_any_element();
    let open_base = base.clone();
    let server = card(cx)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(row(
            "devd",
            text(format!("{base} · version {}", env!("CARGO_PKG_VERSION"))),
        ))
        .child(rule(cx))
        .child(row(
            "Browser interface",
            Button::new("settings.browser", "Open in the browser")
                .icon("external")
                .on_click(move |_, _, cx| cx.open_url(&open_base))
                .into_any_element(),
        ));
    let local_only = p["local_only"] == json!(true);
    let rules = card(cx)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(row("Where projects may go", text(if local_only { "Nowhere: only models on this machine".to_string() } else { "Any provider that is set up".to_string() })))
        .child(rule(cx))
        .child(row("Spend limit per run", text(p["max_cost_usd"].as_f64().map(crate::screens::intelligence::money).unwrap_or_else(|| "None set".to_string()))))
        .child(rule(cx))
        .child(div().px(px(14.0)).py(px(10.0)).text_size(px(size::TEXT_XS)).text_color(t.faint).child("Only a person can change these, with the devd policy set command. An agent cannot loosen the rules it runs under."));
    let column = div()
        .flex()
        .flex_col()
        .gap(px(22.0))
        .child(
            div()
                .child(page_head(
                    "Settings",
                    "How this window looks and which devd it talks to.",
                    div(),
                    cx,
                ))
                .children(super::pages::tab_strip(
                    &devd_app_core::nav::Route::Settings,
                    cx,
                )),
        )
        .child(theme_section(cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(eyebrow("SERVER", cx))
                .child(server),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(eyebrow("POLICY", cx))
                .child(rules),
        );
    // The same width as every other screen, so the title and tabs do not jump sideways when you arrive here.
    ui::page("settings", size::PAGE, column).into_any_element()
}
