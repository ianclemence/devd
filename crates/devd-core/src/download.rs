//! Verified artifact downloads.
//!
//! Everything `devd` installs - PHP, nginx, MariaDB, Node, a Go toolchain - is
//! fetched from its official upstream distribution and **checked before it is
//! used**. That is the whole point of this module, and it is why `devd` can
//! install third-party software without bundling it:
//!
//! * The URL and the expected SHA-256 both come from the upstream's own
//!   catalogue (for PHP, `windows.php.net/.../releases.json` publishes the
//!   digest of every build), not from a hard-coded list in this repository.
//! * A download that does not match its published digest is deleted, and the
//!   install fails. A silent mismatch is the one failure mode that would turn
//!   "download on first run" into a supply-chain hole.
//! * The body is streamed to a `.part` file and renamed only after the digest
//!   matches, so an interrupted download can never be mistaken for a complete
//!   one - which matters because the cache is reused on the next attempt.
//!
//! Downloads run through `rustls`, in-process. There is no shell, no external
//! `curl`, and no certificate-verification escape hatch.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{DevdError, Result};
use crate::util;

/// A remote artifact described by its upstream catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// Absolute HTTPS URL.
    pub url: String,
    /// File name to store the download under.
    pub filename: String,
    /// Expected SHA-256 of the body, lowercase hex, when upstream publishes one.
    pub sha256: Option<String>,
    /// Expected size in bytes, when upstream publishes one.
    pub bytes: Option<u64>,
}

impl Artifact {
    /// Describe an artifact, taking the file name from the URL.
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        let filename = url
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("download.bin")
            .to_string();
        Self {
            url,
            filename,
            sha256: None,
            bytes: None,
        }
    }

    /// Attach the published digest.
    pub fn sha256(mut self, digest: impl Into<String>) -> Self {
        let digest = digest.into().to_lowercase();
        self.sha256 = if digest.is_empty() {
            None
        } else {
            Some(digest)
        };
        self
    }

    /// Attach the published size.
    pub fn bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    /// Override the local file name.
    pub fn filename(mut self, name: impl Into<String>) -> Self {
        self.filename = name.into();
        self
    }
}

/// A progress report, delivered at most a few times a second.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    /// Bytes written so far.
    pub received: u64,
    /// Expected total, when the server declared one.
    pub total: Option<u64>,
}

impl Progress {
    /// Completion ratio in the range 0.0-1.0, when the total is known.
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|t| *t > 0)
            .map(|t| (self.received as f64 / t as f64).clamp(0.0, 1.0))
    }
}

/// A progress callback. Returning `false` cancels the download.
pub type ProgressFn<'a> = dyn Fn(&Progress) -> bool + Send + Sync + 'a;

/// Upper bound on a single download. Nothing `devd` installs is close to this;
/// the limit exists so a wrong URL that streams forever fails instead of
/// filling the disk.
const MAX_DOWNLOAD: u64 = 4 * 1024 * 1024 * 1024;

/// How long to wait for the connection, and for a single read, before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Result of a completed download.
#[derive(Debug, Clone, Serialize)]
pub struct Fetched {
    /// Path of the verified file on disk.
    pub path: PathBuf,
    /// SHA-256 of what was written.
    pub sha256: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Whether the file was served from the local cache rather than the network.
    pub from_cache: bool,
}

/// How many times a download that broke off is resumed before giving up.
const MAX_RESUMES: u32 = 3;

/// A partial download older than this is not a download in progress; it is
/// removed rather than resumed against whatever the server serves now.
const PART_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// Why one attempt to transfer bytes ended.
enum Attempt {
    /// The bytes arrived. Carries the total received and the running digest.
    Done(u64, String),
    /// The connection broke or the stream was cut short. The partial file stays
    /// so the next attempt can continue from it.
    Broken(DevdError),
    /// Retrying will not help: a refusal, a limit, a cancellation.
    Fatal(DevdError),
}

/// Fetch an artifact into `cache_dir`, verifying its published digest.
///
/// A cached file whose digest matches is reused, so a repeated install does not
/// re-download a 32 MB PHP build. Anything that does not match is discarded and
/// re-fetched rather than trusted.
///
/// A download that breaks off is resumed from where it stopped, with a `Range`
/// request, up to three times; a later call resumes it too. The bytes already on
/// disk are re-hashed before more are appended, and the whole file is checked
/// against the published digest at the end, so a server that changed the file in
/// between yields a mismatch and a fresh start, never a corrupt install.
pub fn fetch(
    artifact: &Artifact,
    cache_dir: &Path,
    progress: Option<&ProgressFn<'_>>,
) -> Result<Fetched> {
    fetch_with(artifact, cache_dir, progress, false)
}

