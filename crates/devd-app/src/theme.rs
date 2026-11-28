//! The look of the window, as values.
//!
//! Every colour, size and radius a component uses is named here. A component
//! that writes a hex value of its own is a defect, for the same reason it was in
//! the stylesheet this replaces: the second place a decision lives is the place
//! it goes wrong.
//!
//! # Direction
//!
//! A workbench in graphite. Surfaces are neutral and close in value, so depth
//! comes from a hairline and a step of tone, never a shadow around every box.
//! There is one signal colour, **volt** (a yellow-green), and it means exactly
//! one thing: *this is where you are, or what you are about to do*. Because it
//! is nowhere else, it never has to compete.
//!
//! State has its own four colours (ok, warn, danger, info) and they always sit
//! beside a word or an icon. Emerald for "ok" is deliberately bluer than volt,
//! so "running" is never mistaken for "selected".

use gpui::{App, Global, Hsla, SharedString, rgb, rgba};

use crate::ui::dither::Pattern;

/// The palette in use.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// A stable id, saved with the window's choice. A theme from a file is `mod:<folder>`.
    pub id: SharedString,
    /// What a person calls it.
    pub name: SharedString,
    /// The shape of its background dither.
    pub pattern: Pattern,
    /// The picture behind the screens, as the window's own assets serve it. Empty for none.
    pub art: SharedString,
    /// How strongly the picture shows, as an opacity.
    pub art_strength: f32,
    /// The picture's dots.
    pub art_base: [u8; 3],
    /// The picture's brighter dots: the theme's signal colour.
    pub art_glint: [u8; 3],
    /// A wash of colour over the whole window behind everything: where it starts, where it ends, and its angle.
    pub wash: Option<(Hsla, Hsla, f32)>,
    /// A picture of the person's own behind everything: the file, and how far it is dimmed.
    pub image: Option<(std::path::PathBuf, f32)>,
    /// Whether this is the dark palette.
    pub dark: bool,
    /// The window behind everything.
    pub bg: Hsla,
    /// The rail, lists and side panels: one step up from the window.
    pub surface: Hsla,
    /// Cards, inputs and menus: one step up again.
    pub raised: Hsla,
    /// A row or control under the pointer.
    pub hover: Hsla,
    /// A row or control that is pressed or current.
    pub active: Hsla,
    /// The hairline between things.
    pub line: Hsla,
    /// A firmer line: the edge of an input or a floating surface.
    pub line_strong: Hsla,
    /// Body text.
    pub text: Hsla,
    /// Supporting text.
    pub muted: Hsla,
    /// Labels, placeholders, the least important text that is still read.
    pub faint: Hsla,
    /// The one signal colour.
    pub accent: Hsla,
    /// Text and icons drawn on the accent.
    pub on_accent: Hsla,
    /// The accent as a tint, behind the current item.
    pub accent_soft: Hsla,
    /// Running, passing, healthy.
    pub ok: Hsla,
    /// Needs attention, not broken.
    pub warn: Hsla,
    /// Failed, destructive.
    pub danger: Hsla,
    /// Informational, in progress.
    pub info: Hsla,
    /// Selected text.
    pub selection: Hsla,
    /// The veil behind a dialog or the command menu.
    pub scrim: Hsla,
    /// The background of code.
    pub code: Hsla,
    /// An added line in a diff.
    pub diff_added: Hsla,
    /// The changed words inside an added line.
    pub diff_added_word: Hsla,
    /// A removed line in a diff.
    pub diff_removed: Hsla,
    /// The changed words inside a removed line.
    pub diff_removed_word: Hsla,
}

impl Global for Theme {}

