//! Themes from files: a folder in `mods/` that gives devd a look of its own.
//!
//! A theme is values, never code. A person (or the assistant) writes three colours, the background,
//! the text and the accent, and this module works out the other twenty-one tokens the window uses,
//! in the same proportions as devd's own themes. It then measures what a person has to read and
//! refuses a theme that cannot be read, saying which two colours, how far apart they are, how far
//! apart they need to be, and which way to move. It never quietly changes a colour a person chose.
//!
//! The window paints a [`Palette`]; the assistant writes a theme through [`write`], which checks it
//! first, so a theme written that way is always one the window can use. See `docs/themes.md`.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The file that says what a mod is.
pub const MANIFEST: &str = "mod.toml";
/// The file that holds a theme.
pub const THEME: &str = "theme.toml";

/// The minimum contrast for body text against anything it sits on.
pub const TEXT_MIN: f32 = 4.5;
/// The minimum contrast for muted text against the background.
pub const MUTED_MIN: f32 = 3.0;
/// The minimum contrast for faint text against the background.
pub const FAINT_MIN: f32 = 2.0;
/// The minimum contrast for the accent, and each state colour, against the background.
pub const SIGNAL_MIN: f32 = 2.5;

// ---------------------------------------------------------------------------------------------- files

/// `mod.toml`: what a mod is.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// What a person calls it, such as "Paper". 1 to 32 characters.
    pub name: String,
    /// The mod's own version, such as "0.1.0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// One sentence about how it looks, shown under its name.
    pub description: String,
}

/// `theme.toml`: a theme's colours and its picture.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThemeSpec {
    /// A few words for its colours, such as "Cream and umber".
    pub feel: String,
    /// The colours. Three are needed; the rest are worked out unless given.
    #[serde(alias = "colors")]
    pub colours: Colours,
    /// The picture behind the screens.
    #[serde(default, skip_serializing_if = "Backdrop::is_default")]
    pub backdrop: Backdrop,
}

/// A theme's colours, each `#rrggbb` or `#rgb`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Colours {
    /// The window behind everything.
    pub background: String,
    /// Body text.
    pub text: String,
    /// The one signal colour: where you are, or what you are about to do.
    pub accent: String,
    /// Supporting text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<String>,
    /// The faintest text that is still read: labels and placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faint: Option<String>,
    /// Lists and side panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// Cards, inputs and menus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raised: Option<String>,
    /// Running, passing, healthy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<String>,
    /// Needs attention, not broken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    /// Failed, destructive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub danger: Option<String>,
    /// Informational, in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
}

/// The picture behind the screens: one of devd's scenes, drawn in the theme's colours.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Backdrop {
    /// `ripples`, `pasture`, `silk`, `contours`, `rays` or `none`. Ripples when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    /// The picture's dots. The text colour when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink: Option<String>,
    /// The picture's brighter dots. The accent when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glint: Option<String>,
    /// How strongly the picture shows, 0 to 2, where 1 is the scene's own strength.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
}

impl Backdrop {
    fn is_default(&self) -> bool {
        self == &Backdrop::default()
    }
}

// ---------------------------------------------------------------------------------------------- colour

/// A colour with its strength, as the window paints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Strength, 255 for solid.
    pub a: u8,
}

impl Rgba {
    /// A solid colour.
    pub const fn rgb(hex: u32) -> Rgba {
        Rgba { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
    }

    /// The same colour at another strength.
    pub const fn alpha(self, a: u8) -> Rgba {
        Rgba { a, ..self }
    }

    /// `0xRRGGBBAA`, the form the window's colour functions take.
    pub fn to_u32(self) -> u32 {
        (u32::from(self.r) << 24) | (u32::from(self.g) << 16) | (u32::from(self.b) << 8) | u32::from(self.a)
    }

    /// `#rrggbb`, without its strength.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Red, green and blue.
    pub fn rgb3(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    /// Relative luminance, as the Web Content Accessibility Guidelines define it.
    pub fn luminance(self) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// This colour moved `t` of the way towards `other`.
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round().clamp(0.0, 255.0) as u8;
        Rgba { r: m(self.r, other.r), g: m(self.g, other.g), b: m(self.b, other.b), a: 255 }
    }
}

/// Written as `#rrggbb`, or `#rrggbbaa` when it is not solid.
impl Serialize for Rgba {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.a == 255 {
            s.serialize_str(&self.hex())
        } else {
            s.serialize_str(&format!("{}{:02x}", self.hex(), self.a))
        }
    }
}

