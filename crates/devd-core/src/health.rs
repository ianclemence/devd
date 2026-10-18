//! Health checks.
//!
//! A service is only "up" when something answers. `devd` treats a health check
//! as part of a service definition so the supervisor, the GUI and the agent all
//! share one definition of healthy.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{DevdError, Result};

/// How to decide that something is alive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Check {
    /// Connect to a TCP port.
    Tcp {
        /// Host to connect to.
        #[serde(default = "default_host")]
        host: String,
        /// Port to connect to.
        port: u16,
    },
    /// Issue an HTTP GET and compare the status code.
    Http {
        /// Full URL.
        url: String,
        /// Acceptable status code; `None` accepts any 2xx/3xx.
        #[serde(default)]
        expect: Option<u16>,
    },
    /// Only check that the supervised process is still running.
    Process,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// The result of evaluating a check.
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    /// Whether the check passed.
    pub healthy: bool,
    /// Human-readable explanation.
    pub detail: String,
    /// How long the check took.
    pub latency_ms: u128,
}

impl Health {
    fn ok(detail: impl Into<String>, latency_ms: u128) -> Self {
        Self {
            healthy: true,
            detail: detail.into(),
            latency_ms,
        }
    }
    fn bad(detail: impl Into<String>, latency_ms: u128) -> Self {
        Self {
            healthy: false,
            detail: detail.into(),
            latency_ms,
        }
    }
}

/// Evaluate a check once.
pub fn evaluate(check: &Check) -> Health {
    match check {
        Check::Process => Health::ok("process supervision only", 0),
        Check::Tcp { host, port } => {
            let start = Instant::now();
            match tcp_connect(host, *port, Duration::from_millis(1500)) {
                Ok(()) => Health::ok(format!("tcp {host}:{port} answered"), start.elapsed().as_millis()),
                Err(e) => Health::bad(format!("tcp {host}:{port}: {e}"), start.elapsed().as_millis()),
            }
        }
        Check::Http { url, expect } => {
            let start = Instant::now();
            match http_status(url, Duration::from_millis(2500)) {
                Ok(status) => {
                    let good = match expect {
                        Some(want) => status == *want,
                        None => (200..400).contains(&status),
                    };
                    if good {
                        Health::ok(format!("HTTP {status} from {url}"), start.elapsed().as_millis())
                    } else {
                        Health::bad(format!("HTTP {status} (unexpected) from {url}"), start.elapsed().as_millis())
                    }
                }
                Err(e) => Health::bad(format!("{url}: {e}"), start.elapsed().as_millis()),
            }
        }
    }
}

/// Poll a check until it passes or the deadline expires.
pub fn wait_until_healthy(check: &Check, timeout: Duration, interval: Duration) -> Health {
    let deadline = Instant::now() + timeout;
    let mut last = evaluate(check);
    while !last.healthy && Instant::now() < deadline {
        std::thread::sleep(interval);
        last = evaluate(check);
    }
    last
}

/// Attempt a TCP connection with a timeout.
pub fn tcp_connect(host: &str, port: u16, timeout: Duration) -> Result<()> {
    let addrs = (host, port)
        .to_socket_addrs()
        .map_err(|e| DevdError::invalid(format!("cannot resolve {host}:{port}: {e}")))?;
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(DevdError::Io(last_err.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "no addresses")
    })))
}

/// A minimal HTTP/1.1 GET that returns the status code.
///
/// Implemented directly on a socket so `devd-core` needs no HTTP client.
pub fn http_status(url: &str, timeout: Duration) -> Result<u16> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| DevdError::invalid(format!("unsupported scheme in `{url}`")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rfind(':') {
        Some(i) => (
            &authority[..i],
            authority[i + 1..].parse::<u16>().unwrap_or(80),
        ),
        None => (authority, 80u16),
    };

    let mut stream = TcpStream::connect_timeout(
        &(host, port)
            .to_socket_addrs()
            .map_err(|e| DevdError::invalid(format!("cannot resolve {host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| DevdError::invalid("no address resolved"))?,
        timeout,
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: devd-health/0.1\r\n\r\n"
    );
    stream.write_all(request.as_bytes())?;

    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf)?;
    let head = String::from_utf8_lossy(&buf[..n]);
    let status_line = head.lines().next().unwrap_or_default();
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| DevdError::invalid(format!("malformed HTTP response: {status_line:?}")))?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn tcp_check_detects_a_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = evaluate(&Check::Tcp {
            host: "127.0.0.1".into(),
            port,
        });
        assert!(h.healthy, "{}", h.detail);
    }

    #[test]
    fn tcp_check_reports_closed_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let h = evaluate(&Check::Tcp {
            host: "127.0.0.1".into(),
            port,
        });
        assert!(!h.healthy);
    }

    #[test]
    fn http_status_reads_a_real_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            }
        });
        let status = http_status(&format!("http://127.0.0.1:{port}/"), Duration::from_secs(3)).unwrap();
        assert_eq!(status, 204);
    }

    #[test]
    fn process_check_is_always_healthy() {
        assert!(evaluate(&Check::Process).healthy);
    }
}