impl Theme {
    /// Every theme, in the order the picker shows them: three for the evening, then two for the day.
    pub const ORDER: [&'static str; 5] = ["sling", "dawn", "psalm", "sharon", "daybreak"];

    /// This theme as the browser client paints it: its name, whether it is dark, and every colour
    /// token under the window's own names. The page is a replica of the window, so it is given the
    /// window's palette rather than one of its own. Written to `ui-themes.json` by a test, which
    /// `devd-server` compiles in; see `every_theme_is_written_for_the_page` below.
    /// (Used by `every_theme_is_written_for_the_page`; kept public because it describes the
    /// palette rather than any one test's expectation of it.)
    #[allow(dead_code)]
    pub fn to_page(&self) -> serde_json::Value {
        // The page takes CSS colours, so the window's colours are given in the form it reads.
        let c = |colour: gpui::Hsla| -> String {
            let rgba = colour.to_rgb();
            format!(
                "rgba({}, {}, {}, {:.3})",
                (rgba.r * 255.0).round() as u8,
                (rgba.g * 255.0).round() as u8,
                (rgba.b * 255.0).round() as u8,
                rgba.a
            )
        };
        let mut colours = serde_json::Map::new();
        for (name, colour) in [
            ("bg", self.bg),
            ("surface", self.surface),
            ("raised", self.raised),
            ("hover", self.hover),
            ("active", self.active),
            ("line", self.line),
            ("line-strong", self.line_strong),
            ("text", self.text),
            ("muted", self.muted),
            ("faint", self.faint),
            ("accent", self.accent),
            ("on-accent", self.on_accent),
            ("accent-soft", self.accent_soft),
            ("ok", self.ok),
            ("warn", self.warn),
            ("danger", self.danger),
            ("info", self.info),
            ("selection", self.selection),
            ("scrim", self.scrim),
            ("code", self.code),
            ("diff-added", self.diff_added),
            ("diff-added-word", self.diff_added_word),
            ("diff-removed", self.diff_removed),
            ("diff-removed-word", self.diff_removed_word),
        ] {
            colours.insert(name.to_string(), serde_json::Value::String(c(colour)));
        }
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "dark": self.dark,
            "colours": colours,
            "art": if self.art.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(self.art.to_string()) },
            "art_strength": self.art_strength,
        })
    }

    /// A built-in theme by id. An unknown id is Sling.
    pub fn by_id(id: &str) -> Self {
        match id {
            "dawn" => Self::shepherd(true),
            "psalm" => Self::harp(true),
            "sharon" | "parchment" => Self::shepherd(false),
            "daybreak" | "linen" => Self::harp(false),
            _ => Self::dark(),
        }
        .with_art()
    }

    /// Any theme by id: a built-in one, or `mod:<folder>` from the library. A theme from a file that
    /// is gone, or cannot be used, is Sling: a theme is never a reason the window fails to open.
    pub fn resolve(id: &str, library: &Library) -> Self {
        match id.strip_prefix("mod:") {
            Some(folder) => library.get(folder).map(Self::from_mod).unwrap_or_else(|| Self::by_id("sling")),
            None => Self::by_id(id),
        }
    }

    /// Whether this theme comes from a file in the mods folder.
    pub fn is_mod(&self) -> bool {
        self.id.starts_with("mod:")
    }

    /// A theme from a file, worked out and checked by devd: its palette, painted as it is.
    pub fn from_mod(m: &devd_core::theme::ThemeMod) -> Self {
        let mut theme = Self::from_palette(&m.palette);
        theme.id = format!("mod:{}", m.id).into();
        theme.name = m.manifest.name.trim().to_string().into();
        theme.image = m.palette.image.as_ref().map(|n| m.dir.join(n)).filter(|f| f.is_file()).map(|f| (f, m.palette.image_dim));
        theme
    }

    /// A built-in theme made from three colours the way a theme file is: every other colour worked out and the contrast
    /// of everything a person reads checked. A built-in that could not be read is a mistake in devd, caught by a test.
    fn derived(id: &'static str, name: &'static str, pattern: Pattern, colours: (&str, &str, &str), scene: &str) -> Self {
        use devd_core::theme::{Backdrop, Colours, ThemeSpec};
        let spec = ThemeSpec {
            feel: name.into(),
            colours: Colours { background: colours.0.into(), text: colours.1.into(), accent: colours.2.into(), ..Colours::default() },
            backdrop: Backdrop { scene: Some(scene.into()), ..Backdrop::default() },
        };
        let palette = devd_core::theme::derive(&spec).unwrap_or_else(|problems| panic!("built-in theme {id} cannot be read: {problems:?}"));
        let mut theme = Self::from_palette(&palette);
        theme.id = SharedString::new_static(id);
        theme.name = SharedString::new_static(name);
        theme.pattern = pattern;
        theme
    }

    /// The window's colours for a worked-out palette.
    fn from_palette(p: &devd_core::theme::Palette) -> Self {
        use devd_core::theme::Scene;
        let t = &p.tokens;
        let c = |colour: devd_core::theme::Rgba| -> Hsla { rgba(colour.to_u32()).into() };
        let pattern = match p.scene {
            Scene::Pasture => Pattern::Pasture,
            Scene::Silk => Pattern::Silk,
            Scene::Contours => Pattern::Contours,
            Scene::Rays => Pattern::Rays,
            Scene::Glow => Pattern::Glow,
            Scene::Mesh => Pattern::Mesh,
            Scene::Ripples | Scene::None => Pattern::Ripples,
        };
        let art = if p.scene == Scene::None || p.strength <= 0.0 {
            SharedString::default()
        } else {
            art_path(pattern, p.ink.rgb3(), p.glint.rgb3(), p.origin.code(), p.seed).into()
        };
        Self {
            id: SharedString::default(),
            name: SharedString::default(),
            pattern,
            art,
            art_strength: (pattern.strength(p.dark) * p.strength).min(0.6),
            art_base: p.ink.rgb3(),
            art_glint: p.glint.rgb3(),
            wash: p.wash.map(|w| (c(w.from), c(w.to), w.angle)),
            image: None,
            dark: p.dark,
            bg: c(t.bg),
            surface: c(t.surface),
            raised: c(t.raised),
            hover: c(t.hover),
            active: c(t.active),
            line: c(t.line),
            line_strong: c(t.line_strong),
            text: c(t.text),
            muted: c(t.muted),
            faint: c(t.faint),
            accent: c(t.accent),
            on_accent: c(t.on_accent),
            accent_soft: c(t.accent_soft),
            ok: c(t.ok),
            warn: c(t.warn),
            danger: c(t.danger),
            info: c(t.info),
            selection: c(t.selection),
            scrim: c(t.scrim),
            code: c(t.code),
            diff_added: c(t.diff_added),
            diff_added_word: c(t.diff_added_word),
            diff_removed: c(t.diff_removed),
            diff_removed_word: c(t.diff_removed_word),
        }
    }

    /// A built-in theme's picture: its own scene in its own colours, made once and kept.
    fn with_art(mut self) -> Self {
        let (base, glint) = match self.id.as_ref() {
            "dawn" => ([0xe6, 0xd0, 0xc4], [0xee, 0x7b, 0x5c]),
            "sharon" => ([0x8c, 0x3b, 0x52], [0xb0, 0x2a, 0x58]),
            "psalm" => ([0xb4, 0xaa, 0xd6], [0xa8, 0x93, 0xff]),
            "daybreak" => ([0x3a, 0x4a, 0x7a], [0x25, 0x59, 0xd6]),
            _ => ([0xd9, 0xd6, 0xcc], [0xd4, 0xf2, 0x4a]),
        };
        self.art_base = base;
        self.art_glint = glint;
        self.art = format!("art/{}.png", self.id).into();
        self.art_strength = self.pattern.strength(self.dark);
        self
    }

    /// A choice saved before every look had a name of its own: a family and a lightness.
    pub fn from_saved(id: &str, dark: bool) -> Self {
        match (id, dark) {
            ("stone", true) => Self::by_id("sling"),
            ("stone", false) => Self::by_id("daybreak"),
            ("shepherd", true) => Self::by_id("dawn"),
            ("shepherd", false) => Self::by_id("sharon"),
            ("harp", true) => Self::by_id("psalm"),
            ("harp", false) => Self::by_id("daybreak"),
            _ => Self::by_id(id),
        }
    }

    /// The theme after this one, going round the picker's order: the built-in themes, then the ones
    /// from files that can be used.
    pub fn next(&self, library: &Library) -> Self {
        let mut ids: Vec<String> = Self::ORDER.iter().map(|id| id.to_string()).collect();
        ids.extend(library.usable().map(|m| format!("mod:{}", m.id)));
        let at = ids.iter().position(|id| id.as_str() == self.id.as_ref()).unwrap_or(0);
        Self::resolve(&ids[(at + 1) % ids.len()], library)
    }

    /// Sling: graphite with volt, a stone in still water.
    fn dark() -> Self {
        Self {
            id: SharedString::new_static("sling"),
            name: SharedString::new_static("Sling"),
            pattern: Pattern::Ripples,
            dark: true,
            bg: rgb(0x0e0e0d).into(),
            surface: rgb(0x141413).into(),
            raised: rgb(0x1b1b19).into(),
            hover: rgba(0xffffff12).into(),
            active: rgba(0xffffff1c).into(),
            line: rgba(0xffffff12).into(),
            line_strong: rgba(0xffffff24).into(),
            text: rgb(0xeeede9).into(),
            muted: rgb(0xa5a39d).into(),
            faint: rgb(0x6e6c66).into(),
            accent: rgb(0xd4f24a).into(),
            on_accent: rgb(0x10130a).into(),
            accent_soft: rgba(0xd4f24a1c).into(),
            ok: rgb(0x3fcf8e).into(),
            warn: rgb(0xf5a524).into(),
            danger: rgb(0xf2555a).into(),
            info: rgb(0x6aa6ff).into(),
            selection: rgba(0xd4f24a3d).into(),
            scrim: rgba(0x000000a6).into(),
            code: rgb(0x161615).into(),
            diff_added: rgba(0x3fcf8e1a).into(),
            diff_added_word: rgba(0x3fcf8e52).into(),
            diff_removed: rgba(0xf2555a1a).into(),
            diff_removed_word: rgba(0xf2555a57).into(),
            art: SharedString::default(),
            art_strength: 0.0,
            art_base: [0; 3],
            art_glint: [0; 3],
            wash: None,
            image: None,
        }
    }

    /// Dawn over the pasture (dark) and Sharon, a rose-tinted map (light).
    fn shepherd(dark: bool) -> Self {
        if dark {
            // Warm charcoal with a coral accent: the first light on a hill.
            Self::derived("dawn", "Dawn", Pattern::Pasture, ("#15100e", "#f3e9e2", "#ee7b5c"), "pasture")
        } else {
            Self {
                id: SharedString::new_static("sharon"),
                name: SharedString::new_static("Sharon"),
                pattern: Pattern::Contours,
                dark: false,
                bg: rgb(0xf9eef0).into(),
                surface: rgb(0xf3e1e5).into(),
                raised: rgb(0xfffafb).into(),
                hover: rgba(0x8c3b5210).into(),
                active: rgba(0x8c3b521c).into(),
                line: rgba(0x8c3b5216).into(),
                line_strong: rgba(0x8c3b522b).into(),
                text: rgb(0x2c1820).into(),
                muted: rgb(0x735661).into(),
                faint: rgb(0xa4868f).into(),
                accent: rgb(0xb02a58).into(),
                on_accent: rgb(0xffffff).into(),
                accent_soft: rgba(0xb02a5818).into(),
                ok: rgb(0x2f7d55).into(),
                warn: rgb(0xa86b00).into(),
                danger: rgb(0xb23a32).into(),
                info: rgb(0x3b6a9a).into(),
                selection: rgba(0xb02a5833).into(),
                scrim: rgba(0x2a10184d).into(),
                code: rgb(0xf6e6ea).into(),
                diff_added: rgba(0x3f7d4317).into(),
                diff_added_word: rgba(0x3f7d4347).into(),
                diff_removed: rgba(0xb23a3214).into(),
                diff_removed_word: rgba(0xb23a3242).into(),
                art: SharedString::default(),
                art_strength: 0.0,
                art_base: [0; 3],
                art_glint: [0; 3],
                wash: None,
                image: None,
            }
        }
    }

    /// Psalm, silk and gold in the dark (dark), and Daybreak, rays of morning light (light).
    fn harp(dark: bool) -> Self {
        if dark {
            // Deep indigo with a violet accent: silk in the dark.
            Self::derived("psalm", "Psalm", Pattern::Silk, ("#100e1c", "#ece9f7", "#a893ff"), "silk")
        } else {
            // Cool paper with an azure accent: morning, clear.
            Self::derived("daybreak", "Daybreak", Pattern::Rays, ("#f3f6fb", "#14213a", "#2559d6"), "rays")
        }
    }

    /// The brighter tone scattered through the background art: the theme's signal colour.
    pub fn art_glint_rgb(&self) -> [u8; 3] {
        self.art_glint
    }

    /// The colour of this theme's background dots.
    pub fn art_rgb(&self) -> [u8; 3] {
        self.art_base
    }

    /// Every colour token, so a test can prove a new theme has all of them.
    #[cfg(test)]
    pub fn colour_tokens(&self) -> [(&'static str, Hsla); 24] {
        [
            ("bg", self.bg),
            ("surface", self.surface),
            ("raised", self.raised),
            ("hover", self.hover),
            ("active", self.active),
            ("line", self.line),
            ("line_strong", self.line_strong),
            ("text", self.text),
            ("muted", self.muted),
            ("faint", self.faint),
            ("accent", self.accent),
            ("on_accent", self.on_accent),
            ("accent_soft", self.accent_soft),
            ("ok", self.ok),
            ("warn", self.warn),
            ("danger", self.danger),
            ("info", self.info),
            ("selection", self.selection),
            ("scrim", self.scrim),
            ("code", self.code),
            ("diff_added", self.diff_added),
            ("diff_added_word", self.diff_added_word),
            ("diff_removed", self.diff_removed),
            ("diff_removed_word", self.diff_removed_word),
        ]
    }

    /// The colour that goes with a state.
    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Neutral => self.faint,
            Tone::Accent => self.accent,
            Tone::Ok => self.ok,
            Tone::Warn => self.warn,
            Tone::Danger => self.danger,
            Tone::Info => self.info,
        }
    }
}

