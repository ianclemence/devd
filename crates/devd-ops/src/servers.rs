//! Servers you can reach over SSH: DigitalOcean, Hetzner, Vultr, a Ploi-managed machine, a Raspberry Pi.
//!
//! A server is a name, an address, a user, a port and a key from `~/.ssh`. devd talks to it with the system's OpenSSH
//! client, one non-interactive command at a time, and pins the server's host key in its own known-hosts file (never
//! yours): the first `server.connect` records the key and shows its fingerprint, every later connection refuses a
//! server whose key has changed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use devd_core::capability::Ctx;
use devd_core::store_ops::OpsItem;
use devd_core::{util, DevdError, Result};

/// The kind under which servers are stored.
pub const KIND: &str = "server";

/// Names a provider label may take. It is only a label, for the person looking at the list.
pub const PROVIDERS: &[&str] = &["digitalocean", "hetzner", "vultr", "linode", "aws", "ploi", "forge", "other"];

/// A server, as the capabilities show it.
#[derive(Debug, Clone, Serialize)]
pub struct Server {
    /// Name, unique.
    pub name: String,
    /// Address or hostname.
    pub host: String,
    /// SSH user.
    pub user: String,
    /// SSH port.
    pub port: u16,
    /// The key in `~/.ssh` it is reached with.
    pub key: String,
    /// A label such as `hetzner`.
    pub provider: String,
    /// Whether its host key has been pinned.
    pub host_key_pinned: bool,
    /// The pinned key's fingerprint.
    pub fingerprint: Option<String>,
    /// When it last answered.
    pub last_ok_at: Option<String>,
}

/// A name for a server or a deploy target: letters, digits, `-`, `_`, `.`, starting with a letter or digit.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 48 && name.starts_with(|c: char| c.is_ascii_alphanumeric()) && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// A host: a name or an address, nothing a shell or an option parser could read as more.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && !host.starts_with('-') && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:_".contains(c))
}

/// An SSH user name.
pub fn valid_user(user: &str) -> bool {
    !user.is_empty() && user.len() <= 32 && !user.starts_with('-') && user.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// Quote a string for a POSIX shell: inside single quotes, with each quote closed, escaped and reopened.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn of(item: &OpsItem) -> Server {
    let d = &item.data;
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    Server {
        name: item.id.clone(),
        host: s("host"),
        user: s("user"),
        port: d["port"].as_u64().unwrap_or(22) as u16,
        key: s("key"),
        provider: s("provider"),
        host_key_pinned: d["fingerprint"].as_str().is_some(),
        fingerprint: d["fingerprint"].as_str().map(str::to_string),
        last_ok_at: d["last_ok_at"].as_str().map(str::to_string),
    }
}

/// The servers.
pub fn list(ctx: &Ctx) -> Result<Vec<Server>> {
    Ok(ctx.store.ops_list(KIND, None)?.iter().map(of).collect())
}

/// One server.
pub fn get(ctx: &Ctx, name: &str) -> Result<Server> {
    ctx.store.ops_get(KIND, name)?.map(|i| of(&i)).ok_or_else(|| DevdError::not_found(format!("no server named `{name}`; `devd call server.list` shows them")))
}

