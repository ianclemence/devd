//! Prove that a site is actually served, rather than merely recorded.
//!
//! Every other site capability observes one part of the picture: `site.list`
//! reads the record, `hosts.show` reads the hosts block, and the service status
//! says whether the web server process is running. None of them answers the
//! question a person actually asks - "does opening this address give me the
//! site?" - and none of them can honestly satisfy a goal that promised a working
//! address.
//!
//! This module performs the same checks in order and stops at the first that
//! fails, so the report names the reason rather than a vague failure:
//!
//! 1. the hostname is in devd's managed hosts block (or resolves without one);
//! 2. it resolves to an address on this machine;
//! 3. something is listening on the site's port;
//! 4. a TLS site completes a handshake that chains to devd's own authority;
//! 5. an HTTP request answers below 500.
//!
//! It is a read. It opens one connection and sends one `GET /`.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use devd_core::capability::Ctx;
use devd_core::state::Site;
use devd_core::{DevdError, Result};

/// How long to wait for the site's socket.
const CONNECT: Duration = Duration::from_millis(1_500);
/// How long to wait for the answer, once connected.
const READ: Duration = Duration::from_secs(5);

/// What a verification observed.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Hostname that was checked.
    pub host: String,
    /// Address that was requested.
    pub url: String,
    /// Whether the hostname is in devd's managed hosts block.
    pub in_hosts: bool,
    /// Whether the hostname resolves to at least one address.
    pub resolves: bool,
    /// Whether something accepted a connection on the site's port.
    pub listening: bool,
    /// Whether a TLS handshake completed against devd's authority.
    pub tls: bool,
    /// HTTP status, when the request produced one.
    pub status: Option<u16>,
    /// Whether the address served the site.
    pub ok: bool,
    /// One sentence saying what happened and, when it failed, why.
    pub detail: String,
}

impl Report {
    /// The report as the structured payload a capability returns.
    pub fn data(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Verify one site.
pub fn check(ctx: &Ctx, site: &Site) -> Result<Report> {
    let url = crate::url_for(ctx, site);
    let tls = url.starts_with("https://");
    let port = if tls {
        ctx.settings.https_port
    } else {
        ctx.settings.http_port
    };

    let in_hosts = crate::hosts::current_entries()
        .unwrap_or_default()
        .iter()
        .any(|e| e.host.eq_ignore_ascii_case(&site.host));
    let managed_or_known = in_hosts || crate::resolves_without_hosts(&site.host);

    let addrs: Vec<SocketAddr> = (site.host.as_str(), port)
        .to_socket_addrs()
        .map(|it| it.collect())
        .unwrap_or_default();
    let resolves = !addrs.is_empty();
    let listening = addrs
        .iter()
        .any(|addr| TcpStream::connect_timeout(addr, CONNECT).is_ok());

    let mut report = Report {
        host: site.host.clone(),
        url: url.clone(),
        in_hosts,
        resolves,
        listening,
        tls: false,
        status: None,
        ok: false,
        detail: String::new(),
    };

    if !managed_or_known {
        report.detail = format!(
            "{} is not in the hosts file, so this machine does not resolve it; run `devd hosts sync`",
            site.host
        );
        return Ok(report);
    }
    if !resolves {
        report.detail = format!("{} did not resolve to an address", site.host);
        return Ok(report);
    }
    if !listening {
        report.detail = format!(
            "nothing is listening on {} for {}; start the web server",
            port, site.host
        );
        return Ok(report);
    }

    let ca = if tls {
        let path = devd_services::tls::ca_cert_path(ctx);
        if !path.is_file() {
            report.detail = format!(
                "{} is served over TLS but there is no devd certificate authority to verify it against",
                site.host
            );
            return Ok(report);
        }
        Some(path)
    } else {
        None
    };

    match probe(&url, ca.as_deref()) {
        Ok(status) => {
            report.tls = tls;
            report.status = Some(status);
            report.ok = status < 500;
            report.detail = if report.ok {
                format!("{} answered HTTP {status}", site.host)
            } else {
                format!(
                    "{} answered HTTP {status}; the web server or the program behind it is failing",
                    site.host
                )
            };
        }
        Err(err) => {
            report.detail = if tls {
                format!(
                    "the TLS handshake or request to {} failed: {err}; the certificate may not chain to devd's authority",
                    site.host
                )
            } else {
                format!("the request to {} failed: {err}", site.host)
            };
        }
    }
    Ok(report)
}

/// Request one URL and return its status.
///
/// Redirects are not followed: a redirect is an answer, and following it would
/// hide which address actually served the site. A proxy from the environment is
/// deliberately ignored, because a local site must be reached directly.
fn probe(url: &str, ca: Option<&Path>) -> std::result::Result<u16, String> {
    let mut builder = ureq::AgentBuilder::new()
        .redirects(0)
        .try_proxy_from_env(false)
        .user_agent("devd-site-verify")
        .timeout_connect(CONNECT)
        .timeout_read(READ);
    if let Some(ca) = ca {
        builder = builder.tls_config(tls_config(ca)?);
    }
    match builder.build().get(url).call() {
        Ok(response) => Ok(response.status()),
        Err(ureq::Error::Status(status, _)) => Ok(status),
        Err(err) => Err(err.to_string()),
    }
}

/// A client configuration that trusts devd's authority and nothing else.
///
/// The only host this client is used for is a local site, whose leaf is issued
/// by that authority. Adding the machine's whole trust store would make a local
/// name that resolves elsewhere verifiable against a public certificate, which
/// is exactly the confusion this check exists to avoid.
fn tls_config(
    ca: &Path,
) -> std::result::Result<std::sync::Arc<ureq::rustls::ClientConfig>, String> {
    use base64::Engine;

    let pem =
        std::fs::read_to_string(ca).map_err(|e| format!("{} is unreadable: {e}", ca.display()))?;
    let mut roots = ureq::rustls::RootCertStore::empty();
    for block in pem.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let Some(body) = block.split("-----END CERTIFICATE-----").next() else {
            continue;
        };
        let encoded: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let der = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("{} is not valid PEM: {e}", ca.display()))?;
        roots
            .add(ureq::rustls::pki_types::CertificateDer::from(der))
            .map_err(|e| format!("{} is not a usable root certificate: {e}", ca.display()))?;
    }
    if roots.is_empty() {
        return Err(format!("{} contains no certificate", ca.display()));
    }
    let provider = std::sync::Arc::new(ureq::rustls::crypto::ring::default_provider());
    let config = ureq::rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(std::sync::Arc::new(config))
}

