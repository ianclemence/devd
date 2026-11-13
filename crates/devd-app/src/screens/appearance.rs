//! The appearance section of Settings: choosing how the window looks.
//!
//! A theme is chosen by looking at it. Each one is shown as a small window drawn in its
//! own colours, with its own background scene behind it, so what is on the card is what the whole
//! app will become. They stand alone: there is no switch between two forms of one theme, because
//! each look has its own name, its own palette and its own scene.
//!
//! Nothing here holds state of its own: the cards read the window's current theme and a click
//! hands a choice to the shell, which saves it and repaints.

use gpui::{div, img, prelude::*, px, App, ClickEvent, Context, Div, ElementId, FontWeight, ObjectFit, SharedString};

use crate::shell::Shell;
use crate::theme::{size, theme, Theme, Tone};
use crate::ui::{self, eyebrow};

/// What a person is told about each theme: how it feels, and what is behind it.
const CHARACTER: [(&str, &str, &str); 5] = [
    ("sling", "Graphite and volt", "Ripples spreading across still water. Precise, quiet and exact."),
    ("dawn", "Amber and charcoal", "First light over the pasture, hill behind hill. Warm and unhurried."),
    ("psalm", "Midnight and gold", "Silk catching lamplight, a thread of gold through every fold."),
    ("sharon", "Blush and rose", "A rose-tinted map of the hills, surveyed line by line. Soft and deliberate."),
    ("daybreak", "Cream and bronze", "Morning light fanning in from the corner. Open and bright."),
];

/// The section: a heading and a grid of the themes, three to a row.
pub fn section(cx: &mut Context<Shell>) -> Div {
    let current = theme(cx).clone();
    let mut grid = div().flex().flex_col().gap(px(14.0));
    for row in CHARACTER.chunks(3) {
        let mut line = div().flex().gap(px(14.0));
        for (id, feel, scene) in row {
            let card = theme_card(id, feel, scene, &current, cx);
            line = line.child(card);
        }
        grid = grid.child(line);
    }
    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(eyebrow("APPEARANCE", cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).child("Theme"))
                .child(div().text_size(px(size::TEXT_SM)).text_color(current.muted).child("Each theme is a whole look: its own colours and its own picture behind every screen.")),
        )
        .child(grid)
}

/// One theme: its preview, its name and character, and the colours it is made of.
fn theme_card(id: &'static str, feel: &'static str, scene: &'static str, current: &Theme, cx: &mut Context<Shell>) -> impl IntoElement {
    let selected = current.id == id;
    let shown = Theme::by_id(id);
    let (line, accent, hover, raised) = (current.line, current.accent, current.hover, current.raised);
    let mut name = div().flex().items_center().gap(px(8.0)).child(div().text_size(px(size::TEXT_MD)).font_weight(FontWeight::SEMIBOLD).child(shown.name));
    if selected {
        name = name.child(ui::pill("In use", Tone::Ok, cx));
    }
    let mut swatches = div().flex().items_center().gap(px(6.0));
    for colour in [shown.bg, shown.surface, shown.raised, shown.muted, shown.accent] {
        swatches = swatches.child(div().size(px(16.0)).flex_none().rounded_full().bg(colour).border_1().border_color(current.line_strong));
    }
    let body = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(4.0))
        .child(name)
        .child(div().text_size(px(size::TEXT_SM)).font_weight(FontWeight::MEDIUM).text_color(current.text).child(feel))
        .child(div().text_size(px(size::TEXT_XS)).line_height(px(16.0)).text_color(current.muted).child(scene));
    let card = div()
        .id(ElementId::Name(format!("appearance.theme.{id}").into()))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .p(px(10.0))
        .rounded(px(size::RADIUS_L + 4.0))
        .border_1()
        .cursor_pointer()
        .child(preview(&shown, selected, cx))
        .child(body)
        .child(div().px(px(4.0)).pb(px(2.0)).child(swatches));
    let card = if selected { card.border_color(accent).bg(raised).shadow_sm() } else { card.border_color(line).hover(move |s| s.bg(hover)) };
    card.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.choose_theme(id, cx)))
}