pub(crate) fn fetch_with(
    artifact: &Artifact,
    cache_dir: &Path,
    progress: Option<&ProgressFn<'_>>,
    allow_http: bool,
) -> Result<Fetched> {
    let secure = artifact.url.starts_with("https://");
    if !secure && !(allow_http && artifact.url.starts_with("http://")) {
        return Err(DevdError::invalid(format!(
            "refusing to download over an insecure scheme: {}",
            artifact.url
        )));
    }
    std::fs::create_dir_all(cache_dir)?;
    let target = cache_dir.join(&artifact.filename);

    // A cached copy that matches its published digest is exactly as trustworthy
    // as a fresh download, and costs no bandwidth.
    if target.is_file() {
        let sha = util::sha256_file(&target)?;
        if Some(&sha) == artifact.sha256.as_ref() {
            return Ok(Fetched {
                bytes: std::fs::metadata(&target)?.len(),
                path: target,
                sha256: sha,
                from_cache: true,
            });
        }
        // Present but not what upstream published: it is not evidence of
        // anything, so remove it rather than leave it to be picked up later.
        let _ = std::fs::remove_file(&target);
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .timeout_write(READ_TIMEOUT)
        .user_agent(&format!("devd/{}", env!("CARGO_PKG_VERSION")))
        .build();

    let part = cache_dir.join(format!("{}.part", artifact.filename));
    if let Ok(meta) = std::fs::metadata(&part) {
        let age = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .unwrap_or_default();
        if age > PART_MAX_AGE {
            let _ = std::fs::remove_file(&part);
        }
    }

    let mut last: Option<DevdError> = None;
    let mut outcome = None;
    for _ in 0..=MAX_RESUMES {
        match transfer(artifact, &agent, &part, progress) {
            Attempt::Done(received, sha256) => {
                outcome = Some((received, sha256));
                break;
            }
            Attempt::Broken(e) => last = Some(e),
            Attempt::Fatal(e) => return Err(e),
        }
    }
    let Some((received, sha256)) = outcome else {
        return Err(last.unwrap_or_else(|| {
            DevdError::Io(std::io::Error::other("the download kept breaking off"))
        }));
    };

    if let Some(expected) = &artifact.sha256 {
        if !expected.eq_ignore_ascii_case(&sha256) {
            let _ = std::fs::remove_file(&part);
            return Err(DevdError::invalid(format!(
                "digest mismatch for {}: upstream publishes {expected}, the download hashed to {sha256}; \
                 the file was deleted and nothing was installed",
                artifact.filename
            )));
        }
    }
    if let Some(expected) = artifact.bytes {
        // Published sizes are rounded to a decimal ("32.3MB" for a 33873909
        // byte file), so they cannot be compared exactly. They still catch a
        // truncated or truncated-and-padded download cheaply, which matters for
        // the builds that publish no digest at all.
        let tolerance = expected / 20; // 5%
        let delta = expected.abs_diff(received);
        if delta > tolerance {
            let _ = std::fs::remove_file(&part);
            return Err(DevdError::invalid(format!(
                "size mismatch for {}: upstream publishes about {expected} bytes, the download has \
                 {received}; the file was deleted and nothing was installed",
                artifact.filename
            )));
        }
    }

    // Only now does the file get its real name, so a half-written `.part` can
    // never be picked up as a cache hit.
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }
    std::fs::rename(&part, &target)?;

    Ok(Fetched {
        path: target,
        sha256,
        bytes: received,
        from_cache: false,
    })
}