/// Register a server. Nothing is contacted until `server.connect`.
pub fn add(ctx: &Ctx, name: &str, host: &str, user: &str, port: Option<u16>, key: &str, provider: Option<&str>) -> Result<Server> {
    if !valid_name(name) {
        return Err(DevdError::invalid("a server name is letters, digits, `-`, `_` and `.`, up to 48 characters"));
    }
    if !valid_host(host) {
        return Err(DevdError::invalid("the host is a name or an address (letters, digits, `.`, `-`, `:`)"));
    }
    if !valid_user(user) {
        return Err(DevdError::invalid("the user is letters, digits, `-`, `_` and `.`"));
    }
    if port == Some(0) {
        return Err(DevdError::invalid("the port is 1 to 65535"));
    }
    if !crate::ssh::valid_name(key) {
        return Err(DevdError::invalid("the key is the name of a key in ~/.ssh, e.g. `id_ed25519`"));
    }
    let provider = provider.unwrap_or("other");
    if !PROVIDERS.contains(&provider) {
        return Err(DevdError::invalid(format!("the provider label is one of {}", PROVIDERS.join(", "))));
    }
    if ctx.store.ops_get(KIND, name)?.is_some() {
        return Err(DevdError::conflict(format!("a server named `{name}` already exists")));
    }
    ctx.store.ops_put(KIND, name, "", &json!({ "host": host, "user": user, "port": port.unwrap_or(22), "key": key, "provider": provider }))?;
    get(ctx, name)
}

/// Forget a server and the links that deploy to it. The machine itself is not touched.
pub fn remove(ctx: &Ctx, name: &str) -> Result<usize> {
    get(ctx, name)?;
    let mut links = 0;
    for l in ctx.store.ops_list(crate::remote::KIND, None)? {
        if l.data["server"].as_str() == Some(name) {
            ctx.store.ops_delete(crate::remote::KIND, &l.id)?;
            links += 1;
        }
    }
    ctx.store.ops_delete(KIND, name)?;
    Ok(links)
}

/// devd's own known-hosts file.
pub fn known_hosts(ctx: &Ctx) -> PathBuf {
    ctx.paths.root().join("secrets").join("ssh").join("known_hosts")
}

/// The ssh program: `DEVD_SSH` when set (tests), the Windows OpenSSH client, Git's, or whatever `ssh` is on PATH.
pub fn ssh_program() -> PathBuf {
    if let Some(p) = std::env::var_os("DEVD_SSH").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    for c in ["C:\\Windows\\System32\\OpenSSH\\ssh.exe", "C:\\Program Files\\Git\\usr\\bin\\ssh.exe", "/usr/bin/ssh"] {
        if Path::new(c).is_file() {
            return PathBuf::from(c);
        }
    }
    PathBuf::from("ssh")
}

fn slashy(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// The arguments that make ssh non-interactive and pin the host key. `first_contact` records an unknown key; otherwise a
/// key that is unknown or changed is refused.
pub fn ssh_args(ctx: &Ctx, s: &Server, first_contact: bool) -> Result<Vec<String>> {
    let key = crate::ssh::dir()?.join(&s.key);
    if !key.is_file() {
        return Err(DevdError::not_found(format!("the key `{}` is not in ~/.ssh; `ssh.keygen` makes one and `ssh.adopt` takes one in", s.key)));
    }
    let kh = known_hosts(ctx);
    if let Some(dir) = kh.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(vec![
        "-p".into(),
        s.port.to_string(),
        "-i".into(),
        slashy(&key),
        "-o".into(),
        "IdentitiesOnly=yes".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("StrictHostKeyChecking={}", if first_contact { "accept-new" } else { "yes" }),
        "-o".into(),
        format!("UserKnownHostsFile={}", slashy(&kh)),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        format!("{}@{}", s.user, s.host),
    ])
}

/// What a remote command produced.
#[derive(Debug, Clone, Serialize)]
pub struct Remote {
    /// Exit code (255 is ssh's own failure).
    pub code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Seconds it took.
    pub seconds: f64,
}

/// Run one command on a server. `stdin` is a file streamed to it (an archive). Waits up to `timeout`.
pub fn run(ctx: &Ctx, s: &Server, command: &str, stdin: Option<&Path>, timeout: Duration, first_contact: bool) -> Result<Remote> {
    if command.contains('\0') || command.len() > 200_000 {
        return Err(DevdError::invalid("a remote command is text without NUL characters, up to 200 000 bytes"));
    }
    let mut args = ssh_args(ctx, s, first_contact)?;
    args.push(command.to_string());
    let mut cmd = Command::new(ssh_program());
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    match stdin {
        Some(f) => {
            cmd.stdin(std::fs::File::open(f)?);
        }
        None => {
            cmd.stdin(Stdio::null());
        }
    }
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| DevdError::unsupported(format!("could not start ssh ({}): {e}; install the OpenSSH client", ssh_program().display())))?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut b);
        }
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(o) = err.as_mut() {
            let _ = o.read_to_end(&mut b);
        }
        b
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break -2;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(DevdError::internal(format!("waiting for ssh: {e}"))),
        }
    };
    let cut = |b: Vec<u8>| {
        let s = String::from_utf8_lossy(&b).to_string();
        if s.len() > 400_000 {
            format!("...{}", &s[s.len() - 400_000..])
        } else {
            s
        }
    };
    let stdout = cut(t_out.join().unwrap_or_default());
    let stderr = cut(t_err.join().unwrap_or_default());
    if status == -2 {
        return Err(DevdError::unsupported(format!("the command on {} did not finish within {} seconds and was stopped", s.name, timeout.as_secs())));
    }
    Ok(Remote { code: status, stdout, stderr, seconds: started.elapsed().as_secs_f64() })
}

