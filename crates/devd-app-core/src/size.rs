//! The interface's proportions, in one place for both interfaces.
//!
//! The native window draws every screen itself and takes its numbers from here
//! (`crate::theme::size` re-exports them). The browser client is a page, and a page
//! cannot read a Rust constant: it is handed this same table at startup
//! (`/api/sizes`, which `layout.js` reads before the page draws), and its stylesheet
//! is written in terms of the custom properties that table fills in. A change to
//! the spacing of the product is therefore made once, and both interfaces move
//! together instead of drifting apart.
//!
//! What is *not* here is colour: the window's themes and the page's themes are two
//! palettes that must agree too, and they are the next part of this table to move.

/// One number, with the name it is drawn and styled by in the page.
pub struct Size {
    /// The name the page calls it (`rail`), used as `--size-rail` in CSS and `SIZES.rail` in script.
    pub name: &'static str,
    /// The value, in logical pixels as the window draws them (CSS pixels in the page).
    pub px: f32,
    /// What it is for, so a reader of the table knows which screen it moves.
    pub what: &'static str,
}

/// Every size both interfaces share, in the order they appear on a screen.
pub const SIZES: &[Size] = &[
    Size { name: "rail", px: 56.0, what: "the rail of screens down the left edge" },
    Size { name: "topbar", px: 44.0, what: "the bar across the top of the window" },
    Size { name: "status", px: 26.0, what: "the strip along the bottom" },
    Size { name: "page", px: 1040.0, what: "the widest a page's content column gets" },
    Size { name: "page-wide", px: 1240.0, what: "the same, for diffs and file trees" },
    Size { name: "sidebar", px: 288.0, what: "a list beside the main content" },
    Size { name: "collapsed", px: 48.0, what: "that list put away: one column of controls" },
    Size { name: "row", px: 36.0, what: "a row in a list" },
    Size { name: "control", px: 28.0, what: "a button, an input, a tab" },
    Size { name: "reading", px: 820.0, what: "the widest a column of reading text gets" },
    Size { name: "radius-s", px: 4.0, what: "a badge, a checkbox" },
    Size { name: "radius", px: 6.0, what: "a button, an input, a row" },
    Size { name: "radius-l", px: 10.0, what: "a card, a menu, a dialog" },
    Size { name: "text-2xs", px: 11.0, what: "a count in a corner" },
    Size { name: "text-xs", px: 12.0, what: "metadata, the status strip" },
    Size { name: "text-sm", px: 13.0, what: "labels and second lines" },
    Size { name: "text", px: 14.0, what: "the interface's body size" },
    Size { name: "text-md", px: 15.0, what: "reading text: a conversation, a document" },
    Size { name: "text-lg", px: 17.0, what: "a section's lead" },
    Size { name: "text-xl", px: 20.0, what: "a screen's title" },
    Size { name: "text-display", px: 28.0, what: "the one large line on Home" },
];

/// The size called `name`, when there is one.
pub fn find(name: &str) -> Option<&'static Size> {
    SIZES.iter().find(|s| s.name == name)
}

/// The table as CSS custom properties (`--size-rail: 56px;`), which the page's stylesheet uses.
pub fn css() -> String {
    let mut out = String::from(":root{");
    for s in SIZES {
        out.push_str(&format!("--size-{}:{}px;", s.name, s.px));
    }
    out.push('}');
    out
}

/// The table as a script, so page code that must compute a number (a scroll offset,
/// a terminal's width) reads the same values the stylesheet does.
pub fn script() -> String {
    let mut out = String::from("window.DEVD_SIZES={");
    for (i, s) in SIZES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{:?}:{}", s.name, s.px));
    }
    out.push_str("};");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_unique_and_has_a_value_and_a_reason() {
        let mut names: Vec<&str> = SIZES.iter().map(|s| s.name).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "a name appears twice");
        for s in SIZES {
            assert!(s.px > 0.0, "{}", s.name);
            assert!(!s.what.is_empty(), "{}", s.name);
            assert!(s.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        }
    }

    #[test]
    fn the_table_is_written_for_the_page_in_both_forms() {
        let css = css();
        assert!(css.starts_with(":root{--size-"));
        assert!(css.contains("--size-rail:56px;"));
        let script = script();
        assert!(script.starts_with("window.DEVD_SIZES={"));
        assert!(script.contains("\"reading\":820"));
    }
}
