//! Structural checks on the interface files. They cannot see pixels, but they
//! catch the regressions that actually happen: a sidebar entry that opens
//! nothing, an action with no handler, a logo asset with transparent padding.

use base64::Engine;

const APP: &str = include_str!("../ui/app.js");
const HTML: &str = include_str!("../ui/index.html");
const CSS: &str = include_str!("../ui/style.css");

fn quoted_after(src: &str, marker: &str) -> Vec<String> {
    // every "word" inside the first [...] or {...} block following `marker`
    let start = src.find(marker).unwrap_or_else(|| panic!("{marker} not found"));
    let rest = &src[start..];
    let end = rest.find("];").or_else(|| rest.find("};")).unwrap();
    let block = &rest[..end];
    let mut out = Vec::new();
    let mut it = block.split('"');
    it.next();
    while let (Some(w), _) = (it.next(), it.next()) {
        out.push(w.to_string());
    }
    out
}

#[test]
fn every_destination_has_a_page() {
    let mut views: Vec<String> = Vec::new();
    // the rail
    for chunk in APP.split("{ view: \"").skip(1) {
        views.push(chunk.split('"').next().unwrap().to_string());
    }
    // tabs and the search palette's "go to" list
    for (start, end) in [("var TABS = {", "var AREA_HOME"), ("var GO_TO = [", "function palActions")] {
        let block = &APP[APP.find(start).unwrap()..];
        let block = &block[..block.find(end).unwrap()];
        for chunk in block.split("[\"").skip(1) {
            let name = chunk.split('"').next().unwrap();
            if chunk[name.len()..].starts_with("\", \"") && name.chars().all(|c| c.is_ascii_lowercase()) {
                views.push(name.to_string());
            }
        }
    }
    views.sort();
    views.dedup();
    assert!(views.len() > 15, "{views:?}");
    let render = &APP[APP.find("  function render() {").unwrap()..];
    let render = &render[..render.find("// ----").unwrap()];
    let server: Vec<String> = quoted_after(APP, "var SERVER_VIEWS = [");
    for v in views {
        let handled = render.contains(&format!("state.view === \"{v}\"")) || server.contains(&v) || v == "project";
        assert!(handled, "`{v}` is in the navigation but render() has no page for it");
    }
}

#[test]
fn every_action_in_the_markup_has_a_handler() {
    let handlers: Vec<&str> = APP.split("\n    ").filter_map(|l| {
        let l = l.trim_start_matches('"');
        l.split(['"', ':']).next().filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
    }).collect();
    let mut used: Vec<String> = Vec::new();
    for src in [APP, HTML] {
        for chunk in src.split("data-action=\"").skip(1) {
            let name = chunk.split('"').next().unwrap();
            if name.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !name.is_empty() {
                used.push(name.to_string());
            }
        }
    }
    used.sort();
    used.dedup();
    let dynamic = ["start-service", "stop-service"];
    for u in used {
        assert!(handlers.contains(&u.as_str()) || APP.contains(&format!("\"{u}\":")) || APP.contains(&format!("    {u}: function")) || dynamic.contains(&u.as_str()), "data-action=\"{u}\" has no handler");
    }
}

#[test]
fn the_sidebar_is_short_and_search_is_not_about_capabilities() {
    assert!(!HTML.contains("Search capabilities"));
    assert!(HTML.contains("id=\"health-chip\"") && !HTML.contains("health-pill"));
    assert!(HTML.contains("id=\"subnav\""));
    let rail = &APP[APP.find("var groups = [").unwrap()..];
    let rail = &rail[..rail.find("el(\"rail\")").unwrap()];
    let items = rail.matches("{ view: \"").count();
    assert!(items <= 14, "{items} rail entries; related pages belong in tabs");
    assert!(CSS.contains(".subtab"));
}

#[test]
fn every_icon_comes_from_the_same_logo() {
    // The artwork has one source: `assets/devd-logo.png`. Everything else is
    // generated from it by `scripts/make-icons.py`, so the mark in the window,
    // the icon on the taskbar and the favicon are the same picture.
    let desktop = include_bytes!("../../devd-desktop/icons/128x128.png");
    let marker = "data:image/png;base64,";
    let start = HTML.find(marker).expect("inline logo") + marker.len();
    let end = HTML[start..].find('"').expect("inline logo end") + start;
    let inline = &HTML[start..end];
    let encoded = base64::engine::general_purpose::STANDARD.encode(desktop);
    assert_eq!(inline, encoded, "the in-app mark is not the desktop icon");

    assert_eq!(&desktop[1..4], b"PNG");
    let width = u32::from_be_bytes(desktop[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(desktop[20..24].try_into().unwrap());
    assert_eq!((width, height), (128, 128));
    assert_eq!(desktop[25], 2, "the artwork is opaque: colour type 2 (RGB)");
}

#[test]
fn the_logo_has_no_transparent_padding_and_is_square() {
    let marker = "data:image/png;base64,";
    let start = HTML.find(marker).expect("inline logo") + marker.len();
    let b64: String = HTML[start..].chars().take_while(|c| *c != '"').collect();
    let png = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
    assert_eq!(&png[1..4], b"PNG");
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!(w, h, "the logo is square, so a square box shows all of it");
    assert!(w >= 80, "crisp at 40 CSS px on a 2x display");
    // PNG colour type 2 is RGB: no alpha channel, so no transparent frame can exist.
    assert_eq!(png[25], 2, "the logo asset has no alpha channel");
    let rule = &CSS[CSS.find(".brand-mark {").unwrap()..];
    let rule = &rule[..rule.find('}').unwrap()];
    assert!(rule.contains("object-fit: cover") && !rule.contains("padding") && !rule.contains("margin") && !rule.contains("box-shadow"), "{rule}");
}