/// The fingerprint of a server's pinned host key.
fn fingerprint(ctx: &Ctx, s: &Server) -> Option<String> {
    let keygen = if cfg!(windows) { "ssh-keygen.exe" } else { "ssh-keygen" };
    let candidates = [Path::new("C:\\Windows\\System32\\OpenSSH").join(keygen), Path::new("C:\\Program Files\\Git\\usr\\bin").join(keygen), PathBuf::from("/usr/bin/ssh-keygen")];
    let program = candidates.into_iter().find(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(keygen));
    let host = if s.port == 22 { s.host.clone() } else { format!("[{}]:{}", s.host, s.port) };
    let out = util::run(&program, &["-lf".into(), slashy(&known_hosts(ctx)), "-F".into(), host], None, &[], Some(Duration::from_secs(15))).ok()?;
    out.stdout.lines().find(|l| !l.starts_with('#') && l.contains("SHA256")).map(|l| l.trim().to_string())
}

/// Connect for the first time (or again): record the host key if it is new, refuse it if it changed, and report what the
/// server is. Returns the server with its fingerprint and what it answered.
pub fn connect(ctx: &Ctx, name: &str) -> Result<Value> {
    let s = get(ctx, name)?;
    let first = !s.host_key_pinned;
    let r = run(ctx, &s, "echo devd-ok; uname -sr; (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\") || true; id -un", None, Duration::from_secs(45), first)?;
    if r.code != 0 || !r.stdout.contains("devd-ok") {
        let why = r.stderr.trim();
        let hint = if why.contains("Host key verification failed") || why.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
            " The server's host key is not the one devd pinned. If you rebuilt the server that is expected: remove it with `server.remove` and add it again."
        } else if why.contains("Permission denied") {
            " The server did not accept the key. Add the public key (`ssh.public`) to the user's ~/.ssh/authorized_keys on the server."
        } else {
            ""
        };
        return Err(DevdError::unsupported(format!("could not connect to {}@{}: {}{hint}", s.user, s.host, if why.is_empty() { format!("ssh exited {}", r.code) } else { why.to_string() })));
    }
    let print = fingerprint(ctx, &s);
    let mut item = ctx.store.ops_get(KIND, name)?.ok_or_else(|| DevdError::internal("the server vanished"))?;
    if let Some(p) = &print {
        item.data["fingerprint"] = json!(p);
    }
    item.data["last_ok_at"] = json!(util::now());
    ctx.store.ops_put(KIND, name, "", &item.data)?;
    let lines: Vec<&str> = r.stdout.lines().collect();
    Ok(json!({ "server": get(ctx, name)?, "first_contact": first, "kernel": lines.get(1), "os": lines.get(2), "login_user": lines.get(3), "seconds": r.seconds }))
}