/// What a colour is saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Nothing in particular: stopped, idle, unknown.
    Neutral,
    /// Current or primary.
    Accent,
    /// Running, passing, healthy.
    Ok,
    /// Needs attention.
    Warn,
    /// Failed or destructive.
    Danger,
    /// In progress.
    Info,
}

impl From<devd_app_core::status::Level> for Tone {
    fn from(level: devd_app_core::status::Level) -> Self {
        use devd_app_core::status::Level;
        match level {
            Level::Neutral => Tone::Neutral,
            Level::Info => Tone::Info,
            Level::Ok => Tone::Ok,
            Level::Warn => Tone::Warn,
            Level::Danger => Tone::Danger,
        }
    }
}

/// The palette in use.
pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

/// Read the window's choice, kept in its own data folder. A missing or damaged
/// file means the default; a theme is never a reason the window fails to open.
pub fn load(home: &std::path::Path, library: &Library) -> Theme {
    let text = std::fs::read_to_string(home.join("window-theme")).unwrap_or_default();
    let mut parts = text.split_whitespace();
    let id = parts.next().unwrap_or("sling");
    let dark = parts.next() != Some("light");
    if id.starts_with("mod:") {
        Theme::resolve(id, library)
    } else {
        Theme::from_saved(id, dark)
    }
}

