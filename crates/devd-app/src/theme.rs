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

/// The palette in use.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
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
    /// The dark palette: graphite, with volt.
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x0c0c0e).into(),
            surface: rgb(0x121214).into(),
            raised: rgb(0x19191c).into(),
            hover: rgba(0xffffff0b).into(),
            active: rgba(0xffffff14).into(),
            line: rgba(0xffffff12).into(),
            line_strong: rgba(0xffffff24).into(),
            text: rgb(0xededf0).into(),
            muted: rgb(0xa3a3ad).into(),
            faint: rgb(0x6c6c76).into(),
            accent: rgb(0xd4f24a).into(),
            on_accent: rgb(0x10130a).into(),
            accent_soft: rgba(0xd4f24a1c).into(),
            ok: rgb(0x3fcf8e).into(),
            warn: rgb(0xf5a524).into(),
            danger: rgb(0xf2555a).into(),
            info: rgb(0x6aa6ff).into(),
            selection: rgba(0xd4f24a3d).into(),
            scrim: rgba(0x000000a6).into(),
            code: rgb(0x151518).into(),
            diff_added: rgba(0x3fcf8e1a).into(),
            diff_added_word: rgba(0x3fcf8e52).into(),
            diff_removed: rgba(0xf2555a1a).into(),
            diff_removed_word: rgba(0xf2555a57).into(),
        }
    }

    /// The light palette: paper, with the same signal darkened to stay legible.
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: rgb(0xfbfbfa).into(),
            surface: rgb(0xf4f4f2).into(),
            raised: rgb(0xffffff).into(),
            hover: rgba(0x0000000a).into(),
            active: rgba(0x00000012).into(),
            line: rgba(0x00000014).into(),
            line_strong: rgba(0x00000029).into(),
            text: rgb(0x16161a).into(),
            muted: rgb(0x55555e).into(),
            faint: rgb(0x8a8a93).into(),
            accent: rgb(0x4d6a00).into(),
            on_accent: rgb(0xffffff).into(),
            accent_soft: rgba(0x4d6a0017).into(),
            ok: rgb(0x0f8a55).into(),
            warn: rgb(0xb26a00).into(),
            danger: rgb(0xc9353b).into(),
            info: rgb(0x2463c9).into(),
            selection: rgba(0x4d6a0033).into(),
            scrim: rgba(0x0000004d).into(),
            code: rgb(0xf0f0ee).into(),
            diff_added: rgba(0x0f8a5517).into(),
            diff_added_word: rgba(0x0f8a5547).into(),
            diff_removed: rgba(0xc9353b14).into(),
            diff_removed_word: rgba(0xc9353b42).into(),
        }
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
