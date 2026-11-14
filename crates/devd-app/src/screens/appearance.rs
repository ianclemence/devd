//! The appearance section of Settings: choosing how the window looks.
//!
//! A theme is chosen by looking at it. Each one is shown as a small window drawn in its
//! own colours, with its own background scene behind it, so what is on the card is what the whole
//! app will become. They stand alone: there is no switch between two forms of one theme, because
//! each look has its own name, its own palette and its own scene.
//!
//! Nothing here holds state of its own: the cards read the window's current theme and a click
//! hands a choice to the shell, which saves it and repaints.

use gpui::{
    App, ClickEvent, Context, Div, ElementId, FontWeight, ObjectFit, SharedString, div, img,
    prelude::*, px,
};

use crate::shell::Shell;
use crate::theme::{Library, Theme, Tone, size, theme};
use crate::ui::{self, Button, ButtonKind, eyebrow};

/// What a person is told about each theme: how it feels, and what is behind it.
const CHARACTER: [(&str, &str, &str); 5] = [
    (
        "sling",
        "Graphite and volt",
        "Ripples spreading across still water. Precise, quiet and exact.",
    ),
    (
        "dawn",
        "Amber and charcoal",
        "First light over the pasture, hill behind hill. Warm and unhurried.",
    ),
    (
        "psalm",
        "Midnight and gold",
        "Silk catching lamplight, a thread of gold through every fold.",
    ),
    (
        "sharon",
        "Blush and rose",
        "A rose-tinted map of the hills, surveyed line by line. Soft and deliberate.",
    ),
    (
        "daybreak",
        "Cream and bronze",
        "Morning light fanning in from the corner. Open and bright.",
    ),
];

/// The section: a heading, the built-in themes three to a row, then the themes from files.
pub fn section(cx: &mut Context<Shell>) -> Div {
    let current = theme(cx).clone();
    let mut grid = div().flex().flex_col().gap(px(14.0));
    for row in CHARACTER.chunks(3) {
        let mut line = div().flex().gap(px(14.0));
        for (id, feel, scene) in row {
            let shown = Theme::by_id(id);
            line = line.child(theme_card(shown, (*feel).into(), (*scene).into(), None, &current, cx));
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
        .child(yours(&current, cx))
}

/// Themes from files in the mods folder: the ones that can be used, as cards like the built-in ones,
/// and the ones that cannot, each with what is wrong and a way to fix it.
fn yours(current: &Theme, cx: &mut Context<Shell>) -> Div {
    let library = Library::of(cx).clone();
    let folder = library.dir.clone();
    let make = Button::new("appearance.make", "Make a theme")
        .icon("plus")
        .kind(ButtonKind::Secondary)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.ask_assistant("/theme ", window, cx);
        }));
    let open = Button::new("appearance.folder", "Open the folder")
        .icon("folder")
        .kind(ButtonKind::Ghost)
        .on_click(move |_, _, _| {
            let _ = std::fs::create_dir_all(&folder);
            let _ = open::that(&folder);
        });
    let head = div()
        .flex()
        .items_end()
        .gap(px(12.0))
        .pt(px(18.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(size::TEXT_LG)).font_weight(FontWeight::SEMIBOLD).child("Your themes"))
                .child(
                    div()
                        .text_size(px(size::TEXT_SM))
                        .text_color(current.muted)
                        .child("Describe a look and the assistant makes it, or write a theme file yourself. Saved changes show at once."),
                ),
        )
        .child(div().flex().flex_none().gap(px(6.0)).child(open).child(make));

    let mut column = div().flex().flex_col().gap(px(14.0)).child(head);
    let usable: Vec<&devd_core::theme::ThemeMod> = library.usable().collect();
    let broken: Vec<(&str, &Vec<String>)> = library
        .found
        .iter()
        .filter_map(|f| f.theme.as_ref().err().map(|problems| (f.id.as_str(), problems)))
        .collect();

    for (id, problems) in &broken {
        let mut list = div().flex().flex_col().gap(px(2.0));
        for problem in problems.iter().take(4) {
            list = list.child(div().min_w_0().child(problem.clone()));
        }
        if problems.len() > 4 {
            list = list.child(div().text_color(current.faint).child(format!("and {} more", problems.len() - 4)));
        }
        let ask = format!(
            "/theme Fix the theme in mods/{id}. devd says:\n{}",
            problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n")
        );
        column = column.child(
            ui::Banner::new(Tone::Warn)
                .title(format!("\u{201c}{id}\u{201d} cannot be used yet"))
                .body(list)
                .action(
                    Button::new(ui::nid("appearance.fix", id.len()), "Fix it with the assistant")
                        .kind(ButtonKind::Primary)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.ask_assistant(&ask, window, cx))),
                )
                .render(cx),
        );
    }

    if usable.is_empty() {
        if broken.is_empty() {
            column = column.child(
                ui::card(cx).child(
                    div().px(px(16.0)).child(ui::empty(
                        "sparkle",
                        "No themes of your own yet",
                        "Choose Make a theme and say how it should look, such as cream paper and brown ink. The assistant picks the colours and a picture, and devd checks that every word can be read.",
                        cx,
                    )),
                ),
            );
        }
        return column;
    }

    let mut grid = div().flex().flex_col().gap(px(14.0));
    for row in usable.chunks(3) {
        let mut line = div().flex().gap(px(14.0));
        for m in row {
            let shown = Theme::from_mod(m);
            line = line.child(theme_card(
                shown,
                m.spec.feel.clone().into(),
                m.manifest.description.clone().into(),
                m.manifest.version.clone(),
                current,
                cx,
            ));
        }
        // An unfilled row keeps its cards the width of the ones above.
        for _ in row.len()..3 {
            line = line.child(div().flex_1());
        }
        grid = grid.child(line);
    }
    column.child(grid)
}

