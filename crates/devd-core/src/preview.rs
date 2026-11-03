//! Which addresses the window may put in a preview frame.
//!
//! The Preview tab renders a page in an `<iframe>`. A frame is a place a page can
//! run script, so the address has to be decided by devd's own code rather than by
//! whatever a person typed or an agent asked for: loopback only, http or https
//! only, no credentials, no fragment tricks.
//!
//! This lives in `devd-core` rather than in the server or the window so there is
//! one rule. A second copy in JavaScript would be a second rule that disagrees
//! eventually, and the first disagreement would be a frame pointed somewhere it
//! should not be.

use std::net::{IpAddr, Ipv6Addr};

/// Why an address cannot be previewed, or the address to preview.
///
/// The returned string is the canonical form to put in the frame: scheme, host
/// and port, with no credentials and no fragment.
pub fn previewable(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("a preview needs an address".into());
    }
    if url.len() > 4096 {
        return Err("the address is too long".into());
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => return Err("an address starts with http:// or https://".into()),
    };
    if scheme != "http" && scheme != "https" {
        return Err(format!("`{scheme}://` cannot be previewed; only http and https can"));
    }
    // Strip the fragment: it never reaches a server and only confuses a frame.
    let rest = rest.split('#').next().unwrap_or(rest);
    if rest.contains('@') {
        return Err("an address with credentials in it cannot be previewed".into());
    }
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let path = &rest[authority.len()..];
    if authority.is_empty() {
        return Err("the address has no host".into());
    }
    let (host, port) = split_host_port(authority).ok_or_else(|| format!("`{authority}` is not a usable host and port"))?;
    if !is_loopback_host(&host) {
        return Err(format!(
            "`{host}` is not on this machine. A preview shows a page devd or another program on this machine serves; use a 127.0.0.1 or localhost address"
        ));
    }
    let port = match port {
        Some(p) => p,
        None => {
            if scheme == "https" {
                443
            } else {
                80
            }
        }
    };
    if port == 0 {
        return Err("a port is a number from 1 to 65535".into());
    }
    Ok(format!("{scheme}://{host}:{port}{path}"))
}

/// Split `host:port`, `[v6]:port` or a bare host.
fn split_host_port(authority: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        host.parse::<Ipv6Addr>().ok()?;
        // The canonical bracket form keeps the colon inside it unambiguous.
        let port = match after.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some(p.parse().ok()?),
            Some(_) => return None,
            None if after.is_empty() => None,
            None => return None,
        };
        return Some((format!("[{host}]"), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => Some((host.to_string(), Some(port.parse().ok()?))),
        Some(_) => None,
        None => Some((authority.to_string(), None)),
    }
}

/// Whether the host names this machine.
///
/// `localhost` and `.localhost` names, `127.0.0.0/8` and `::1`. Nothing else:
/// a LAN address is another machine, and a public one is not a preview of local
/// work.
fn is_loopback_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") {
        return true;
    }
    // A bracketed IPv6 host keeps its brackets in the authority; the address is
    // what is inside them.
    let bare = h.strip_prefix('[').and_then(|r| r.strip_suffix(']')).unwrap_or(&h);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback(),
        };
    }
    // A bare IPv4 written without dots, such as `2130706433`, resolves to
    // 127.0.0.1 on some systems. It is not accepted as a host here.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_address_is_canonicalised_for_the_frame() {
        assert_eq!(previewable("http://127.0.0.1:3000").unwrap(), "http://127.0.0.1:3000");
        assert_eq!(previewable("http://127.0.0.1:3000/").unwrap(), "http://127.0.0.1:3000/");
        assert_eq!(previewable("  http://localhost:5173/app?x=1  ").unwrap(), "http://localhost:5173/app?x=1");
        // A fragment is dropped: it never reaches the server.
        assert_eq!(previewable("http://127.0.0.1:8080/#/route").unwrap(), "http://127.0.0.1:8080/");
        // A default port is named explicitly, so the frame cannot be redirected by a
        // scheme change in the address bar.
        assert_eq!(previewable("http://127.0.0.1/").unwrap(), "http://127.0.0.1:80/");
        assert_eq!(previewable("https://localhost/x").unwrap(), "https://localhost:443/x");
        assert_eq!(previewable("http://[::1]:9000/").unwrap(), "http://[::1]:9000/");
        assert_eq!(previewable("http://shop.localhost:8000/").unwrap(), "http://shop.localhost:8000/");
    }

    #[test]
    fn anything_that_is_not_this_machine_is_refused() {
        for bad in [
            "http://example.com/",
            "https://google.com",
            "http://192.168.1.5:3000/",
            "http://10.0.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://0.0.0.0:3000/",
        ] {
            let e = previewable(bad).unwrap_err();
            assert!(e.contains("not on this machine"), "{bad}: {e}");
        }
    }

    #[test]
    fn a_scheme_or_credential_that_could_run_script_is_refused() {
        for bad in ["javascript:alert(1)", "file:///C:/Windows", "data:text/html,<script>", "ftp://127.0.0.1/", "chrome://settings"] {
            assert!(previewable(bad).is_err(), "{bad}");
        }
        assert!(previewable("http://user:pw@127.0.0.1/").is_err());
    }

    #[test]
    fn a_malformed_address_is_refused_without_panicking() {
        for bad in ["", "   ", "http://", "http://127.0.0.1:abc/", "http://127.0.0.1:99999/", "http://[::1/", "http://127.0.0.1:/"] {
            assert!(previewable(bad).is_err(), "{bad:?}");
        }
    }
}
