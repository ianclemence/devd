//! Drawing what an assistant composed.
//!
//! The catalog and the checks live in `devd-genui`; this is only the pictures. Every component in the
//! catalog is drawn here with the window's own pieces, so a spec looks like part of the app in every
//! theme and nothing the model wrote is ever interpreted as anything but data.
//!
//! A card is drawn as far as it has arrived. While the model is still writing, what is there is drawn
//! and a quiet line shows more is coming; if the finished spec had problems, what could be drawn stays
//! and one line says part of it was left out. A button never does anything on its own: pressing it
//! queues an [`Act`] for the shell, which sends a message, goes somewhere, or shows exactly what it is
//! about to run and asks first.

use devd_app_core::markdown::{self as core_md, Align, Block, UiState};
use devd_genui::Node;
use gpui::{
    App, ClickEvent, Div, ElementId, FontWeight, Hsla, SharedString, Window, div, prelude::*, px,
};
use serde_json::{Value, json};

use super::markdown as ui_md;
use super::{Button, ButtonKind, icon};
use crate::state::Devd;
use crate::theme::{mono, size, theme};

/// What a button asks for. Queued for the shell; never carried out here.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Send this as the person's next message.
    Reply(String),
    /// Go to a devd page (`/sites`) or open a file.
    Open(String),
    /// Run this capability. The shell shows what it will do and asks first unless it only reads.
    Run {
        /// The capability's id.
        capability: String,
        /// Its arguments.
        args: Value,
        /// What the button said.
        label: String,
    },
    /// Open a web address in the browser.
    Url(String),
}

/// The action a button node stands for. `None` when it lacks what its action needs.
pub fn act_of(node: &Node) -> Option<Act> {
    let text = |name: &str| Some(node.str(name).trim().to_string()).filter(|s| !s.is_empty());
    match node.kind {
        "LinkButton" => text("url").map(Act::Url),
        "Button" => match node.str("action") {
            "reply" => text("text").map(Act::Reply),
            "open" => text("target").map(Act::Open),
            "run" => text("capability").map(|capability| Act::Run {
                capability,
                args: node.props.get("args").cloned().unwrap_or_else(|| json!({})),
                label: node.str("label").to_string(),
            }),
            _ => None,
        },
        _ => None,
    }
}

/// Queue an action for the shell, which asks the person where that is needed.
pub fn press(act: Act, cx: &mut App) {
    match act {
        // These two already have one path, shared with every link in the window.
        Act::Url(url) => cx.open_url(&url),
        other => {
            cx.global_mut::<Devd>().pending_ui.push(other);
            cx.refresh_windows();
        }
    }
}

/// Draw a spec. `state` says how far it has got.
pub fn draw(id: &str, tree: Option<&Node>, state: &UiState, cx: &App) -> Div {
    let t = theme(cx);
    let frame = || {
        div()
            .rounded(px(size::RADIUS_L))
            .border_1()
            .border_color(t.line)
            .bg(t.surface)
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .min_w_0()
    };
    let Some(tree) = tree else {
        // The message has begun but no card exists yet.
        return frame()
            .child(super::skeleton(
                ElementId::Name(format!("{id}.sk0").into()),
                180.0,
                cx,
            ))
            .child(super::skeleton(
                ElementId::Name(format!("{id}.sk1").into()),
                280.0,
                cx,
            ));
    };
    let mut card = frame();
    if !tree.str("title").is_empty() || !tree.str("subtitle").is_empty() {
        let mut head = div().flex().flex_col().gap(px(2.0));
        if !tree.str("title").is_empty() {
            head = head.child(
                div()
                    .text_size(px(size::TEXT_LG))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(tree.str("title").to_string()),
            );
        }
        if !tree.str("subtitle").is_empty() {
            head = head.child(
                div()
                    .text_size(px(size::TEXT_SM))
                    .text_color(t.muted)
                    .child(tree.str("subtitle").to_string()),
            );
        }
        card = card.child(head);
    }
    for (n, child) in tree.children.iter().enumerate() {
        card = card.child(node(&format!("{id}.{n}"), child, cx));
    }
    match state {
        UiState::Writing => {
            card = card.child(super::skeleton(
                ElementId::Name(format!("{id}.more").into()),
                120.0,
                cx,
            ))
        }
        // The details are for the model, which is told each one and fixes them; the person is only told something was left out.
        UiState::Broken(_) => {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(size::TEXT_SM))
                    .text_color(t.faint)
                    .child(icon("info", 13.0, t.faint))
                    .child("Part of this could not be drawn."),
            );
        }
        UiState::Done => {}
    }
    card
}

