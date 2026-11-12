//! The components every screen is built from.
//!
//! Each solves a problem that appears in more than one place: a state needs a
//! dot, an action needs a button, a list needs a row that behaves the same
//! under the pointer everywhere. A screen composes these; it does not restyle
//! them.

pub mod diff;
pub mod input;
pub mod markdown;

use std::time::Duration;

use devd_app_core::nav::Route;
use devd_app_core::status::Status;
use gpui::{
    div, img, prelude::*, px, svg, Animation, AnimationExt, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla,
    SharedString, Stateful, Svg, Window,
};

use crate::theme::{mono, size, theme, Tone};

/// An element id made of a name and a number: the nth row of a named list.
pub fn nid(name: &'static str, n: usize) -> ElementId {
    ElementId::NamedInteger(name.into(), n as u64)
}

/// A colour at a fraction of its opacity: the tint behind a pill, a soft ring.
pub fn tint(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: color.a * alpha, ..color }
}

/// An icon from the window's own set, in a colour.
pub fn icon(name: &str, px_size: f32, color: Hsla) -> Svg {
    svg().path(format!("icons/{name}.svg")).size(px(px_size)).flex_none().text_color(color)
}

/// The icon that stands for a screen on the rail and in the command menu.
pub fn screen_icon(route: &Route) -> &'static str {
    match route {
        Route::Home => "home",
        Route::Inbox => "inbox",
        Route::Assistant(_) => "assistant",
        Route::Runs(_) => "runs",
        Route::Team => "team",
        Route::Projects(_) => "folder",
        Route::Sites(_) => "globe",
        Route::Server => "machine",
        Route::Intelligence => "models",
        Route::Activity => "activity",
        Route::Settings => "settings",
    }
}

/// A state, as a small filled circle. Always drawn beside the word for it.
pub fn dot(tone: Tone, cx: &App) -> Div {
    let color = theme(cx).tone(tone);
    div().size(px(7.0)).flex_none().rounded_full().bg(color)
}

/// A state that is happening now: the dot breathes.
///
/// It breathes in four steps on the window's shared beat, not as an animation
/// of its own. An animation asks for a new frame every frame, so one working
/// agent would keep the whole window redrawing at the display's refresh rate.
pub fn live_dot(_id: impl Into<ElementId>, tone: Tone, cx: &App) -> impl IntoElement {
    const STEPS: [f32; 4] = [1.0, 0.72, 0.42, 0.72];
    let color = theme(cx).tone(tone);
    let level = match cx.try_global::<crate::state::Devd>() {
        Some(state) => {
            state.live_seen.set(true);
            STEPS[usize::from(state.beat) % STEPS.len()]
        }
        None => 1.0,
    };
    div().size(px(7.0)).flex_none().rounded_full().bg(color).opacity(level)
}

/// A state's mark: a dot, which breathes while something is happening.
pub fn state_mark(id: impl Into<ElementId>, status: &Status, cx: &App) -> AnyElement {
    if status.live {
        live_dot(id, status.level.into(), cx).into_any_element()
    } else {
        dot(status.level.into(), cx).into_any_element()
    }
}

/// The marks the window can draw, by the name a provider goes by: the file,
/// and whether the artwork carries its own colours. A mark with no colour of
/// its own is drawn in the text colour, so it reads on either palette.
const BRANDS: &[(&str, &str, bool)] = &[
    ("anthropic", "anthropic.svg", false),
    ("claude", "claude.svg", false),
    ("openai", "openai.svg", false),
    ("deepseek", "deepseek.svg", false),
    ("moonshot", "moonshot.svg", false),
    ("gemini", "gemini.svg", false),
    ("google", "gemini.svg", false),
    ("xai", "xai.svg", false),
    ("mistral", "mistral.svg", false),
    ("groq", "groq.svg", true),
    ("together", "together.png", true),
    ("fireworks", "fireworks.svg", true),
    ("openrouter", "openrouter.svg", false),
    ("ollama-cloud", "ollama-cloud.svg", false),
    ("ollama", "ollama.svg", false),
    ("lm-studio", "lmstudio.svg", false),
    ("lmstudio", "lmstudio.svg", false),
    ("cerebras", "cerebras.svg", false),
    ("perplexity", "perplexity.svg", false),
    ("zai", "zai.svg", false),
    ("cohere", "cohere.svg", false),
    ("sambanova", "sambanova.svg", false),
    ("nvidia", "nvidia.svg", false),
    ("deepinfra", "deepinfra.svg", false),
    ("huggingface", "huggingface.svg", false),
];