/// The contrast between two solid colours, from 1 (the same) to 21 (black on white).
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (x, y) = (a.luminance(), b.luminance());
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Read `#rrggbb` or `#rgb`.
pub fn parse_colour(text: &str) -> Option<Rgba> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(Rgba::rgb),
        3 => {
            let long: String = hex.chars().flat_map(|c| [c, c]).collect();
            u32::from_str_radix(&long, 16).ok().map(Rgba::rgb)
        }
        _ => None,
    }
}

const WHITE: Rgba = Rgba::rgb(0xffffff);
const BLACK: Rgba = Rgba::rgb(0x000000);

// ---------------------------------------------------------------------------------------------- palette

/// The scene behind the screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scene {
    /// Rings spreading across still water.
    Ripples,
    /// A low sun over layered hills.
    Pasture,
    /// Folds of silk catching the light.
    Silk,
    /// The contour lines of a hand-drawn map.
    Contours,
    /// Rays of light fanning from the corner.
    Rays,
    /// No picture.
    None,
}

impl Scene {
    /// Every scene, by the name a file uses.
    pub const ALL: [(&'static str, Scene); 6] = [
        ("ripples", Scene::Ripples),
        ("pasture", Scene::Pasture),
        ("silk", Scene::Silk),
        ("contours", Scene::Contours),
        ("rays", Scene::Rays),
        ("none", Scene::None),
    ];

    /// The scene a file names.
    pub fn named(name: &str) -> Option<Scene> {
        Self::ALL.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).map(|(_, s)| *s)
    }

    /// The name a file uses.
    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, s)| *s == self).map(|(n, _)| *n).unwrap_or("none")
    }
}

/// Every colour the window uses, in the order `design.md` lists them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Tokens {
    /// The window behind everything.
    pub bg: Rgba,
    /// Lists and side panels.
    pub surface: Rgba,
    /// Cards, inputs and menus.
    pub raised: Rgba,
    /// A row under the pointer.
    pub hover: Rgba,
    /// A row that is pressed or current.
    pub active: Rgba,
    /// The hairline between things.
    pub line: Rgba,
    /// A firmer line.
    pub line_strong: Rgba,
    /// Body text.
    pub text: Rgba,
    /// Supporting text.
    pub muted: Rgba,
    /// Labels and placeholders.
    pub faint: Rgba,
    /// The signal colour.
    pub accent: Rgba,
    /// Text on the accent.
    pub on_accent: Rgba,
    /// The accent as a tint.
    pub accent_soft: Rgba,
    /// Running, passing, healthy.
    pub ok: Rgba,
    /// Needs attention.
    pub warn: Rgba,
    /// Failed, destructive.
    pub danger: Rgba,
    /// In progress.
    pub info: Rgba,
    /// Selected text.
    pub selection: Rgba,
    /// The veil behind a dialog.
    pub scrim: Rgba,
    /// The background of code.
    pub code: Rgba,
    /// An added line.
    pub diff_added: Rgba,
    /// Changed words in an added line.
    pub diff_added_word: Rgba,
    /// A removed line.
    pub diff_removed: Rgba,
    /// Changed words in a removed line.
    pub diff_removed_word: Rgba,
}

