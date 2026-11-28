//! The page's icon family as a sprite.
//!
//! The browser client draws the window's icons, not its own. The window's family is one 16-unit
//! grid, a 1.5-unit stroke, round caps and joins and no fills (`devd-app/src/assets.rs`); this
//! serves those exact shapes as `<symbol>`s, `#win-icon-home` and so on, for the page to stroke in
//! its own text colour. A shape added to the window is one line here, and a test keeps the two
//! lists the same.

/// Every icon the page may draw, with the window's name for it and its paths.
const ICONS: &[(&str, &str)] = &[
    ("home", r#"<path d="M2.5 7.2 8 2.5l5.5 4.7V13a.5.5 0 0 1-.5.5H9.8V9.8H6.2v3.7H3a.5.5 0 0 1-.5-.5Z"/>"#),
    ("inbox", r#"<path d="M2.5 9.5 4.3 3.4a.6.6 0 0 1 .6-.4h6.2a.6.6 0 0 1 .6.4l1.8 6.1v3a.5.5 0 0 1-.5.5H3a.5.5 0 0 1-.5-.5Z"/><path d="M2.5 9.5h3.2l.8 1.6h3l.8-1.6h3.2"/>"#),
    ("assistant", r#"<path d="M8 2.2c.5 3.2 2.1 4.8 5.3 5.3-3.2.5-4.8 2.1-5.3 5.3-.5-3.2-2.1-4.8-5.3-5.3C5.900 7 7.500 5.400 8 2.200Z"/><path d="M12.6 11.400v2.400M11.400 12.600h2.400"/>"#),
    ("runs", r#"<path d="M8 2.200 2.500 5 8 7.800 13.500 5Z"/><path d="M2.500 8 8 10.800 13.500 8"/><path d="M2.500 11 8 13.800 13.500 11"/>"#),
    ("team", r#"<circle cx="6" cy="5.500" r="2.200"/><path d="M2 13c0-2.200 1.800-3.700 4-3.700s4 1.500 4 3.700"/><path d="M10.600 3.500a2.200 2.200 0 0 1 0 4.100M11.800 9.600c1.400.4 2.200 1.700 2.200 3.400"/>"#),
    ("folder", r#"<path d="M2.500 4.500a1 1 0 0 1 1-1h2.600l1.400 1.500h5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1Z"/>"#),
    ("globe", r#"<circle cx="8" cy="8" r="5.500"/><path d="M2.500 8h11M8 2.500c1.800 1.700 1.800 9.300 0 11M8 2.500c-1.800 1.700-1.800 9.300 0 11"/>"#),
    ("machine", r#"<rect x="2" y="2.500" width="12" height="8" rx="1"/><path d="M5.500 13.500h5M8 10.500v3"/>"#),
    ("models", r#"<path d="M8 2 13 5v6l-5 3-5-3V5Z"/><path d="M8 8 13 5M8 8 3 5M8 8v6"/>"#),
    ("settings", r#"<circle cx="8" cy="8" r="2"/><path d="M8 1.800v2M8 12.200v2M1.800 8h2M12.200 8h2M3.600 3.600l1.400 1.400M11 11l1.400 1.400M12.400 3.600 11 5M5 11l-1.400 1.400"/>"#),
    ("search", r#"<circle cx="7" cy="7" r="4.500"/><path d="m10.500 10.500 3 3"/>"#),
    ("chevron-right", r#"<path d="m6 3 5 5-5 5"/>"#),
    ("chevron-down", r#"<path d="m3 6 5 5 5-5"/>"#),
    ("arrow-left", r#"<path d="M13 8H3M7 4 3 8l4 4"/>"#),
    ("arrow-right", r#"<path d="M3 8h10M9 4l4 4-4 4"/>"#),
    ("sidebar", r#"<rect x="2" y="3" width="12" height="10" rx="1.300"/><path d="M6 3v10"/>"#),
    ("external", r#"<path d="M9 2.5h4.5V7M13 3 7.5 8.5M12 9.5v3.5a.5.5 0 0 1-.5.5h-8a.5.5 0 0 1-.5-.5v-8a.5.5 0 0 1 .5-.5H7"/>"#),
    ("terminal", r#"<rect x="2" y="2.5" width="12" height="11" rx="1"/><path d="m4.8 6 2.2 2-2.2 2M8.6 10.4h3"/>"#),
    ("branch", r#"<circle cx="4.5" cy="3.5" r="1.6"/><circle cx="4.5" cy="12.5" r="1.6"/><circle cx="11.5" cy="5.5" r="1.6"/><path d="M4.5 5.1v5.8M11.5 7.1c0 2.2-1.7 2.6-3.4 3.1-1.6.5-2.6.9-3.6 2.3"/>"#),
    ("clock", r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 5v3.2l2.2 1.4"/>"#),
    ("bolt", r#"<path d="M8.8 2 3.5 9h4l-.8 5 5.8-7H8.4Z"/>"#),
    ("wrench", r#"<path d="M10.8 2.4a3.4 3.4 0 0 0-3.3 4.4L2.8 11.5a1.6 1.6 0 0 0 2.2 2.2l4.7-4.7a3.4 3.4 0 0 0 4.4-3.3l-2 2-1.7-.5-.5-1.7Z"/>"#),
    ("plus", r#"<path d="M8 3v10M3 8h10"/>"#),
    ("x", r#"<path d="m4 4 8 8M12 4l-8 8"/>"#),
    ("check", r#"<path d="m3 8.5 3.2 3L13 4.5"/>"#),
    ("alert", r#"<path d="M8 2.5 14 13H2Z"/><path d="M8 6.5v3M8 11.3v.2"/>"#),
    ("info", r#"<circle cx="8" cy="8" r="5.5"/><path d="M8 7.4v3.1M8 5.2v.2"/>"#),
    ("file", r#"<path d="M4 2.5h5l3 3v8H4Z"/><path d="M9 2.5v3h3"/>"#),
    ("play", r#"<path d="M5 3.5 12 8l-7 4.5Z"/>"#),
    ("more", r#"<circle cx="3.5" cy="8" r=".9"/><circle cx="8" cy="8" r=".9"/><circle cx="12.5" cy="8" r=".9"/>"#),
    ("sun", r#"<circle cx="8" cy="8" r="2.8"/><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1"/>"#),
];

const HEAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" aria-hidden="true" focusable="false" width="0" height="0" style="position:absolute;width:0;height:0;overflow:hidden">"#;

/// The whole family as one sprite.
pub fn sprite() -> String {
    let mut out = String::from(HEAD);
    for (name, body) in ICONS {
        out.push_str(&format!(
            r#"<symbol id="win-icon-{name}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{body}</symbol>"#
        ));
    }
    out.push_str("</svg>");
    out
}

/// Whether the family has a shape by this name.
pub fn has(name: &str) -> bool {
    ICONS.iter().any(|(n, _)| *n == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_draws_shapes_the_window_has() {
        for item in crate::pages::RAIL {
            assert!(has(item.icon), "the rail asks for `{}`, which the window does not draw", item.icon);
        }
        for name in ["home", "inbox", "assistant", "runs", "team", "models", "folder", "globe", "machine", "clock", "settings"] {
            assert!(has(name), "{name}");
        }
    }

    #[test]
    fn the_sprite_is_one_document_with_a_symbol_per_shape() {
        let out = sprite();
        assert!(out.starts_with(HEAD));
        assert!(out.ends_with("</svg>"));
        assert_eq!(out.matches("<symbol").count(), ICONS.len());
        assert!(out.contains(r#"id="win-icon-home""#));
        let mut names: Vec<&str> = ICONS.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "a shape is drawn twice");
    }
}