/// The mark for a name, or nothing.
///
/// An exact name is the normal case. A name with an instance number after it,
/// `anthropic-2`, is the same vendor. A name that merely starts with a brand
/// is something else, and marking it would be a lie.
fn brand_file(name: &str) -> Option<(&'static str, bool)> {
    let name = name.to_ascii_lowercase();
    let hit = |key: &str| {
        name == key || name.strip_prefix(key).is_some_and(|rest| rest.trim_start_matches(['-', '_', '.', ' ']).starts_with(|c: char| c.is_ascii_digit()) && rest.len() <= 4)
    };
    BRANDS.iter().filter(|(key, ..)| hit(key)).max_by_key(|(key, ..)| key.len()).map(|(_, file, own)| (*file, *own))
}

/// A vendor's mark on a small tile. A vendor devd has no artwork for gets its
/// initial: a mark is never invented.
pub fn brand(name: &str, label: &str, box_px: f32, cx: &App) -> Div {
    let t = theme(cx);
    let tile = div().size(px(box_px)).flex().flex_none().items_center().justify_center().rounded(px(size::RADIUS)).bg(t.hover);
    let mark = box_px * 0.6;
    match brand_file(name) {
        Some((file, true)) => tile.child(img(SharedString::from(format!("brands/{file}"))).size(px(mark)).rounded(px(3.0))),
        Some((file, false)) => tile.child(svg().path(format!("brands/{file}")).size(px(mark)).flex_none().text_color(t.text)),
        None => tile.child(div().text_size(px(size::TEXT_SM)).font_weight(FontWeight::SEMIBOLD).text_color(t.muted).child(label.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default())),
    }
}

