//! The small site icon before a web link, as in other assistants' answers.
//!
//! Text in the window is one run laid out by the text engine, which cannot hold a picture in the middle of
//! a line. So room is made for the icon *in the text*: two figure spaces (which do not break) are put
//! before the link, the text is laid out around them, and the icon is painted into that room once the
//! layout says where it landed. The link's colour and underline start after the room, so the icon sits
//! outside them, and a click on the room is a click on nothing.
//!
//! The icon itself is asked of devd (`web.favicon`), which gets it from the site and keeps it. Until it
//! arrives, or when the site has none, a tile with the site's first letter stands in; an address on this
//! machine or a private network is a globe and is never looked up.

use std::path::PathBuf;

use devd_app_core::sync::QueryKey;
use gpui::{App, Div, ObjectFit, div, img, prelude::*, px};
use serde_json::json;

use super::{icon, tint};
use crate::state::devd;
use crate::theme::theme;

/// The room reserved before a link. Figure spaces are as wide as digits and are never a place to wrap.
pub const ROOM: &str = "\u{2007}\u{2007}";
/// The size of the icon, in pixels.
pub const SIZE: f32 = 13.0;

/// The host a web address points at, lower case and without a port, or `None` when it is not a web address
/// or has credentials in it.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => authority,
    };
    Some(
        host.trim_matches(['[', ']'])
            .trim_end_matches('.')
            .to_ascii_lowercase(),
    )
    .filter(|h| !h.is_empty())
}

/// Whether an address is on this machine or a private network, so it gets a globe and no lookup.
pub fn is_local(host: &str) -> bool {
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".test")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".home.arpa")
        || !host.contains('.') && !host.contains(':')
    {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// The letter that stands for a site: the first of its name, without `www.`.
pub fn letter(host: &str) -> String {
    host.strip_prefix("www.")
        .unwrap_or(host)
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

/// Whether the person has site icons on. Until the setting has arrived they are, which is the default.
pub fn enabled(cx: &App) -> bool {
    let flags = devd(cx).sync.watch(&QueryKey::bare("flags.list"));
    flags
        .list("")
        .iter()
        .find(|f| f["key"] == "ui.site_icons")
        .is_none_or(|f| f["value"] != json!(false))
}

/// Start looking up a site's icon if that has not been done, and say where it is when it has.
pub fn lookup(host: &str, cx: &App) -> Option<PathBuf> {
    if is_local(host) {
        return None;
    }
    let found = devd(cx)
        .sync
        .watch(&QueryKey::new("web.favicon", json!({ "host": host })));
    let data = found.data.as_deref()?;
    (data["found"] == json!(true))
        .then(|| data["path"].as_str().map(PathBuf::from))
        .flatten()
}

/// The icon for a host, drawn at [`SIZE`].
pub fn tile(host: &str, cx: &App) -> Div {
    let t = theme(cx);
    let base = div()
        .size(px(SIZE))
        .flex_none()
        .rounded(px(3.0))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    if is_local(host) {
        return base.child(icon("globe", SIZE - 1.0, t.faint));
    }
    match lookup(host, cx) {
        // On a pale backing, so a dark mark (the Rust gear, a black wordmark) is seen on a dark theme too.
        Some(path) => base
            .bg(gpui::rgba(0xffffffee))
            .p(px(1.0))
            .child(img(path).size_full().object_fit(ObjectFit::Contain)),
        None => base.bg(tint(t.muted, 0.22)).child(
            div()
                .text_size(px(8.5))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(t.muted)
                .child(letter(host)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_of_an_address_is_lower_case_without_port_or_credentials() {
        assert_eq!(
            host_of("https://Docs.RS/serde?x=1#top").as_deref(),
            Some("docs.rs")
        );
        assert_eq!(
            host_of("http://example.com:8080/a").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("https://[::1]:3000/").as_deref(), Some("::1"));
        for not in [
            "ftp://x.com",
            "mailto:a@b.com",
            "https://user:pw@evil.com/",
            "https://",
            "/relative",
            "javascript:alert(1)",
        ] {
            assert_eq!(host_of(not), None, "{not}");
        }
    }

    #[test]
    fn this_machine_and_private_networks_are_never_looked_up() {
        for local in [
            "localhost",
            "shop.test",
            "app.localhost",
            "printer.local",
            "intranet",
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.9",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "nas.home.arpa",
        ] {
            assert!(is_local(local), "{local}");
        }
        for public in [
            "docs.rs",
            "www.rust-lang.org",
            "93.184.216.34",
            "github.com",
        ] {
            assert!(!is_local(public), "{public}");
        }
    }

    #[test]
    fn a_site_without_an_icon_is_known_by_its_first_letter() {
        assert_eq!(letter("www.rust-lang.org"), "R");
        assert_eq!(letter("docs.rs"), "D");
        assert_eq!(letter("9gag.com"), "9");
        assert_eq!(letter("---"), "?");
    }

    #[test]
    fn the_room_is_two_figure_spaces_that_never_break_a_line() {
        assert_eq!(ROOM.chars().count(), 2);
        assert!(ROOM.chars().all(|c| c == '\u{2007}'));
    }
}
