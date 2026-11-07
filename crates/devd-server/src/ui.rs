//! The application UI, compiled into the binary.
//!
//! The UI is embedded rather than loaded from disk, for the same reason the
//! capability registry is: there is exactly one version of each, so they cannot
//! drift. A `devd` binary always serves the UI that matches its API, and there
//! is no directory to get out of sync after an upgrade.
//!
//! Two headers on the HTML matter beyond `Content-Type`:
//!
//! * **`Content-Security-Policy`** denies everything by default and then allows
//!   only same-origin script, style and connect. That is why the UI is three
//!   separate files with no inline handlers: the policy is real, not decorative,
//!   and a future change that introduces an inline script will simply fail
//!   rather than quietly widen what the page may do.
//! * **`X-Devd-App`** lets `devd ui` recognise its own server on a port, so it
//!   can attach to a running instance instead of starting a second one.

use std::io::Write as _;
use std::sync::{Arc, OnceLock};

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::{AppState, INSTANCE_HEADER};

/// The application shell.
const INDEX: &str = include_str!("../ui/index.html");
/// View logic.
const SCRIPT: &str = include_str!("../ui/app.js");
/// Design tokens and layout.
const STYLE: &str = include_str!("../ui/style.css");
/// Window icon.
const FAVICON: &str = include_str!("../ui/favicon.svg");
/// The interface typeface (Inter Tight, a variable font, SIL Open Font License).
/// It ships inside the binary so the interface looks the same on every machine
/// and never asks a font service for anything.
const FONT_INTER_TIGHT: &[u8] = include_bytes!("../ui/fonts/inter-tight-latin.woff2");

// xterm.js renders a real terminal: cursor, colours, scrollback and the
// full-screen programs a text log cannot show. It is MIT, it is vendored rather
// than fetched, and its licences sit beside it.
const XTERM_JS: &str = include_str!("../ui/vendor/xterm.js");
const XTERM_CSS: &str = include_str!("../ui/vendor/xterm.css");
const ADDON_FIT: &str = include_str!("../ui/vendor/addon-fit.js");
const KATEX_JS: &str = include_str!("../ui/vendor/katex.min.js");
const KATEX_CSS: &str = include_str!("../ui/vendor/katex.min.css");
const KATEX_AUTO: &str = include_str!("../ui/vendor/auto-render.min.js");
const MERMAID_JS: &str = include_str!("../ui/vendor/mermaid.min.js");

/// Placeholder substituted with the bearer token at serve time.
///
/// The token is 64 hex characters, so it needs no escaping to be safe in an
/// HTML attribute. The substitution is a plain `replace` rather than a template
/// engine because a template engine in front of the token is one more place a
/// mistake could leak it.
const TOKEN_PLACEHOLDER: &str = "__DEVD_TOKEN__";

/// Policy applied to the HTML: nothing is allowed unless named.
///
/// `connect-src 'self'` is what keeps the token from being usable by a script
/// that somehow got injected: it could not send it anywhere else.
///
/// `style-src` permits inline *styles* because the application uses a few for spacing. Inline
/// style is not script: it cannot execute, and script remains limited to
/// same-origin files.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; connect-src 'self'; img-src 'self' data: blob:; frame-src http: https:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// An embedded asset, compressed and tagged once for the life of the process.
///
/// The script is nearly a megabyte of source; sending it uncompressed and
/// re-sending it on every reload is the slowest thing the interface does. The
/// gzip copy is built on first request and the ETag lets a reload cost a 304
/// instead of the whole file.
struct Packed {
    gzip: Bytes,
    etag: HeaderValue,
}

fn packed(slot: &'static OnceLock<Packed>, bytes: &'static [u8]) -> &'static Packed {
    slot.get_or_init(|| {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        let _ = encoder.write_all(bytes);
        let gzip = encoder.finish().unwrap_or_default();
        let etag = format!("\"{}\"", hex::encode(Sha256::digest(bytes)));
        Packed {
            gzip: Bytes::from(gzip),
            etag: HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("\"0\"")),
        }
    })
}

static SCRIPT_PACKED: OnceLock<Packed> = OnceLock::new();
static STYLE_PACKED: OnceLock<Packed> = OnceLock::new();
static XTERM_JS_PACKED: OnceLock<Packed> = OnceLock::new();
static XTERM_CSS_PACKED: OnceLock<Packed> = OnceLock::new();
static ADDON_FIT_PACKED: OnceLock<Packed> = OnceLock::new();
static KATEX_JS_PACKED: OnceLock<Packed> = OnceLock::new();
static KATEX_CSS_PACKED: OnceLock<Packed> = OnceLock::new();
static KATEX_AUTO_PACKED: OnceLock<Packed> = OnceLock::new();
static MERMAID_PACKED: OnceLock<Packed> = OnceLock::new();

/// A static asset: revalidated by ETag, and sent compressed when the browser
/// says it accepts gzip.
fn asset(
    content_type: &str,
    bytes: &'static [u8],
    slot: &'static OnceLock<Packed>,
    headers: &HeaderMap,
) -> Response {
    let packed = packed(slot, bytes);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(packed.etag.to_str().unwrap_or(""))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let h = response.headers_mut();
        h.insert(header::ETAG, packed.etag.clone());
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        return response;
    }
    let gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("gzip"))
        .unwrap_or(false);
    let body = if gzip {
        Body::from(packed.gzip.clone())
    } else {
        Body::from(bytes)
    };
    let mut response = build(content_type, body, None);
    let h = response.headers_mut();
    // `no-cache` means "ask again", not "do not store": the ETag makes the ask
    // cheap, and a rebuilt binary still serves the new file immediately.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::ETAG, packed.etag.clone());
    if gzip {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    response
}

