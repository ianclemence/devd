//! A small DNS server for local names, so a new site resolves without a hosts
//! file edit.
//!
//! The hosts file works everywhere and needs an administrator to change, once
//! per site, and has no wildcard. A resolver that answers for a few suffixes
//! (`.test` by default) removes all three: every `anything.test` is this
//! machine, instantly, for every program.
//!
//! This module is the server and the check of whether the operating system
//! actually asks it. They are different questions and are reported as such: the
//! server can run perfectly while the system's resolver never consults it. The
//! step that points the system at it ([`enable_commands`]) needs privileges and
//! differs by platform; where it cannot be done the hosts file stays the way
//! names resolve, and nothing here pretends otherwise.
//!
//! Only `A` and `AAAA` for the configured suffixes are answered (with loopback);
//! everything else gets `NXDOMAIN` or `REFUSED`. It never forwards a query: it
//! is not a resolver for the internet and cannot be used as one.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use devd_core::{DevdError, Result};

/// The default suffix answered.
pub const DEFAULT_SUFFIX: &str = "test";
/// The default port: unprivileged, so the server starts without an administrator.
pub const DEFAULT_PORT: u16 = 5354;

const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;
const CLASS_IN: u16 = 1;

/// What a query asked, when it is a well-formed standard query for one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The query id, to echo.
    pub id: u16,
    /// The name, lower-cased, without the trailing dot.
    pub name: String,
    /// Record type.
    pub qtype: u16,
    /// Record class.
    pub qclass: u16,
    /// The question section exactly as received, to echo.
    raw: Vec<u8>,
}

/// Read the single question from a query packet.
pub fn parse_query(packet: &[u8]) -> Option<Question> {
    if packet.len() < 12 {
        return None;
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    // A response, or anything but a standard query, is not for us.
    if flags & 0x8000 != 0 || (flags >> 11) & 0x0f != 0 {
        return None;
    }
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount != 1 {
        return None;
    }
    let mut pos = 12;
    let mut labels: Vec<String> = Vec::new();
    loop {
        let len = *packet.get(pos)? as usize;
        pos += 1;
        if len == 0 {
            break;
        }
        // A compression pointer in a question is not something a client sends.
        if len & 0xc0 != 0 || len > 63 {
            return None;
        }
        let label = packet.get(pos..pos + len)?;
        labels.push(String::from_utf8_lossy(label).to_ascii_lowercase());
        pos += len;
        if labels.iter().map(|l| l.len() + 1).sum::<usize>() > 255 {
            return None;
        }
    }
    let tail = packet.get(pos..pos + 4)?;
    Some(Question {
        id: u16::from_be_bytes([packet[0], packet[1]]),
        name: labels.join("."),
        qtype: u16::from_be_bytes([tail[0], tail[1]]),
        qclass: u16::from_be_bytes([tail[2], tail[3]]),
        raw: packet[12..pos + 4].to_vec(),
    })
}

/// Whether `name` is under one of the suffixes (or is the suffix itself).
pub fn is_ours(name: &str, suffixes: &[String]) -> bool {
    suffixes.iter().any(|s| {
        let s = s.trim_matches('.').to_ascii_lowercase();
        !s.is_empty() && (name == s || name.ends_with(&format!(".{s}")))
    })
}

/// The reply to a query.
pub fn answer(question: &Question, suffixes: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&question.id.to_be_bytes());
    let ours = is_ours(&question.name, suffixes) && question.qclass == CLASS_IN;
    let supported = matches!(question.qtype, TYPE_A | TYPE_AAAA);
    // RD copied as set; RA clear (this is not a recursive resolver); AA set for
    // names that are ours. rcode: 0 ok, 3 name error, 5 refused.
    let (rcode, answers): (u16, u16) = if !ours {
        (5, 0)
    } else if supported {
        (0, 1)
    } else {
        // The name exists, the type does not: NOERROR with no data.
        (0, 0)
    };
    let flags: u16 = 0x8000 | 0x0100 | if ours { 0x0400 } else { 0 } | rcode;
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&answers.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&question.raw);
    if answers == 1 {
        // A pointer to the name in the question section, then the record.
        out.extend_from_slice(&[0xc0, 0x0c]);
        out.extend_from_slice(&question.qtype.to_be_bytes());
        out.extend_from_slice(&CLASS_IN.to_be_bytes());
        out.extend_from_slice(&60u32.to_be_bytes());
        if question.qtype == TYPE_A {
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
        } else {
            out.extend_from_slice(&16u16.to_be_bytes());
            out.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        }
    }
    out
}