/// The asset that draws a scene in two colours, such as `art/scene/contours-2a241d-8a4b14.png`.
/// The path says everything the picture needs, so a theme from a file needs nothing kept for it.
pub fn art_path(pattern: Pattern, base: [u8; 3], glint: [u8; 3], origin: &str, seed: u32) -> String {
    let hex = |c: [u8; 3]| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut name = pattern.name().to_string();
    // The corner and the pattern's number, when they are not the scene's own: `aurora~bl~7`.
    if !origin.is_empty() {
        name.push('~');
        name.push_str(origin);
    }
    if seed != 0 {
        name.push_str(&format!("~{seed}"));
    }
    format!("art/scene/{name}-{}-{}.png", hex(base), hex(glint))
}

/// The themes in the mods folder, as last read: each worked out, or what is wrong with it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    /// The mods folder.
    pub dir: std::path::PathBuf,
    /// Every folder that holds a theme, in the order of their names.
    pub found: Vec<devd_core::theme::Found>,
    /// What the folder looked like when it was read, to tell when to read it again.
    pub stamp: u64,
}

impl Global for Library {}

impl Library {
    /// Read the mods folder in devd's home.
    pub fn read(home: &std::path::Path) -> Self {
        let dir = devd_core::paths::Paths::rooted(home).mods();
        let stamp = devd_core::theme::stamp(&dir);
        let found = devd_core::theme::scan(&dir);
        Self { dir, found, stamp }
    }

