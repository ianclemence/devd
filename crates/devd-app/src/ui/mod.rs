//! The components every screen is built from.
//!
//! Each solves a problem that appears in more than one place: a state needs a
//! dot, an action needs a button, a list needs a row that behaves the same
//! under the pointer everywhere. A screen composes these; it does not restyle
//! them.

pub mod clip;
pub mod diff;
pub mod dither;
pub mod favicon;
pub mod flow;
pub mod genui;
pub mod input;
pub mod links;
pub mod markdown;
pub mod motion;

use std::time::Duration;

use devd_app_core::nav::Route;
use devd_app_core::status::Status;
use gpui::{
    Animation, AnimationExt, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla,
    SharedString, Stateful, Svg, Window, div, img, prelude::*, px, svg,
};

pub use clip::Clip;

use crate::theme::{Tone, mono, size, theme};

/// An element id made of a name and a number: the nth row of a named list.
pub fn nid(name: &'static str, n: usize) -> ElementId {
    ElementId::NamedInteger(name.into(), n as u64)
}

/// A colour at a fraction of its opacity: the tint behind a pill, a soft ring.
pub fn tint(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

/// An icon from the window's own set, in a colour.
pub fn icon(name: &str, px_size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .size(px(px_size))
        .flex_none()
        .text_color(color)
}

/// The icon that stands for a screen on the rail and in the command menu.
pub fn screen_icon(route: &Route) -> &'static str {
    match route {
        Route::Home => "home",
        Route::Inbox => "inbox",
        Route::Assistant(_) => "assistant",
        Route::Runs(_) => "runs",
        Route::Team => "team",
        Route::Projects(_) | Route::Project(..) => "folder",
        Route::Sites(_) => "globe",
        Route::Server => "machine",
        Route::Intelligence => "brain",
        Route::Activity => "activity",
        Route::Settings => "settings",
        Route::Page(id) => match devd_app_core::pages::find(id).map(|p| p.area) {
            Some(devd_app_core::pages::Area::Projects) => "folder",
            Some(devd_app_core::pages::Area::Sites) => "globe",
            Some(devd_app_core::pages::Area::Team) => "team",
            Some(devd_app_core::pages::Area::Server) => "machine",
            _ => "settings",
        },
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
    div()
        .size(px(7.0))
        .flex_none()
        .rounded_full()
        .bg(color)
        .opacity(level)
}

/// The colour a team role is drawn in, wherever it appears: the same hue on its card in a run's chain, on its face in the
/// top bar and beside its name. Known roles keep a hue of their own; any other role (a custom one) gets one from its name,
/// the same every time.
pub fn role_color(role: &str, cx: &App) -> Hsla {
    let hue = match role {
        "planner" => 0.76,
        "product" => 0.83,
        "architect" => 0.66,
        "designer" | "motion" => 0.92,
        "frontend" => 0.58,
        "backend" => 0.47,
        "implementer" => 0.60,
        "qa" => 0.34,
        "security" => 0.01,
        "reviewer" => 0.08,
        "docs" | "content" => 0.12,
        other => {
            // FNV-1a over the name, so a custom role keeps its colour from one run to the next.
            let mut h: u32 = 0x811c_9dc5;
            for b in other.bytes() {
                h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
            }
            (h % 360) as f32 / 360.0
        }
    };
    let dark = theme(cx).dark;
    gpui::hsla(hue, 0.58, if dark { 0.68 } else { 0.38 }, 1.0)
}

/// A few words that appear beside what the pointer rests on.
pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px(px(9.0))
            .py(px(5.0))
            .max_w(px(320.0))
            .rounded(px(size::RADIUS))
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow_md()
            .text_size(px(size::TEXT_XS))
            .text_color(t.text)
            .child(self.0.clone())
    }
}

/// A tooltip for `.tooltip(...)`: `element.tooltip(ui::tip("Open the conversation"))`.
pub fn tip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
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

