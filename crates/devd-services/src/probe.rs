//! Is the service answering in its own language, not merely holding its port?
//!
//! A TCP connect proves that something accepted a connection. It does not prove
//! that the something is the service, that it finished starting, or that it can
//! do its job: a database accepts connections for seconds before it can serve a
//! query, a web server whose configuration failed to load can still hold the
//! socket, and an unrelated program on the port connects just as well. Each
//! service here is asked the smallest question that only the real thing answers
//! correctly, using the protocol it speaks:
//!
//! * web servers and Mailpit: an HTTP request, and any HTTP status line back;
//! * MariaDB and MySQL: the server's initial handshake packet;
//! * PHP FastCGI: a `FCGI_GET_VALUES` management record;
//! * Redis: `PING`; memcached: `version`; PostgreSQL: an `SSLRequest`.
//!
//! A service with no known probe is reported by the TCP check alone, and the
//! result says so, so "ready" is never claimed on evidence it does not have.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use serde::Serialize;

/// What a probe found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Probe {
    /// Whether the service answered correctly.
    pub ready: bool,
    /// How it was asked: `http`, `mysql`, `fastcgi`, `redis`, `memcached`,
    /// `postgres` or `tcp` (only the connection was checked).
    pub via: &'static str,
    /// One sentence on what was seen.
    pub detail: String,
}

/// The protocol to speak to a service, from its kind or name.
pub fn protocol_of(kind: &str, name: &str) -> &'static str {
    let key = if kind.is_empty() { name } else { kind };
    match key {
        "nginx" | "apache" | "mailpit" => "http",
        "mariadb" | "mysql" => "mysql",
        "redis" => "redis",
        "memcached" => "memcached",
        "postgres" | "postgresql" => "postgres",
        k if k.starts_with("php") => "fastcgi",
        _ if name.starts_with("php-cgi") => "fastcgi",
        _ => "tcp",
    }
}

/// Ask the service on `port` whether it is ready.
pub fn probe(kind: &str, name: &str, port: u16, timeout: Duration) -> Probe {
    probe_protocol(protocol_of(kind, name), port, timeout)
}

/// Ask using a named protocol.
pub fn probe_protocol(protocol: &'static str, port: u16, timeout: Duration) -> Probe {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => {
            return Probe {
                ready: false,
                via: if protocol == "tcp" { "tcp" } else { protocol },
                detail: format!("nothing accepted a connection on port {port}: {e}"),
            }
        }
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    match protocol {
        "http" => http(&mut stream, port),
        "mysql" => mysql(&mut stream),
        "fastcgi" => fastcgi(&mut stream),
        "redis" => redis(&mut stream),
        "memcached" => memcached(&mut stream),
        "postgres" => postgres(&mut stream),
        _ => Probe {
            ready: true,
            via: "tcp",
            detail: format!("port {port} accepts connections (no protocol check for this service)"),
        },
    }
}

fn read_some(stream: &mut TcpStream, max: usize) -> Vec<u8> {
    let mut buf = vec![0u8; max];
    let n = stream.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    buf
}

fn http(stream: &mut TcpStream, port: u16) -> Probe {
    let request =
        format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nUser-Agent: devd-probe\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return not_ready("http", "the connection closed before a request could be sent");
    }
    let reply = read_some(stream, 256);
    let line = String::from_utf8_lossy(&reply);
    let first = line.lines().next().unwrap_or("").trim();
    if first.starts_with("HTTP/") {
        Probe {
            ready: true,
            via: "http",
            detail: format!("answered `{first}`"),
        }
    } else if reply.is_empty() {
        not_ready("http", "it accepted the connection and sent nothing back")
    } else {
        not_ready("http", "it answered, but not in HTTP")
    }
}

fn mysql(stream: &mut TcpStream) -> Probe {
    // The server speaks first: a 4-byte packet header, then the protocol version.
    let reply = read_some(stream, 64);
    match reply.get(4) {
        Some(0x0a) => Probe {
            ready: true,
            via: "mysql",
            detail: "sent its handshake".to_string(),
        },
        // An error packet instead of a handshake: the server is up and refusing.
        Some(0xff) => not_ready(
            "mysql",
            "it refused the connection with an error packet (too many connections, or this host is blocked)",
        ),
        _ => not_ready("mysql", "it accepted the connection but sent no handshake"),
    }
}