/// A running server. Dropping it stops it.
pub struct Server {
    /// The address it listens on.
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Listen on loopback `port` (`0` picks one) and answer for `suffixes`.
pub fn serve(port: u16, suffixes: Vec<String>) -> Result<Server> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).map_err(|e| {
        DevdError::conflict(format!(
            "cannot listen for DNS on 127.0.0.1:{port}: {e}; something else may hold the port"
        ))
    })?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    let addr = socket.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while !flag.load(Ordering::SeqCst) {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if let Some(question) = parse_query(&buf[..n]) {
                let _ = socket.send_to(&answer(&question, &suffixes), from);
            }
        }
    });
    Ok(Server { addr, stop })
}

static RUNNING: std::sync::Mutex<Option<Server>> = std::sync::Mutex::new(None);

/// The suffixes devd answers for, from `dns.suffixes` (comma separated).
pub fn suffixes_of(ctx: &devd_core::Ctx) -> Vec<String> {
    let configured = ctx.store.get_setting("dns.suffixes").ok().flatten().unwrap_or_default();
    let list: Vec<String> = configured
        .split(',')
        .map(|s| s.trim().trim_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        vec![DEFAULT_SUFFIX.to_string()]
    } else {
        list
    }
}

/// The port the server uses, from `dns.port`.
pub fn port_of(ctx: &devd_core::Ctx) -> u16 {
    ctx.store
        .get_setting("dns.port")
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Start the process-wide server if `dns.enabled` is not `false` and it is not
/// already running. Returns where it listens, or why it could not start.
pub fn ensure_running(ctx: &devd_core::Ctx) -> Result<Option<SocketAddr>> {
    let disabled = ctx
        .store
        .get_setting("dns.enabled")
        .ok()
        .flatten()
        .is_some_and(|v| v.trim() == "false");
    let mut slot = RUNNING.lock().map_err(|_| DevdError::internal("the DNS server lock was poisoned"))?;
    if disabled {
        *slot = None;
        return Ok(None);
    }
    if let Some(server) = slot.as_ref() {
        return Ok(Some(server.addr));
    }
    let server = serve(port_of(ctx), suffixes_of(ctx))?;
    let addr = server.addr;
    *slot = Some(server);
    Ok(Some(addr))
}

/// Where the process-wide server listens, when it is running.
pub fn running_addr() -> Option<SocketAddr> {
    RUNNING.lock().ok().and_then(|s| s.as_ref().map(|s| s.addr))
}

/// Ask a DNS server directly for the A record of `name`. Used to check that the
/// server answers, which is a separate fact from the system using it.
pub fn query_a(server: SocketAddr, name: &str, timeout: Duration) -> Result<Option<Ipv4Addr>> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    socket.set_read_timeout(Some(timeout))?;
    let mut packet = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.trim_end_matches('.').split('.') {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&TYPE_A.to_be_bytes());
    packet.extend_from_slice(&CLASS_IN.to_be_bytes());
    socket.send_to(&packet, server)?;
    let mut buf = [0u8; 512];
    let (n, _) = socket
        .recv_from(&mut buf)
        .map_err(|e| DevdError::Timeout(format!("the DNS server did not answer: {e}")))?;
    let reply = &buf[..n];
    if reply.len() < 12 || reply[3] & 0x0f != 0 {
        return Ok(None);
    }
    let answers = u16::from_be_bytes([reply[6], reply[7]]);
    if answers == 0 || reply.len() < 4 {
        return Ok(None);
    }
    let rdata = &reply[reply.len() - 4..];
    Ok(Some(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])))
}

/// What a person's operating system does with a local name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Whether devd's own server answered a direct query.
    pub server_answers: bool,
    /// Whether the operating system resolves a name under the suffix to loopback.
    pub system_resolves: bool,
    /// What that means, in a sentence.
    pub summary: String,
}

