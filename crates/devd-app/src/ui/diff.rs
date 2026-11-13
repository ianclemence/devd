//! The diff viewer.
//!
//! One file at a time: its path and counts, then each hunk with line numbers on
//! both sides. A replaced line is tinted, and the words that changed inside it
//! are tinted more, so the eye lands on the three characters that matter.

use std::ops::Range;

use devd_app_core::diff::{FileDiff, Kind, Line, Status};
use gpui::{
    App, Div, ElementId, FontWeight, HighlightStyle, SharedString, StyledText, div, prelude::*, px,
};

use super::{icon, pill};
use crate::theme::{Tone, mono, size, theme};

/// Draw one file's changes.
pub fn file(id: &str, diff: &FileDiff, cx: &App) -> Div {
    let t = theme(cx);
    let (added, removed) = diff.counts();
    let status = match diff.status {
        Status::Added => Some(("new", Tone::Ok)),
        Status::Deleted => Some(("deleted", Tone::Danger)),
        Status::Renamed => Some(("renamed", Tone::Info)),
        Status::Modified => None,
    };
    let mut head = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .h(px(34.0))
        .bg(t.surface)
        .border_b_1()
        .border_color(t.line)
        .child(icon("file", 14.0, t.faint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .text_color(t.text)
                .child(diff.path().to_string()),
        );
    if let Some((word, tone)) = status {
        head = head.child(pill(word, tone, cx));
    }
    head = head
        .child(
            div()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .text_color(t.ok)
                .child(format!("+{added}")),
        )
        .child(
            div()
                .font_family(mono())
                .text_size(px(size::TEXT_SM))
                .text_color(t.danger)
                .child(format!("\u{2212}{removed}")),
        );

    let mut body = div()
        .flex()
        .flex_col()
        .font_family(mono())
        .text_size(px(size::TEXT_SM))
        .line_height(px(19.0));
    if diff.binary {
        body = body.child(
            div()
                .px(px(12.0))
                .py(px(10.0))
                .text_color(t.muted)
                .child("Binary file; there is no text to compare."),
        );
    }
    for (h, hunk) in diff.hunks.iter().enumerate() {
        body = body.child(
            div()
                .px(px(12.0))
                .py(px(3.0))
                .bg(t.hover)
                .text_size(px(size::TEXT_XS))
                .text_color(t.faint)
                .child(format!(
                    "@@ \u{2212}{} +{} @@ {}",
                    hunk.old_start, hunk.new_start, hunk.context
                )),
        );
        for line in &hunk.lines {
            body = body.child(row(line, cx));
        }
        if h + 1 == diff.hunks.len() {
            body = body.child(div().h(px(4.0)));
        }
    }
    div()
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(t.line)
        .bg(t.code)
        .overflow_hidden()
        .child(head)
        .child(
            div()
                .id(ElementId::Name(format!("{id}.lines").into()))
                .overflow_x_scroll()
                .child(body),
        )
}

fn row(line: &Line, cx: &App) -> Div {
    let t = theme(cx);
    let (bg, word, mark, mark_color) = match line.kind {
        Kind::Added => (Some(t.diff_added), t.diff_added_word, "+", t.ok),
        Kind::Removed => (
            Some(t.diff_removed),
            t.diff_removed_word,
            "\u{2212}",
            t.danger,
        ),
        Kind::Context => (None, t.hover, " ", t.faint),
    };
    let number = |n: Option<u32>| {
        div()
            .w(px(44.0))
            .flex_none()
            .flex()
            .justify_end()
            .pr(px(8.0))
            .text_color(t.faint)
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    let shown = if line.text.is_empty() {
        " ".to_string()
    } else {
        line.text.clone()
    };
    let marks: Vec<(Range<usize>, HighlightStyle)> = line
        .changed
        .iter()
        .filter(|(start, end)| {
            start < end
                && *end <= shown.len()
                && shown.is_char_boundary(*start)
                && shown.is_char_boundary(*end)
        })
        .map(|(start, end)| {
            (
                *start..*end,
                HighlightStyle {
                    background_color: Some(word),
                    font_weight: Some(FontWeight::MEDIUM),
                    ..HighlightStyle::default()
                },
            )
        })
        .collect();
    let text = StyledText::new(SharedString::from(shown)).with_highlights(marks);
    let mut el = div()
        .flex()
        .child(number(line.old_no))
        .child(number(line.new_no))
        .child(
            div()
                .w(px(16.0))
                .flex_none()
                .text_color(mark_color)
                .child(mark),
        )
        .child(
            div()
                .pr(px(12.0))
                .whitespace_nowrap()
                .text_color(if line.kind == Kind::Context {
                    t.muted
                } else {
                    t.text
                })
                .child(text),
        );
    if let Some(bg) = bg {
        el = el.bg(bg);
    }
    el
}