fn fastcgi(stream: &mut TcpStream) -> Probe {
    // FCGI_GET_VALUES (type 9), request id 0, asking for one value by name. php-cgi
    // ignores a management record with no content, so the question is real:
    // `FCGI_MPXS_CONNS` (does the server multiplex), 15 bytes, padded to 8.
    const NAME: &[u8] = b"FCGI_MPXS_CONNS";
    let content_length = 2 + NAME.len(); // name length, value length, name
    let padding = (8 - content_length % 8) % 8;
    let mut record = vec![1u8, 9, 0, 0, 0, content_length as u8, padding as u8, 0];
    record.push(NAME.len() as u8);
    record.push(0);
    record.extend_from_slice(NAME);
    record.extend(std::iter::repeat_n(0u8, padding));
    if stream.write_all(&record).is_err() {
        return not_ready("fastcgi", "the connection closed before a record could be sent");
    }
    let reply = read_some(stream, 64);
    // FCGI_GET_VALUES_RESULT (type 10), version 1.
    if reply.len() >= 8 && reply[0] == 1 && reply[1] == 10 {
        Probe {
            ready: true,
            via: "fastcgi",
            detail: "answered a FastCGI management request".to_string(),
        }
    } else if reply.is_empty() {
        not_ready("fastcgi", "it accepted the connection and sent nothing back")
    } else {
        not_ready("fastcgi", "it answered, but not in FastCGI")
    }
}

fn redis(stream: &mut TcpStream) -> Probe {
    if stream.write_all(b"PING\r\n").is_err() {
        return not_ready("redis", "the connection closed before PING could be sent");
    }
    let reply = String::from_utf8_lossy(&read_some(stream, 64)).to_string();
    if reply.starts_with("+PONG") {
        Probe {
            ready: true,
            via: "redis",
            detail: "answered PING".to_string(),
        }
    } else if reply.starts_with("-NOAUTH") || reply.starts_with("-DENIED") {
        // It is alive and protecting itself, which is not a failure to start.
        Probe {
            ready: true,
            via: "redis",
            detail: "is up and requires authentication".to_string(),
        }
    } else if reply.starts_with("-LOADING") {
        not_ready("redis", "is still loading its data set")
    } else {
        not_ready("redis", "did not answer PING")
    }
}

fn memcached(stream: &mut TcpStream) -> Probe {
    if stream.write_all(b"version\r\n").is_err() {
        return not_ready("memcached", "the connection closed before a request could be sent");
    }
    let reply = String::from_utf8_lossy(&read_some(stream, 64)).to_string();
    if reply.starts_with("VERSION") {
        Probe {
            ready: true,
            via: "memcached",
            detail: format!("answered `{}`", reply.trim()),
        }
    } else {
        not_ready("memcached", "did not answer `version`")
    }
}

fn postgres(stream: &mut TcpStream) -> Probe {
    // SSLRequest: length 8, code 80877103. A live server answers `S` or `N`.
    let request = [0u8, 0, 0, 8, 4, 210, 22, 47];
    if stream.write_all(&request).is_err() {
        return not_ready("postgres", "the connection closed before a request could be sent");
    }
    match read_some(stream, 1).first() {
        Some(b'S') | Some(b'N') => Probe {
            ready: true,
            via: "postgres",
            detail: "answered the startup negotiation".to_string(),
        },
        _ => not_ready("postgres", "did not answer the startup negotiation"),
    }
}

