//! Port inspection.
//!
//! Knowing who holds port 80 is the difference between a useful error message
//! and a mysterious failure, so `devd` parses the platform's own listener table
//! rather than guessing.

use std::net::TcpListener;

use serde::Serialize;

use crate::error::Result;
use crate::util;

/// One listening socket.
#[derive(Debug, Clone, Serialize)]
pub struct Listener {
    /// Local port.
    pub port: u16,
    /// `tcp` or `tcp6`.
    pub protocol: String,
    /// Local address as reported by the platform.
    pub address: String,
    /// Owning process id, when the platform reports it.
    pub pid: Option<u32>,
    /// Owning program name, when known.
    pub program: Option<String>,
}

/// Whether anything is already listening on `port`.
///
/// Two sources are consulted, because neither is complete on its own:
///
/// 1. The operating system's own listener table, which catches a socket bound to
///    a *specific* address. On Windows a wildcard bind can succeed even while
///    `127.0.0.1:<port>` is in use, so testing by binding alone is not enough.
/// 2. An actual bind attempt, which catches listeners the table does not show -
///    for example another user's socket on Linux, which `ss` hides unless run
///    with elevated privileges.
///
/// The answer can always race with another process; a caller that must win the
/// race should hold the bound socket rather than re-check.
pub fn is_free(port: u16) -> bool {
    if matches!(find_listener(port), Ok(Some(_))) {
        return false;
    }
    // Where the listener table is unavailable (no `ss` in a minimal container)
    // or a wildcard bind succeeds beside a loopback listener, an actual
    // connection attempt is the honest test.
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok() {
        return false;
    }
    can_bind(port)
}

/// Whether a wildcard listener can currently be created on `port`.
pub fn can_bind(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok() || TcpListener::bind(("::", port)).is_ok()
}

/// Every listening TCP socket the platform will report.
pub fn listeners() -> Result<Vec<Listener>> {
    #[cfg(windows)]
    {
        listeners_windows()
    }
    #[cfg(not(windows))]
    {
        listeners_linux()
    }
}

#[cfg(windows)]
fn listeners_windows() -> Result<Vec<Listener>> {
    let Some(netstat) = util::which("netstat") else {
        return Ok(Vec::new());
    };
    let out = util::run(
        &netstat,
        &["-ano".to_string()],
        None,
        &[],
        Some(std::time::Duration::from_secs(20)),
    )?;
    let mut seen = Vec::new();
    for line in out.stdout.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || cols[0] != "TCP" {
            continue;
        }
        if !cols[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        let Some(port) = port_of(cols[1]) else {
            continue;
        };
        let pid = cols[4].parse::<u32>().ok();
        seen.push(Listener {
            port,
            protocol: "tcp".to_string(),
            address: cols[1].to_string(),
            pid,
            // Resolved in one pass below: asking `tasklist` once per socket
            // turned a one-second check into a forty-second one on a machine
            // with a few dozen listeners.
            program: None,
        });
    }

    let pids: Vec<u32> = seen.iter().filter_map(|l| l.pid).collect();
    let names = process_names(&pids);
    for listener in &mut seen {
        listener.program = listener.pid.and_then(|pid| names.get(&pid).cloned());
    }
    Ok(seen)
}

/// Resolve process image names for many pids without starting a process per
/// socket. `tasklist` takes over a second on a busy machine; the kernel answers
/// each query in microseconds.
#[cfg(windows)]
fn process_names(pids: &[u32]) -> std::collections::HashMap<u32, String> {
    use std::collections::HashMap;

    let mut map = HashMap::new();
    for pid in pids {
        if let Some(stem) = crate::process::process_image(*pid) {
            map.insert(*pid, format!("{stem}.exe"));
        }
    }
    map
}

#[cfg(not(windows))]
fn listeners_linux() -> Result<Vec<Listener>> {
    let Some(ss) = util::which("ss") else {
        return Ok(listeners_proc());
    };
    let out = util::run(
        &ss,
        &["-ltnp".to_string()],
        None,
        &[],
        Some(std::time::Duration::from_secs(20)),
    )?;
    let mut seen = Vec::new();
    for line in out.stdout.lines() {
        if !line.trim_start().starts_with("LISTEN") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        let Some(port) = port_of(cols[3]) else {
            continue;
        };
        let pid = extract_pid(line);
        seen.push(Listener {
            port,
            protocol: if cols[3].contains("::") {
                "tcp6"
            } else {
                "tcp"
            }
            .to_string(),
            address: cols[3].to_string(),
            pid,
            program: pid.and_then(program_name),
        });
    }
    Ok(seen)
}

/// Listening TCP sockets from the kernel's own table, for hosts without `ss`.
/// Process ids are not available this way, so `pid` and `program` are `None`.
#[cfg(not(windows))]
fn listeners_proc() -> Vec<Listener> {
    let mut out = Vec::new();
    for (file, proto) in [("/proc/net/tcp", "tcp"), ("/proc/net/tcp6", "tcp6")] {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            // local_address is col 1 (ADDR:PORT in hex); st is col 3, 0A = LISTEN.
            if cols.len() < 4 || cols[3] != "0A" {
                continue;
            }
            let Some((addr, port_hex)) = cols[1].rsplit_once(':') else { continue };
            let Ok(port) = u16::from_str_radix(port_hex, 16) else { continue };
            out.push(Listener {
                port,
                protocol: proto.to_string(),
                address: format!("{addr}:{port}"),
                pid: None,
                program: None,
            });
        }
    }
    out
}

fn port_of(addr: &str) -> Option<u16> {
    let idx = addr.rfind(':')?;
    addr[idx + 1..].trim().parse().ok()
}

fn extract_pid(line: &str) -> Option<u32> {
    let idx = line.find("pid=")?;
    let rest = &line[idx + 4..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Best-effort program name for a process id.
pub fn program_name(pid: u32) -> Option<String> {
    if cfg!(windows) {
        let tasklist = util::which("tasklist")?;
        let args = vec![
            "/FI".to_string(),
            format!("PID eq {pid}"),
            "/FO".to_string(),
            "CSV".to_string(),
            "/NH".to_string(),
        ];
        let out = util::run(
            &tasklist,
            &args,
            None,
            &[],
            Some(std::time::Duration::from_secs(10)),
        )
        .ok()?;
        let first = out.stdout.lines().next()?;
        let name = first.split(',').next()?.trim_matches('"').to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    } else {
        let path = format!("/proc/{pid}/comm");
        std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
    }
}

/// Find the listener holding `port`, if any.
pub fn find_listener(port: u16) -> Result<Option<Listener>> {
    Ok(listeners()?.into_iter().find(|l| l.port == port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ports_from_addresses() {
        assert_eq!(port_of("0.0.0.0:80"), Some(80));
        assert_eq!(port_of("[::]:443"), Some(443));
        assert_eq!(port_of("127.0.0.1:5432"), Some(5432));
        assert_eq!(port_of("garbage"), None);
    }

    #[test]
    fn extracted_pid_from_ss_line() {
        let line = r#"LISTEN 0 511 0.0.0.0:80 0.0.0.0:* users:(("nginx",pid=1234,fd=6))"#;
        assert_eq!(extract_pid(line), Some(1234));
    }

    #[test]
    fn free_port_is_reported_free() {
        // Bind an ephemeral port, learn it, drop the listener, then check.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(is_free(port));
    }

    #[test]
    fn occupied_port_is_reported_busy() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!is_free(port));
    }

    #[test]
    fn listing_listeners_does_not_error() {
        // On an unprivileged or unusual environment this may be empty, but it
        // must never fail outright.
        let _ = listeners().unwrap();
    }
}
