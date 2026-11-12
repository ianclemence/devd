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

use gpui::{rgb, rgba, App, Global, Hsla};

use crate::ui::dither::Pattern;

/// The palette in use.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// A stable id, saved with the window's choice.
    pub id: &'static str,
    /// What a person calls it.
    pub name: &'static str,
    /// The shape of its background dither.
    pub pattern: Pattern,
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
    /// Every theme, by id and the name a person reads.
    pub const ALL: [(&'static str, &'static str); 3] = [("stone", "Stone"), ("shepherd", "Shepherd"), ("harp", "Harp")];

    /// A theme by id, in its dark or light form. An unknown id is Stone.
    pub fn by_id(id: &str, dark: bool) -> Self {
        match id {
            "shepherd" => Self::shepherd(dark),
            "harp" => Self::harp(dark),
            _ => Self::stone(dark),
        }
    }

    /// The current palette, when a caller asks without naming it.
    pub fn stone(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// The dark palette: smooth stone, with volt.
    pub fn dark() -> Self {
        Self {
            id: "stone",
            name: "Stone",
            pattern: Pattern::Corner,
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
        }
    }

    /// The light palette: warm paper, with the same signal darkened to stay legible.
    pub fn light() -> Self {
        Self {
            id: "stone",
            name: "Stone",
            pattern: Pattern::Corner,
            dark: false,
            bg: rgb(0xfaf9f6).into(),
            surface: rgb(0xf3f1ec).into(),
            raised: rgb(0xffffff).into(),
            hover: rgba(0x00000010).into(),
            active: rgba(0x0000001a).into(),
            line: rgba(0x00000014).into(),
            line_strong: rgba(0x00000029).into(),
            text: rgb(0x1a1917).into(),
            muted: rgb(0x5a5852).into(),
            faint: rgb(0x8c8982).into(),
            accent: rgb(0x4d6a00).into(),
            on_accent: rgb(0xffffff).into(),
            accent_soft: rgba(0x4d6a0017).into(),
            ok: rgb(0x0f8a55).into(),
            warn: rgb(0xb26a00).into(),
            danger: rgb(0xc9353b).into(),
            info: rgb(0x2463c9).into(),
            selection: rgba(0x4d6a0033).into(),
            scrim: rgba(0x0000004d).into(),
            code: rgb(0xefede7).into(),
            diff_added: rgba(0x0f8a5517).into(),
            diff_added_word: rgba(0x0f8a5547).into(),
            diff_removed: rgba(0xc9353b14).into(),
            diff_removed_word: rgba(0xc9353b42).into(),
        }
    }

    /// Wool, olive and the light of a field at dawn.
    pub fn shepherd(dark: bool) -> Self {
        if dark {
            Self {
                id: "shepherd",
                name: "Shepherd",
                pattern: Pattern::Woven,
                dark: true,
                bg: rgb(0x12110d).into(),
                surface: rgb(0x191710).into(),
                raised: rgb(0x211e16).into(),
                hover: rgba(0xf5e6c012).into(),
                active: rgba(0xf5e6c01c).into(),
                line: rgba(0xf5e6c012).into(),
                line_strong: rgba(0xf5e6c026).into(),
                text: rgb(0xf0ead9).into(),
                muted: rgb(0xb3a98f).into(),
                faint: rgb(0x7d7460).into(),
                accent: rgb(0xd9a441).into(),
                on_accent: rgb(0x1a1305).into(),
                accent_soft: rgba(0xd9a4411f).into(),
                ok: rgb(0x79bf7d).into(),
                warn: rgb(0xe0a23a).into(),
                danger: rgb(0xd25b4d).into(),
                info: rgb(0x86a9c4).into(),
                selection: rgba(0xd9a4413d).into(),
                scrim: rgba(0x0b0905a6).into(),
                code: rgb(0x18150f).into(),
                diff_added: rgba(0x79bf7d1a).into(),
                diff_added_word: rgba(0x79bf7d52).into(),
                diff_removed: rgba(0xd25b4d1a).into(),
                diff_removed_word: rgba(0xd25b4d57).into(),
            }
        } else {
            Self {
                id: "shepherd",
                name: "Shepherd",
                pattern: Pattern::Woven,
                dark: false,
                bg: rgb(0xf6f1e4).into(),
                surface: rgb(0xefe8d7).into(),
                raised: rgb(0xfffdf7).into(),
                hover: rgba(0x6b5b2e10).into(),
                active: rgba(0x6b5b2e1c).into(),
                line: rgba(0x6b5b2e16).into(),
                line_strong: rgba(0x6b5b2e2b).into(),
                text: rgb(0x2b261c).into(),
                muted: rgb(0x6b6252).into(),
                faint: rgb(0x9a907c).into(),
                accent: rgb(0x8a5a00).into(),
                on_accent: rgb(0xffffff).into(),
                accent_soft: rgba(0x8a5a0018).into(),
                ok: rgb(0x3f7d43).into(),
                warn: rgb(0xa86b00).into(),
                danger: rgb(0xb23a32).into(),
                info: rgb(0x356d93).into(),
                selection: rgba(0x8a5a0033).into(),
                scrim: rgba(0x241c0b4d).into(),
                code: rgb(0xf0eadb).into(),
                diff_added: rgba(0x3f7d4317).into(),
                diff_added_word: rgba(0x3f7d4347).into(),
                diff_removed: rgba(0xb23a3214).into(),
                diff_removed_word: rgba(0xb23a3242).into(),
            }
        }
    }

    /// Indigo and gold, the strings of an instrument in a quiet room.
    pub fn harp(dark: bool) -> Self {
        if dark {
            Self {
                id: "harp",
                name: "Harp",
                pattern: Pattern::Strings,
                dark: true,
                bg: rgb(0x0d0f16).into(),
                surface: rgb(0x121520).into(),
                raised: rgb(0x191d2a).into(),
                hover: rgba(0xe8e2ff12).into(),
                active: rgba(0xe8e2ff1e).into(),
                line: rgba(0xe8e2ff12).into(),
                line_strong: rgba(0xe8e2ff26).into(),
                text: rgb(0xeae8f3).into(),
                muted: rgb(0xa6a3b9).into(),
                faint: rgb(0x706d84).into(),
                accent: rgb(0xe0b45c).into(),
                on_accent: rgb(0x1b1405).into(),
                accent_soft: rgba(0xe0b45c1f).into(),
                ok: rgb(0x62c7a0).into(),
                warn: rgb(0xd9a441).into(),
                danger: rgb(0xe06a74).into(),
                info: rgb(0x86a8ff).into(),
                selection: rgba(0xe0b45c3d).into(),
                scrim: rgba(0x06070da6).into(),
                code: rgb(0x111521).into(),
                diff_added: rgba(0x62c7a01a).into(),
                diff_added_word: rgba(0x62c7a052).into(),
                diff_removed: rgba(0xe06a741a).into(),
                diff_removed_word: rgba(0xe06a7457).into(),
            }
        } else {
            Self {
                id: "harp",
                name: "Harp",
                pattern: Pattern::Strings,
                dark: false,
                bg: rgb(0xf6f3ea).into(),
                surface: rgb(0xeee9dd).into(),
                raised: rgb(0xfffdf8).into(),
                hover: rgba(0x3a315a10).into(),
                active: rgba(0x3a315a1c).into(),
                line: rgba(0x3a315a16).into(),
                line_strong: rgba(0x3a315a2b).into(),
                text: rgb(0x211d2b).into(),
                muted: rgb(0x5f5a6e).into(),
                faint: rgb(0x8c8798).into(),
                accent: rgb(0x7a5200).into(),
                on_accent: rgb(0xffffff).into(),
                accent_soft: rgba(0x7a520018).into(),
                ok: rgb(0x1f7a5c).into(),
                warn: rgb(0x9a6400).into(),
                danger: rgb(0xb83048).into(),
                info: rgb(0x3559a8).into(),
                selection: rgba(0x7a520033).into(),
                scrim: rgba(0x1b17304d).into(),
                code: rgb(0xefeade).into(),
                diff_added: rgba(0x1f7a5c17).into(),
                diff_added_word: rgba(0x1f7a5c47).into(),
                diff_removed: rgba(0xb8304814).into(),
                diff_removed_word: rgba(0xb8304842).into(),
            }
        }
    }

    /// The colour of this theme's background dots.
    pub fn art_rgb(&self) -> [u8; 3] {
        match (self.id, self.dark) {
            ("shepherd", true) => [0xd9, 0xc9, 0x9a],
            ("shepherd", false) => [0x6b, 0x5b, 0x2e],
            ("harp", true) => [0xc9, 0xb8, 0xd9],
            ("harp", false) => [0x3a, 0x31, 0x5a],
            (_, true) => [0xd9, 0xd6, 0xcc],
            _ => [0x3a, 0x38, 0x30],
        }
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
pub fn load(home: &std::path::Path) -> Theme {
    let text = std::fs::read_to_string(home.join("window-theme")).unwrap_or_default();
    let mut parts = text.split_whitespace();
    let id = parts.next().unwrap_or("stone");
    let dark = parts.next() != Some("light");
    Theme::by_id(id, dark)
}

/// Remember the choice for the next window.
pub fn save(home: &std::path::Path, theme: &Theme) -> std::io::Result<()> {
    std::fs::write(home.join("window-theme"), format!("{} {}\n", theme.id, if theme.dark { "dark" } else { "light" }))
}

/// Sizes, in logical pixels. One scale for the whole window.
pub mod size {
    /// The rail of screens down the left edge.
    pub const RAIL: f32 = 56.0;
    /// The bar across the top: where you are, and the command menu.
    pub const TOPBAR: f32 = 44.0;
    /// The strip along the bottom: health, mode, version.
    pub const STATUS: f32 = 26.0;
    /// A list that sits beside the main content.
    pub const SIDEBAR: f32 = 288.0;
    /// A row in a list.
    pub const ROW: f32 = 36.0;
    /// A button, an input, a tab.
    pub const CONTROL: f32 = 28.0;
    /// The widest a column of reading text gets.
    pub const READING: f32 = 760.0;

    /// A badge, a checkbox.
    pub const RADIUS_S: f32 = 4.0;
    /// A button, an input, a row.
    pub const RADIUS: f32 = 6.0;
    /// A card, a menu, a dialog.
    pub const RADIUS_L: f32 = 10.0;

    /// Metadata, the status strip.
    pub const TEXT_XS: f32 = 11.0;
    /// Labels and second lines.
    pub const TEXT_SM: f32 = 12.0;
    /// The interface's body size.
    pub const TEXT: f32 = 13.0;
    /// Reading text: a conversation, a document.
    pub const TEXT_MD: f32 = 14.0;
    /// A section's lead.
    pub const TEXT_LG: f32 = 16.0;
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

    fn variants() -> Vec<Theme> {
        Theme::ALL.iter().flat_map(|(id, _)| [Theme::by_id(id, true), Theme::by_id(id, false)]).collect()
    }

    #[test]
    fn every_theme_has_every_colour_token() {
        for theme in variants() {
            assert!(matches!(theme.id, "stone" | "shepherd" | "harp"), "{}", theme.id);
            for (name, colour) in theme.colour_tokens() {
                assert!(colour.a > 0.0, "{} has no {}", theme.id, name);
                assert!(colour.a <= 1.0 && colour.l >= 0.0 && colour.l <= 1.0, "{} {name} is out of range", theme.id);
            }
            // Text has to be readable on the window; a theme where they are the
            // same colour is not a theme.
            assert_ne!(theme.text.l, theme.bg.l, "{}", theme.id);
        }
    }

    #[test]
    fn each_theme_has_its_own_pattern_and_its_own_background() {
        let patterns: Vec<Pattern> = variants().iter().map(|t| t.pattern).collect();
        assert!(patterns.contains(&Pattern::Corner));
        assert!(patterns.contains(&Pattern::Woven));
        assert!(patterns.contains(&Pattern::Strings));
        let stone = Theme::by_id("stone", true);
        let shepherd = Theme::by_id("shepherd", true);
        let harp = Theme::by_id("harp", true);
        assert_ne!(stone.bg, shepherd.bg);
        assert_ne!(shepherd.bg, harp.bg);
        assert_ne!(stone.art_rgb(), shepherd.art_rgb());
        assert_ne!(shepherd.art_rgb(), harp.art_rgb());
    }

    #[test]
    fn a_chosen_theme_is_remembered() {
        let dir = std::env::temp_dir().join(format!("devd-theme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chosen = Theme::by_id("harp", false);
        save(&dir, &chosen).unwrap();
        assert_eq!(load(&dir), chosen);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Theme::dark(), "a missing file is the default, not a failure");
    }

    #[test]
    fn an_unknown_theme_falls_back_to_stone_and_light_is_separate() {
        assert_eq!(Theme::by_id("nope", true), Theme::dark());
        assert_eq!(Theme::by_id("nope", false), Theme::light());
        assert!(!Theme::by_id("harp", false).dark);
        assert!(Theme::by_id("harp", true).dark);
    }
}
