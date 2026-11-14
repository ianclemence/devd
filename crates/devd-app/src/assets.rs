//! The window's icons, drawn for it and compiled in.
//!
//! One family: a 16-unit grid, a 1.5-unit stroke, round caps and joins, no
//! fills. Each is small enough to read here, and gpui tints an icon with the
//! text colour, so there is one file per shape and no file per colour.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Serves `icons/<name>.svg` from the table below.
pub struct Assets;

macro_rules! icon {
    ($body:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">"#,
            $body,
            "</svg>"
        )
    };
}

/// Every icon, by name.
const ICONS: &[(&str, &str)] = &[
    (
        "home",
        icon!(
            r#"<path d="M2.5 7.2 8 2.5l5.5 4.7V13a.5.5 0 0 1-.5.5H9.8V9.8H6.2v3.7H3a.5.5 0 0 1-.5-.5Z"/>"#
        ),
    ),
    (
        "inbox",
        icon!(
            r#"<path d="M2.5 9.5 4.3 3.4a.6.6 0 0 1 .6-.4h6.2a.6.6 0 0 1 .6.4l1.8 6.1v3a.5.5 0 0 1-.5.5H3a.5.5 0 0 1-.5-.5Z"/><path d="M2.5 9.5h3.2l.8 1.6h3l.8-1.6h3.2"/>"#
        ),
    ),
    (
        "assistant",
        icon!(
            r#"<path d="M8 2.2c.5 3.2 2.1 4.8 5.3 5.3-3.2.5-4.8 2.1-5.3 5.3-.5-3.2-2.1-4.8-5.3-5.3C5.900 7 7.500 5.400 8 2.200Z"/><path d="M12.6 11.400v2.400M11.400 12.600h2.400"/>"#
        ),
    ),
    (
        "runs",
        icon!(
            r#"<path d="M8 2.200 2.500 5 8 7.800 13.500 5Z"/><path d="M2.500 8 8 10.800 13.500 8"/><path d="M2.500 11 8 13.800 13.500 11"/>"#
        ),
    ),
    (
        "team",
        icon!(
            r#"<circle cx="6" cy="5.500" r="2.200"/><path d="M2 13c0-2.200 1.800-3.700 4-3.700s4 1.500 4 3.700"/><path d="M10.600 3.500a2.200 2.200 0 0 1 0 4.100M11.800 9.600c1.400.4 2.200 1.700 2.200 3.400"/>"#
        ),
    ),
    (
        "folder",
        icon!(
            r#"<path d="M2.500 4.500a1 1 0 0 1 1-1h2.600l1.400 1.500h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1Z"/>"#
        ),
    ),
    (
        "globe",
        icon!(
            r#"<circle cx="8" cy="8" r="5.500"/><path d="M2.500 8h11M8 2.500c1.700 1.600 2.500 3.400 2.500 5.500S9.700 11.900 8 13.500C6.300 11.900 5.500 10.100 5.500 8S6.300 4.100 8 2.500Z"/>"#
        ),
    ),
    (
        "machine",
        icon!(
            r#"<rect x="2.500" y="3" width="11" height="4.200" rx="1"/><rect x="2.500" y="8.800" width="11" height="4.200" rx="1"/><path d="M5 5.100h.01M5 10.900h.01"/>"#
        ),
    ),
    (
        "models",
        icon!(
            r#"<path d="M3 4.500h4M10.500 4.500H13M3 11.500h2.500M9 11.500h4"/><circle cx="8.700" cy="4.500" r="1.700"/><circle cx="7.200" cy="11.500" r="1.700"/>"#
        ),
    ),
    (
        "activity",
        icon!(r#"<circle cx="8" cy="8" r="5.500"/><path d="M8 5v3.200l2 1.300"/>"#),
    ),
    (
        "settings",
        icon!(
            r#"<circle cx="8" cy="8" r="2"/><path d="M8 2.500v1.600M8 11.900v1.600M2.500 8h1.600M11.900 8h1.600M4.100 4.100l1.100 1.100M10.800 10.800l1.100 1.100M4.100 11.900l1.100-1.100M10.800 5.200l1.100-1.100"/>"#
        ),
    ),
    (
        "search",
        icon!(r#"<circle cx="7.200" cy="7.200" r="4.200"/><path d="m10.300 10.300 3.200 3.200"/>"#),
    ),
    ("plus", icon!(r#"<path d="M8 3.500v9M3.500 8h9"/>"#)),
    ("check", icon!(r#"<path d="m3.500 8.500 3 3 6-7"/>"#)),
    ("x", icon!(r#"<path d="m4 4 8 8M12 4l-8 8"/>"#)),
    (
        "chevron-right",
        icon!(r#"<path d="m6 3.500 4.500 4.500L6 12.500"/>"#),
    ),
    (
        "chevron-down",
        icon!(r#"<path d="m3.500 6 4.500 4.500L12.500 6"/>"#),
    ),
    (
        "arrow-down",
        icon!(r#"<path d="M8 3v10M4.500 9.500 8 13l3.500-3.500"/>"#),
    ),
    (
        "arrow-up",
        icon!(r#"<path d="M8 13V3.500M4 7.300 8 3.300l4 4"/>"#),
    ),
    (
        "arrow-left",
        icon!(r#"<path d="M13 8H3.500M7.300 4l-4 4 4 4"/>"#),
    ),
    (
        "arrow-right",
        icon!(r#"<path d="M3 8h9.500M8.700 4l4 4-4 4"/>"#),
    ),
    (
        "external",
        icon!(
            r#"<path d="M9.500 2.500h4v4M13.500 2.500 7.500 8.500M11.500 9.500v3a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h3"/>"#
        ),
    ),
    (
        "terminal",
        icon!(
            r#"<rect x="2" y="3" width="12" height="10" rx="1.300"/><path d="m4.800 6.300 2 1.700-2 1.700M8.500 10h2.700"/>"#
        ),
    ),
    (
        "branch",
        icon!(
            r#"<circle cx="4.500" cy="3.800" r="1.500"/><circle cx="4.500" cy="12.200" r="1.500"/><circle cx="11.500" cy="5.500" r="1.500"/><path d="M4.500 5.300v5.400M11.500 7c0 2.200-2.200 2.700-7 3.200"/>"#
        ),
    ),
    ("play", icon!(r#"<path d="M5 3.300v9.400l7.500-4.700Z"/>"#)),
    (
        "stop",
        icon!(r#"<rect x="4" y="4" width="8" height="8" rx="1.200"/>"#),
    ),
    (
        "refresh",
        icon!(r#"<path d="M13 8a5 5 0 1 1-1.500-3.600"/><path d="M13 2.800v2.900h-2.900"/>"#),
    ),
    (
        "alert",
        icon!(r#"<path d="M8 2.500 14 13H2Z"/><path d="M8 6.500v3M8 11.300h.01"/>"#),
    ),
    (
        "info",
        icon!(r#"<circle cx="8" cy="8" r="5.500"/><path d="M8 7.300v3.500M8 5.200h.01"/>"#),
    ),
    (
        "camera",
        icon!(
            r#"<path d="M2 5.500A1.500 1.500 0 0 1 3.500 4H5l1-1.500h4L11 4h1.500A1.500 1.500 0 0 1 14 5.500v6a1.500 1.500 0 0 1-1.500 1.500h-9A1.500 1.500 0 0 1 2 11.500z"/><circle cx="8" cy="8.500" r="2.250"/>"#
        ),
    ),
    (
        "copy",
        icon!(
            r#"<rect x="5.500" y="5.500" width="8" height="8" rx="1.200"/><path d="M10.500 5.500V3.700a1.200 1.200 0 0 0-1.200-1.200H3.700a1.200 1.200 0 0 0-1.200 1.200v5.600a1.200 1.200 0 0 0 1.200 1.200h1.800"/>"#
        ),
    ),
    (
        "file",
        icon!(
            r#"<path d="M4 2.500h5l3 3v7.500a.5.500 0 0 1-.5.500h-7.500a.5.500 0 0 1-.5-.5V3a.5.500 0 0 1 .5-.5Z"/><path d="M9 2.500v3h3"/>"#
        ),
    ),
    (
        "diff",
        icon!(
            r#"<path d="M5 3v5M2.500 5.500h5M2.800 11.500h4.400M11 3v10M8.800 5.200 11 3l2.200 2.200"/>"#
        ),
    ),
    (
        "command",
        icon!(
            r#"<path d="M6 6h4v4H6ZM6 6V4.500A1.500 1.500 0 1 0 4.500 6H6ZM10 6V4.500A1.500 1.500 0 1 1 11.500 6H10ZM6 10v1.500A1.500 1.500 0 1 1 4.500 10H6ZM10 10v1.500a1.500 1.500 0 1 0 1.500-1.500H10Z"/>"#
        ),
    ),
    (
        "lock",
        icon!(
            r#"<rect x="3.500" y="7.200" width="9" height="6.300" rx="1.200"/><path d="M5.500 7.200V5.300a2.500 2.500 0 0 1 5 0v1.900"/>"#
        ),
    ),
    (
        "database",
        icon!(
            r#"<ellipse cx="8" cy="4.200" rx="5" ry="1.900"/><path d="M3 4.200v7.600c0 1 2.200 1.900 5 1.900s5-.9 5-1.900V4.200M3 8c0 1 2.200 1.900 5 1.900S13 9 13 8"/>"#
        ),
    ),
    (
        "brain",
        icon!(
            r#"<g transform="scale(0.6667)" stroke-width="2.250"><path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.770 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.770 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.500 4.500 0 0 1-3-4 4.500 4.500 0 0 1-3 4"/><path d="M17.599 6.500a3 3 0 0 0 .399-1.375"/><path d="M6.003 5.125A3 3 0 0 0 6.401 6.500"/><path d="M3.477 10.896a4 4 0 0 1 .585-.396"/><path d="M19.938 10.500a4 4 0 0 1 .585.396"/><path d="M6 18a4 4 0 0 1-1.967-.516"/><path d="M19.967 17.484A4 4 0 0 1 18 18"/></g>"#
        ),
    ),
    (
        "trash",
        icon!(
            r#"<path d="M2.500 4.500h11M6 4.500V3.200c0-.4.3-.7.7-.7h2.600c.4 0 .7.3.7.7v1.300M4 4.500l.5 8c0 .6.5 1 1 1h5c.5 0 1-.4 1-1l.5-8M6.500 7v4M9.500 7v4"/>"#
        ),
    ),
    (
        "pause",
        icon!(r#"<path d="M5.500 3.500v9M10.500 3.500v9"/>"#),
    ),
    (
        "clock",
        icon!(r#"<circle cx="8" cy="8" r="5.500"/><path d="M8 5v3.200l2 1.300"/>"#),
    ),
    (
        "wrench",
        icon!(
            r#"<path d="M10.500 2.500a3 3 0 0 0-2.800 4L2.800 11.400a1.200 1.200 0 0 0 1.700 1.700l4.900-4.800a3 3 0 0 0 4-2.800l-1.800 1.800-1.700-.4-.4-1.700 1.800-1.800Z"/>"#
        ),
    ),
    (
        "bolt",
        icon!(r#"<path d="M9 2 3.500 9h4L7 14l5.500-7h-4Z"/>"#),
    ),
    (
        "more",
        icon!(r#"<path d="M3.500 8h.01M8 8h.01M12.500 8h.01"/>"#),
    ),
    (
        "sun",
        icon!(
            r#"<circle cx="8" cy="8" r="2.800"/><path d="M8 1.800v1.400M8 12.800v1.400M1.800 8h1.400M12.800 8h1.400M3.600 3.600l1 1M11.400 11.400l1 1M3.600 12.400l1-1M11.400 4.600l1-1"/>"#
        ),
    ),
    (
        "sidebar",
        icon!(r#"<rect x="2" y="3" width="12" height="10" rx="1.300"/><path d="M6 3v10"/>"#),
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // A vendor's own mark, from the table devd's server compiles in, so the
        // window and the browser interface show the same artwork.
        if let Some(file) = path.strip_prefix("brands/") {
            return Ok(devd_server::ui::brand_bytes(file).map(Cow::Borrowed));
        }
        // The application's own logo: the one file the installer, the taskbar and the web page also use.
        if path == "brand/devd-logo-rail.png" {
            return Ok(Some(Cow::Borrowed(
                include_bytes!("../../../assets/devd-logo-rail.png").as_slice(),
            )));
        }
        if path == "brand/devd-logo.png" {
            return Ok(Some(Cow::Borrowed(
                include_bytes!("../../../assets/devd-logo.png").as_slice(),
            )));
        }
        // A scene drawn in a theme file's colours: `art/scene/<scene>-<rrggbb>-<rrggbb>.png`.
        // The path says everything the picture needs; each one is made once and kept.
        if let Some(rest) = path
            .strip_prefix("art/scene/")
            .and_then(|p| p.strip_suffix(".png"))
        {
            return Ok(scene_picture(rest).map(Cow::Owned));
        }
        // The background art. There is one picture per theme, made the first time it is
        // asked for and kept.
        if let Some(id) = path
            .strip_prefix("art/")
            .and_then(|p| p.strip_suffix(".png"))
        {
            static PICTURES: [std::sync::OnceLock<Vec<u8>>; 5] = [
                std::sync::OnceLock::new(),
                std::sync::OnceLock::new(),
                std::sync::OnceLock::new(),
                std::sync::OnceLock::new(),
                std::sync::OnceLock::new(),
            ];
            let Some(slot) = crate::theme::Theme::ORDER
                .iter()
                .position(|known| *known == id)
            else {
                return Ok(None);
            };
            let bytes = PICTURES[slot].get_or_init(|| {
                let theme = crate::theme::Theme::by_id(id);
                crate::ui::dither::png_as(theme.art_rgb(), theme.art_glint_rgb(), theme.pattern)
            });
            return Ok(Some(Cow::Borrowed(bytes.as_slice())));
        }
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        Ok(name
            .and_then(|name| ICONS.iter().find(|(known, _)| *known == name))
            .map(|(_, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(name, _)| format!("icons/{name}.svg"))
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

/// The picture a scene path names, such as `contours-2a241d-8a4b14`, or nothing for a path that does
/// not name one. Pictures already made are kept, so a theme switched to again costs nothing.
fn scene_picture(spec: &str) -> Option<Vec<u8>> {
    static MADE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
        std::sync::LazyLock::new(Default::default);
    let mut parts = spec.split('-');
    let pattern = crate::ui::dither::Pattern::named(parts.next()?)?;
    let colour = |hex: &str| -> Option<[u8; 3]> {
        let n = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6)?;
        Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
    };
    let (base, glint) = (colour(parts.next()?)?, colour(parts.next()?)?);
    if parts.next().is_some() {
        return None;
    }
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    Some(
        made.entry(spec.to_string())
            .or_insert_with(|| crate::ui::dither::png_as(base, glint, pattern))
            .clone(),
    )
}

/// Whether an icon of this name exists. Asking for one that does not draws
/// nothing, silently, so the names used in the window are checked by a test.
#[cfg(test)]
pub fn has_icon(name: &str) -> bool {
    ICONS.iter().any(|(known, _)| *known == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_served_and_names_are_unique() {
        for (i, (name, svg)) in ICONS.iter().enumerate() {
            assert!(
                svg.starts_with("<svg ") && svg.ends_with("</svg>"),
                "{name}"
            );
            assert!(
                ICONS[i + 1..].iter().all(|(other, _)| other != name),
                "{name} is declared twice"
            );
            let loaded = Assets.load(&format!("icons/{name}.svg")).unwrap();
            assert_eq!(loaded.as_deref(), Some(svg.as_bytes()));
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert!(Assets.load("other/home.svg").unwrap().is_none());
    }

    #[test]
    fn the_rail_has_an_icon_for_every_screen() {
        for route in devd_app_core::nav::Route::primary() {
            assert!(
                has_icon(crate::ui::screen_icon(&route)),
                "{}",
                route.title()
            );
        }
    }

    #[test]
    fn every_theme_has_a_background_of_its_own() {
        let mut seen = Vec::new();
        for id in crate::theme::Theme::ORDER {
            let path = format!("art/{id}.png");
            let bytes = Assets
                .load(&path)
                .unwrap()
                .unwrap_or_else(|| panic!("{path} is not served"));
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{path}");
            assert!(
                !seen.contains(&bytes.to_vec()),
                "{id} has the same background as another theme"
            );
            seen.push(bytes.to_vec());
        }
        assert!(Assets.load("art/nope.png").unwrap().is_none());
    }

    #[test]
    fn a_scene_is_drawn_in_any_two_colours_a_theme_file_asks_for() {
        let path = crate::theme::art_path(crate::ui::dither::Pattern::Contours, [0x2a, 0x24, 0x1d], [0x8a, 0x4b, 0x14]);
        let bytes = Assets.load(&path).unwrap().expect("the scene is served");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let other = crate::theme::art_path(crate::ui::dither::Pattern::Contours, [0x2a, 0x24, 0x1d], [0x14, 0x4b, 0x8a]);
        assert_ne!(Assets.load(&other).unwrap().unwrap(), bytes, "other colours, another picture");
        for bad in ["art/scene/waves-2a241d-8a4b14.png", "art/scene/contours-2a241d.png", "art/scene/contours-zzzzzz-8a4b14.png", "art/scene/contours-2a241d-8a4b14-00.png"] {
            assert!(Assets.load(bad).unwrap().is_none(), "{bad}");
        }
    }
}
