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
    "61617b2bdd1ff12d0bc870212e6744a49f3ca56b13481ea4263bc8a4a8aae126";

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
    let target = platform_target()?;
    Some(format!("devd-v{version}-{target}"))
}

/// The release asset for the running platform's desktop window program.
pub fn desktop_asset_name(version: &str) -> Option<String> {
    let target = platform_target()?;
    Some(format!("devd-desktop-v{version}-{target}"))
}

fn platform_target() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("x86_64-pc-windows-gnu.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("x86_64-apple-darwin")
    } else {
        None
    }
}

/// The window programs installed beside the command-line program, under any name an installer has given them.
pub fn windows_beside(cli: &Path) -> Vec<PathBuf> {
    ["devd-app", "devd-desktop"]
        .iter()
        .map(|name| cli.with_file_name(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }))
        .filter(|path| path.is_file())
        .collect()
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
    /// `desktop` when a window program is installed, `command line` otherwise.
    pub app: String,
    /// The release page, for an unsigned release the footer may only open.
    pub url: String,
}

/// Compare the running version with the latest release.
pub fn check(url: &str, allow_http: bool) -> Result<Check> {
    let release = latest(url, allow_http)?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    // An installed window is updated along with the command, so an update then needs both files.
    let desktop = util::cli_exe().is_ok_and(|cli| !windows_beside(&cli).is_empty());
    let window_published = desktop_asset_name(&release.version).is_some_and(|n| release.asset(&n).is_some());
    let wanted = asset_name(&release.version).filter(|_| !desktop || window_published);
    Ok(Check {
        newer: is_newer(&current, &release.version),
        signed: release.asset(SUMS).is_some() && release.asset(SUMS_SIG).is_some(),
        asset: wanted.filter(|n| release.asset(n).is_some()),
        url: format!("https://github.com/ianclemence/devd/releases/tag/{}", release.tag),
        latest: release.version,
        current,
        app: if desktop { "desktop" } else { "command line" }.to_string(),
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
    /// The window programs replaced with it, for the window to restart into.
    pub window: Vec<PathBuf>,
}

/// Download, verify and install a release: the command-line program, and the window program beside it when there is one.
///
/// Everything is downloaded and verified before anything is replaced, and a replacement that fails puts back the ones
/// already made, so the two programs are not left at different versions. A running program can be renamed on every
/// platform devd supports, so each new file takes its place at once; what is running changes when it next starts.
pub fn apply(release: &Release, options: &Options<'_>) -> Result<Applied> {
    let asset_file = asset_name(&release.version)
        .ok_or_else(|| DevdError::unsupported("no update is published for this platform"))?;
    let command = fetch_verified(release, options, &asset_file, options.smoke_test)?;
    let windows = windows_beside(&options.target);
    let mut swaps = vec![(options.target.clone(), command)];
    if !windows.is_empty() {
        let asset_file = desktop_asset_name(&release.version)
            .ok_or_else(|| DevdError::unsupported("no desktop update is published for this platform"))?;
        // Not asked for its version: starting it would open a window, and the signed checksum is the trust here.
        let window = fetch_verified(release, options, &asset_file, false)?;
        swaps.extend(windows.iter().map(|target| (target.clone(), window.clone())));
    }

    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (target, fetched) in &swaps {
        match replace(target, fetched) {
            Ok(previous) => done.push((target.clone(), previous)),
            Err(e) => {
                for (target, previous) in done.iter().rev() {
                    // The new file is moved out of the way if it cannot be removed, and the old one put back.
                    let _ = std::fs::remove_file(target).or_else(|_| std::fs::rename(target, aside(target, "undone")));
                    let _ = std::fs::rename(previous, target);
                }
                return Err(DevdError::Io(std::io::Error::other(format!(
                    "could not install {}: {e}; the old programs were left in place",
                    target.display()
                ))));
            }
        }
    }
    Ok(Applied {
        version: release.version.clone(),
        previous: done[0].1.clone(),
        installed: options.target.clone(),
        window: windows,
    })
}

/// A name beside `target` that nothing holds.
fn aside(target: &Path, what: &str) -> PathBuf {
    let plain = PathBuf::from(format!("{}.{what}", target.display()));
    // An earlier copy that is still running cannot be removed; the new one is then kept beside it.
    if !plain.exists() || std::fs::remove_file(&plain).is_ok() {
        return plain;
    }
    PathBuf::from(format!("{}.{what}-{}", target.display(), std::process::id()))
}

/// Put `fetched` where `target` is, keeping the old program. Returns where the old one went.
fn replace(target: &Path, fetched: &Path) -> Result<PathBuf> {
    let previous = aside(target, &format!("before-{}", env!("CARGO_PKG_VERSION")));
    std::fs::rename(target, &previous)?;
    let installed = std::fs::copy(fetched, target)
        .map_err(DevdError::Io)
        .and_then(|_| make_executable(target));
    if let Err(e) = installed {
        // Put the old one back rather than leave nothing installed.
        let _ = std::fs::remove_file(target);
        let _ = std::fs::rename(&previous, target);
        return Err(e);
    }
    Ok(previous)
}

/// Verify the release's signature and checksum, download the named asset and
/// return the path it was cached at. The signature is checked before anything
/// else is trusted, and a failed check installs nothing.
fn fetch_verified(release: &Release, options: &Options<'_>, asset_file: &str, smoke_test: bool) -> Result<PathBuf> {
    let sums = release
        .asset(SUMS)
        .ok_or_else(|| DevdError::invalid(format!("{} has no {SUMS}; it cannot be verified, so it will not be installed", release.tag)))?;
    let sig = release
        .asset(SUMS_SIG)
        .ok_or_else(|| DevdError::invalid(format!("{} is not signed ({SUMS_SIG} is missing); it will not be installed", release.tag)))?;
    let asset = release.asset(asset_file).ok_or_else(|| {
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
        .find(|(_, name)| name == asset_file)
        .map(|(hash, _)| hash)
        .ok_or_else(|| DevdError::invalid(format!("the signed checksums do not list `{asset_file}`")))?;
    let artifact = download::Artifact::new(asset.url.clone())
        .filename(asset_file.to_string())
        .sha256(expected);
    let fetched = download::fetch_with(&artifact, &options.cache, None, options.allow_http)?;

    // 3. The command-line program must run and say it is the version being
    // installed. The desktop window is not asked: starting it would open a
    // window, and the signed checksum is the trust here.
    if smoke_test {
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
    Ok(fetched.path)
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
    fn the_desktop_update_is_its_own_asset() {
        // The window program and the command are separate downloads. A footer
        // update must fetch the one the person is actually looking at.
        let command = asset_name("9.9.9").expect("a platform with updates");
        let desktop = desktop_asset_name("9.9.9").expect("a platform with updates");
        assert!(command.starts_with("devd-v9.9.9-"), "{command}");
        assert!(desktop.starts_with("devd-desktop-v9.9.9-"), "{desktop}");
        assert_ne!(command, desktop);
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

    /// A release with both programs, signed with `seed`. Without `with_window` the window's file is not served.
    fn both(seed: &str, with_window: bool) -> Release {
        let (command, window) = (asset_name("9.9.9").unwrap(), desktop_asset_name("9.9.9").unwrap());
        let hash = |b: &[u8]| hex::encode(ring::digest::digest(&ring::digest::SHA256, b));
        let sums = format!("{}  {command}\n{}  {window}\n", hash(b"the new command"), hash(b"the new window"));
        let signature = hex::encode(sign(seed, sums.as_bytes()).unwrap());
        let mut routes = vec![("/sums", sums.into_bytes()), ("/sig", signature.into_bytes()), ("/command", b"the new command".to_vec())];
        if with_window {
            routes.push(("/window", b"the new window".to_vec()));
        }
        let base = serve(routes);
        let asset = |name: String, path: &str| Asset { name, url: format!("{base}/{path}") };
        Release {
            tag: "v9.9.9".into(),
            version: "9.9.9".into(),
            assets: vec![asset(SUMS.into(), "sums"), asset(SUMS_SIG.into(), "sig"), asset(command, "command"), asset(window, "window")],
        }
    }

    fn window_in(dir: &Path) -> PathBuf {
        let path = dir.join(if cfg!(windows) { "devd-app.exe" } else { "devd-app" });
        std::fs::write(&path, b"the old window").unwrap();
        path
    }

    #[test]
    fn the_window_beside_the_command_is_updated_with_it() {
        let seed = generate_seed().unwrap();
        let public = public_key_of(&seed).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (o, window) = (options(&public, dir.path()), window_in(dir.path()));
        let applied = apply(&both(&seed, true), &o).unwrap();
        assert_eq!(applied.window, vec![window.clone()]);
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the new command");
        assert_eq!(std::fs::read(&window).unwrap(), b"the new window");
    }

    #[test]
    fn nothing_is_replaced_when_the_window_cannot_be_downloaded() {
        let seed = generate_seed().unwrap();
        let public = public_key_of(&seed).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (o, window) = (options(&public, dir.path()), window_in(dir.path()));
        assert!(apply(&both(&seed, false), &o).is_err());
        assert_eq!(std::fs::read(&o.target).unwrap(), b"the old program");
        assert_eq!(std::fs::read(&window).unwrap(), b"the old window");
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