/// A read-only look at a server: what is installed and how full it is.
pub fn info(ctx: &Ctx, name: &str) -> Result<Value> {
    let s = get(ctx, name)?;
    let script = "echo \"os=$( . /etc/os-release 2>/dev/null && echo $PRETTY_NAME )\"; echo \"uptime=$(uptime -p 2>/dev/null)\"; \
        echo \"load=$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)\"; echo \"memory=$(free -m 2>/dev/null | awk '/Mem:/ {print $3\"/\"$2\" MB\"}')\"; \
        echo \"disk=$(df -h / 2>/dev/null | awk 'NR==2 {print $3\"/\"$2\" (\"$5\")\"}')\"; \
        for t in nginx php node composer mysql mariadb redis-server supervisord certbot git ufw; do v=$(command -v $t >/dev/null 2>&1 && echo yes || echo no); echo \"has_$t=$v\"; done; \
        echo \"php_version=$(php -r 'echo PHP_VERSION;' 2>/dev/null)\"; echo \"node_version=$(node -v 2>/dev/null)\"; echo \"nginx_version=$(nginx -v 2>&1 | cut -d/ -f2)\"; \
        echo \"sudo=$(sudo -n true 2>/dev/null && echo passwordless || echo no)\"";
    let r = run(ctx, &s, script, None, Duration::from_secs(60), false)?;
    if r.code != 0 {
        return Err(DevdError::unsupported(format!("could not read {}: {}", s.name, r.stderr.trim())));
    }
    let mut v = serde_json::Map::new();
    for l in r.stdout.lines() {
        if let Some((k, val)) = l.split_once('=') {
            v.insert(k.to_string(), json!(val.trim()));
        }
    }
    Ok(json!({ "server": s.name, "host": s.host, "info": v }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_hosts_and_users_cannot_carry_options_or_shell_syntax() {
        assert!(valid_name("hetzner-prod") && valid_name("do.1") && !valid_name("-x") && !valid_name("a b") && !valid_name(""));
        assert!(valid_host("203.0.113.7") && valid_host("web1.example.com") && valid_host("2001:db8::1") && !valid_host("-oProxyCommand=x") && !valid_host("a;b") && !valid_host("a b"));
        assert!(valid_user("deploy") && valid_user("ploi") && !valid_user("-x") && !valid_user("a b") && !valid_user("root;id"));
    }

    #[test]
    fn shell_quoting_survives_quotes_spaces_and_dollars() {
        assert_eq!(shq("plain"), "'plain'");
        assert_eq!(shq("it's"), "'it'\\''s'");
        assert_eq!(shq("a b $HOME `x`"), "'a b $HOME `x`'");
        assert_eq!(shq(""), "''");
    }

    #[test]
    fn the_ssh_arguments_pin_the_host_key_and_never_ask_a_question() {
        let d = tempfile::tempdir().unwrap();
        let ctx = crate::tests_support::ctx(d.path());
        let home = d.path().join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh").join("k1"), "key").unwrap();
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
        let s = Server { name: "a".into(), host: "203.0.113.7".into(), user: "deploy".into(), port: 2222, key: "k1".into(), provider: "hetzner".into(), host_key_pinned: false, fingerprint: None, last_ok_at: None };
        let first = ssh_args(&ctx, &s, true).unwrap().join(" ");
        assert!(first.contains("StrictHostKeyChecking=accept-new") && first.contains("BatchMode=yes") && first.contains("-p 2222") && first.ends_with("deploy@203.0.113.7"), "{first}");
        assert!(first.contains("IdentitiesOnly=yes") && first.contains("secrets/ssh/known_hosts"), "{first}");
        let later = ssh_args(&ctx, &s, false).unwrap().join(" ");
        assert!(later.contains("StrictHostKeyChecking=yes"), "{later}");
        let missing = Server { key: "nokey".into(), ..s };
        assert!(ssh_args(&ctx, &missing, false).unwrap_err().to_string().contains("not in ~/.ssh"));
    }
}
