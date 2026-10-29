//! Updating devd from its own releases, with a signature in the way.
//!
//! A program that replaces itself with whatever a URL returns is a way into
//! every machine it is installed on. This one trusts exactly one thing: an
//! Ed25519 signature, made with a key only the maintainer holds, over the
//! release's `SHA256SUMS.txt`. The public half is compiled into the program.
//! The order is fixed and every step refuses on failure:
//!
//! 1. find the latest release and its checksum file and signature;
//! 2. verify the signature over the checksum file - a tampered or unsigned
//!    release stops here;
//! 3. download the program and check its SHA-256 against the signed checksum;
//! 4. run the new program and check it reports the version being installed;
//! 5. only then replace the installed one, keeping the old one beside it.
//!
//! GitHub, the download host and the network between are not trusted for
//! anything but delivery. The signature is not a code-signing certificate; it
//! is what makes an update this program's own business to check.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::Serialize;

use crate::{download, util, DevdError, Result};

/// The key that signs releases, as 64 hex digits. Releases signed by any other
/// key are refused.
pub const RELEASE_PUBLIC_KEY: &str =
    "a2d470f4f555cf179ae4e72c89b8e19c6241371f5b77e5b36d46619bd0562fae";

/// Where the latest release is described.
pub const LATEST_URL: &str = "https://api.github.com/repos/ianclemence/devd/releases/latest";

/// The name of the checksum file, and of its signature.
pub const SUMS: &str = "SHA256SUMS.txt";
/// The signature file.
pub const SUMS_SIG: &str = "SHA256SUMS.txt.sig";

/// A key pair's seed, 32 random bytes, as 64 hex digits.
pub fn generate_seed() -> Result<String> {
    use ring::rand::SecureRandom;
    let mut seed = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| DevdError::internal("the system random source failed"))?;
    Ok(hex::encode(seed))
}

fn key_pair(seed_hex: &str) -> Result<Ed25519KeyPair> {
    let seed = hex::decode(seed_hex.trim())
        .map_err(|_| DevdError::invalid("the signing key is not hexadecimal"))?;
    Ed25519KeyPair::from_seed_unchecked(&seed)
        .map_err(|_| DevdError::invalid("the signing key must be 32 bytes (64 hex digits)"))
}

/// The public key for a signing seed, as hex.
pub fn public_key_of(seed_hex: &str) -> Result<String> {
    Ok(hex::encode(key_pair(seed_hex)?.public_key().as_ref()))
}

/// Sign `message` with the seed.
pub fn sign(seed_hex: &str, message: &[u8]) -> Result<Vec<u8>> {
    Ok(key_pair(seed_hex)?.sign(message).as_ref().to_vec())
}

/// [`sign`], as the hex text a `.sig` file holds.
pub fn sign_hex(seed_hex: &str, message: &[u8]) -> Result<String> {
    Ok(hex::encode(sign(seed_hex, message)?))
}

/// Whether `signature` is `public_key_hex`'s signature over `message`.
pub fn verify(public_key_hex: &str, message: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = hex::decode(public_key_hex.trim()) else {
        return false;
    };
    UnparsedPublicKey::new(&ED25519, key)
        .verify(message, signature)
        .is_ok()
}