/// A site's certificate chain, for a caller that wants to report it.
pub fn certificate(ctx: &Ctx, host: &str) -> Option<PathBuf> {
    let path = devd_services::tls::cert_path_for_host(ctx, host);
    path.is_file().then_some(path)
}

/// Build the payload a capability returns from a report.
///
/// Kept separate from [`check`] so the capability and its tests agree on the
/// shape the evidence classifier reads: `host`, `status` and `ok`.
pub fn payload(report: &Report) -> Value {
    report.data()
}

/// A failure that names the site, for a caller that asked about one that is gone.
pub fn missing(name: &str) -> DevdError {
    DevdError::not_found(format!("no site named `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    /// Answer one request with a fixed status line.
    fn serve_once(listener: TcpListener, status: &'static str) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                drain_request(&mut stream);
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
                // Close both directions politely. Dropping a socket that still
                // has unread request bytes makes Windows send RST, which can
                // discard the response before the client has read it.
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        })
    }

    /// Read the request head, so the response is not racing an unread buffer.
    fn drain_request(stream: &mut TcpStream) {
        use std::io::Read;

        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
    }

    #[test]
    fn a_real_answer_is_read_without_following_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = serve_once(listener, "200 OK");
        let status = probe(&format!("http://127.0.0.1:{port}/"), None).unwrap();
        assert_eq!(status, 200);
        handle.join().unwrap();
    }

    #[test]
    fn a_redirect_is_an_answer_not_something_to_follow() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                drain_request(&mut stream);
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nlocation: /elsewhere\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });
        let status = probe(&format!("http://127.0.0.1:{port}/"), None).unwrap();
        assert_eq!(status, 302);
        handle.join().unwrap();
    }

    #[test]
    fn a_missing_authority_is_an_error_not_an_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("devd-ca.crt");
        std::fs::write(&empty, "").unwrap();
        assert!(tls_config(&empty).is_err());
    }
}