/// The marks of the tools, runtimes, services and stacks devd looks after, by the id devd knows each by: the file. Every
/// one carries its owner's own colours and is never recoloured or redrawn.
const TOOL_MARKS: &[(&str, &str)] = &[
    ("php", "php.svg"),
    ("node", "node.svg"),
    ("nodejs", "node.svg"),
    ("go", "go.svg"),
    ("python", "python.svg"),
    ("ruby", "ruby.svg"),
    ("rust", "rust.svg"),
    ("java", "java.svg"),
    ("dotnet", "dotnet.svg"),
    ("aspnet-core", "dotnet.svg"),
    ("kotlin", "kotlin.svg"),
    ("swift", "swift.svg"),
    ("dart", "dart.svg"),
    ("flutter", "flutter.svg"),
    ("javascript", "javascript.svg"),
    ("typescript", "typescript.svg"),
    ("mariadb", "mariadb.svg"),
    ("mysql", "mysql.svg"),
    ("postgresql", "postgresql.svg"),
    ("postgres", "postgresql.svg"),
    ("redis", "redis.svg"),
    ("valkey", "valkey.png"),
    ("sqlite", "sqlite.svg"),
    ("elasticsearch", "elasticsearch.svg"),
    ("meilisearch", "meilisearch.svg"),
    ("typesense", "typesense.png"),
    ("minio", "minio.svg"),
    ("nginx", "nginx.svg"),
    ("mailpit", "mailpit.svg"),
    ("phpmyadmin", "phpmyadmin.svg"),
    ("xdebug", "xdebug.png"),
    ("docker", "docker.svg"),
    ("git", "git.svg"),
    ("gh", "gh.svg"),
    ("gitlab", "gitlab.svg"),
    ("composer", "composer.svg"),
    ("npm", "npm.svg"),
    ("pnpm", "pnpm.svg"),
    ("yarn", "yarn.svg"),
    ("bun", "bun.svg"),
    ("uv", "uv.svg"),
    ("poetry", "poetry.svg"),
    ("bundler", "bundler.svg"),
    ("maven", "maven.svg"),
    ("gradle", "gradle.svg"),
    ("vite", "vite.svg"),
    ("android-sdk", "android.svg"),
    ("adb", "android.svg"),
    ("android", "android.svg"),
    ("build-tools", "build-tools.svg"),
    ("expo", "expo.svg"),
    ("eas-cli", "eascli.svg"),
    ("react-native", "react-native.svg"),
    ("react-vite", "react-vite.svg"),
    ("nextjs", "nextjs.svg"),
    ("astro", "astro.svg"),
    ("vue", "vue.svg"),
    ("sveltekit", "sveltekit.svg"),
    ("express", "express.svg"),
    ("fastapi", "fastapi.svg"),
    ("flask", "flask.svg"),
    ("django", "django.svg"),
    ("rails", "rails.svg"),
    ("laravel", "laravel.svg"),
    ("slim", "slim.svg"),
    ("cakephp", "cakephp.svg"),
    ("spring-boot", "spring-boot.svg"),
    ("tailwind", "tailwind.svg"),
    // The coding agents devd can connect to.
    ("claude", "claude.svg"),
    ("claude-code", "claude.svg"),
    ("claude-desktop", "claude.svg"),
    ("cursor", "cursor.svg"),
    ("vscode", "vscode.png"),
    ("codex", "openai.svg"),
    ("opencode", "opencode.svg"),
];

/// The mark for a tool, runtime, service or stack. An exact id first; otherwise its first word, so `php-cgi-8.4-2` is PHP.
/// Nothing is guessed beyond that: a name with no mark gets none.
fn tool_mark_file(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    let find = |key: &str| TOOL_MARKS.iter().find(|(k, _)| *k == key).map(|(_, f)| *f);
    find(&name).or_else(|| name.split(['-', '_', '.', ' ']).next().and_then(find))
}