    /// Whether the folder has changed since it was read.
    pub fn is_stale(&self) -> bool {
        devd_core::theme::stamp(&self.dir) != self.stamp
    }

    /// The themes that can be used.
    pub fn usable(&self) -> impl Iterator<Item = &devd_core::theme::ThemeMod> {
        self.found.iter().filter_map(|f| f.theme.as_ref().ok())
    }

    /// A usable theme by its folder's name.
    pub fn get(&self, folder: &str) -> Option<&devd_core::theme::ThemeMod> {
        self.usable().find(|m| m.id == folder)
    }

    /// The library in use.
    pub fn of(cx: &App) -> &Library {
        cx.try_global::<Library>().unwrap_or(&EMPTY)
    }
}

static EMPTY: std::sync::LazyLock<Library> = std::sync::LazyLock::new(Library::default);

/// Remember the choice for the next window.
pub fn save(home: &std::path::Path, theme: &Theme) -> std::io::Result<()> {
    std::fs::write(
        home.join("window-theme"),
        format!(
            "{} {}\n",
            theme.id.as_ref(),
            if theme.dark { "dark" } else { "light" }
        ),
    )
}

/// Sizes, in logical pixels. One scale for the whole window.
///
/// The numbers themselves live in `devd_app_core::size`, the table the browser client is handed at
/// startup too, so the window and the page cannot drift apart. These names are the window's own
/// spelling of that table, and a test there checks each one against it.
pub mod size {
    /// The rail of screens down the left edge.
    pub const RAIL: f32 = 56.0;
    /// The bar across the top: where you are, and the command menu.
    pub const TOPBAR: f32 = 44.0;
    /// The strip along the bottom: health, mode, version.
    pub const STATUS: f32 = 26.0;
    /// The widest a page's content column gets. One number for the whole app: screens used 860,
    /// 900, 980 and 1040 for the same job, so the column changed width as you moved between them.
    pub const PAGE: f32 = 1040.0;