impl Tokens {
    /// Every token with its name.
    pub fn list(&self) -> [(&'static str, Rgba); 24] {
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
}

/// A theme worked out in full: what the window paints.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Palette {
    /// Whether the background is dark.
    pub dark: bool,
    /// Every colour.
    pub tokens: Tokens,
    /// The picture behind the screens.
    pub scene: Scene,
    /// The picture's dots.
    pub ink: Rgba,
    /// The picture's brighter dots.
    pub glint: Rgba,
    /// How strongly the picture shows, 0 to 2, against the scene's own strength.
    pub strength: f32,
}

/// Work out a theme in full and check that it can be read. Every problem is returned at once.
pub fn derive(spec: &ThemeSpec) -> Result<Palette, Vec<String>> {
    let mut problems = Vec::new();
    let mut required = |name: &str, value: &str| match parse_colour(value) {
        Some(c) => Some(c),
        None => {
            problems.push(bad_colour(&format!("colours.{name}"), value));
            None
        }
    };
    let bg = required("background", &spec.colours.background);
    let text = required("text", &spec.colours.text);
    let accent = required("accent", &spec.colours.accent);
    let optional = |problems: &mut Vec<String>, name: &str, value: &Option<String>| -> Option<Rgba> {
        let value = value.as_deref()?;
        let found = parse_colour(value);
        if found.is_none() {
            problems.push(bad_colour(name, value));
        }
        found
    };
    let c = &spec.colours;
    let muted = optional(&mut problems, "colours.muted", &c.muted);
    let faint = optional(&mut problems, "colours.faint", &c.faint);
    let surface = optional(&mut problems, "colours.surface", &c.surface);
    let raised = optional(&mut problems, "colours.raised", &c.raised);
    let ok = optional(&mut problems, "colours.ok", &c.ok);
    let warn = optional(&mut problems, "colours.warn", &c.warn);
    let danger = optional(&mut problems, "colours.danger", &c.danger);
    let info = optional(&mut problems, "colours.info", &c.info);
    let ink = optional(&mut problems, "backdrop.ink", &spec.backdrop.ink);
    let glint = optional(&mut problems, "backdrop.glint", &spec.backdrop.glint);
    let scene = match spec.backdrop.scene.as_deref() {
        None => Scene::Ripples,
        Some(name) => Scene::named(name).unwrap_or_else(|| {
            problems.push(format!(
                "backdrop.scene is \"{name}\", which is not a scene devd draws. Use one of: {}.",
                Scene::ALL.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
            ));
            Scene::Ripples
        }),
    };
    let strength = spec.backdrop.strength.unwrap_or(1.0);
    if !(0.0..=2.0).contains(&strength) {
        problems.push(format!("backdrop.strength is {strength}; it goes from 0 (no picture) to 2 (twice the scene's own strength)."));
    }
    if spec.feel.trim().is_empty() {
        problems.push("feel is empty: give a few words for the colours, such as \"Cream and umber\".".into());
    }
    let (Some(bg), Some(text), Some(accent)) = (bg, text, accent) else {
        return Err(problems);
    };

    let dark = bg.luminance() < text.luminance();
    let tokens = if dark {
        Tokens {
            bg,
            surface: surface.unwrap_or(bg.mix(text, 0.027)),
            raised: raised.unwrap_or(bg.mix(text, 0.058)),
            hover: text.alpha(0x12),
            active: text.alpha(0x1c),
            line: text.alpha(0x12),
            line_strong: text.alpha(0x24),
            text,
            muted: muted.unwrap_or(text.mix(bg, 0.33)),
            faint: faint.unwrap_or(text.mix(bg, 0.55)),
            accent,
            on_accent: on(accent, bg, text),
            accent_soft: accent.alpha(0x1c),
            ok: ok.unwrap_or(Rgba::rgb(0x3fcf8e)),
            warn: warn.unwrap_or(Rgba::rgb(0xf5a524)),
            danger: danger.unwrap_or(Rgba::rgb(0xf2555a)),
            info: info.unwrap_or(Rgba::rgb(0x6aa6ff)),
            selection: accent.alpha(0x3d),
            scrim: bg.mix(BLACK, 0.5).alpha(0xa6),
            code: bg.mix(text, 0.035),
            diff_added: ok.unwrap_or(Rgba::rgb(0x3fcf8e)).alpha(0x1a),
            diff_added_word: ok.unwrap_or(Rgba::rgb(0x3fcf8e)).alpha(0x52),
            diff_removed: danger.unwrap_or(Rgba::rgb(0xf2555a)).alpha(0x1a),
            diff_removed_word: danger.unwrap_or(Rgba::rgb(0xf2555a)).alpha(0x57),
        }
    } else {
        Tokens {
            bg,
            surface: surface.unwrap_or(bg.mix(text, 0.04)),
            raised: raised.unwrap_or(bg.mix(WHITE, 0.6)),
            hover: text.alpha(0x10),
            active: text.alpha(0x1c),
            line: text.alpha(0x16),
            line_strong: text.alpha(0x2b),
            text,
            muted: muted.unwrap_or(text.mix(bg, 0.29)),
            faint: faint.unwrap_or(text.mix(bg, 0.5)),
            accent,
            on_accent: on(accent, bg, text),
            accent_soft: accent.alpha(0x18),
            ok: ok.unwrap_or(Rgba::rgb(0x1f7a5c)),
            warn: warn.unwrap_or(Rgba::rgb(0x9a6400)),
            danger: danger.unwrap_or(Rgba::rgb(0xb83048)),
            info: info.unwrap_or(Rgba::rgb(0x3559a8)),
            selection: accent.alpha(0x33),
            scrim: text.alpha(0x4d),
            code: bg.mix(text, 0.03),
            diff_added: ok.unwrap_or(Rgba::rgb(0x1f7a5c)).alpha(0x17),
            diff_added_word: ok.unwrap_or(Rgba::rgb(0x1f7a5c)).alpha(0x47),
            diff_removed: danger.unwrap_or(Rgba::rgb(0xb83048)).alpha(0x14),
            diff_removed_word: danger.unwrap_or(Rgba::rgb(0xb83048)).alpha(0x42),
        }
    };
    problems.extend(check(&tokens, dark));
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Palette { dark, tokens, scene, ink: ink.unwrap_or(text), glint: glint.unwrap_or(accent), strength })
}