/// One attempt: continue `part` if it has bytes, otherwise start it.
fn transfer(
    artifact: &Artifact,
    agent: &ureq::Agent,
    part: &Path,
    progress: Option<&ProgressFn<'_>>,
) -> Attempt {
    let existing = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut request = agent.get(&artifact.url);
    if existing > 0 {
        request = request.set("Range", &format!("bytes={existing}-"));
    }
    let response = match request.call() {
        Ok(r) => r,
        // The partial file does not fit what the server has now (a different
        // file, or already complete and mislabelled): begin again from nothing.
        Err(ureq::Error::Status(416, _)) => {
            let _ = std::fs::remove_file(part);
            return Attempt::Broken(DevdError::Io(std::io::Error::other(
                "the partial download no longer matches the server; starting again",
            )));
        }
        Err(ureq::Error::Status(code, _)) => {
            return Attempt::Fatal(DevdError::Command {
                program: artifact.url.clone(),
                code: code as i32,
                stderr: format!("the server refused the download with HTTP {code}"),
            })
        }
        Err(other) => {
            return Attempt::Broken(DevdError::Io(std::io::Error::other(format!(
                "failed to reach {}: {other}",
                artifact.url
            ))))
        }
    };

    // 206 continues the file; 200 means the server ignored the range and is
    // sending all of it, so what was on disk is discarded.
    let resumed = response.status() == 206
        && response
            .header("Content-Range")
            .and_then(|v| v.trim().strip_prefix("bytes "))
            .and_then(|v| v.split('-').next())
            .and_then(|v| v.trim().parse::<u64>().ok())
            == Some(existing);
    let start = if resumed { existing } else { 0 };

    let declared_body: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    let declared_total = declared_body.map(|b| b + start);
    if let Some(total) = declared_total {
        if total > MAX_DOWNLOAD {
            return Attempt::Fatal(DevdError::invalid(format!(
                "{} is {} bytes, which exceeds the {} byte download limit",
                artifact.url, total, MAX_DOWNLOAD
            )));
        }
    }

    let mut hasher = Sha256::new();
    let mut file = if resumed {
        // Hash what is already there; the digest must cover the whole file.
        match File::open(part) {
            Ok(mut old) => {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match old.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buf[..n]),
                        Err(e) => return Attempt::Fatal(e.into()),
                    }
                }
            }
            Err(e) => return Attempt::Fatal(e.into()),
        }
        match std::fs::OpenOptions::new().append(true).open(part) {
            Ok(f) => f,
            Err(e) => return Attempt::Fatal(e.into()),
        }
    } else {
        match File::create(part) {
            Ok(f) => f,
            Err(e) => return Attempt::Fatal(e.into()),
        }
    };

    let mut reader = response.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = start;
    let mut last_report = std::time::Instant::now();

    loop {
        let read = match reader.read(&mut buf) {
            Ok(n) => n,
            Err(e) => return Attempt::Broken(e.into()),
        };
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > MAX_DOWNLOAD {
            drop(file);
            let _ = std::fs::remove_file(part);
            return Attempt::Fatal(DevdError::invalid(format!(
                "{} exceeded the {} byte download limit",
                artifact.url, MAX_DOWNLOAD
            )));
        }
        hasher.update(&buf[..read]);
        if let Err(e) = file.write_all(&buf[..read]) {
            return Attempt::Fatal(e.into());
        }

        if let Some(report) = progress {
            let now = std::time::Instant::now();
            // Report at most every 100 ms: a progress callback that runs per
            // chunk of a 30 MB file is thousands of UI updates for no benefit.
            if now.duration_since(last_report) >= Duration::from_millis(100) {
                last_report = now;
                if !report(&Progress {
                    received,
                    total: declared_total,
                }) {
                    // What arrived is kept: starting the same download again
                    // continues from it.
                    return Attempt::Fatal(DevdError::denied(format!(
                        "the download of {} was cancelled",
                        artifact.filename
                    )));
                }
            }
        }
    }

    if let Err(e) = file.sync_all() {
        return Attempt::Fatal(e.into());
    }
    drop(file);

    // A stream that ended early is a broken connection, not a short file.
    if let Some(total) = declared_total {
        if received < total {
            return Attempt::Broken(DevdError::Io(std::io::Error::other(format!(
                "the connection closed after {received} of {total} bytes"
            ))));
        }
    }
    Attempt::Done(received, hex::encode(hasher.finalize()))
}

/// What a cache pass removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evicted {
    /// Files removed.
    pub files: usize,
    /// Bytes they held.
    pub bytes: u64,
}

/// Extensions of what `fetch` leaves in the cache. Nothing else is touched:
/// the directory also holds the search index and scratch files that are not
/// downloads and are not this function's to delete.
const DOWNLOADED: &[&str] = &[
    "zip", "7z", "gz", "tgz", "xz", "bz2", "tar", "exe", "msi", "bin", "deb", "rpm", "dmg", "pkg",
    "part",
];

