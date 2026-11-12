//! Markdown, drawn with the window's own components.
//!
//! `devd_app_core::markdown` reads the text into blocks; this draws them. There
//! is no HTML and no webview in between, so a reply can contain nothing the
//! window did not choose to draw.

use std::ops::Range;

use devd_app_core::markdown::{spans, Align, Block, Inline, Item, WidgetKind};
use gpui::{
    div, prelude::*, px, App, Div, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveText, SharedString,
    StrikethroughStyle, StyledText, UnderlineStyle,
};
use serde_json::Value;

use super::{icon, tint};
use crate::theme::{mono, size, theme, Tone};

/// Draw a document. `id` must be unique among documents on screen: it names
/// the links inside, which is how a click finds the right one.
pub fn document(id: &str, blocks: &[Block], cx: &App) -> Div {
    let mut column = div().flex().flex_col().gap(px(10.0)).min_w_0();
    for (n, block) in blocks.iter().enumerate() {
        column = column.child(draw(&format!("{id}.{n}"), block, cx));
    }
    column
}

fn draw(id: &str, block: &Block, cx: &App) -> Div {
    let t = theme(cx);
    match block {
        Block::Paragraph(inlines) => div().min_w_0().child(text(id, inlines, cx)),
        Block::Heading { level, text: inlines } => {
            let px_size = match level {
                1 => size::TEXT_XL,
                2 => 17.0,
                3 => 15.0,
                _ => size::TEXT_MD,
            };
            div()
                .pt(px(if *level <= 2 { 8.0 } else { 4.0 }))
                .text_size(px(px_size))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text)
                .child(text(id, inlines, cx))
        }
        Block::Rule => div().h(px(1.0)).my(px(4.0)).bg(t.line),
        // A patch is drawn as a diff, not as text that happens to start with + and -.
        Block::Code { lang, text } if lang == "diff" || lang == "patch" => {
            let files = devd_app_core::diff::parse(text);
            if files.iter().all(|file| file.hunks.is_empty() && !file.binary) {
                return code(id, lang, text, cx);
            }
            let mut column = div().flex().flex_col().gap(px(8.0));
            for (n, file) in files.iter().enumerate() {
                column = column.child(super::diff::file(&format!("{id}.f{n}"), file, cx));
            }
            column
        }
        Block::Code { lang, text } => code(id, lang, text, cx),
        Block::Quote(inner) => div()
            .pl(px(12.0))
            .border_l_2()
            .border_color(t.line_strong)
            .text_color(t.muted)
            .child(document(id, inner, cx)),
        Block::List { ordered, start, items } => list(id, *ordered, *start, items, cx),
        Block::Table { head, align, rows } => table(id, head, align, rows, cx),
        Block::Widget { kind, data } => widget(id, *kind, data, cx),
    }
}

/// A run of inline content as one wrapped text element, with its links live.
fn text(id: &str, inlines: &[Inline], cx: &App) -> impl IntoElement {
    let t = theme(cx);
    let mut content = String::new();
    let mut styles: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(Range<usize>, String)> = Vec::new();
    for span in spans(inlines) {
        let range = content.len()..content.len() + span.text.len();
        content.push_str(&span.text);
        let mut style = HighlightStyle::default();
        let mut styled = false;
        if span.strong {
            style.font_weight = Some(FontWeight::SEMIBOLD);
            styled = true;
        }
        if span.emphasis {
            style.font_style = Some(FontStyle::Italic);
            styled = true;
        }
        if span.strike {
            style.strikethrough = Some(StrikethroughStyle { thickness: px(1.0), color: Some(t.muted) });
            style.color = Some(t.muted);
            styled = true;
        }
        if span.code {
            style.background_color = Some(t.hover);
            style.color = Some(t.text);
            styled = true;
        }
        if let Some(url) = span.link {
            style.color = Some(t.info);
            style.underline = Some(UnderlineStyle { thickness: px(1.0), color: Some(tint(t.info, 0.5)), wavy: false });
            links.push((range.clone(), url));
            styled = true;
        }
        if styled && !range.is_empty() {
            styles.push((range, style));
        }
    }
    let styled = StyledText::new(SharedString::from(content)).with_highlights(styles);
    let ranges: Vec<Range<usize>> = links.iter().map(|(range, _)| range.clone()).collect();
    let urls: Vec<String> = links.into_iter().map(|(_, url)| url).collect();
    InteractiveText::new(ElementId::Name(id.to_string().into()), styled).on_click(ranges, move |index, _window, cx| {
        if let Some(url) = urls.get(index) {
            cx.open_url(url);
        }
    })
}