fn tone(name: &str, cx: &App) -> Hsla {
    let t = theme(cx);
    match name {
        "ok" | "green" | "success" => t.ok,
        "warn" | "orange" | "yellow" | "warning" => t.warn,
        "danger" | "red" | "error" => t.danger,
        "info" | "blue" => t.info,
        "muted" | "neutral" => t.muted,
        _ => t.accent,
    }
}

fn title(n: &Node, cx: &App) -> Option<Div> {
    let t = theme(cx);
    Some(n.str("title")).filter(|s| !s.is_empty()).map(|s| {
        div()
            .text_size(px(size::TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.muted)
            .child(s.to_string())
    })
}

fn markdown_of(id: &str, source: &str, cx: &App) -> Div {
    ui_md::document(id, &core_md::parse(source), cx)
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn node(id: &str, n: &Node, cx: &App) -> Div {
    // An element that hides what overflows can be squeezed by its parent; a drawn component always keeps its full height.
    node_inner(id, n, cx).flex_none()
}

fn node_inner(id: &str, n: &Node, cx: &App) -> Div {
    let t = theme(cx);
    match n.kind {
        "Section" => {
            let mut section = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .children(title(n, cx).map(|d| d.pt(px(4.0))));
            for (k, child) in n.children.iter().enumerate() {
                section = section.child(node(&format!("{id}.{k}"), child, cx));
            }
            section
        }
        "Text" => {
            let colour = if n.str("tone").is_empty() {
                t.text
            } else {
                tone(n.str("tone"), cx)
            };
            div()
                .text_color(colour)
                .child(markdown_of(id, n.str("text"), cx))
        }
        "Callout" => {
            let colour = tone(
                if n.str("tone").is_empty() {
                    "info"
                } else {
                    n.str("tone")
                },
                cx,
            );
            let mut body = div().flex_1().min_w_0().flex().flex_col().gap(px(3.0));
            if !n.str("title").is_empty() {
                body = body.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(n.str("title").to_string()),
                );
            }
            div()
                .flex()
                .rounded(px(size::RADIUS))
                .overflow_hidden()
                .bg(super::tint(colour, 0.10))
                .child(div().w(px(3.0)).flex_none().bg(colour))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(px(12.0))
                        .py(px(9.0))
                        .child(body.child(markdown_of(id, n.str("text"), cx))),
                )
        }
        "Fields" => {
            let mut rows = div()
                .flex()
                .flex_col()
                .rounded(px(size::RADIUS))
                .border_1()
                .border_color(t.line)
                .overflow_hidden();
            for (k, f) in n.list("items").iter().enumerate() {
                let mut row = div()
                    .flex()
                    .gap(px(12.0))
                    .px(px(12.0))
                    .py(px(7.0))
                    .text_size(px(size::TEXT))
                    .child(
                        div()
                            .w(px(140.0))
                            .flex_none()
                            .text_color(t.muted)
                            .child(scalar(&f["label"])),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(t.text)
                            .child(scalar(&f["value"])),
                    );
                if k > 0 {
                    row = row.border_t_1().border_color(t.line);
                }
                rows = rows.child(row);
            }
            rows
        }
        "Table" => {
            let head: Vec<Vec<core_md::Inline>> = n
                .list("columns")
                .iter()
                .map(|c| core_md::inline(&scalar(c)))
                .collect();
            let body: Vec<Vec<Vec<core_md::Inline>>> = n
                .list("rows")
                .iter()
                .map(|r| {
                    r.as_array()
                        .map(|cells| cells.iter().map(|c| core_md::inline(&scalar(c))).collect())
                        .unwrap_or_default()
                })
                .collect();
            // A column of numbers lines up on its right edge.
            let align: Vec<Align> = (0..head.len())
                .map(|c| {
                    let numeric = !n.list("rows").is_empty()
                        && n.list("rows").iter().all(|r| {
                            r.get(c).is_some_and(|v| {
                                v.is_number()
                                    || scalar(v)
                                        .trim_end_matches(['%', ' '])
                                        .replace(',', "")
                                        .parse::<f64>()
                                        .is_ok()
                            })
                        });
                    if numeric { Align::End } else { Align::Start }
                })
                .collect();
            let block = Block::Table {
                head,
                align,
                rows: body,
            };
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(title(n, cx))
                .child(ui_md::document(id, &[block], cx))
        }
        "Chart" => chart(id, n, cx),
        "Metrics" => ui_md::widget_node(
            id,
            core_md::WidgetKind::Metrics,
            &Value::Object(n.props.clone()),
            cx,
        ),
        "Steps" => ui_md::widget_node(
            id,
            core_md::WidgetKind::Steps,
            &Value::Object(n.props.clone()),
            cx,
        ),
        "Timeline" => {
            let widget = ui_md::widget_node(
                id,
                core_md::WidgetKind::Timeline,
                &Value::Object(n.props.clone()),
                cx,
            );
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(title(n, cx))
                .child(widget)
        }
        "Details" => ui_md::widget_node(
            id,
            core_md::WidgetKind::Details,
            &Value::Object(n.props.clone()),
            cx,
        ),
        "Code" => {
            let block = Block::Code {
                lang: if n.str("title").is_empty() {
                    n.str("language").to_string()
                } else {
                    n.str("title").to_string()
                },
                text: n.str("code").to_string(),
            };
            ui_md::document(id, &[block], cx)
        }
        "Diff" => ui_md::document(
            id,
            &[Block::Code {
                lang: "diff".into(),
                text: n.str("patch").to_string(),
            }],
            cx,
        ),
        "FileList" => {
            let mut list = div().flex().flex_col().gap(px(5.0)).children(title(n, cx));
            for (k, p) in n.list("paths").iter().enumerate() {
                let path = scalar(p);
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(size::TEXT_SM))
                        .child(icon("file", 13.0, t.faint))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(mono())
                                .child(ui_md::linked(&format!("{id}.f{k}"), &path, cx)),
                        ),
                );
            }
            list
        }
        "Divider" => div().h(px(1.0)).bg(t.line),
        "Actions" => {
            let mut row = div().flex().flex_wrap().gap(px(8.0)).pt(px(2.0));
            for (k, b) in n.children.iter().enumerate() {
                row = row.child(button(id, k, b));
            }
            row
        }
        // Buttons are only drawn inside Actions, which the checker enforces; one found elsewhere is drawn alone.
        "Button" | "LinkButton" => div().flex().child(button(id, 0, n)),
        _ => div(),
    }
}