/// Keep the download cache within `max_bytes`, and drop what nobody has used
/// for `max_age`.
///
/// A cached download is only a saved transfer, so deleting one costs bandwidth
/// and nothing else. Partial downloads go first once they are stale; then
/// anything past `max_age`; then the least recently used until the total fits.
pub fn evict(cache_dir: &Path, max_bytes: u64, max_age: Duration) -> Result<Evicted> {
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    for entry in std::fs::read_dir(cache_dir)?.flatten() {
        let path = entry.path();
        let downloaded = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| DOWNLOADED.contains(&e.to_ascii_lowercase().as_str()));
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || !downloaded {
            continue;
        }
        let used = meta
            .accessed()
            .or_else(|_| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        // A file that was only just written has been "used" by the writer even
        // when the filesystem does not record access times.
        let used = used.max(meta.modified().unwrap_or(std::time::UNIX_EPOCH));
        files.push((path, meta.len(), used));
    }
    let mut removed = Evicted::default();
    let mut drop_file = |path: &Path, len: u64| {
        if std::fs::remove_file(path).is_ok() {
            removed.files += 1;
            removed.bytes += len;
            true
        } else {
            false
        }
    };

    let now = std::time::SystemTime::now();
    files.retain(|(path, len, used)| {
        let age = now.duration_since(*used).unwrap_or_default();
        let part = path.extension().is_some_and(|e| e == "part");
        let limit = if part { PART_MAX_AGE } else { max_age };
        !(age > limit && drop_file(path, *len))
    });

    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.2);
    for (path, len, _) in &files {
        if total <= max_bytes {
            break;
        }
        if drop_file(path, *len) {
            total = total.saturating_sub(*len);
        }
    }
    Ok(removed)
}

/// Fetch a small JSON document - a version catalogue - and parse it.
pub fn fetch_json<T: for<'de> Deserialize<'de>>(url: &str, timeout: Duration) -> Result<T> {
    let body = fetch_text(url, timeout)?;
    serde_json::from_str(&body)
        .map_err(|e| DevdError::invalid(format!("{url} did not return the expected JSON: {e}")))
}