fn not_ready(via: &'static str, detail: &str) -> Probe {
    Probe {
        ready: false,
        via,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A server that, for each connection, runs `serve` on the stream.
    fn server(serve: impl Fn(TcpStream) + Send + 'static) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream);
            }
        });
        port
    }

    const T: Duration = Duration::from_secs(2);

    #[test]
    fn a_web_server_must_answer_in_http_not_just_accept() {
        let ok = server(|mut s| {
            let _ = read_some(&mut s, 512);
            let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
        });
        let p = probe_protocol("http", ok, T);
        assert!(p.ready && p.via == "http", "{p:?}");
        assert!(p.detail.contains("404"), "any status is an answer: {p:?}");

        let mute = server(|mut s| {
            let _ = read_some(&mut s, 512);
        });
        assert!(!probe_protocol("http", mute, T).ready);

        let wrong = server(|mut s| {
            let _ = read_some(&mut s, 512);
            let _ = s.write_all(b"+PONG\r\n");
        });
        let p = probe_protocol("http", wrong, T);
        assert!(!p.ready && p.detail.contains("not in HTTP"), "{p:?}");
    }

    #[test]
    fn a_database_is_ready_when_it_sends_its_handshake_and_not_when_it_sends_an_error() {
        let ok = server(|mut s| {
            let _ = s.write_all(&[0x4a, 0, 0, 0, 0x0a, b'1', b'1', b'.', b'4', 0]);
        });
        assert!(probe_protocol("mysql", ok, T).ready);

        let refusing = server(|mut s| {
            let _ = s.write_all(&[0x10, 0, 0, 0, 0xff, 0x10, 0x04, b'T']);
        });
        let p = probe_protocol("mysql", refusing, T);
        assert!(!p.ready && p.detail.contains("error packet"), "{p:?}");

        // Accepting and saying nothing is exactly the starting-up state.
        let silent = server(|s| {
            std::thread::sleep(Duration::from_millis(300));
            drop(s);
        });
        assert!(!probe_protocol("mysql", silent, Duration::from_millis(150)).ready);
    }

    #[test]
    fn php_fastcgi_answers_a_management_record() {
        let ok = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(&[1, 10, 0, 0, 0, 0, 0, 0]);
        });
        assert!(probe_protocol("fastcgi", ok, T).ready);
        let http_server = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
        });
        assert!(!probe_protocol("fastcgi", http_server, T).ready);
    }

    #[test]
    fn redis_memcached_and_postgres_are_asked_in_their_own_words() {
        let redis_ok = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(b"+PONG\r\n");
        });
        assert!(probe_protocol("redis", redis_ok, T).ready);

        let redis_auth = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(b"-NOAUTH Authentication required.\r\n");
        });
        let p = probe_protocol("redis", redis_auth, T);
        assert!(p.ready && p.detail.contains("authentication"), "{p:?}");

        let redis_loading = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(b"-LOADING Redis is loading the dataset in memory\r\n");
        });
        assert!(!probe_protocol("redis", redis_loading, T).ready);

        let memcached = server(|mut s| {
            let _ = read_some(&mut s, 64);
            let _ = s.write_all(b"VERSION 1.6.21\r\n");
        });
        assert!(probe_protocol("memcached", memcached, T).ready);

        let postgres = server(|mut s| {
            let mut buf = [0u8; 8];
            let _ = s.read_exact(&mut buf);
            assert_eq!(buf, [0, 0, 0, 8, 4, 210, 22, 47]);
            let _ = s.write_all(b"N");
        });
        assert!(probe_protocol("postgres", postgres, T).ready);
    }

    #[test]
    fn a_closed_port_is_not_ready_and_an_unknown_service_is_only_checked_for_a_connection() {
        let port = {
            let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        let p = probe_protocol("http", port, Duration::from_millis(300));
        assert!(!p.ready && p.detail.contains("nothing accepted"), "{p:?}");

        let open = server(|_| {});
        let p = probe("mongodb", "mongodb", open, T);
        assert!(p.ready && p.via == "tcp", "{p:?}");
        assert!(p.detail.contains("no protocol check"));
    }

    #[test]
    fn a_service_is_matched_to_its_protocol_by_kind_then_name() {
        assert_eq!(protocol_of("nginx", "nginx"), "http");
        assert_eq!(protocol_of("apache", "apache"), "http");
        assert_eq!(protocol_of("mariadb", "mariadb"), "mysql");
        assert_eq!(protocol_of("redis", "redis"), "redis");
        assert_eq!(protocol_of("", "php-cgi-8.4"), "fastcgi");
        assert_eq!(protocol_of("php-cgi", "php-cgi-8.4"), "fastcgi");
        assert_eq!(protocol_of("mongodb", "mongodb"), "tcp");
    }

    /// Read-only check against real services: `DEVD_PROBE_LIVE=http:80,mysql:3306,fastcgi:9000`.
    /// Each probe sends one harmless request (a GET, a PING, a handshake read).
    #[test]
    #[ignore = "needs real services; set DEVD_PROBE_LIVE"]
    fn real_services_pass_their_own_probes() {
        let Ok(list) = std::env::var("DEVD_PROBE_LIVE") else {
            return;
        };
        for item in list.split(',') {
            let (protocol, port) = item.split_once(':').expect("protocol:port");
            let protocol: &'static str = match protocol {
                "http" => "http",
                "mysql" => "mysql",
                "fastcgi" => "fastcgi",
                "redis" => "redis",
                "memcached" => "memcached",
                "postgres" => "postgres",
                _ => "tcp",
            };
            let p = probe_protocol(protocol, port.parse().unwrap(), Duration::from_secs(3));
            println!("LIVE {protocol}:{port} -> {p:?}");
            assert!(p.ready, "{protocol} on {port}: {p:?}");
        }
    }
}