    /// The same, for a page that shows diffs and file trees and needs the room.
    pub const PAGE_WIDE: f32 = 1240.0;

    /// A list that sits beside the main content.
    pub const SIDEBAR: f32 = 288.0;

    /// That same list put away: a column wide enough for one control.
    pub const COLLAPSED: f32 = 48.0;
    /// A row in a list.
    pub const ROW: f32 = 36.0;
    /// A button, an input, a tab.
    pub const CONTROL: f32 = 28.0;
    /// The widest a column of reading text gets.
    pub const READING: f32 = 820.0;

    /// A badge, a checkbox.
    pub const RADIUS_S: f32 = 4.0;
    /// A button, an input, a row.
    pub const RADIUS: f32 = 6.0;
    /// A card, a menu, a dialog.
    pub const RADIUS_L: f32 = 10.0;

    /// Metadata, the status strip.
    pub const TEXT_XS: f32 = 12.0;
    /// Labels and second lines.
    pub const TEXT_SM: f32 = 13.0;
    /// The interface's body size.
    pub const TEXT: f32 = 14.0;
    /// Reading text: a conversation, a document.
    pub const TEXT_MD: f32 = 15.0;
    /// A section's lead.
    pub const TEXT_LG: f32 = 17.0;
    /// A screen's title.
    pub const TEXT_XL: f32 = 20.0;
    /// The one large line on Home.
    pub const TEXT_DISPLAY: f32 = 28.0;
}

/// The interface typeface. gpui resolves this name to the platform's own UI
/// font (Segoe UI, SF Pro, the desktop's default), so text looks native on
/// each system and needs no bundled file.
pub const SANS: &str = ".SystemUIFont";