fn code(id: &str, lang: &str, source: &str, cx: &App) -> Div {
    let t = theme(cx);
    let mut lines = div().flex().flex_col();
    for line in source.split('\n') {
        // An empty line still takes a line's height.
        let shown = if line.is_empty() { " ".to_string() } else { line.replace('\t', "    ") };
        lines = lines.child(div().whitespace_nowrap().child(shown));
    }
    let mut block = div().rounded(px(size::RADIUS)).border_1().border_color(t.line).bg(t.code).overflow_hidden();
    if !lang.is_empty() {
        block = block.child(
            div()
                .px(px(12.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(t.line)
                .text_size(px(size::TEXT_XS))
                .font_family(mono())
                .text_color(t.faint)
                .child(lang.to_string()),
        );
    }
    block.child(
        div()
            .id(ElementId::Name(format!("{id}.code").into()))
            .overflow_x_scroll()
            .px(px(12.0))
            .py(px(10.0))
            .font_family(mono())
            .text_size(px(size::TEXT_SM))
            .line_height(px(19.0))
            .text_color(t.text)
            .child(lines),
    )
}

fn list(id: &str, ordered: bool, start: u64, items: &[Item], cx: &App) -> Div {
    let t = theme(cx);
    let mut column = div().flex().flex_col().gap(px(5.0));
    for (n, item) in items.iter().enumerate() {
        let marker = match item.checked {
            Some(true) => div().pt(px(3.0)).child(icon("check", 13.0, t.ok)),
            Some(false) => div().mt(px(4.0)).size(px(12.0)).rounded(px(3.0)).border_1().border_color(t.line_strong),
            None if ordered => div().font_family(mono()).text_size(px(size::TEXT_SM)).text_color(t.faint).child(format!("{}.", start + n as u64)),
            None => div().mt(px(8.0)).size(px(4.0)).rounded_full().bg(t.faint),
        };
        column = column.child(
            div()
                .flex()
                .items_start()
                .gap(px(9.0))
                .child(div().w(px(18.0)).flex().flex_none().justify_end().child(marker))
                .child(div().flex_1().min_w_0().child(document(&format!("{id}.{n}"), &item.blocks, cx))),
        );
    }
    column
}

fn table(id: &str, head: &[Vec<Inline>], align: &[Align], rows: &[Vec<Vec<Inline>>], cx: &App) -> Div {
    let t = theme(cx);
    let cell = |inner: Div, column: usize| {
        let inner = inner.flex_1().min_w_0().px(px(10.0)).py(px(6.0));
        match align.get(column) {
            Some(Align::End) => inner.flex().justify_end(),
            Some(Align::Center) => inner.flex().justify_center(),
            _ => inner,
        }
    };
    let mut header = div().flex().bg(t.hover);
    for (c, inlines) in head.iter().enumerate() {
        header = header.child(cell(
            div().text_size(px(size::TEXT_XS)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(text(&format!("{id}.h{c}"), inlines, cx)),
            c,
        ));
    }
    let mut body = div().flex().flex_col().rounded(px(size::RADIUS)).border_1().border_color(t.line).overflow_hidden().child(header);
    for (r, row) in rows.iter().enumerate() {
        let mut line = div().flex().border_t_1().border_color(t.line);
        for (c, inlines) in row.iter().enumerate() {
            line = line.child(cell(div().text_size(px(size::TEXT)).child(text(&format!("{id}.{r}.{c}"), inlines, cx)), c));
        }
        body = body.child(line);
    }
    body
}

/// The colour a widget's JSON names, or the accent when it names none.
fn named(color: Option<&str>, cx: &App) -> gpui::Hsla {
    let t = theme(cx);
    match color.unwrap_or_default() {
        "green" | "success" | "teal" => t.ok,
        "red" | "error" | "pink" => t.danger,
        "orange" | "yellow" | "warning" => t.warn,
        "blue" | "info" | "violet" => t.info,
        _ => t.accent,
    }
}

fn widget(id: &str, kind: WidgetKind, data: &Value, cx: &App) -> Div {
    let t = theme(cx);
    let frame = || div().rounded(px(size::RADIUS_L)).border_1().border_color(t.line).bg(t.surface).p(px(14.0));
    let title = data["title"].as_str().map(|s| div().pb(px(10.0)).font_weight(FontWeight::MEDIUM).text_color(t.text).child(s.to_string()));
    match kind {
        WidgetKind::Metrics => {
            let mut row = div().flex().flex_wrap().gap(px(28.0));
            for item in data["items"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let delta = item["delta"].as_str().unwrap_or_default();
                let tone = match item["delta_tone"].as_str() {
                    Some("success") => Tone::Ok,
                    Some("error") => Tone::Danger,
                    _ if delta.starts_with('+') => Tone::Ok,
                    _ if delta.starts_with('-') => Tone::Danger,
                    _ => Tone::Neutral,
                };
                let value = match &item["value"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                row = row.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(div().text_size(px(size::TEXT_XS)).text_color(t.faint).child(item["label"].as_str().unwrap_or_default().to_string()))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap(px(6.0))
                                .child(div().text_size(px(22.0)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(value))
                                .child(div().text_size(px(size::TEXT_SM)).text_color(t.tone(tone)).child(delta.to_string())),
                        ),
                );
            }
            frame().child(row)
        }
        WidgetKind::Chart => {
            let bars = data["bars"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let unit = data["unit"].as_str().unwrap_or_default();
            let max = bars.iter().filter_map(|b| b["value"].as_f64()).fold(0.0f64, f64::max).max(f64::MIN_POSITIVE);
            let mut column = frame().flex().flex_col().gap(px(7.0)).children(title);
            for bar in bars.iter().take(12) {
                let value = bar["value"].as_f64().unwrap_or(0.0).max(0.0);
                let color = named(bar["color"].as_str().or(bar["tone"].as_str()), cx);
                column = column.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(div().w(px(120.0)).flex_none().truncate().text_size(px(size::TEXT_SM)).text_color(t.muted).child(bar["label"].as_str().unwrap_or_default().to_string()))
                        .child(div().flex_1().h(px(8.0)).rounded_full().bg(t.hover).child(div().h_full().rounded_full().bg(color).w(gpui::relative((value / max) as f32))))
                        .child(div().w(px(72.0)).flex_none().font_family(mono()).text_size(px(size::TEXT_SM)).text_color(t.text).child(format!("{value} {unit}"))),
                );
            }
            match data["takeaway"].as_str() {
                Some(takeaway) => column.child(div().pt(px(6.0)).text_size(px(size::TEXT_SM)).text_color(t.muted).child(takeaway.to_string())),
                None => column,
            }
        }
        WidgetKind::Steps => {
            let current = data["current"].as_u64().unwrap_or(0) as usize;
            let mut row = div().flex().flex_wrap().items_center().gap(px(8.0));
            for (n, step) in data["items"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
                let (color, weight) = match n.cmp(&current) {
                    std::cmp::Ordering::Less => (t.ok, FontWeight::NORMAL),
                    std::cmp::Ordering::Equal => (t.accent, FontWeight::SEMIBOLD),
                    std::cmp::Ordering::Greater => (t.faint, FontWeight::NORMAL),
                };
                if n > 0 {
                    row = row.child(div().w(px(18.0)).h(px(1.0)).bg(t.line_strong));
                }
                row = row.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().size(px(7.0)).rounded_full().bg(color))
                        .child(div().text_size(px(size::TEXT_SM)).font_weight(weight).text_color(if n == current { t.text } else { t.muted }).child(step.as_str().unwrap_or_default().to_string())),
                );
            }
            frame().children(title).child(row)
        }
        WidgetKind::Timeline => {
            let mut column = frame().flex().flex_col().gap(px(8.0));
            for item in data["items"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                column = column.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(10.0))
                        .child(div().w(px(52.0)).flex_none().font_family(mono()).text_size(px(size::TEXT_XS)).text_color(t.faint).child(item["at"].as_str().unwrap_or_default().to_string()))
                        .child(div().mt(px(6.0)).size(px(6.0)).flex_none().rounded_full().bg(named(item["tone"].as_str(), cx)))
                        .child(div().flex_1().text_size(px(size::TEXT)).child(item["text"].as_str().unwrap_or_default().to_string())),
                );
            }
            column
        }
        WidgetKind::Details => {
            let body = devd_app_core::markdown::parse(data["body"].as_str().unwrap_or_default());
            frame().children(title).child(document(&format!("{id}.d"), &body, cx))
        }
    }
}