/// Text on the accent: whichever of white and the theme's own darkest colour reads better.
fn on(accent: Rgba, bg: Rgba, text: Rgba) -> Rgba {
    let deep = if bg.luminance() < text.luminance() { accent.mix(BLACK, 0.9) } else { text };
    if contrast(WHITE, accent) >= contrast(deep, accent) { WHITE } else { deep }
}

fn bad_colour(name: &str, value: &str) -> String {
    format!("{name} is \"{value}\", which is not a colour devd reads. Write it as #rrggbb, such as #2a241d.")
}

/// What a person has to be able to read, measured. Every shortfall is a sentence that names the two
/// colours, the contrast they reach, the contrast they need, and which way to move.
pub fn check(t: &Tokens, dark: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let (towards, away) = if dark { ("lighter", "darker") } else { ("darker", "lighter") };
    let mut need = |what: &str, colour: Rgba, against: &str, ground: Rgba, min: f32| {
        let ratio = contrast(colour, ground);
        if ratio < min {
            problems.push(format!(
                "The {what} is too close to the {against}: {:.1}:1, and it needs at least {}:1. Make the {what} {towards} or the {against} {away}.",
                ratio, trim(min)
            ));
        }
    };
    need("text", t.text, "background", t.bg, TEXT_MIN);
    // The surfaces sit a step from the background: when the text already fails on the background, saying
    // so twice more adds nothing a person can act on.
    if contrast(t.text, t.bg) >= TEXT_MIN {
        need("text", t.text, "surface", t.surface, TEXT_MIN);
        need("text", t.text, "raised surface", t.raised, TEXT_MIN);
    }
    need("muted text", t.muted, "background", t.bg, MUTED_MIN);
    need("faint text", t.faint, "background", t.bg, FAINT_MIN);
    need("accent", t.accent, "background", t.bg, SIGNAL_MIN);
    for (name, colour) in [("ok colour", t.ok), ("warn colour", t.warn), ("danger colour", t.danger), ("info colour", t.info)] {
        need(name, colour, "background", t.bg, SIGNAL_MIN);
    }
    let on_accent = contrast(t.on_accent, t.accent);
    if on_accent < TEXT_MIN {
        problems.push(format!(
            "Text on the accent cannot be read: the best devd can do is {on_accent:.1}:1, and it needs at least {}:1. Make the accent deeper or brighter, so white or a dark colour reads on it.",
            trim(TEXT_MIN)
        ));
    }
    problems
}

fn trim(n: f32) -> String {
    let s = format!("{n:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

// ---------------------------------------------------------------------------------------------- folders

/// A theme read from its folder and worked out.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeMod {
    /// The folder's name, such as `paper`.
    pub id: String,
    /// `mod.toml`.
    pub manifest: Manifest,
    /// `theme.toml`.
    pub spec: ThemeSpec,
    /// What the window paints.
    pub palette: Palette,
}

/// One folder in `mods/` that holds a theme: worked out, or what is wrong with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The folder's name.
    pub id: String,
    /// The theme, or every problem with it.
    pub theme: Result<ThemeMod, Vec<String>>,
}