/// `(sha256, file name)` pairs from a checksum file, in either the `sha256sum`
/// form (`hash  name`, `hash *name`) or PowerShell's.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (hash, name) = line.split_once(char::is_whitespace)?;
            let name = name.trim().trim_start_matches('*');
            (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (hash.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

/// Whether `latest` is a later version than `current` (both `x.y.z`, optionally
/// prefixed with `v`). An unparseable version is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    fn parts(v: &str) -> Option<(u64, u64, u64)> {
        let v = v.trim().trim_start_matches('v');
        let mut it = v.split(['.', '-']).map(|p| p.parse::<u64>().ok());
        Some((it.next()??, it.next()??, it.next()??))
    }
    match (parts(current), parts(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// The release asset for the running platform's command-line program.
pub fn asset_name(version: &str) -> Option<String> {
    let target = if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-gnu.exe"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        return None;
    };
    Some(format!("devd-v{version}-{target}"))
}

/// A published asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asset {
    /// File name.
    pub name: String,
    /// Where to download it.
    pub url: String,
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    /// The tag, for example `v0.16.0`.
    pub tag: String,
    /// The version, without the `v`.
    pub version: String,
    /// Its assets.
    pub assets: Vec<Asset>,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Parse GitHub's description of a release.
pub fn parse_release(json: &serde_json::Value) -> Result<Release> {
    let tag = json["tag_name"]
        .as_str()
        .ok_or_else(|| DevdError::invalid("the release has no tag"))?
        .to_string();
    let assets = json["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        tag,
        assets,
    })
}

fn get(url: &str, allow_http: bool, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    if !url.starts_with("https://") && !(allow_http && url.starts_with("http://")) {
        return Err(DevdError::invalid(format!("refusing {url}: updates are fetched over https only")));
    }
    let response = ureq::get(url)
        .set("User-Agent", &format!("devd/{}", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json, application/octet-stream")
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| DevdError::Io(std::io::Error::other(format!("could not fetch {url}: {e}"))))?;
    let mut body = Vec::new();
    response.into_reader().take(limit).read_to_end(&mut body)?;
    Ok(body)
}

/// The latest published release.
pub fn latest(url: &str, allow_http: bool) -> Result<Release> {
    let body = get(url, allow_http, 4 * 1024 * 1024)?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| DevdError::invalid(format!("the release description is not JSON: {e}")))?;
    parse_release(&json)
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// The running version.
    pub current: String,
    /// The latest published version.
    pub latest: String,
    /// Whether the latest is newer.
    pub newer: bool,
    /// Whether the release carries a signed checksum file, which an update needs.
    pub signed: bool,
    /// The file an update would install on this platform, when the release has it.
    pub asset: Option<String>,
}

/// Compare the running version with the latest release.
pub fn check(url: &str, allow_http: bool) -> Result<Check> {
    let release = latest(url, allow_http)?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    let wanted = asset_name(&release.version);
    Ok(Check {
        newer: is_newer(&current, &release.version),
        signed: release.asset(SUMS).is_some() && release.asset(SUMS_SIG).is_some(),
        asset: wanted
            .filter(|n| release.asset(n).is_some()),
        latest: release.version,
        current,
    })
}

/// How to apply an update. Everything has a default for real use; tests change
/// the key, the destination and the smoke check.
pub struct Options<'a> {
    /// The public key releases must be signed with.
    pub public_key: &'a str,
    /// The installed program to replace.
    pub target: PathBuf,
    /// Where downloads are kept.
    pub cache: PathBuf,
    /// Run the downloaded program with `--version` before installing it.
    pub smoke_test: bool,
    /// Allow `http://` (for a local test server only).
    pub allow_http: bool,
}

/// What an update did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    /// The version now installed.
    pub version: String,
    /// The program that was replaced, kept so the update can be undone.
    pub previous: PathBuf,
    /// The installed program.
    pub installed: PathBuf,
}