/// Fetch a small text document - a `SHASUMS256.txt`, a checksum list - as a
/// string. Used for the digests that upstreams publish outside their JSON index.
pub fn fetch_text(url: &str, timeout: Duration) -> Result<String> {
    if !url.starts_with("https://") {
        return Err(DevdError::invalid(format!(
            "refusing to fetch metadata over an insecure scheme: {url}"
        )));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .user_agent(&format!("devd/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => DevdError::Command {
            program: url.to_string(),
            code: code as i32,
            stderr: format!("the server refused the request with HTTP {code}"),
        },
        other => DevdError::Io(std::io::Error::other(format!(
            "failed to reach {url}: {other}"
        ))),
    })?;
    response
        .into_string()
        .map_err(|e| DevdError::Io(std::io::Error::other(format!("reading {url}: {e}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_https_is_refused_before_any_network_use() {
        let artifact = Artifact::new("http://example.invalid/php.zip");
        let err = fetch(&artifact, Path::new("D:/tmp/devd-download-test"), None).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
        assert!(err.to_string().contains("insecure"), "{err}");
    }

    /// A server that serves `body`, honours `Range`, and can drop the connection
    /// after `cut` bytes on its first answer.
    fn range_server(body: Vec<u8>, cut_first: Option<usize>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://127.0.0.1:{}/file.zip", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let mut ranges = Vec::new();
            let mut served = 0;
            listener.set_nonblocking(false).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            while std::time::Instant::now() < deadline {
                let (mut conn, _) = match listener.accept() {
                    Ok(c) => c,
                    Err(_) => break,
                };
                let mut raw = vec![0u8; 4096];
                let n = conn.read(&mut raw).unwrap_or(0);
                let head = String::from_utf8_lossy(&raw[..n]).to_string();
                let range = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("range: bytes=").map(|r| r.trim_end_matches('-').to_string()))
                    .and_then(|r| r.parse::<usize>().ok());
                ranges.push(range.map(|r| r.to_string()).unwrap_or_else(|| "none".into()));
                let (status, from) = match range {
                    Some(r) => ("206 Partial Content", r),
                    None => ("200 OK", 0),
                };
                let slice = &body[from..];
                let mut header = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", slice.len());
                if range.is_some() {
                    header.push_str(&format!("content-range: bytes {from}-{}/{}\r\n", body.len() - 1, body.len()));
                }
                header.push_str("\r\n");
                let _ = conn.write_all(header.as_bytes());
                if let (0, Some(cut)) = (served, cut_first) {
                    let _ = conn.write_all(&slice[..cut.min(slice.len())]);
                    // Close early: a connection that dies mid-body.
                } else {
                    let _ = conn.write_all(slice);
                    served += 1;
                    let _ = conn.flush();
                    if range.is_some() || cut_first.is_none() {
                        break;
                    }
                }
                served += 1;
            }
            ranges
        });
        (url, handle)
    }

    #[test]
    fn a_download_cut_off_midway_is_resumed_not_restarted_and_still_verified() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let digest = hex::encode(Sha256::digest(&body));
        let (url, server) = range_server(body.clone(), Some(60_000));
        let dir = tempfile::tempdir().unwrap();
        let artifact = Artifact::new(url).sha256(digest.clone());
        let fetched = fetch_with(&artifact, dir.path(), None, true).unwrap();
        let ranges = server.join().unwrap();

        assert_eq!(fetched.sha256, digest);
        assert_eq!(std::fs::read(&fetched.path).unwrap(), body);
        // The first request asked for everything; the second continued.
        assert_eq!(ranges.first().map(String::as_str), Some("none"), "{ranges:?}");
        assert!(ranges.iter().skip(1).any(|r| r != "none"), "resumed with a Range: {ranges:?}");
        assert!(!dir.path().join("file.zip.part").exists());
    }

    #[test]
    fn a_partial_file_that_does_not_hash_to_the_digest_is_deleted_not_installed() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 7) as u8).collect();
        let dir = tempfile::tempdir().unwrap();
        // A stale partial from a different file.
        std::fs::write(dir.path().join("file.zip.part"), vec![9u8; 10_000]).unwrap();
        let (url, server) = range_server(body.clone(), None);
        let artifact = Artifact::new(url).sha256(hex::encode(Sha256::digest(&body)));
        let err = fetch_with(&artifact, dir.path(), None, true).unwrap_err();
        server.join().unwrap();
        assert!(err.to_string().contains("digest mismatch"), "{err}");
        assert!(!dir.path().join("file.zip").exists(), "nothing was installed");
        assert!(!dir.path().join("file.zip.part").exists(), "the bad partial is gone");
    }

    #[test]
    fn eviction_removes_old_and_surplus_downloads_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, len: usize, age_days: u64| {
            let path = dir.path().join(name);
            std::fs::write(&path, vec![1u8; len]).unwrap();
            let when = std::time::SystemTime::now() - Duration::from_secs(age_days * 86_400);
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_times(std::fs::FileTimes::new().set_accessed(when).set_modified(when)).unwrap();
        };
        write("old.zip", 100, 90);
        write("mid.zip", 100, 5);
        write("new.zip", 100, 1);
        write("search.db", 5_000, 400);
        write("notes.txt", 5_000, 400);
        std::fs::create_dir_all(dir.path().join("skills-src-x")).unwrap();

        // Too old goes first.
        let gone = evict(dir.path(), u64::MAX, Duration::from_secs(30 * 86_400)).unwrap();
        assert_eq!((gone.files, gone.bytes), (1, 100));
        assert!(!dir.path().join("old.zip").exists());

        // Then least recently used, until the total fits.
        let gone = evict(dir.path(), 100, Duration::from_secs(365 * 86_400)).unwrap();
        assert_eq!(gone.files, 1);
        assert!(!dir.path().join("mid.zip").exists());
        assert!(dir.path().join("new.zip").exists());

        // The index, scratch files and directories are not downloads.
        assert!(dir.path().join("search.db").exists());
        assert!(dir.path().join("notes.txt").exists());
        assert!(dir.path().join("skills-src-x").is_dir());
    }

    #[test]
    fn filename_comes_from_the_url() {
        let a = Artifact::new(
            "https://windows.php.net/downloads/releases/php-8.3.35-nts-Win32-vs16-x64.zip",
        );
        assert_eq!(a.filename, "php-8.3.35-nts-Win32-vs16-x64.zip");
        assert_eq!(a.sha256, None);
    }

    #[test]
    fn digest_is_normalised_and_empty_becomes_none() {
        let a = Artifact::new("https://example.invalid/x.zip").sha256("ABCDEF");
        assert_eq!(a.sha256.as_deref(), Some("abcdef"));
        let b = Artifact::new("https://example.invalid/x.zip").sha256("");
        assert_eq!(b.sha256, None);
    }

    #[test]
    fn progress_fraction_is_bounded() {
        let p = Progress {
            received: 50,
            total: Some(100),
        };
        assert_eq!(p.fraction(), Some(0.5));
        let unknown = Progress {
            received: 50,
            total: None,
        };
        assert_eq!(unknown.fraction(), None);
    }

    #[test]
    fn json_helper_refuses_plain_http() {
        let err = fetch_json::<serde_json::Value>("http://example.invalid/x.json", READ_TIMEOUT)
            .unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }
}