/// The application shell with the session token injected.
pub async fn index(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let html = INDEX.replace(TOKEN_PLACEHOLDER, &state.token);
    let gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("gzip"))
        .unwrap_or(false);
    // The token is per session, so the page itself is never cached; it is
    // small, and compressing it costs less than the round trip it saves.
    let mut body = Vec::new();
    if gzip {
        let mut encoder = flate2::write::GzEncoder::new(&mut body, flate2::Compression::fast());
        let _ = encoder.write_all(html.as_bytes());
        let _ = encoder.finish();
    } else {
        body = html.into_bytes();
    }
    let mut response = build("text/html; charset=utf-8", Body::from(body), Some(state.port));
    let h = response.headers_mut();
    if gzip {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    response
}

/// The view logic.
pub async fn script(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        SCRIPT.as_bytes(),
        &SCRIPT_PACKED,
        &headers,
    )
}

/// The stylesheet.
pub async fn style(headers: HeaderMap) -> Response {
    asset(
        "text/css; charset=utf-8",
        STYLE.as_bytes(),
        &STYLE_PACKED,
        &headers,
    )
}

/// xterm.js - the terminal renderer.
pub async fn xterm(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        XTERM_JS.as_bytes(),
        &XTERM_JS_PACKED,
        &headers,
    )
}

/// xterm.js's stylesheet.
pub async fn xterm_style(headers: HeaderMap) -> Response {
    asset(
        "text/css; charset=utf-8",
        XTERM_CSS.as_bytes(),
        &XTERM_CSS_PACKED,
        &headers,
    )
}

/// The fit addon, which sizes the terminal to its box.
pub async fn xterm_fit(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        ADDON_FIT.as_bytes(),
        &ADDON_FIT_PACKED,
        &headers,
    )
}

/// KaTeX: math set in the browser, offline.
pub async fn katex(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        KATEX_JS.as_bytes(),
        &KATEX_JS_PACKED,
        &headers,
    )
}

/// KaTeX's stylesheet, with its fonts served from this binary.
pub async fn katex_style(headers: HeaderMap) -> Response {
    asset(
        "text/css; charset=utf-8",
        KATEX_CSS.as_bytes(),
        &KATEX_CSS_PACKED,
        &headers,
    )
}

/// KaTeX's auto-render extension.
pub async fn katex_auto(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        KATEX_AUTO.as_bytes(),
        &KATEX_AUTO_PACKED,
        &headers,
    )
}

/// One KaTeX font file.
pub async fn katex_font(Path(file): Path<String>) -> Response {
    match crate::katex_fonts::bytes(&file) {
        Some(bytes) => build("font/woff2", Body::from(bytes), None),
        None => missing(),
    }
}

/// The diagram renderer, offline.
pub async fn mermaid(headers: HeaderMap) -> Response {
    asset(
        "text/javascript; charset=utf-8",
        MERMAID_JS.as_bytes(),
        &MERMAID_PACKED,
        &headers,
    )
}

/// The window icon.
pub async fn favicon() -> Response {
    build("image/svg+xml", Body::from(FAVICON), None)
}

/// The interface typeface.
pub async fn font(Path(file): Path<String>) -> Response {
    if file != "inter-tight-latin.woff2" {
        return missing();
    }
    build("font/woff2", Body::from(FONT_INTER_TIGHT), None)
}

/// The brand marks of the services and providers devd supports.
///
/// These are the owners' own artwork, taken from their own sites and from a
/// curated brand registry. They are served from here rather than fetched from
/// the internet at runtime, so the interface never depends on a third party
/// being reachable and never shows a mark that has been tampered with.
///
/// The name is a file name and nothing else: the lookup is against a fixed
/// list, so a request cannot reach outside the directory.
pub async fn brand(Path(file): Path<String>) -> Response {
    let Some((name, extension)) = file.rsplit_once('.') else {
        return missing();
    };
    if !matches!(extension, "svg" | "png") {
        return missing();
    }
    // A file name is letters, digits and dashes. Anything else is not a brand.
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return missing();
    }
    let Ok(bytes) = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui/brands")
            .join(&file),
    ) else {
        return missing();
    };
    let content_type = if extension == "svg" {
        "image/svg+xml"
    } else {
        "image/png"
    };
    build(content_type, Body::from(bytes), None)
}

/// A brand mark that does not exist.
fn missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "no such brand mark",
    )
        .into_response()
}

fn build(content_type: &str, body: Body, port: Option<u16>) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("text/plain")),
    );
    // Nothing here is cacheable: the HTML carries a per-session token, and the
    // assets are small enough that revalidation costs more than the transfer.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    // A loopback UI has no business being framed by anything, including itself.
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    if let Some(port) = port {
        if let Ok(value) = HeaderValue::from_str(&format!("{} ({port})", env!("CARGO_PKG_VERSION")))
        {
            headers.insert(header::HeaderName::from_static(INSTANCE_HEADER), value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_carries_the_placeholder() {
        assert!(INDEX.contains(TOKEN_PLACEHOLDER));
    }

    #[test]
    fn the_shell_ships_no_inline_script_or_style() {
        // The CSP forbids both, so a regression that added one would produce a
        // blank window rather than a test failure. Assert it here instead.
        let lower = INDEX.to_lowercase();
        assert!(
            !lower.contains("<script>"),
            "inline script would be blocked by CSP"
        );
        assert!(
            !lower.contains(" style="),
            "inline style attribute would be blocked by CSP"
        );
        assert!(
            !lower.contains("onclick="),
            "inline handler would be blocked by CSP"
        );
        assert!(lower.contains("src=\"/app.js\""));
        assert!(lower.contains("href=\"/style.css\""));
    }
}