/// A word naming a state, tinted with that state's colour.
pub fn pill(text: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let t = theme(cx);
    let color = if tone == Tone::Neutral { t.muted } else { t.tone(tone) };
    div()
        .h(px(18.0))
        .px(px(6.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(size::RADIUS_S))
        .bg(tint(color, if tone == Tone::Neutral { 0.10 } else { 0.14 }))
        .text_size(px(size::TEXT_XS))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(text.into())
}

/// A key, drawn as a key. A sequence is given as separate keys.
pub fn kbd(keys: &str, cx: &App) -> Div {
    let t = theme(cx);
    let mut row = div().flex().flex_none().items_center().gap(px(3.0));
    for key in keys.split_whitespace() {
        row = row.child(
            div()
                .h(px(18.0))
                .min_w(px(18.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(size::RADIUS_S))
                .border_1()
                .border_color(t.line)
                .bg(t.hover)
                .text_size(px(10.5))
                .font_family(mono())
                .text_color(t.muted)
                .child(key_label(key)),
        );
    }
    row
}

/// A key name as it is printed on the key, for this platform.
fn key_label(key: &str) -> String {
    key.split('-')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_string(),
            "cmd" | "secondary" => if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }.to_string(),
            "alt" => if cfg!(target_os = "macos") { "⌥" } else { "Alt" }.to_string(),
            "shift" => "Shift".to_string(),
            "enter" => "Enter".to_string(),
            "escape" => "Esc".to_string(),
            other if other.len() == 1 => other.to_uppercase(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The small uppercase heading above a group.
pub fn eyebrow(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(size::TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme(cx).faint)
        .child(text.into())
}

/// Somebody's face: initials in a circle, ringed when they are working.
pub fn face(initials: impl Into<SharedString>, tone: Tone, working: bool, cx: &App) -> Div {
    let t = theme(cx);
    let color = if tone == Tone::Neutral { t.muted } else { t.tone(tone) };
    div()
        .size(px(20.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(tint(color, 0.16))
        .border_1()
        .border_color(if working { color } else { tint(color, 0.0) })
        .text_size(px(9.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(initials.into())
}

/// How much a button asks for attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one thing this screen is for. At most one per view.
    Primary,
    /// An ordinary action.
    Secondary,
    /// An action that should stay out of the way until wanted.
    Ghost,
    /// An action that removes or cannot be undone.
    Danger,
}

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A button.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<&'static str>,
    keys: Option<&'static str>,
    kind: ButtonKind,
    disabled: bool,
    on_click: Option<Click>,
}

impl Button {
    /// A button with a label.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self { id: id.into(), label: Some(label.into()), icon: None, keys: None, kind: ButtonKind::Secondary, disabled: false, on_click: None }
    }

    /// A button that is only an icon. `label` is still required: it is what a
    /// tooltip and a screen reader say.
    pub fn icon_only(id: impl Into<ElementId>, icon: &'static str) -> Self {
        Self { id: id.into(), label: None, icon: Some(icon), keys: None, kind: ButtonKind::Ghost, disabled: false, on_click: None }
    }

    /// Put an icon before the label.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Show the key that does the same thing.
    pub fn keys(mut self, keys: &'static str) -> Self {
        self.keys = Some(keys);
        self
    }

    /// How much attention it asks for.
    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    /// Draw it as unavailable and ignore clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// What happens when it is pressed.
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme(cx).clone();
        let (bg, fg, border, hover) = match self.kind {
            ButtonKind::Primary => (t.accent, t.on_accent, tint(t.accent, 0.0), tint(t.accent, 0.88)),
            ButtonKind::Secondary => (t.raised, t.text, t.line_strong, t.active),
            ButtonKind::Ghost => (tint(t.text, 0.0), t.muted, tint(t.text, 0.0), t.hover),
            ButtonKind::Danger => (tint(t.danger, 0.12), t.danger, tint(t.danger, 0.30), tint(t.danger, 0.2)),
        };
        let square = self.label.is_none();
        let mut el = div()
            .id(self.id)
            .h(px(size::CONTROL))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(size::RADIUS))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_size(px(size::TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(fg);
        el = if square { el.w(px(size::CONTROL)) } else { el.px(px(10.0)) };
        if let Some(name) = self.icon {
            el = el.child(icon(name, 14.0, fg));
        }
        if let Some(label) = self.label {
            el = el.child(label);
        }
        if let Some(keys) = self.keys {
            el = el.child(kbd(keys, cx));
        }
        if self.disabled {
            return el.opacity(0.45).cursor_not_allowed();
        }
        el = el.cursor_pointer().hover(move |s| s.bg(hover)).active(|s| s.opacity(0.82));
        match self.on_click {
            Some(on_click) => el.on_click(on_click),
            None => el,
        }
    }
}

/// A list row: the same height, radius and response to the pointer everywhere.
pub fn row(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    let hover = t.hover;
    let el = div()
        .id(id)
        .min_h(px(size::ROW))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .rounded(px(size::RADIUS))
        .text_size(px(size::TEXT))
        .text_color(t.text)
        .cursor_pointer();
    if selected {
        el.bg(t.active)
    } else {
        el.hover(move |s| s.bg(hover))
    }
}

/// A bounded group: a hairline box with a step of tone, for content that
/// belongs together. Never a box around a single row.
pub fn card(cx: &App) -> Div {
    let t = theme(cx);
    div().rounded(px(size::RADIUS_L)).border_1().border_color(t.line).bg(t.surface)
}

/// The hairline between two things.
pub fn rule(cx: &App) -> Div {
    div().h(px(1.0)).w_full().flex_none().bg(theme(cx).line)
}

/// A screen with nothing to list, saying so and offering what resolves it.
pub fn empty(icon_name: &'static str, title: impl Into<SharedString>, detail: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .py(px(56.0))
        .child(
            div()
                .size(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(size::RADIUS_L))
                .bg(t.hover)
                .child(icon(icon_name, 18.0, t.faint)),
        )
        .child(div().text_size(px(size::TEXT_MD)).font_weight(FontWeight::MEDIUM).text_color(t.text).child(title.into()))
        .child(div().max_w(px(380.0)).text_size(px(size::TEXT)).text_color(t.muted).child(detail.into()))
}

/// A line of placeholder while a first load is in flight. It appears only
/// when there is truly nothing to show: a cached value is always drawn instead.
pub fn skeleton(id: impl Into<ElementId>, width: f32, cx: &App) -> impl IntoElement {
    let color = theme(cx).hover;
    div()
        .h(px(10.0))
        .w(px(width))
        .rounded(px(size::RADIUS_S))
        .bg(color)
        .with_animation(id, Animation::new(Duration::from_millis(1400)).repeat(), |el, t| {
            let wave = (t * std::f32::consts::TAU).cos() * 0.5 + 0.5;
            el.opacity(0.45 + 0.55 * wave)
        })
}

/// A page's heading: its title, one line under it, and its actions on the right.
pub fn page_head(title: impl Into<SharedString>, detail: impl Into<SharedString>, actions: Div, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .flex()
        .items_end()
        .justify_between()
        .gap(px(16.0))
        .pb(px(18.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(div().text_size(px(size::TEXT_XL)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(title.into()))
                .child(div().text_size(px(size::TEXT)).text_color(t.muted).child(detail.into())),
        )
        .child(actions)
}

/// A scrolling column, centred, no wider than is comfortable to read across.
pub fn page(id: impl Into<ElementId>, max_width: f32, content: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .child(div().w_full().max_w(px(max_width)).mx_auto().px(px(32.0)).py(px(28.0)).child(content))
}

/// The last part of a path, which is how a project is named in a list.
pub fn leaf(path: &str) -> &str {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path)
}

/// "3m", "2h", "5d": how long ago an RFC 3339 time was.
pub fn ago(rfc3339: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(rfc3339) else { return String::new() };
    let secs = (chrono::Utc::now() - then.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=2_591_999 => format!("{}d", secs / 86_400),
        _ => format!("{}mo", secs / 2_592_000),
    }
}

/// A count with a unit a person reads: 1,234,567 tokens is "1.2M".
pub fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_named_by_its_folder() {
        assert_eq!(leaf(r"D:\devd\www\nairobiunwind"), "nairobiunwind");
        assert_eq!(leaf("/srv/www/shop/"), "shop");
        assert_eq!(leaf("shop"), "shop");
    }

    #[test]
    fn every_mark_in_the_table_is_compiled_in() {
        for (name, file, _) in BRANDS {
            assert!(devd_server::ui::brand_bytes(file).is_some(), "{name} names {file}, which is not in the binary");
        }
    }

    #[test]
    fn a_mark_is_for_the_vendor_and_its_instances_only() {
        assert_eq!(brand_file("anthropic").map(|b| b.0), Some("anthropic.svg"));
        assert_eq!(brand_file("anthropic-2").map(|b| b.0), Some("anthropic.svg"));
        assert_eq!(brand_file("Ollama-Cloud").map(|b| b.0), Some("ollama-cloud.svg"));
        assert_eq!(brand_file("ollama").map(|b| b.0), Some("ollama.svg"));
        assert_eq!(brand_file("openai-compatible-gateway"), None);
        assert_eq!(brand_file("work-gateway"), None);
    }

    #[test]
    fn large_counts_are_shortened() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(10_435), "10.4k");
        assert_eq!(compact(107_300_000), "107.3M");
    }

    #[test]
    fn keys_are_printed_as_they_are_on_the_keyboard() {
        assert_eq!(key_label("g"), "G");
        assert_eq!(key_label("escape"), "Esc");
        assert!(key_label("ctrl-shift-p").ends_with("Shift+P"));
    }

    #[test]
    fn a_time_that_cannot_be_read_is_blank_not_wrong() {
        assert_eq!(ago("yesterday"), "");
        assert_eq!(ago(&chrono::Utc::now().to_rfc3339()), "now");
    }
}