/// A tool's, runtime's, service's or stack's own mark on a small pale tile, which keeps a dark mark readable on a dark theme.
/// One with no mark gets its initial, never something made up.
pub fn tool_mark(name: &str, label: &str, box_px: f32, cx: &App) -> Div {
    let t = theme(cx);
    let tile = div()
        .size(px(box_px))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size::RADIUS));
    match tool_mark_file(name) {
        Some(file) => tile
            .bg(gpui::rgba(0xfffffff0))
            .child(img(SharedString::from(format!("brands/{file}"))).size(px(box_px * 0.66))),
        None => tile.bg(t.hover).child(
            div()
                .text_size(px(size::TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(
                    label
                        .chars()
                        .find(|c| c.is_alphanumeric())
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default(),
                ),
        ),
    }
}

/// The mark for a name, or nothing.
///
/// An exact name is the normal case. A name with an instance number after it,
/// `anthropic-2`, is the same vendor. A name that merely starts with a brand
/// is something else, and marking it would be a lie.
fn brand_file(name: &str) -> Option<(&'static str, bool)> {
    let name = name.to_ascii_lowercase();
    let hit = |key: &str| {
        name == key
            || name.strip_prefix(key).is_some_and(|rest| {
                rest.trim_start_matches(['-', '_', '.', ' '])
                    .starts_with(|c: char| c.is_ascii_digit())
                    && rest.len() <= 4
            })
    };
    BRANDS
        .iter()
        .filter(|(key, ..)| hit(key))
        .max_by_key(|(key, ..)| key.len())
        .map(|(_, file, own)| (*file, *own))
}

/// A vendor's mark on a small tile. A vendor devd has no artwork for gets its
/// initial: a mark is never invented.
pub fn brand(name: &str, label: &str, box_px: f32, cx: &App) -> Div {
    let t = theme(cx);
    let tile = div()
        .size(px(box_px))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size::RADIUS))
        .bg(t.hover);
    let mark = box_px * 0.6;
    match brand_file(name) {
        Some((file, true)) => tile.child(
            img(SharedString::from(format!("brands/{file}")))
                .size(px(mark))
                .rounded(px(3.0)),
        ),
        Some((file, false)) => tile.child(
            svg()
                .path(format!("brands/{file}"))
                .size(px(mark))
                .flex_none()
                .text_color(t.text),
        ),
        None => tile.child(
            div()
                .text_size(px(size::TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(
                    label
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default(),
                ),
        ),
    }
}

/// A word naming a state, tinted with that state's colour.
pub fn pill(text: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let t = theme(cx);
    let color = if tone == Tone::Neutral {
        t.muted
    } else {
        t.tone(tone)
    };
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
            "cmd" | "secondary" => if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl"
            }
            .to_string(),
            "alt" => if cfg!(target_os = "macos") {
                "⌥"
            } else {
                "Alt"
            }
            .to_string(),
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
    let color = if tone == Tone::Neutral {
        t.muted
    } else {
        t.tone(tone)
    };
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

/// A face in a colour of its own: a role's, so the same agent is the same colour on its card, in the top bar and in a list.
pub fn face_in(initials: impl Into<SharedString>, color: Hsla, working: bool) -> Div {
    div()
        .size(px(22.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(tint(color, 0.18))
        .border_1()
        .border_color(if working { color } else { tint(color, 0.0) })
        .text_size(px(10.0))
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
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            keys: None,
            kind: ButtonKind::Secondary,
            disabled: false,
            on_click: None,
        }
    }

    /// A button that is only an icon. `label` is still required: it is what a
    /// tooltip and a screen reader say.
    pub fn icon_only(id: impl Into<ElementId>, icon: &'static str) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: Some(icon),
            keys: None,
            kind: ButtonKind::Ghost,
            disabled: false,
            on_click: None,
        }
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
            ButtonKind::Primary => (
                t.accent,
                t.on_accent,
                tint(t.accent, 0.0),
                tint(t.accent, 0.88),
            ),
            ButtonKind::Secondary => (t.raised, t.text, t.line_strong, t.active),
            ButtonKind::Ghost => (tint(t.text, 0.0), t.muted, tint(t.text, 0.0), t.hover),
            ButtonKind::Danger => (
                tint(t.danger, 0.12),
                t.danger,
                tint(t.danger, 0.30),
                tint(t.danger, 0.2),
            ),
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
        el = if square {
            el.w(px(size::CONTROL))
        } else {
            el.px(px(10.0))
        };
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
        el = el
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .active(|s| s.opacity(0.82));
        match self.on_click {
            Some(on_click) => el.on_click(move |event, window, cx| {
                // A button inside a clickable row is a button, not the row:
                // pressing Delete must not also open the row's detail.
                cx.stop_propagation();
                on_click(event, window, cx)
            }),
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

/// The first `max` characters of a line, with an ellipsis when it was cut.
pub fn clip_chars(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{}…", head.trim_end())
    } else {
        head
    }
}

/// A small icon button for the end of a row: out of sight until the row is pointed at, so a list stays quiet and
/// the action is still one click away. `danger` tints it when the pointer is on it, for what removes something.
/// `group` names the row, which must carry `.group(group)`.
pub fn row_action(
    id: impl Into<ElementId>,
    group: &'static str,
    icon_name: &'static str,
    tip_text: &'static str,
    danger: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let t = theme(cx);
    let (hover_bg, hover_fg, rest) = if danger {
        (tint(t.danger, 0.14), t.danger, t.faint)
    } else {
        (t.hover, t.text, t.faint)
    };
    div().invisible().group_hover(group, |s| s.visible()).child(
        div()
            .id(id)
            .size(px(26.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(size::RADIUS))
            .cursor_pointer()
            .text_color(rest)
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .child(
                svg()
                    .path(format!("icons/{icon_name}.svg"))
                    .size(px(14.0))
                    .flex_none()
                    .text_color(gpui::black())
                    .text_color(rest),
            )
            .tooltip(tip(tip_text))
            .on_click(move |ev, window, cx| {
                cx.stop_propagation();
                on_click(ev, window, cx)
            }),
    )
}

/// How long something has been going on: "45s", "2m 05s", "1h 02m".
pub fn age(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// A banner: something the person should know before what follows it, or should act on.
///
/// Every banner in the app is this one shape, so a warning on a run's page and one in a conversation look like
/// relatives: a hairline in the tone, a faint wash of it, the tone's mark level with the first line, an optional title
/// in the text colour, what happened under it, and the actions under that, never squeezed beside the words. Words wrap;
/// nothing is cut off.
pub struct Banner {
    tone: Tone,
    title: Option<SharedString>,
    body: Option<AnyElement>,
    actions: Vec<AnyElement>,
}

impl Banner {
    /// A banner in a tone. `Danger` for what stopped or failed, `Warn` for what is held or needs a decision, `Info`
    /// for what is worth knowing, `Ok` for what went well.
    pub fn new(tone: Tone) -> Self {
        Self { tone, title: None, body: None, actions: Vec::new() }
    }

    /// The line that names what happened.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// What happened, in words.
    pub fn text(self, text: impl Into<SharedString>) -> Self {
        let text: SharedString = text.into();
        self.body(div().child(text))
    }

    /// What happened, as an element (a formatted message, a list).
    pub fn body(mut self, body: impl IntoElement) -> Self {
        self.body = Some(body.into_any_element());
        self
    }

    /// Something to do about it. The first one added is the main one.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// Draw it.
    pub fn render(self, cx: &App) -> Div {
        let t = theme(cx);
        let colour = t.tone(self.tone);
        let mark = match self.tone {
            Tone::Danger | Tone::Warn => "alert",
            Tone::Ok => "check",
            _ => "info",
        };
        let titled = self.title.is_some();
        let mut words = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .text_size(px(size::TEXT))
            .line_height(px(20.0))
            .text_color(t.text);
        if let Some(title) = self.title {
            words = words.child(div().font_weight(FontWeight::SEMIBOLD).child(title));
        }
        if let Some(body) = self.body {
            words = words.child(div().min_w_0().text_color(if titled { t.muted } else { t.text }).child(body));
        }
        if !self.actions.is_empty() {
            words = words.child(div().pt(px(8.0)).flex().flex_wrap().items_center().gap(px(8.0)).children(self.actions));
        }
        div()
            .w_full()
            .flex()
            .items_start()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(size::RADIUS_L))
            .border_1()
            .border_color(tint(colour, 0.32))
            .bg(tint(colour, 0.07))
            .child(div().h(px(20.0)).flex_none().flex().items_center().child(icon(mark, 15.0, colour)))
            .child(words)
    }
}

/// A strip that says what is on screen may be old: the last refresh failed, and what is shown is
/// what was known before. Never shown as if it were current, and never as "offline" or "stopped":
/// a failed read says nothing about the thing itself.
pub fn stale(snapshot: &devd_app_core::sync::Snapshot, cx: &App) -> Option<Div> {
    let failure = snapshot
        .error
        .as_ref()
        .filter(|_| snapshot.data.is_some())?;
    Some(
        div().mb(px(14.0)).child(
            Banner::new(Tone::Warn)
                .text(format!(
                    "Could not refresh just now: {}. What you see may be out of date.",
                    failure.message.trim_end_matches('.')
                ))
                .render(cx),
        ),
    )
}

/// A bounded group: a hairline box with a step of tone, for content that
/// belongs together. Never a box around a single row.
pub fn card(cx: &App) -> Div {
    let t = theme(cx);
    div()
        .rounded(px(size::RADIUS_L))
        .border_1()
        .border_color(t.line)
        .bg(t.surface)
}

/// The hairline between two things.
pub fn rule(cx: &App) -> Div {
    div().h(px(1.0)).w_full().flex_none().bg(theme(cx).line)
}

/// A screen with nothing to list, saying so and offering what resolves it.
pub fn empty(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    cx: &App,
) -> Div {
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
        .child(
            div()
                .text_size(px(size::TEXT_MD))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
                .child(title.into()),
        )
        .child(
            // Centred like the title above it, so a sentence that wraps does not hang from the left.
            div()
                .max_w(px(420.0))
                .text_center()
                .text_size(px(size::TEXT))
                .line_height(px(21.0))
                .text_color(t.muted)
                .child(detail.into()),
        )
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
        .with_animation(
            id,
            Animation::new(Duration::from_millis(1400)).repeat(),
            |el, t| {
                let wave = (t * std::f32::consts::TAU).cos() * 0.5 + 0.5;
                el.opacity(0.45 + 0.55 * wave)
            },
        )
}

/// A page's heading: its title, one line under it, and its actions on the right.
pub fn page_head(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    actions: Div,
    cx: &App,
) -> Div {
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
                .child(
                    div()
                        .text_size(px(size::TEXT_XL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.text)
                        .child(title.into()),
                )
                .child(
                    div()
                        .text_size(px(size::TEXT))
                        .text_color(t.muted)
                        .child(detail.into()),
                ),
        )
        .child(actions)
}

/// The dotted art behind a screen's top corner. Faint, behind everything, and never
/// under dense content: it belongs to the screens that open on a greeting or a question.
pub fn art(cx: &App) -> Div {
    let t = theme(cx);
    // A theme from a file may have no picture at all.
    if t.art.is_empty() {
        return div();
    }
    let file = t.art.clone();
    let el = div()
        .absolute()
        .right(px(0.0))
        .w(px(dither::WIDTH as f32))
        .h(px(dither::HEIGHT as f32))
        .opacity(t.art_strength);
    let el = if t.pattern.stands_on_the_bottom() {
        el.bottom(px(0.0))
    } else {
        el.top(px(0.0))
    };
    el.child(
        img(file)
            .w(px(dither::WIDTH as f32))
            .h(px(dither::HEIGHT as f32))
            .object_fit(gpui::ObjectFit::Fill),
    )
}

/// A scrolling column, centred, no wider than is comfortable to read across.
pub fn page(id: impl Into<ElementId>, max_width: f32, content: impl IntoElement) -> Stateful<Div> {
    div().id(id).size_full().overflow_y_scroll().child(
        div()
            .w_full()
            .max_w(px(max_width))
            .mx_auto()
            .px(px(32.0))
            .py(px(28.0))
            .child(content),
    )
}

/// The last part of a path, which is how a project is named in a list.
pub fn leaf(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

/// "3m", "2h", "5d": how long until an RFC 3339 time is.
///
/// The counterpart of [`ago`](#method.ago): a timetable's next run is in the
/// future, and reading it as "now" was wrong.
pub fn until(rfc3339: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return String::new();
    };
    let secs = (then.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
    if secs <= 0 {
        return "now".into();
    }
    // Round up: a timetable three minutes away should not read as "2m".
    match secs {
        0..=59 => "less than a minute".into(),
        60..=3599 => format!("{}m", (secs + 59) / 60),
        3600..=86_399 => format!("{}h", (secs + 3599) / 3600),
        86_400..=2_591_999 => format!("{}d", (secs + 86_399) / 86_400),
        _ => format!("{}mo", (secs + 2_591_999) / 2_592_000),
    }
}

/// A future time in words: "in 3m", or "due now" when it has passed.
pub fn in_time(rfc3339: &str) -> String {
    let left = until(rfc3339);
    if left == "now" {
        "due now".to_string()
    } else {
        format!("in {left}")
    }
}

/// "3m", "2h", "5d": how long ago an RFC 3339 time was.
pub fn ago(rfc3339: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return String::new();
    };
    let secs = (chrono::Utc::now() - then.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
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
    fn a_future_time_reads_as_in_three_minutes_not_three_minutes_ago() {
        let soon = (chrono::Utc::now() + chrono::Duration::minutes(3)).to_rfc3339();
        assert_eq!(until(&soon), "3m");
        assert_eq!(in_time(&soon), "in 3m");
        let past = (chrono::Utc::now() - chrono::Duration::minutes(3)).to_rfc3339();
        assert_eq!(until(&past), "now");
        assert_eq!(in_time(&past), "due now");
        assert_eq!(until("not a time"), "");
    }

    #[test]
    fn every_mark_in_the_table_is_compiled_in() {
        for (name, file, _) in BRANDS {
            assert!(
                devd_server::ui::brand_bytes(file).is_some(),
                "{name} names {file}, which is not in the binary"
            );
        }
    }

    #[test]
    fn a_mark_is_for_the_vendor_and_its_instances_only() {
        assert_eq!(brand_file("anthropic").map(|b| b.0), Some("anthropic.svg"));
        assert_eq!(
            brand_file("anthropic-2").map(|b| b.0),
            Some("anthropic.svg")
        );
        assert_eq!(
            brand_file("Ollama-Cloud").map(|b| b.0),
            Some("ollama-cloud.svg")
        );
        assert_eq!(brand_file("ollama").map(|b| b.0), Some("ollama.svg"));
        assert_eq!(brand_file("openai-compatible-gateway"), None);
        assert_eq!(brand_file("work-gateway"), None);
    }

    #[test]
    fn a_wait_is_said_in_the_largest_sensible_unit() {
        assert_eq!(
            (age(7), age(59), age(60), age(125), age(3600), age(3725)),
            (
                "7s".to_string(),
                "59s".to_string(),
                "1m 00s".to_string(),
                "2m 05s".to_string(),
                "1h 00m".to_string(),
                "1h 02m".to_string()
            )
        );
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

    #[test]
    fn every_mark_a_tool_may_have_is_in_the_binary_and_a_service_pool_shares_its_runtimes() {
        for (name, file) in TOOL_MARKS {
            assert!(
                devd_server::ui::brand_bytes(file).is_some(),
                "{name} names {file}, which is not in the binary"
            );
        }
        assert_eq!(tool_mark_file("php-cgi-8.4-2"), Some("php.svg"));
        assert_eq!(tool_mark_file("PHP"), Some("php.svg"));
        assert_eq!(tool_mark_file("aspnet-core"), Some("dotnet.svg"));
        assert_eq!(tool_mark_file("mariadb"), Some("mariadb.svg"));
        assert_eq!(
            tool_mark_file("imagemagick"),
            None,
            "a tool devd has no artwork for has none"
        );
        assert_eq!(tool_mark_file("nuxt"), None);
    }
}