/// The typeface for code, paths, ids and numbers that should line up.
pub fn mono() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_the_window_draws_comes_from_the_shared_table() {
        // The browser client is handed this same table (`/sizes.js`). A name missing from it, or a
        // number that differs from the one the window draws, is exactly the drift it exists to stop.
        let window: &[(&str, f32)] = &[
            ("rail", size::RAIL),
            ("topbar", size::TOPBAR),
            ("status", size::STATUS),
            ("page", size::PAGE),
            ("page-wide", size::PAGE_WIDE),
            ("sidebar", size::SIDEBAR),
            ("collapsed", size::COLLAPSED),
            ("row", size::ROW),
            ("control", size::CONTROL),
            ("reading", size::READING),
            ("radius-s", size::RADIUS_S),
            ("radius", size::RADIUS),
            ("radius-l", size::RADIUS_L),
            ("text-xs", size::TEXT_XS),
            ("text-sm", size::TEXT_SM),
            ("text", size::TEXT),
            ("text-md", size::TEXT_MD),
            ("text-lg", size::TEXT_LG),
            ("text-xl", size::TEXT_XL),
            ("text-display", size::TEXT_DISPLAY),
        ];
        for (name, value) in window {
            let found = devd_app_core::size::find(name).unwrap_or_else(|| panic!("{name} is not in the shared table"));
            assert_eq!(found.px, *value, "{name}");
        }
    }

    /// Write the window's five palettes for the browser client (`devd-server` compiles the file in
    /// and serves it as `window.DEVD_THEMES`). A test rather than a build script, so the file is
    /// produced by the same code that draws the window, and a change to a palette shows up as a
    /// modified file in the change it came with.
    #[test]
    fn every_theme_is_written_for_the_page() {
        let mut out = serde_json::Map::new();
        for id in Theme::ORDER {
            let theme = Theme::by_id(id);
            out.insert(id.to_string(), theme.to_page());
        }
        // A sandboxed build may only write inside `target`; then the file is written there for a
        // person (or a later step) to copy in, and the check below still runs on what was built.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui-themes.json");
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(out.clone())).unwrap() + "\n";
        let written = std::fs::write(&path, &text);
        if written.is_err() {
            let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("target");
            let _ = std::fs::write(out_dir.join("ui-themes.json"), text);
        }
        assert_eq!(out.len(), 5);
        for id in Theme::ORDER {
            assert!(out[id]["colours"]["bg"].is_string(), "{id} has no palette");
        }
    }

    fn all() -> Vec<Theme> {
        Theme::ORDER.iter().map(|id| Theme::by_id(id)).collect()
    }

    #[test]
    fn every_theme_has_every_colour_token() {
        for theme in all() {
            for (name, colour) in theme.colour_tokens() {
                assert!(colour.a > 0.0, "{} has no {}", theme.id, name);
                assert!(
                    colour.a <= 1.0 && colour.l >= 0.0 && colour.l <= 1.0,
                    "{} {name} is out of range",
                    theme.id
                );
            }
            // Text has to be readable on the window; a theme where they are the same colour is not a theme.
            assert_ne!(theme.text.l, theme.bg.l, "{}", theme.id);
        }
    }

    #[test]
    fn six_themes_each_with_its_own_name_scene_and_colours() {
        let themes = all();
        for (i, a) in themes.iter().enumerate() {
            assert_eq!(a.id.as_ref(), Theme::ORDER[i], "the order and the themes agree");
            for b in &themes[i + 1..] {
                assert_ne!(a.id, b.id);
                assert_ne!(a.name, b.name);
                assert_ne!(a.pattern, b.pattern, "{} and {} share a scene", a.id, b.id);
                assert_ne!(a.bg, b.bg, "{} and {} share a background", a.id, b.id);
                assert_ne!(
                    a.art_rgb(),
                    b.art_rgb(),
                    "{} and {} share a dot colour",
                    a.id,
                    b.id
                );
            }
        }
        assert_eq!(themes.iter().filter(|t| t.dark).count(), 3);
        assert_eq!(
            themes.iter().filter(|t| !t.dark).count(),
            2,
            "two for the day, not three"
        );
    }

    #[test]
    fn no_two_themes_share_a_button_colour_and_each_accent_is_a_different_hue() {
        let themes = all();
        for (i, a) in themes.iter().enumerate() {
            for b in &themes[i + 1..] {
                assert_ne!(a.accent, b.accent, "{} and {} share an accent", a.id, b.id);
                let gap = (a.accent.h - b.accent.h).abs();
                let gap = gap.min(1.0 - gap);
                assert!(gap > 0.04, "{} and {} have accents of nearly the same hue ({gap:.3})", a.id, b.id);
            }
        }
    }

    /// A window colour as the theme files' check reads it.
    fn core(c: Hsla) -> devd_core::theme::Rgba {
        let c: gpui::Rgba = c.into();
        let b = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        devd_core::theme::Rgba { r: b(c.r), g: b(c.g), b: b(c.b), a: b(c.a) }
    }

    fn tokens_of(t: &Theme) -> devd_core::theme::Tokens {
        devd_core::theme::Tokens {
            bg: core(t.bg),
            surface: core(t.surface),
            raised: core(t.raised),
            hover: core(t.hover),
            active: core(t.active),
            line: core(t.line),
            line_strong: core(t.line_strong),
            text: core(t.text),
            muted: core(t.muted),
            faint: core(t.faint),
            accent: core(t.accent),
            on_accent: core(t.on_accent),
            accent_soft: core(t.accent_soft),
            ok: core(t.ok),
            warn: core(t.warn),
            danger: core(t.danger),
            info: core(t.info),
            selection: core(t.selection),
            scrim: core(t.scrim),
            code: core(t.code),
            diff_added: core(t.diff_added),
            diff_added_word: core(t.diff_added_word),
            diff_removed: core(t.diff_removed),
            diff_removed_word: core(t.diff_removed_word),
        }
    }

    #[test]
    fn devds_own_themes_pass_the_check_a_theme_file_has_to_pass() {
        for theme in all() {
            let problems = devd_core::theme::check(&tokens_of(&theme), theme.dark);
            assert!(problems.is_empty(), "{}: {problems:?}", theme.id);
        }
    }

    fn paper(dir: &std::path::Path) -> Library {
        let spec = devd_core::theme::ThemeSpec {
            feel: "Cream and umber".into(),
            colours: devd_core::theme::Colours {
                background: "#efe6d3".into(),
                text: "#2a241d".into(),
                accent: "#8a4b14".into(),
                ..Default::default()
            },
            backdrop: devd_core::theme::Backdrop { scene: Some("contours".into()), ..Default::default() },
        };
        let manifest = devd_core::theme::Manifest { name: "Paper".into(), version: None, description: "Cream paper.".into() };
        devd_core::theme::write(&dir.join("mods"), "paper", &manifest, &spec).unwrap();
        Library::read(dir)
    }

    #[test]
    fn a_theme_from_a_file_is_painted_as_devd_worked_it_out() {
        let dir = tempfile::tempdir().unwrap();
        let library = paper(dir.path());
        let m = library.get("paper").expect("paper is usable");
        let t = Theme::from_mod(m);
        assert_eq!((t.id.as_ref(), t.name.as_ref(), t.dark), ("mod:paper", "Paper", false));
        assert!(t.is_mod());
        assert_eq!(tokens_of(&t), m.palette.tokens, "every token reaches the window unchanged");
        assert_eq!(t.pattern, Pattern::Contours);
        assert_eq!(t.art.as_ref(), "art/scene/contours-2a241d-8a4b14.png");
        assert!(t.art_strength > 0.0 && t.art_strength <= 0.6);
        assert_eq!(Theme::resolve("mod:paper", &library), t);
    }

    #[test]
    fn a_theme_from_a_file_is_remembered_and_one_that_is_gone_falls_back_to_sling() {
        let dir = tempfile::tempdir().unwrap();
        let library = paper(dir.path());
        let chosen = Theme::resolve("mod:paper", &library);
        save(dir.path(), &chosen).unwrap();
        assert_eq!(load(dir.path(), &library), chosen);
        std::fs::remove_dir_all(dir.path().join("mods").join("paper")).unwrap();
        assert!(library.is_stale(), "removing the folder is noticed");
        let gone = Library::read(dir.path());
        assert_eq!(load(dir.path(), &gone).id.as_ref(), "sling");
        assert_eq!(Theme::resolve("mod:nothing", &gone).id.as_ref(), "sling");
    }

    #[test]
    fn going_round_the_themes_includes_the_ones_from_files() {
        let dir = tempfile::tempdir().unwrap();
        let library = paper(dir.path());
        let last = Theme::by_id("daybreak");
        let next = last.next(&library);
        assert_eq!(next.id.as_ref(), "mod:paper");
        assert_eq!(next.next(&library).id.as_ref(), "sling");
    }

    #[test]
    fn the_words_light_and_dark_do_not_name_a_theme() {
        for theme in all() {
            let name = theme.name.to_lowercase();
            assert!(
                !name.contains("light") && !name.contains("dark"),
                "{}",
                theme.name
            );
        }
    }

    #[test]
    fn a_chosen_theme_is_remembered() {
        let dir = std::env::temp_dir().join(format!("devd-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for id in Theme::ORDER {
            let chosen = Theme::by_id(id);
            save(&dir, &chosen).unwrap();
            assert_eq!(load(&dir, &Library::default()), chosen, "{id}");
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            load(&dir, &Library::default()),
            Theme::by_id("sling"),
            "a missing file is the default, not a failure"
        );
    }

    #[test]
    fn a_choice_saved_by_an_earlier_window_still_opens_the_matching_theme() {
        let dir = std::env::temp_dir().join(format!("devd-theme-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (saved, expected) in [
            ("stone dark", "sling"),
            ("stone light", "daybreak"),
            ("shepherd dark", "dawn"),
            ("shepherd light", "sharon"),
            ("harp dark", "psalm"),
            ("harp light", "daybreak"),
            ("linen dark", "daybreak"),
            ("parchment dark", "sharon"),
        ] {
            std::fs::write(dir.join("window-theme"), saved).unwrap();
            assert_eq!(load(&dir, &Library::default()).id.as_ref(), expected, "{saved}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_theme_falls_back_to_sling_and_next_goes_round_the_order() {
        assert_eq!(Theme::by_id("nope"), Theme::by_id("sling"));
        let mut theme = Theme::by_id("sling");
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(theme.id.to_string());
            theme = theme.next(&Library::default());
        }
        assert_eq!(seen, Theme::ORDER);
        assert_eq!(theme.id, "sling", "after the last comes the first");
    }
}