/// A small window, drawn in a theme's own colours with its own scene behind it.
fn preview(p: &Theme, selected: bool, cx: &App) -> Div {
    let file = format!("art/{}.png", p.id);
    // The scene is shown stronger than in the window: it is the point of the card.
    let scene = div().absolute().inset_0().opacity((p.pattern.strength(p.dark) * 3.8).min(0.70)).child(img(SharedString::from(file)).size_full().object_fit(ObjectFit::Fill));
    let bar = |w: f32, h: f32, colour: gpui::Hsla| div().w(px(w)).h(px(h)).flex_none().rounded_full().bg(colour);

    // The rail: four places, the first one current.
    let mut rail = div().absolute().top_0().bottom_0().left_0().w(px(32.0)).bg(p.surface).border_r_1().border_color(p.line).flex().flex_col().items_center().gap(px(8.0)).pt(px(14.0));
    for n in 0..4 {
        let place = div().size(px(18.0)).flex().items_center().justify_center().rounded(px(5.0));
        rail = rail.child(if n == 0 {
            place.bg(p.accent_soft).child(div().size(px(7.0)).rounded(px(2.0)).bg(p.accent))
        } else {
            place.child(div().size(px(7.0)).rounded(px(2.0)).bg(p.faint))
        });
    }

    // A card with three rows: a state, a line of text, and on the first row a signal pill.
    let mut rows = div().flex().flex_col().rounded(px(7.0)).bg(p.raised).border_1().border_color(p.line).overflow_hidden();
    for (n, (state, width)) in [(p.ok, 86.0), (p.warn, 62.0), (p.ok, 104.0)].into_iter().enumerate() {
        let mut row = div().h(px(22.0)).px(px(9.0)).flex().items_center().gap(px(8.0)).child(div().size(px(6.0)).flex_none().rounded_full().bg(state)).child(bar(width, 4.5, p.muted));
        if n > 0 {
            row = row.border_t_1().border_color(p.line);
        }
        if n == 0 {
            row = row.child(div().flex_1()).child(div().w(px(28.0)).h(px(11.0)).rounded(px(4.0)).bg(p.accent_soft).flex().items_center().justify_center().child(bar(14.0, 3.0, p.accent)));
        }
        rows = rows.child(row);
    }

    let content = div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .left(px(32.0))
        .px(px(14.0))
        .pt(px(13.0))
        .flex()
        .flex_col()
        .gap(px(9.0))
        .child(div().flex().flex_col().gap(px(5.0)).child(bar(66.0, 7.0, p.text)).child(bar(120.0, 4.0, p.faint)))
        .child(rows)
        .child(div().h(px(15.0)).w(px(60.0)).rounded(px(5.0)).bg(p.accent).flex().items_center().justify_center().child(bar(30.0, 3.5, p.on_accent)));

    let mut frame = div().relative().w_full().h(px(158.0)).overflow_hidden().rounded(px(size::RADIUS_L)).bg(p.bg).border_1().border_color(p.line_strong).child(scene).child(rail).child(content);
    if selected {
        // The tick, in the colour of the theme being worn.
        let badge = div().absolute().top(px(8.0)).right(px(8.0)).size(px(20.0)).flex().items_center().justify_center().rounded_full().bg(theme(cx).accent).shadow_sm().child(ui::icon("check", 12.0, theme(cx).on_accent));
        frame = frame.child(badge);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_a_card_and_every_card_a_theme_in_the_same_order() {
        let cards: Vec<&str> = CHARACTER.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(Theme::ORDER.to_vec(), cards, "the picker and the theme table must list the same themes in the same order");
        for (id, feel, scene) in CHARACTER {
            assert!(!feel.is_empty() && scene.ends_with('.'), "{id}");
        }
    }

    #[test]
    fn the_words_light_and_dark_are_not_used_to_describe_a_theme() {
        for (id, feel, scene) in CHARACTER {
            let text = format!("{feel} {scene}").to_lowercase();
            assert!(!text.contains("light mode") && !text.contains("dark mode") && !text.contains("dark ") && !text.contains(" dark"), "{id}: {text}");
        }
    }
}
