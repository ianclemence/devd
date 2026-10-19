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

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::{AppState, INSTANCE_HEADER};

/// The application shell.
const INDEX: &str = include_str!("../ui/index.html");
/// View logic.
const SCRIPT: &str = include_str!("../ui/app.js");
/// Design tokens and layout.
const STYLE: &str = include_str!("../ui/style.css");
/// Window icon.
const FAVICON: &str = include_str!("../ui/favicon.svg");
/// Vendored xterm.js, for the agent terminal.
const XTERM_JS: &str = include_str!("../ui/vendor/xterm.js");
const XTERM_CSS: &str = include_str!("../ui/vendor/xterm.css");
const FIT_JS: &str = include_str!("../ui/vendor/addon-fit.js");

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
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-src http: https:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The application shell with the session token injected.
pub async fn index(State(state): State<Arc<AppState>>) -> Response {
    let html = INDEX.replace(TOKEN_PLACEHOLDER, &state.token);
    build("text/html; charset=utf-8", Body::from(html), Some(state.port))
}

/// The view logic.
pub async fn script() -> Response {
    build("text/javascript; charset=utf-8", Body::from(SCRIPT), None)
}

/// The stylesheet.
pub async fn style() -> Response {
    build("text/css; charset=utf-8", Body::from(STYLE), None)
}

/// The window icon.
pub async fn favicon() -> Response {
    build("image/svg+xml", Body::from(FAVICON), None)
}

/// The vendored terminal emulator.
pub async fn xterm_js() -> Response {
    build("text/javascript; charset=utf-8", Body::from(XTERM_JS), None)
}

pub async fn xterm_css() -> Response {
    build("text/css; charset=utf-8", Body::from(XTERM_CSS), None)
}

pub async fn xterm_fit() -> Response {
    build("text/javascript; charset=utf-8", Body::from(FIT_JS), None)
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
        if let Ok(value) = HeaderValue::from_str(&format!("{} ({port})", env!("CARGO_PKG_VERSION"))) {
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
        assert!(!lower.contains("<script>"), "inline script would be blocked by CSP");
        assert!(!lower.contains(" style="), "inline style attribute would be blocked by CSP");
        assert!(!lower.contains("onclick="), "inline handler would be blocked by CSP");
        assert!(lower.contains("src=\"/app.js\""));
        assert!(lower.contains("href=\"/style.css\""));
    }
}