/// Check both halves: the server directly, and the system resolver.
pub fn status(server: Option<SocketAddr>, suffix: &str) -> Status {
    let probe = format!("devd-dns-check.{suffix}");
    let server_answers = server
        .and_then(|s| query_a(s, &probe, Duration::from_secs(2)).ok().flatten())
        .is_some_and(|ip| ip == Ipv4Addr::LOCALHOST);
    let system_resolves = std::net::ToSocketAddrs::to_socket_addrs(&(probe.as_str(), 80))
        .map(|mut it| it.any(|a| a.ip().is_loopback()))
        .unwrap_or(false);
    let summary = match (server_answers, system_resolves) {
        (true, true) => format!("any name under .{suffix} resolves to this machine, with no hosts entry"),
        (true, false) => format!("devd's DNS server answers, but this system does not ask it for .{suffix} names; run `devd call dns.enable` (it needs administrator rights), or keep using the hosts file"),
        (false, true) => format!(".{suffix} names resolve, but not through devd's server (a wildcard in your own resolver, perhaps)"),
        (false, false) => format!("devd's DNS server is not running, and .{suffix} names do not resolve; sites are reached through the hosts file"),
    };
    Status {
        server_answers,
        system_resolves,
        summary,
    }
}

/// The commands that point this system at the server, for the current platform.
///
/// Returned rather than hidden because they change how the whole machine
/// resolves names and need an administrator: a person should be able to read
/// exactly what will run. `None` means this platform cannot be pointed at a
/// non-standard port, and the reason says what to do instead.
pub fn enable_commands(suffix: &str, port: u16) -> std::result::Result<Vec<Vec<String>>, String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if cfg!(target_os = "macos") {
        // /etc/resolver/<suffix> is read by the system resolver and takes a port.
        return Ok(vec![
            s(&["mkdir", "-p", "/etc/resolver"]),
            vec![
                "sh".into(),
                "-c".into(),
                format!("printf 'nameserver 127.0.0.1\\nport {port}\\n' > /etc/resolver/{suffix}"),
            ],
        ]);
    }
    if cfg!(target_os = "linux") {
        // systemd-resolved can route one domain to a server on any port.
        return Ok(vec![
            vec![
                "resolvectl".into(),
                "dns".into(),
                "lo".into(),
                format!("127.0.0.1:{port}"),
            ],
            vec!["resolvectl".into(), "domain".into(), "lo".into(), format!("~{suffix}")],
        ]);
    }
    if port != 53 {
        return Err(format!(
            "Windows can only send a name suffix to a DNS server on port 53, and devd's runs on {port}. \
             Run `devd call dns.enable` with the port set to 53 if nothing holds port 53 (Windows' Internet Connection Sharing and Hyper-V do), \
             otherwise keep using the hosts file"
        ));
    }
    Ok(vec![vec![
        "powershell.exe".into(),
        "-NoProfile".into(),
        "-Command".into(),
        format!("Add-DnsClientNrptRule -Namespace '.{suffix}' -NameServers '127.0.0.1'"),
    ]])
}