/// One theme: its preview, its name and character, and the colours it is made of.
fn theme_card(
    shown: Theme,
    feel: SharedString,
    scene: SharedString,
    version: Option<String>,
    current: &Theme,
    cx: &mut Context<Shell>,
) -> impl IntoElement {
    let id = shown.id.clone();
    let selected = current.id == shown.id;
    let (line, accent, hover, raised) =
        (current.line, current.accent, current.hover, current.raised);
    let mut name = div().flex().items_center().gap(px(8.0)).child(
        div()
            .text_size(px(size::TEXT_MD))
            .font_weight(FontWeight::SEMIBOLD)
            .child(shown.name.clone()),
    );
    if selected {
        name = name.child(ui::pill("In use", Tone::Ok, cx));
    }
    if let Some(version) = version {
        name = name.child(div().flex_none().text_size(px(size::TEXT_XS)).text_color(current.faint).child(version));
    }
    let mut swatches = div().flex().items_center().gap(px(6.0));
    for colour in [
        shown.bg,
        shown.surface,
        shown.raised,
        shown.muted,
        shown.accent,
    ] {
        swatches = swatches.child(
            div()
                .size(px(16.0))
                .flex_none()
                .rounded_full()
                .bg(colour)
                .border_1()
                .border_color(current.line_strong),
        );
    }
    let body = div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(4.0))
        .child(name)
        .child(
            div()
                .text_size(px(size::TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(current.text)
                .child(feel),
        )
        .child(
            div()
                .text_size(px(size::TEXT_XS))
                .line_height(px(16.0))
                .text_color(current.muted)
                .child(scene),
        );
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
    let card = if selected {
        card.border_color(accent).bg(raised).shadow_sm()
    } else {
        card.border_color(line).hover(move |s| s.bg(hover))
    };
    card.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.choose_theme(&id, cx)))
}

/// A small window, drawn in a theme's own colours with its own scene behind it.
fn preview(p: &Theme, selected: bool, cx: &App) -> Div {
    // The scene is shown stronger than in the window: it is the point of the card.
    let scene = div()
        .absolute()
        .inset_0()
        .opacity((p.art_strength * 3.8).min(0.70))
        .children((!p.art.is_empty()).then(|| img(p.art.clone()).size_full().object_fit(ObjectFit::Fill)));
    let bar = |w: f32, h: f32, colour: gpui::Hsla| {
        div()
            .w(px(w))
            .h(px(h))
            .flex_none()
            .rounded_full()
            .bg(colour)
    };

    // The rail: four places, the first one current.
    let mut rail = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(px(32.0))
        .bg(p.surface)
        .border_r_1()
        .border_color(p.line)
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .pt(px(14.0));
    for n in 0..4 {
        let place = div()
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0));
        rail = rail.child(if n == 0 {
            place
                .bg(p.accent_soft)
                .child(div().size(px(7.0)).rounded(px(2.0)).bg(p.accent))
        } else {
            place.child(div().size(px(7.0)).rounded(px(2.0)).bg(p.faint))
        });
    }

    // A card with three rows: a state, a line of text, and on the first row a signal pill.
    let mut rows = div()
        .flex()
        .flex_col()
        .rounded(px(7.0))
        .bg(p.raised)
        .border_1()
        .border_color(p.line)
        .overflow_hidden();
    for (n, (state, width)) in [(p.ok, 86.0), (p.warn, 62.0), (p.ok, 104.0)]
        .into_iter()
        .enumerate()
    {
        let mut row = div()
            .h(px(22.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().size(px(6.0)).flex_none().rounded_full().bg(state))
            .child(bar(width, 4.5, p.muted));
        if n > 0 {
            row = row.border_t_1().border_color(p.line);
        }
        if n == 0 {
            row = row.child(div().flex_1()).child(
                div()
                    .w(px(28.0))
                    .h(px(11.0))
                    .rounded(px(4.0))
                    .bg(p.accent_soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(bar(14.0, 3.0, p.accent)),
            );
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
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(bar(66.0, 7.0, p.text))
                .child(bar(120.0, 4.0, p.faint)),
        )
        .child(rows)
        .child(
            div()
                .h(px(15.0))
                .w(px(60.0))
                .rounded(px(5.0))
                .bg(p.accent)
                .flex()
                .items_center()
                .justify_center()
                .child(bar(30.0, 3.5, p.on_accent)),
        );

    let mut frame = div()
        .relative()
        .w_full()
        .h(px(158.0))
        .overflow_hidden()
        .rounded(px(size::RADIUS_L))
        .bg(p.bg)
        .border_1()
        .border_color(p.line_strong)
        .child(scene)
        .child(rail)
        .child(content);
    if selected {
        // The tick, in the colour of the theme being worn.
        let badge = div()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .size(px(20.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme(cx).accent)
            .shadow_sm()
            .child(ui::icon("check", 12.0, theme(cx).on_accent));
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
        assert_eq!(
            Theme::ORDER.to_vec(),
            cards,
            "the picker and the theme table must list the same themes in the same order"
        );
        for (id, feel, scene) in CHARACTER {
            assert!(!feel.is_empty() && scene.ends_with('.'), "{id}");
        }
    }

    #[test]
    fn the_words_light_and_dark_are_not_used_to_describe_a_theme() {
        for (id, feel, scene) in CHARACTER {
            let text = format!("{feel} {scene}").to_lowercase();
            assert!(
                !text.contains("light mode")
                    && !text.contains("dark mode")
                    && !text.contains("dark ")
                    && !text.contains(" dark"),
                "{id}: {text}"
            );
        }
    }
}