/// Whether a name can be a mod's folder: lowercase letters, digits and hyphens, 1 to 40 of them,
/// starting with a letter or digit.
pub fn valid_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

/// Check a theme given as values, as [`write`] does before it writes anything.
pub fn check_values(id: &str, manifest: &Manifest, spec: &ThemeSpec) -> Result<Palette, Vec<String>> {
    let mut problems = Vec::new();
    if !valid_id(id) {
        problems.push(format!(
            "\"{id}\" cannot name a theme's folder: use 1 to 40 lowercase letters, digits and hyphens, such as \"paper\" or \"night-shift\"."
        ));
    }
    let name = manifest.name.trim();
    if name.is_empty() || name.chars().count() > 32 {
        problems.push("name is what a person calls the theme: give it 1 to 32 characters, such as \"Paper\".".into());
    }
    if manifest.description.trim().is_empty() {
        problems.push("description is empty: give one sentence about how the theme looks.".into());
    }
    match derive(spec) {
        Ok(palette) if problems.is_empty() => Ok(palette),
        Ok(_) => Err(problems),
        Err(more) => {
            problems.extend(more);
            Err(problems)
        }
    }
}

/// Read a theme's folder and work it out.
pub fn load(dir: &Path) -> Result<ThemeMod, Vec<String>> {
    let id = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let read = |file: &str| -> Result<String, String> {
        std::fs::read_to_string(dir.join(file)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{file} is missing from {}.", dir.display()),
            _ => format!("{file} could not be read: {e}."),
        })
    };
    let mut problems = Vec::new();
    let manifest = read(MANIFEST).and_then(|text| toml::from_str::<Manifest>(&text).map_err(|e| toml_problem(MANIFEST, &e)));
    let spec = read(THEME).and_then(|text| toml::from_str::<ThemeSpec>(&text).map_err(|e| toml_problem(THEME, &e)));
    let (manifest, spec) = match (manifest, spec) {
        (Ok(m), Ok(s)) => (m, s),
        (m, s) => {
            problems.extend(m.err());
            problems.extend(s.err());
            return Err(problems);
        }
    };
    let palette = check_values(&id, &manifest, &spec)?;
    Ok(ThemeMod { id, manifest, spec, palette })
}

/// A TOML error, said plainly: the file, the line, and what was wrong.
fn toml_problem(file: &str, e: &toml::de::Error) -> String {
    let message = e.message().trim_end_matches('.').replace('\n', " ");
    format!("{file}: {message}.")
}

/// Every folder in `mods/` that holds a theme, in the order of their names.
pub fn scan(mods: &Path) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(mods) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = entries
        .flatten()
        .filter(|e| e.path().join(THEME).is_file())
        .map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let theme = if valid_id(&id) {
                load(&e.path())
            } else {
                Err(vec![format!("The folder \"{id}\" cannot name a theme: use lowercase letters, digits and hyphens.")])
            };
            Found { id, theme }
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// A number that changes whenever a theme file in `mods/` is added, removed or saved, so a
/// window can tell when to read the folder again without reading every file each time.
pub fn stamp(mods: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let Ok(entries) = std::fs::read_dir(mods) else {
        return 0;
    };
    let mut seen: Vec<(PathBuf, u64, u128)> = Vec::new();
    for entry in entries.flatten() {
        for file in [MANIFEST, THEME] {
            let path = entry.path().join(file);
            if let Ok(meta) = std::fs::metadata(&path) {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                seen.push((path, meta.len(), modified));
            }
        }
    }
    seen.sort();
    seen.hash(&mut hasher);
    hasher.finish()
}