/// The commands that undo [`enable_commands`].
pub fn disable_commands(suffix: &str) -> Vec<Vec<String>> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if cfg!(target_os = "macos") {
        return vec![s(&["rm", "-f", &format!("/etc/resolver/{suffix}")])];
    }
    if cfg!(target_os = "linux") {
        return vec![s(&["resolvectl", "revert", "lo"])];
    }
    vec![vec![
        "powershell.exe".into(),
        "-NoProfile".into(),
        "-Command".into(),
        format!("Get-DnsClientNrptRule | Where-Object {{ $_.Namespace -eq '.{suffix}' }} | Remove-DnsClientNrptRule -Force"),
    ]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffixes() -> Vec<String> {
        vec!["test".to_string()]
    }

    fn query(name: &str, qtype: u16) -> Vec<u8> {
        let mut packet = vec![0xab, 0xcd, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            packet.push(label.len() as u8);
            packet.extend_from_slice(label.as_bytes());
        }
        packet.push(0);
        packet.extend_from_slice(&qtype.to_be_bytes());
        packet.extend_from_slice(&CLASS_IN.to_be_bytes());
        packet
    }

    #[test]
    fn a_name_under_the_suffix_answers_with_loopback_and_anything_else_is_refused() {
        let q = parse_query(&query("Blog.TEST", TYPE_A)).unwrap();
        assert_eq!(q.name, "blog.test");
        let reply = answer(&q, &suffixes());
        assert_eq!(&reply[..2], &[0xab, 0xcd], "the id is echoed");
        assert_eq!(reply[3] & 0x0f, 0, "NOERROR");
        assert_eq!(u16::from_be_bytes([reply[6], reply[7]]), 1);
        assert_eq!(&reply[reply.len() - 4..], &[127, 0, 0, 1]);

        let v6 = answer(&parse_query(&query("blog.test", TYPE_AAAA)).unwrap(), &suffixes());
        assert_eq!(&v6[v6.len() - 16..], &Ipv6Addr::LOCALHOST.octets());

        let foreign = answer(&parse_query(&query("example.com", TYPE_A)).unwrap(), &suffixes());
        assert_eq!(foreign[3] & 0x0f, 5, "REFUSED: this is not a resolver for the internet");
        assert_eq!(u16::from_be_bytes([foreign[6], foreign[7]]), 0);

        let mx = answer(&parse_query(&query("blog.test", 15)).unwrap(), &suffixes());
        assert_eq!(mx[3] & 0x0f, 0, "the name exists, the type does not");
        assert_eq!(u16::from_be_bytes([mx[6], mx[7]]), 0);
    }

    #[test]
    fn the_suffix_matches_whole_labels_only() {
        let s = suffixes();
        assert!(is_ours("a.b.test", &s));
        assert!(is_ours("test", &s));
        assert!(!is_ours("attest", &s));
        assert!(!is_ours("test.example.com", &s));
        assert!(!is_ours("anything", &[]));
    }

    #[test]
    fn malformed_and_non_query_packets_are_ignored() {
        assert!(parse_query(&[]).is_none());
        assert!(parse_query(&[0; 11]).is_none());
        let mut response = query("a.test", TYPE_A);
        response[2] |= 0x80;
        assert!(parse_query(&response).is_none(), "a response is not a query");
        let mut inverse = query("a.test", TYPE_A);
        inverse[2] |= 0x08;
        assert!(parse_query(&inverse).is_none(), "only standard queries");
        let mut truncated = query("a.test", TYPE_A);
        truncated.truncate(15);
        assert!(parse_query(&truncated).is_none());
        let mut pointer = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xc0, 0x0c, 0, 0, 1, 0, 1];
        assert!(parse_query(&pointer).is_none(), "a compression pointer in a question");
        pointer.clear();
    }

    #[test]
    fn a_real_socket_gets_a_real_answer_and_foreign_names_get_none() {
        let server = serve(0, suffixes()).unwrap();
        let ip = query_a(server.addr, "anything.at.all.test", Duration::from_secs(2)).unwrap();
        assert_eq!(ip, Some(Ipv4Addr::LOCALHOST));
        assert_eq!(query_a(server.addr, "example.com", Duration::from_secs(2)).unwrap(), None);
        let status = status(Some(server.addr), "test");
        assert!(status.server_answers);
    }

    #[test]
    fn the_status_tells_the_server_from_the_system_apart() {
        let none = status(None, "test");
        assert!(!none.server_answers);
        assert!(none.summary.contains("hosts file") || none.summary.contains("resolve"), "{}", none.summary);
    }

    #[test]
    fn enabling_names_what_will_run_and_refuses_what_the_platform_cannot_do() {
        let commands = enable_commands("test", 53);
        assert!(commands.is_ok());
        assert!(!disable_commands("test").is_empty());
        if cfg!(windows) {
            let err = enable_commands("test", 5354).unwrap_err();
            assert!(err.contains("port 53"), "{err}");
            let ok = enable_commands("test", 53).unwrap();
            assert!(ok[0].join(" ").contains("Add-DnsClientNrptRule"));
        } else {
            assert!(enable_commands("test", 5354).is_ok());
        }
    }

    #[test]
    fn a_port_that_is_taken_is_reported_not_swallowed() {
        let first = serve(0, suffixes()).unwrap();
        let err = serve(first.addr.port(), suffixes());
        assert!(err.is_err());
    }
}