fn button(id: &str, k: usize, n: &Node) -> gpui::AnyElement {
    let kind = match n.str("style") {
        "primary" => ButtonKind::Primary,
        "danger" => ButtonKind::Danger,
        _ => ButtonKind::Secondary,
    };
    let eid = ElementId::Name(SharedString::from(format!("{id}.b{k}")));
    let mut b = Button::new(eid, n.str("label").to_string()).kind(kind);
    b = match (n.kind, n.str("action")) {
        ("LinkButton", _) => b.icon("external"),
        (_, "reply") => b.icon("arrow-right"),
        (_, "open") => b.icon("arrow-right"),
        (_, "run") => b.icon("play"),
        _ => b,
    };
    match act_of(n) {
        Some(act) => b
            .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| press(act.clone(), cx))
            .into_any_element(),
        None => b.disabled(true).into_any_element(),
    }
}

fn chart(id: &str, n: &Node, cx: &App) -> Div {
    let t = theme(cx);
    let style = n.str("style");
    if style != "column" && style != "share" {
        let widget = ui_md::widget_node(
            id,
            core_md::WidgetKind::Chart,
            &Value::Object(n.props.clone()),
            cx,
        );
        return widget;
    }
    let unit = n.str("unit");
    let bars = n.list("bars");
    let frame = div()
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(t.line)
        .bg(t.surface)
        .p(px(14.0))
        .flex()
        .flex_col()
        .gap(px(10.0));
    let frame = match title(n, cx) {
        Some(h) => frame.child(h),
        None => frame,
    };
    let total: f64 = bars
        .iter()
        .filter_map(|b| b["value"].as_f64())
        .map(|v| v.max(0.0))
        .sum::<f64>()
        .max(f64::MIN_POSITIVE);
    let max = bars
        .iter()
        .filter_map(|b| b["value"].as_f64())
        .fold(0.0f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let shades = |k: usize, tone_name: &str| {
        if tone_name.is_empty() {
            [t.accent, t.info, t.ok, t.warn, t.danger, t.muted][k % 6]
        } else {
            tone(tone_name, cx)
        }
    };
    let body = if style == "share" {
        let mut strip = div()
            .flex()
            .h(px(14.0))
            .rounded_full()
            .overflow_hidden()
            .bg(t.hover);
        let mut legend = div().flex().flex_wrap().gap_x(px(16.0)).gap_y(px(4.0));
        for (k, b) in bars.iter().enumerate() {
            let v = b["value"].as_f64().unwrap_or(0.0).max(0.0);
            let colour = shades(k, b["tone"].as_str().unwrap_or_default());
            strip = strip.child(
                div()
                    .h_full()
                    .bg(colour)
                    .w(gpui::relative((v / total) as f32)),
            );
            legend = legend.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(size::TEXT_SM))
                    .child(div().size(px(8.0)).rounded_full().bg(colour))
                    .child(div().text_color(t.muted).child(scalar(&b["label"])))
                    .child(div().font_family(mono()).text_color(t.text).child(format!(
                        "{}{}  {:.0}%",
                        scalar(&b["value"]),
                        if unit.is_empty() {
                            String::new()
                        } else {
                            format!(" {unit}")
                        },
                        v / total * 100.0
                    ))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(strip)
            .child(legend)
    } else {
        let mut columns = div().flex().items_end().gap(px(10.0)).h(px(150.0));
        for (k, b) in bars.iter().enumerate() {
            let v = b["value"].as_f64().unwrap_or(0.0).max(0.0);
            let colour = shades(k, b["tone"].as_str().unwrap_or_default());
            columns = columns.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .font_family(mono())
                            .text_size(px(size::TEXT_XS))
                            .text_color(t.text)
                            .child(format!(
                                "{}{}",
                                scalar(&b["value"]),
                                if unit.len() <= 2 { unit } else { "" }
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_w(px(44.0))
                            .rounded_t(px(4.0))
                            .bg(colour)
                            .h(gpui::relative((v / max).clamp(0.02, 1.0) as f32 * 0.78)),
                    ),
            );
        }
        let mut labels = div().flex().gap(px(10.0));
        for b in bars {
            labels = labels.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .text_size(px(size::TEXT_XS))
                    .text_color(t.muted)
                    .child(div().truncate().child(scalar(&b["label"]))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(columns)
            .child(div().h(px(1.0)).bg(t.line))
            .child(labels)
    };
    let frame = frame.child(body);
    match n.str("takeaway") {
        "" => frame,
        takeaway => frame.child(
            div()
                .text_size(px(size::TEXT_SM))
                .text_color(t.muted)
                .child(takeaway.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_genui::check;
    use serde_json::json;

    fn buttons(spec: Value) -> Vec<Option<Act>> {
        let tree = check(&spec).expect("valid");
        let mut out = Vec::new();
        fn walk(n: &Node, out: &mut Vec<Option<Act>>) {
            if n.kind == "Button" || n.kind == "LinkButton" {
                out.push(act_of(n));
            }
            n.children.iter().for_each(|c| walk(c, out));
        }
        walk(&tree, &mut out);
        out
    }

    #[test]
    fn each_kind_of_button_becomes_the_one_action_it_names() {
        let acts = buttons(
            json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Actions", "children": [
                {"type": "Button", "props": {"label": "Ask", "action": "reply", "text": "Yes, go on"}},
                {"type": "Button", "props": {"label": "See", "action": "open", "target": "/sites"}},
                {"type": "Button", "props": {"label": "Restart", "action": "run", "capability": "site.restart", "args": {"name": "shop"}}},
                {"type": "LinkButton", "props": {"label": "Docs", "url": "https://example.com/x"}}
            ]}]}),
        );
        assert_eq!(acts[0], Some(Act::Reply("Yes, go on".into())));
        assert_eq!(acts[1], Some(Act::Open("/sites".into())));
        assert_eq!(
            acts[2],
            Some(Act::Run {
                capability: "site.restart".into(),
                args: json!({"name": "shop"}),
                label: "Restart".into()
            })
        );
        assert_eq!(acts[3], Some(Act::Url("https://example.com/x".into())));
    }

    #[test]
    fn a_run_button_without_arguments_runs_with_none() {
        let acts = buttons(
            json!({"type": "Card", "props": {"title": "t"}, "children": [{"type": "Actions", "children": [{"type": "Button", "props": {"label": "Refresh", "action": "run", "capability": "site.list"}}]}]}),
        );
        assert_eq!(
            acts[0],
            Some(Act::Run {
                capability: "site.list".into(),
                args: json!({}),
                label: "Refresh".into()
            })
        );
    }
}
