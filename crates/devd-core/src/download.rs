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

/// Fetch an artifact into `cache_dir`, verifying its published digest.
///
/// A cached file whose digest matches is reused, so a repeated install does not
/// re-download a 32 MB PHP build. Anything that does not match is discarded and
/// re-fetched rather than trusted.
pub fn fetch(
    artifact: &Artifact,
    cache_dir: &Path,
    progress: Option<&ProgressFn<'_>>,
) -> Result<Fetched> {
    if !artifact.url.starts_with("https://") {
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

    let response = agent.get(&artifact.url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => DevdError::Command {
            program: artifact.url.clone(),
            code: code as i32,
            stderr: format!("the server refused the download with HTTP {code}"),
        },
        other => DevdError::Io(std::io::Error::other(format!(
            "failed to reach {}: {other}",
            artifact.url
        ))),
    })?;

    let declared: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(total) = declared {
        if total > MAX_DOWNLOAD {
            return Err(DevdError::invalid(format!(
                "{} is {} bytes, which exceeds the {} byte download limit",
                artifact.url, total, MAX_DOWNLOAD
            )));
        }
    }

    let part = cache_dir.join(format!("{}.part", artifact.filename));
    let mut file = File::create(&part)?;
    let mut reader = response.into_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_report = std::time::Instant::now();

    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > MAX_DOWNLOAD {
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(DevdError::invalid(format!(
                "{} exceeded the {} byte download limit",
                artifact.url, MAX_DOWNLOAD
            )));
        }
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read])?;

        if let Some(report) = progress {
            let now = std::time::Instant::now();
            // Report at most every 100 ms: a progress callback that runs per
            // chunk of a 30 MB file is thousands of UI updates for no benefit.
            if now.duration_since(last_report) >= Duration::from_millis(100) {
                last_report = now;
                if !report(&Progress {
                    received,
                    total: declared,
                }) {
                    drop(file);
                    let _ = std::fs::remove_file(&part);
                    return Err(DevdError::denied(format!(
                        "the download of {} was cancelled",
                        artifact.filename
                    )));
                }
            }
        }
    }

    file.sync_all()?;
    drop(file);

    let sha256 = hex::encode(hasher.finalize());
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