/// Download, verify and install a release.
pub fn apply(release: &Release, options: &Options<'_>) -> Result<Applied> {
    let sums = release
        .asset(SUMS)
        .ok_or_else(|| DevdError::invalid(format!("{} has no {SUMS}; it cannot be verified, so it will not be installed", release.tag)))?;
    let sig = release
        .asset(SUMS_SIG)
        .ok_or_else(|| DevdError::invalid(format!("{} is not signed ({SUMS_SIG} is missing); it will not be installed", release.tag)))?;
    let asset_file = asset_name(&release.version)
        .ok_or_else(|| DevdError::unsupported("no update is published for this platform"))?;
    let asset = release.asset(&asset_file).ok_or_else(|| {
        DevdError::not_found(format!("{} has no `{asset_file}` for this platform", release.tag))
    })?;

    // 1. The signature, before anything else is trusted.
    let sums_bytes = get(&sums.url, options.allow_http, 1024 * 1024)?;
    let sig_bytes = get(&sig.url, options.allow_http, 4096)?;
    let sig_bytes = match std::str::from_utf8(&sig_bytes).ok().and_then(|t| hex::decode(t.trim()).ok()) {
        Some(decoded) => decoded,
        None => sig_bytes,
    };
    if !verify(options.public_key, &sums_bytes, &sig_bytes) {
        return Err(DevdError::denied(format!(
            "the signature on {} does not verify with devd's release key; nothing was installed",
            release.tag
        )));
    }

    // 2. The program, against the signed checksum.
    let expected = parse_sums(&String::from_utf8_lossy(&sums_bytes))
        .into_iter()
        .find(|(_, name)| *name == asset_file)
        .map(|(hash, _)| hash)
        .ok_or_else(|| DevdError::invalid(format!("the signed checksums do not list `{asset_file}`")))?;
    let artifact = download::Artifact::new(asset.url.clone())
        .filename(asset_file.clone())
        .sha256(expected);
    let fetched = download::fetch_with(&artifact, &options.cache, None, options.allow_http)?;

    // 3. The program must run and say it is the version being installed.
    if options.smoke_test {
        make_executable(&fetched.path)?;
        let out = util::run(&fetched.path, &["--version".to_string()], None, &[], Some(Duration::from_secs(20)))?;
        if !out.stdout.contains(&release.version) {
            return Err(DevdError::invalid(format!(
                "the downloaded program reports `{}`, not {}; it was not installed",
                out.stdout.trim(),
                release.version
            )));
        }
    }

    // 4. Replace, keeping the old one. A running program can be renamed on every
    // platform devd supports, so the new file takes its place without a restart.
    let previous = PathBuf::from(format!("{}.before-{}", options.target.display(), env!("CARGO_PKG_VERSION")));
    let _ = std::fs::remove_file(&previous);
    std::fs::rename(&options.target, &previous)?;
    if let Err(e) = std::fs::copy(&fetched.path, &options.target) {
        // Put the old one back rather than leave nothing installed.
        let _ = std::fs::rename(&previous, &options.target);
        return Err(DevdError::Io(std::io::Error::other(format!(
            "could not install the new program: {e}; the old one was left in place"
        ))));
    }
    make_executable(&options.target)?;
    Ok(Applied {
        version: release.version.clone(),
        previous,
        installed: options.target.clone(),
    })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn a_signature_verifies_only_for_its_message_and_its_key() {
        let seed = generate_seed().unwrap();
        let public = public_key_of(&seed).unwrap();
        let sig = sign(&seed, b"abc  file\n").unwrap();
        assert!(verify(&public, b"abc  file\n", &sig));
        assert!(!verify(&public, b"abd  file\n", &sig), "a changed message");
        let other = public_key_of(&generate_seed().unwrap()).unwrap();
        assert!(!verify(&other, b"abc  file\n", &sig), "another key");
        assert!(!verify(&public, b"abc  file\n", &sig[..63]), "a truncated signature");
        assert!(!verify("not hex", b"x", &sig));
        // The placeholder key shipped before the first signed release verifies nothing.
        assert!(!verify(RELEASE_PUBLIC_KEY, b"abc  file\n", &sig));
    }

    #[test]
    fn checksum_files_parse_in_both_common_forms() {
        let hash = "a".repeat(64);
        let text = format!("{hash}  devd-v1.exe\r\n{} *devd-v2.exe\nnot a line\n", "B".repeat(64));
        let parsed = parse_sums(&text);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], (hash, "devd-v1.exe".to_string()));
        assert_eq!(parsed[1].0, "b".repeat(64));
        assert_eq!(parsed[1].1, "devd-v2.exe");
    }

    #[test]
    fn only_a_later_version_is_newer() {
        assert!(is_newer("0.15.1", "0.16.0"));
        assert!(is_newer("0.15.1", "v0.15.2"));
        assert!(is_newer("0.9.9", "0.10.0"));
        assert!(!is_newer("0.16.0", "0.16.0"));
        assert!(!is_newer("0.16.0", "0.15.9"));
        assert!(!is_newer("0.16.0", "garbage"));
    }

    #[test]
    fn a_release_is_read_from_the_github_description() {
        let json = serde_json::json!({
            "tag_name": "v0.16.0",
            "assets": [
                { "name": "SHA256SUMS.txt", "browser_download_url": "https://x/s" },
                { "name": "ignored", "browser_download_url": null }
            ]
        });
        let release = parse_release(&json).unwrap();
        assert_eq!(release.version, "0.16.0");
        assert_eq!(release.assets.len(), 1);
        assert!(parse_release(&serde_json::json!({})).is_err());
    }

    /// Serve fixed bodies by path.
    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut raw = vec![0u8; 4096];
                let n = stream.read(&mut raw).unwrap_or(0);
                let head = String::from_utf8_lossy(&raw[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = routes.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone());
                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", Vec::new()),
                };
                let _ = stream.write_all(
                    format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes(),
                );
                let _ = stream.write_all(&body);
            }
        });
        base
    }

    /// A release served locally, signed with `seed`, whose program is `program`.
    fn release(seed: &str, program: &[u8], tamper_sums_after_signing: bool) -> (Release, String) {
        let version = "9.9.9";
        let name = asset_name(version).expect("a platform with updates");
        let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, program));
        let sums = format!("{digest}  {name}\n");
        let signature = hex::encode(sign(seed, sums.as_bytes()).unwrap());
        let served_sums = if tamper_sums_after_signing {
            format!("{}  {name}\n", "0".repeat(64))
        } else {
            sums
        };
        let base = serve(vec![
            ("/sums", served_sums.into_bytes()),
            ("/sig", signature.into_bytes()),
            ("/program", program.to_vec()),
        ]);
        let release = Release {
            tag: format!("v{version}"),
            version: version.to_string(),
            assets: vec![
                Asset { name: SUMS.into(), url: format!("{base}/sums") },
                Asset { name: SUMS_SIG.into(), url: format!("{base}/sig") },
                Asset { name: name.clone(), url: format!("{base}/program") },
            ],
        };
        (release, name)
    }

    fn options<'a>(public: &'a str, dir: &Path) -> Options<'a> {
        let target = dir.join("devd-installed");
        std::fs::write(&target, b"the old program").unwrap();
        Options {
            public_key: public,
            target,
            cache: dir.join("cache"),
            smoke_test: false,
            allow_http: true,
        }
    }

    #[test]
    fn a_signed_release_replaces_the_program_and_keeps_the_old_one() {
        let seed = generate_seed().unwrap();
        let public = public_key_of(&seed).unwrap();
        let (release, _) = release(&seed, b"the new program", false);
        let dir = tempfile::tempdir().unwrap();
        let o = options(&public, dir.path());

        let applied = apply(&release, &o).unwrap();
        assert_eq!(applied.version, "9.9.9");
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the new program");
        assert_eq!(std::fs::read(&applied.previous).unwrap(), b"the old program");
    }

    #[test]
    fn a_release_signed_by_another_key_is_refused_and_nothing_changes() {
        let attacker = generate_seed().unwrap();
        let ours = public_key_of(&generate_seed().unwrap()).unwrap();
        let (release, _) = release(&attacker, b"malware", false);
        let dir = tempfile::tempdir().unwrap();
        let o = options(&ours, dir.path());

        let err = apply(&release, &o).unwrap_err();
        assert_eq!(err.code(), "permission_denied");
        assert!(err.to_string().contains("does not verify"), "{err}");
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the old program");
    }

    #[test]
    fn a_program_that_does_not_match_the_signed_checksum_is_refused() {
        let seed = generate_seed().unwrap();
        let public = public_key_of(&seed).unwrap();
        // The checksum file served is not the one that was signed.
        let (release, _) = release(&seed, b"the new program", true);
        let dir = tempfile::tempdir().unwrap();
        let o = options(&public, dir.path());
        let err = apply(&release, &o).unwrap_err();
        assert_eq!(err.code(), "permission_denied", "{err}");
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the old program");

        // A correct signature over sums that the program does not match.
        let (mut release, name) = release_with_wrong_program(&seed);
        release.assets.retain(|a| a.name != "other");
        let dir = tempfile::tempdir().unwrap();
        let o = options(&public, dir.path());
        let err = apply(&release, &o).unwrap_err();
        assert!(err.to_string().contains("digest mismatch"), "{name}: {err}");
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the old program");
    }

    /// Sums signed for one program, a different program served.
    fn release_with_wrong_program(seed: &str) -> (Release, String) {
        let version = "9.9.9";
        let name = asset_name(version).unwrap();
        let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, b"the program that was signed"));
        let sums = format!("{digest}  {name}\n");
        let signature = hex::encode(sign(seed, sums.as_bytes()).unwrap());
        let base = serve(vec![
            ("/sums", sums.into_bytes()),
            ("/sig", signature.into_bytes()),
            ("/program", b"a different program".to_vec()),
        ]);
        (
            Release {
                tag: "v9.9.9".into(),
                version: version.into(),
                assets: vec![
                    Asset { name: SUMS.into(), url: format!("{base}/sums") },
                    Asset { name: SUMS_SIG.into(), url: format!("{base}/sig") },
                    Asset { name: name.clone(), url: format!("{base}/program") },
                ],
            },
            name,
        )
    }

    #[test]
    fn an_unsigned_release_is_refused_before_anything_is_downloaded() {
        let release = Release {
            tag: "v9.9.9".into(),
            version: "9.9.9".into(),
            assets: vec![Asset { name: SUMS.into(), url: "http://127.0.0.1:1/sums".into() }],
        };
        let dir = tempfile::tempdir().unwrap();
        let o = options(RELEASE_PUBLIC_KEY, dir.path());
        let err = apply(&release, &o).unwrap_err();
        assert!(err.to_string().contains("not signed"), "{err}");
    }

    #[test]
    fn updates_are_only_fetched_over_https() {
        assert!(get("http://example.com/x", false, 10).is_err());
    }
}
