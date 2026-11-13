//! Markdown, drawn with the window's own components.
//!
//! `devd_app_core::markdown` reads the text into blocks; this draws them. There
//! is no HTML and no webview in between, so a reply can contain nothing the
//! window did not choose to draw.

use std::ops::Range;

use devd_app_core::markdown::{spans, Align, Block, Inline, Item, WidgetKind};
use gpui::{
    canvas, div, prelude::*, px, App, Div, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveText, MouseButton, SharedString,
    StrikethroughStyle, StyledText, UnderlineStyle,
};
use serde_json::Value;

use super::links::Link;
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
        Block::Ui { tree, state } => super::genui::draw(id, tree.as_ref(), state, cx),
    }
}

/// Plain text (what a person wrote, not markdown) with its files and addresses clickable.
pub fn linked(id: &str, source: &str, cx: &App) -> gpui::AnyElement {
    let inlines = [Inline::Text(source.to_string())];
    text(id, &inlines, cx).into_any_element()
}

/// A run of inline content as one wrapped text element, with its links live.
fn text(id: &str, inlines: &[Inline], cx: &App) -> impl IntoElement {
    let t = theme(cx);
    let mut content = String::new();
    let mut styles: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut links: Vec<(Range<usize>, Link)> = Vec::new();
    let root = super::links::root(cx);
    // A file or address the agent mentioned is a link even when it was not written as one.
    let mut found: Vec<(Range<usize>, Link)> = Vec::new();
    for span in spans(inlines) {
        // What a person wrote or an agent said can hold a devd:// address: it is drawn as a chip.
        let (shown, inside_links) = if span.link.is_none() && !span.code { super::links::present(&span.text, root.as_deref()) } else { (span.text.clone(), Vec::new()) };
        let range = content.len()..content.len() + shown.len();
        content.push_str(&shown);
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
            links.push((range.clone(), Link::Url(url)));
            styled = true;
        } else if span.code {
            // `src/main.rs` is a file; `cargo test` is not.
            if let Some(link) = super::links::resolve_token(&span.text, root.as_deref()) {
                found.push((range.clone(), link));
            }
        } else {
            for (inside, link) in inside_links {
                found.push((range.start + inside.start..range.start + inside.end, link));
            }
        }
        if styled && !range.is_empty() {
            styles.push((range, style));
        }
    }
    let link_style = HighlightStyle {
        color: Some(t.info),
        underline: Some(UnderlineStyle { thickness: px(1.0), color: Some(tint(t.info, 0.5)), wavy: false }),
        ..HighlightStyle::default()
    };
    for (range, link) in found {
        // The link's colour and underline go over the span's own style (a code span keeps its tint).
        let mut style = styles.iter().find(|(r, _)| r.start <= range.start && range.end <= r.end).map(|(_, s)| *s).unwrap_or_default();
        style.color = link_style.color;
        style.underline = link_style.underline;
        // A place in devd is a chip: tinted, so it reads as something to press.
        if matches!(link, Link::Route(_)) {
            style.background_color = Some(tint(t.info, 0.14));
            style.underline = None;
        }
        styles.push((range.clone(), style));
        links.push((range, link));
    }
    // Highlights must be in order and must not overlap: where a link sits inside a styled span, the link's wins.
    styles.sort_by_key(|(r, _)| (r.start, r.end));
    let mut clean: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (range, style) in styles {
        match clean.last_mut() {
            Some((last, _)) if range.start < last.end => {
                // Overlap: the earlier, wider style is cut where the narrower one begins.
                if range.start > last.start {
                    last.end = range.start;
                    clean.push((range, style));
                } else {
                    clean.pop();
                    clean.push((range, style));
                }
            }
            _ => clean.push((range, style)),
        }
    }
    let mut styles = clean;
    links.sort_by_key(|(r, _)| r.start);
    // A web link gets the site's icon before it. Room for the icon is made in the text itself, and every
    // range after it moves along; the link's own colour and underline start after the room.
    let mut icons: Vec<(usize, String)> = Vec::new();
    if super::favicon::enabled(cx) {
        let mut points: Vec<(usize, String)> = links.iter().filter_map(|(r, l)| if let Link::Url(url) = l { super::favicon::host_of(url).map(|h| (r.start, h)) } else { None }).collect();
        points.dedup_by_key(|(at, _)| *at);
        if !points.is_empty() {
            let link_ranges: Vec<Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
            let room = super::favicon::ROOM.len();
            let before = |i: usize, inclusive: bool| points.iter().filter(|(p, _)| *p < i || (inclusive && *p == i)).count() * room;
            let mut widened = String::with_capacity(content.len() + points.len() * room);
            let mut last = 0;
            for (at, host) in &points {
                widened.push_str(&content[last..*at]);
                widened.push_str(super::favicon::ROOM);
                // Where the link's own text begins: the icon is painted just before it, wherever the line broke.
                icons.push((widened.len(), host.clone()));
                last = *at;
            }
            widened.push_str(&content[last..]);
            let remap = |r: &Range<usize>| {
                let is_link = link_ranges.contains(r);
                r.start + before(r.start, is_link)..r.end + before(r.end, false)
            };
            for (r, _) in styles.iter_mut() {
                *r = remap(r);
            }
            for (r, _) in links.iter_mut() {
                *r = remap(r);
            }
            content = widened;
        }
    }
    let styled = StyledText::new(SharedString::from(content)).with_highlights(styles);
    let layout = styled.layout().clone();
    let ranges: Vec<Range<usize>> = links.iter().map(|(range, _)| range.clone()).collect();
    let targets: Vec<Link> = links.into_iter().map(|(_, link)| link).collect();
    // gpui's own click on text listens for the release only from the frame after the press,
    // so a quick click on a slow frame is lost. The release is handled here, against the
    // text's layout; the element inside is kept for its pointing-hand cursor over a link.
    let hit = ranges.clone();
    let pointer = InteractiveText::new(ElementId::Name(id.to_string().into()), styled).on_click(ranges.clone(), |_, _, _| {});
    // Start the lookups now, so an icon is on its way even if the text has not been laid out yet.
    for (_, host) in &icons {
        let _ = super::favicon::lookup(host, cx);
    }
    // The icons are painted over the room made for them, once the text engine has said where that is.
    let place = layout.clone();
    let icon_layer = canvas(
        move |_bounds, window, cx| {
            let line = place.line_height();
            let size = px(super::favicon::SIZE);
            let mut tiles = Vec::new();
            for (at, host) in &icons {
                if let Some(pos) = place.position_for_index(*at) {
                    let mut el = super::favicon::tile(host, cx).into_any_element();
                    el.prepaint_as_root(gpui::point(pos.x - size - px(2.0), pos.y + (line - size) / 2.0), gpui::size(size, size).into(), window, cx);
                    tiles.push(el);
                }
            }
            tiles
        },
        |_bounds, mut tiles, window, cx| {
            for tile in tiles.iter_mut() {
                tile.paint(window, cx);
            }
        },
    )
    .absolute()
    .size_full();
    div().min_w_0().relative().on_mouse_up(MouseButton::Left, move |event, _window, cx| {
        if let Ok(at) = layout.index_for_position(event.position)
            && let Some(n) = hit.iter().position(|range| range.contains(&at))
        {
            super::links::open(&targets[n], cx);
        }
    })
    .child(pointer)
    .child(icon_layer)
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
            // Alignment is of the text, not of a box around it: a box that is itself a row squeezes a narrow column to nothing.
            Some(Align::End) => inner.text_right(),
            Some(Align::Center) => inner.text_center(),
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
        "green" | "success" | "teal" | "ok" => t.ok,
        "red" | "error" | "pink" | "danger" => t.danger,
        "orange" | "yellow" | "warning" | "warn" => t.warn,
        "blue" | "info" | "violet" => t.info,
        "muted" | "neutral" => t.muted,
        _ => t.accent,
    }
}

/// One of the declared visuals, for the components that share its drawing.
pub(super) fn widget_node(id: &str, kind: WidgetKind, data: &Value, cx: &App) -> Div {
    widget(id, kind, data, cx)
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