/// Check a theme and, only if it passes, write its folder. The files are written by devd, so a
/// theme saved this way is always well formed.
pub fn write(mods: &Path, id: &str, manifest: &Manifest, spec: &ThemeSpec) -> Result<ThemeMod, Vec<String>> {
    let palette = check_values(id, manifest, spec)?;
    let dir = mods.join(id);
    let io = |what: &str, e: std::io::Error| vec![format!("{what} could not be written in {}: {e}.", dir.display())];
    std::fs::create_dir_all(&dir).map_err(|e| io("The folder", e))?;
    let manifest_text = toml::to_string_pretty(manifest).map_err(|e| vec![e.to_string()])?;
    let theme_text = toml::to_string_pretty(spec).map_err(|e| vec![e.to_string()])?;
    for (file, text) in [(MANIFEST, manifest_text), (THEME, theme_text)] {
        // Written beside and moved into place, so a window reading the folder never sees half a file.
        let part = dir.join(format!("{file}.part"));
        std::fs::write(&part, text).map_err(|e| io(file, e))?;
        std::fs::rename(&part, dir.join(file)).map_err(|e| io(file, e))?;
    }
    Ok(ThemeMod { id: id.to_string(), manifest: manifest.clone(), spec: spec.clone(), palette })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(bg: &str, text: &str, accent: &str) -> ThemeSpec {
        ThemeSpec {
            feel: "Cream and umber".into(),
            colours: Colours { background: bg.into(), text: text.into(), accent: accent.into(), ..Colours::default() },
            backdrop: Backdrop::default(),
        }
    }

    fn manifest() -> Manifest {
        Manifest { name: "Paper".into(), version: Some("0.1.0".into()), description: "Cream paper and umber ink.".into() }
    }

    #[test]
    fn three_colours_make_a_whole_light_theme_that_reads() {
        let p = derive(&spec("#efe6d3", "#2a241d", "#8a4b14")).unwrap();
        assert!(!p.dark);
        let t = p.tokens;
        // Cards are lighter than the window on a pale theme; lists a step darker.
        assert!(t.raised.luminance() > t.bg.luminance() && t.surface.luminance() < t.bg.luminance());
        assert!(contrast(t.text, t.bg) >= TEXT_MIN && contrast(t.muted, t.bg) >= MUTED_MIN);
        assert_eq!(t.line.rgb3(), t.text.rgb3(), "lines take the text's tint");
        assert!(t.line.a < 0x30 && t.hover.a < 0x30);
        assert_eq!(p.scene, Scene::Ripples);
        assert_eq!((p.ink, p.glint), (t.text, t.accent));
    }

    #[test]
    fn three_colours_make_a_whole_dark_theme_that_reads() {
        let p = derive(&spec("#1f1a15", "#f0e7d7", "#e0a050")).unwrap();
        assert!(p.dark);
        let t = p.tokens;
        assert!(t.surface.luminance() > t.bg.luminance() && t.raised.luminance() > t.surface.luminance());
        assert!(contrast(t.on_accent, t.accent) >= TEXT_MIN);
        assert_eq!(t.ok, Rgba::rgb(0x3fcf8e), "the dark state colours when none are given");
    }

    #[test]
    fn a_colour_given_is_used_as_written() {
        let mut s = spec("#efe6d3", "#2a241d", "#8a4b14");
        s.colours.surface = Some("#e5dac4".into());
        s.colours.ok = Some("#2e6b3a".into());
        s.backdrop = Backdrop { scene: Some("Contours".into()), ink: Some("#6a5f50".into()), glint: None, strength: Some(1.4) };
        let p = derive(&s).unwrap();
        assert_eq!(p.tokens.surface, Rgba::rgb(0xe5dac4));
        assert_eq!(p.tokens.ok, Rgba::rgb(0x2e6b3a));
        assert_eq!(p.tokens.diff_added.rgb3(), [0x2e, 0x6b, 0x3a]);
        assert_eq!((p.scene, p.ink, p.strength), (Scene::Contours, Rgba::rgb(0x6a5f50), 1.4));
    }

    #[test]
    fn a_theme_that_cannot_be_read_is_refused_with_numbers_and_a_direction() {
        let problems = derive(&spec("#efe6d3", "#b8ad98", "#8a4b14")).unwrap_err();
        let text = problems.iter().find(|p| p.starts_with("The text is too close to the background")).expect("text problem");
        assert!(text.contains(":1, and it needs at least 4.5:1"), "{text}");
        assert!(text.contains("Make the text darker or the background lighter"), "{text}");
        // A dark theme is told the other way round.
        let dark = derive(&spec("#101010", "#3a3a3a", "#d4f24a")).unwrap_err();
        assert!(dark.iter().any(|p| p.contains("Make the text lighter or the background darker")), "{dark:?}");
    }

    #[test]
    fn an_accent_nothing_reads_on_is_refused() {
        // A mid grey: neither white nor near-black reaches 4.5:1 on it.
        let problems = derive(&spec("#ffffff", "#111111", "#7a7a7a")).unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("Text on the accent cannot be read")), "{problems:?}");
    }

    #[test]
    fn every_mistake_in_a_file_is_reported_at_once() {
        let mut s = spec("cream", "#2a241d", "#12345");
        s.backdrop.scene = Some("waves".into());
        s.backdrop.strength = Some(3.0);
        s.feel = " ".into();
        let problems = derive(&s).unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("colours.background is \"cream\"")), "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("colours.accent is \"#12345\"")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("\"waves\", which is not a scene")), "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("backdrop.strength is 3")), "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("feel is empty")), "{problems:?}");
    }

    #[test]
    fn colours_are_read_in_both_forms_and_nothing_else() {
        assert_eq!(parse_colour("#abc"), Some(Rgba::rgb(0xaabbcc)));
        assert_eq!(parse_colour(" #2A241D "), Some(Rgba::rgb(0x2a241d)));
        for bad in ["2a241d", "#2a241", "#gggggg", "#", "red"] {
            assert_eq!(parse_colour(bad), None, "{bad}");
        }
        assert!((contrast(WHITE, BLACK) - 21.0).abs() < 0.01);
        assert!((contrast(WHITE, WHITE) - 1.0).abs() < 0.001);
    }

    #[test]
    fn a_misspelt_key_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let theme = dir.path().join("paper");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join(MANIFEST), "name = \"Paper\"\ndescription = \"Cream.\"\n").unwrap();
        std::fs::write(theme.join(THEME), "feel = \"Cream\"\n[colours]\nbackground = \"#efe6d3\"\ntext = \"#2a241d\"\naccent = \"#8a4b14\"\nmutd = \"#6a5f50\"\n").unwrap();
        let problems = load(&theme).unwrap_err();
        assert!(problems[0].starts_with("theme.toml:") && problems[0].contains("mutd"), "{problems:?}");
        // "colors" is read as "colours".
        std::fs::write(theme.join(THEME), "feel = \"Cream\"\n[colors]\nbackground = \"#efe6d3\"\ntext = \"#2a241d\"\naccent = \"#8a4b14\"\n").unwrap();
        assert!(load(&theme).is_ok());
    }

    #[test]
    fn a_saved_theme_is_found_again_as_written_and_a_bad_one_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path().join("mods");
        let s = spec("#efe6d3", "#2a241d", "#8a4b14");
        let before = stamp(&mods);
        let saved = write(&mods, "paper", &manifest(), &s).unwrap();
        assert_ne!(stamp(&mods), before, "a new theme changes the stamp");
        let found = scan(&mods);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].theme.as_ref().unwrap(), &saved);
        assert!(!mods.join("paper").join("theme.toml.part").exists());

        let unreadable = spec("#efe6d3", "#e8dcc5", "#8a4b14");
        assert!(write(&mods, "faded", &manifest(), &unreadable).is_err());
        assert!(!mods.join("faded").exists(), "a refused theme leaves nothing behind");
        assert!(write(&mods, "Bad Name", &manifest(), &s).unwrap_err()[0].contains("cannot name a theme's folder"));
    }

    #[test]
    fn a_broken_folder_is_listed_with_its_problems_and_folders_without_a_theme_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let mods = dir.path();
        std::fs::create_dir_all(mods.join("half")).unwrap();
        std::fs::write(mods.join("half").join(THEME), "feel = \"x\"").unwrap();
        std::fs::create_dir_all(mods.join("code-only")).unwrap();
        std::fs::write(mods.join("code-only").join(MANIFEST), "name = \"x\"\ndescription = \"y\"\n").unwrap();
        let found = scan(mods);
        assert_eq!(found.len(), 1);
        let problems = found[0].theme.as_ref().unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("mod.toml is missing")), "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("theme.toml:")), "{problems:?}");
    }

    #[test]
    fn folder_names_follow_the_rule() {
        for good in ["paper", "night-shift", "a", "theme2"] {
            assert!(valid_id(good), "{good}");
        }
        for bad in ["", "Paper", "-x", "has space", "dots.no", &"x".repeat(41)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }
}
