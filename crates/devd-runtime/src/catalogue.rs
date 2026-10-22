//! Upstream version catalogues.
//!
//! `devd` does not carry a hard-coded list of "known good" versions. Each
//! runtime has one upstream that publishes both the builds and their digests,
//! and `devd` reads that:
//!
//! | Runtime | Catalogue | Digest source |
//! |---------|-----------|---------------|
//! | PHP | `windows.php.net/.../releases.json` | the same document |
//! | Node | `nodejs.org/dist/index.json` | `dist/<version>/SHASUMS256.txt` |
//! | Go | `go.dev/dl/?mode=json` | the same document |
//!
//! The consequence is that `devd runtime install php@8.3` follows PHP's own
//! release stream: a patch release published five minutes ago is installable,
//! and a build devd has never seen is still verified against the vendor's
//! published SHA-256.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use devd_core::download;
use devd_core::platform::Os;
use devd_core::{DevdError, Result};

/// Where PHP publishes Windows builds and their digests.
const PHP_RELEASES: &str = "https://windows.php.net/downloads/releases/releases.json";
const PHP_BASE: &str = "https://windows.php.net/downloads/releases/";

/// Where Node publishes its version index.
const NODE_INDEX: &str = "https://nodejs.org/dist/index.json";
const NODE_BASE: &str = "https://nodejs.org/dist/";

/// Where Go publishes its release list.
const GO_RELEASES: &str = "https://go.dev/dl/?mode=json";

/// How long to wait for a catalogue document.
/// python-build-standalone: complete CPython builds (pip, venv, the standard library) for
/// every platform. Each release publishes a `SHA256SUMS` file for all of its archives, and
/// `releases/latest/download` always names the newest.
const PYTHON_SUMS: &str = "https://github.com/astral-sh/python-build-standalone/releases/latest/download/SHA256SUMS";
const PYTHON_BASE: &str = "https://github.com/astral-sh/python-build-standalone/releases/download/";

/// RubyInstaller's releases, with the SHA-256 GitHub records for each asset.
const RUBY_RELEASES: &str = "https://api.github.com/repos/oneclick/rubyinstaller2/releases?per_page=30";

const CATALOGUE_TIMEOUT: Duration = Duration::from_secs(30);

/// One installable build published by an upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Available {
    /// Runtime kind: `php`, `node` or `go`.
    pub kind: String,
    /// Exact version.
    pub version: String,
    /// Release series, e.g. `8.3`.
    pub series: String,
    /// Build flavour, e.g. `nts`, `ts`, `lts`, `current`, `stable`.
    pub channel: String,
    /// Absolute download URL.
    pub url: String,
    /// Published SHA-256, when the catalogue carries one.
    pub sha256: Option<String>,
    /// Published size in bytes.
    pub bytes: Option<u64>,
    /// Publication timestamp, when the catalogue carries one.
    pub published: Option<String>,
    /// Name of the executable the build provides.
    pub executable: String,
}

/// Order versions so that `8.10` sorts after `8.9` rather than before it.
pub fn version_key(version: &str) -> Vec<u32> {
    version
        .trim_start_matches('v')
        .split(['.', '-'])
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0)
        })
        .collect()
}

/// The release series a version belongs to (`8.3.35` -> `8.3`).
pub fn series_of(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version.to_string()
    }
}

/// Sort newest first.
fn sort_newest_first(list: &mut [Available]) {
    list.sort_by(|a, b| {
        version_key(&b.version)
            .cmp(&version_key(&a.version))
            .then_with(|| a.channel.cmp(&b.channel))
    });
}

/// Every PHP build Windows.php.net currently publishes.
///
/// PHP is the one runtime with no official binary for Linux: `php.net` ships
/// source only, and every distribution builds it. Rather than pretend otherwise,
/// the Linux path returns an empty catalogue and the caller explains that the
/// system PHP is what `devd` will use.
pub fn php(thread_safe: bool) -> Result<Vec<Available>> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    let doc: BTreeMap<String, Value> = download::fetch_json(PHP_RELEASES, CATALOGUE_TIMEOUT)?;

    let mut list = Vec::new();
    for (series, entry) in doc {
        let Some(version) = entry.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        let published = entry
            .get("mtime")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        // Build keys look like `nts-vs17-x64` / `ts-vc15-x64`; the toolset
        // number changes between releases, so it is matched, not hard-coded.
        for (key, build) in entry.as_object().into_iter().flatten() {
            let Some((flavour, _toolset, arch)) = split_build_key(key) else {
                continue;
            };
            if arch != "x64" {
                continue;
            }
            let is_ts = flavour == "ts";
            if is_ts != thread_safe {
                continue;
            }
            // The build itself is nested: `nts-vs17-x64` holds `zip`, `mtime`
            // and the debug/devel packs, and the artifact's path, digest and
            // size all live inside `zip`.
            let Some(zip) = build.get("zip") else {
                continue;
            };
            let Some(path) = zip.get("path").and_then(|v| v.as_str()) else {
                continue;
            };
            list.push(Available {
                kind: "php".to_string(),
                version: version.to_string(),
                series: series.clone(),
                channel: if is_ts { "ts" } else { "nts" }.to_string(),
                url: format!("{PHP_BASE}{path}"),
                sha256: zip
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                bytes: zip
                    .get("size")
                    .and_then(|v| v.as_str())
                    .and_then(parse_size),
                published: build
                    .get("mtime")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| published.clone()),
                executable: "php.exe".to_string(),
            });
        }
    }
    sort_newest_first(&mut list);
    Ok(list)
}

/// Split `nts-vs17-x64` into `("nts", "vs17", "x64")`.
fn split_build_key(key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = key.split('-');
    let flavour = parts.next()?;
    let toolset = parts.next()?;
    let arch = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !matches!(flavour, "nts" | "ts") {
        return None;
    }
    if !(toolset.starts_with("vs") || toolset.starts_with("vc")) {
        return None;
    }
    if !matches!(arch, "x64" | "x86") {
        return None;
    }
    Some((flavour, toolset, arch))
}

/// Parse the `"31.25MB"` form PHP publishes.
fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.find(|c: char| c.is_ascii_alphabetic())?);
    let value: f64 = number.trim().parse().ok()?;
    let multiplier: f64 = match unit.trim().to_ascii_uppercase().as_str() {
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((value * multiplier) as u64)
}

/// Node builds for this platform.
/// The python-build-standalone target for this machine.
fn python_triple() -> Option<&'static str> {
    Some(if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        return None;
    })
}

/// Every Python the newest python-build-standalone release has for this machine.
pub fn python() -> Result<Vec<Available>> {
    let triple = python_triple().ok_or_else(|| DevdError::unsupported("python-build-standalone has no build for this platform"))?;
    let sums = download::fetch_text(PYTHON_SUMS, CATALOGUE_TIMEOUT)?;
    let mut list = python_from(&sums, triple);
    sort_newest_first(&mut list);
    Ok(list)
}

/// Read `SHA256SUMS` (`<sha256>  cpython-3.12.15+20261021-<triple>-install_only.tar.gz`).
pub(crate) fn python_from(sums: &str, triple: &str) -> Vec<Available> {
    let suffix = format!("-{triple}-install_only.tar.gz");
    let mut out = Vec::new();
    for line in sums.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else { continue };
        let Some(rest) = name.strip_prefix("cpython-") else { continue };
        let Some(stem) = rest.strip_suffix(&suffix) else { continue };
        let Some((version, tag)) = stem.split_once('+') else { continue };
        if digest.len() != 64 || version.split('.').count() != 3 || version.split('.').any(|p| p.parse::<u32>().is_err()) {
            continue;
        }
        out.push(Available {
            kind: "python".to_string(),
            version: version.to_string(),
            series: series_of(version),
            channel: "stable".to_string(),
            url: format!("{PYTHON_BASE}{tag}/cpython-{version}%2B{tag}{suffix}"),
            sha256: Some(digest.to_ascii_lowercase()),
            bytes: None,
            published: None,
            executable: if cfg!(windows) { "python.exe" } else { "bin/python3" }.to_string(),
        });
    }
    out
}

/// Ruby for Windows: RubyInstaller's `.7z` archives.
pub fn ruby() -> Result<Vec<Available>> {
    let body = download::fetch_text(RUBY_RELEASES, CATALOGUE_TIMEOUT)?;
    let mut list = ruby_from(&body)?;
    sort_newest_first(&mut list);
    Ok(list)
}

pub(crate) fn ruby_from(json_text: &str) -> Result<Vec<Available>> {
    let releases: serde_json::Value = serde_json::from_str(json_text).map_err(|e| DevdError::internal(format!("the RubyInstaller release list is not JSON: {e}")))?;
    let mut out: Vec<Available> = Vec::new();
    for release in releases.as_array().into_iter().flatten() {
        if release["prerelease"].as_bool().unwrap_or(false) || release["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        for file in release["assets"].as_array().into_iter().flatten() {
            let Some(name) = file["name"].as_str() else { continue };
            // rubyinstaller-3.3.5-1-x64.7z (the DevKit installers are .exe and are not wanted)
            let Some(rest) = name.strip_prefix("rubyinstaller-").and_then(|r| r.strip_suffix("-x64.7z")) else { continue };
            let version: String = rest.split('-').next().unwrap_or("").to_string();
            let digest = file["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_ascii_lowercase());
            let (Some(url), Some(sha256)) = (file["browser_download_url"].as_str(), digest) else { continue };
            if version.split('.').count() != 3 || out.iter().any(|a| a.version == version) {
                continue;
            }
            out.push(Available {
                kind: "ruby".to_string(),
                series: series_of(&version),
                version,
                channel: "stable".to_string(),
                url: url.to_string(),
                sha256: Some(sha256),
                bytes: file["size"].as_u64(),
                published: release["published_at"].as_str().map(str::to_string),
                executable: "bin/ruby.exe".to_string(),
            });
        }
    }
    Ok(out)
}

pub fn node() -> Result<Vec<Available>> {
    #[derive(serde::Deserialize)]
    struct Row {
        version: String,
        #[serde(default)]
        lts: Value,
        #[serde(default)]
        date: Option<String>,
    }

    let rows: Vec<Row> = download::fetch_json(NODE_INDEX, CATALOGUE_TIMEOUT)?;
    let mut list = Vec::new();
    for row in rows {
        // The index lists every build including nightlies under unusual names;
        // anything that is not `vX.Y.Z` is not a release.
        if !row.version.starts_with('v') || row.version.matches('.').count() != 2 {
            continue;
        }
        let Ok(archive) = node_artifact(&row.version) else {
            continue;
        };
        let lts = match &row.lts {
            Value::String(name) => Some(name.clone()),
            _ => None,
        };
        list.push(Available {
            kind: "node".to_string(),
            version: row.version.trim_start_matches('v').to_string(),
            series: series_of(row.version.trim_start_matches('v')),
            channel: if lts.is_some() { "lts" } else { "current" }.to_string(),
            url: format!("{NODE_BASE}{}/{archive}", row.version),
            // Node publishes digests in a per-release SHASUMS256.txt, fetched at
            // install time rather than for every listing.
            sha256: None,
            bytes: None,
            published: row.date,
            executable: if cfg!(windows) { "node.exe" } else { "node" }.to_string(),
        });
    }
    sort_newest_first(&mut list);
    Ok(list)
}

/// The file name Node publishes for this platform, e.g.
/// `node-v22.14.0-win-x64.zip`.
pub fn node_artifact(version: &str) -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => {
            return Err(DevdError::unsupported(format!(
                "no Node build is published for this architecture ({other})"
            )))
        }
    };
    let (os, extension) = node_platform(Os::current());
    Ok(format!("node-{version}-{os}-{arch}.{extension}"))
}

/// The operating-system word and archive type Node uses in its file names.
pub fn node_platform(os: Os) -> (&'static str, &'static str) {
    match os {
        Os::Windows => ("win", "zip"),
        Os::Linux => ("linux", "tar.gz"),
        Os::MacOs => ("darwin", "tar.gz"),
    }
}

/// The operating-system word Go uses in its release list.
pub fn go_os(os: Os) -> &'static str {
    match os {
        Os::Windows => "windows",
        Os::Linux => "linux",
        Os::MacOs => "darwin",
    }
}

/// Go toolchains for this platform.
pub fn go() -> Result<Vec<Available>> {
    #[derive(serde::Deserialize)]
    struct Row {
        version: String,
        #[serde(default)]
        stable: bool,
        #[serde(default)]
        files: Vec<File>,
    }
    #[derive(serde::Deserialize)]
    struct File {
        filename: String,
        os: String,
        arch: String,
        sha256: String,
        #[serde(default)]
        size: u64,
        #[serde(default)]
        kind: String,
    }

    let rows: Vec<Row> = download::fetch_json(GO_RELEASES, CATALOGUE_TIMEOUT)?;
    let want_os = go_os(Os::current());
    let want_arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => {
            return Err(DevdError::unsupported(format!(
                "no Go toolchain is published for this architecture ({other})"
            )))
        }
    };

    let mut list = Vec::new();
    for row in rows {
        if !row.stable {
            continue;
        }
        let Some(file) = row
            .files
            .iter()
            .find(|f| f.os == want_os && f.arch == want_arch && f.kind == "archive")
        else {
            continue;
        };
        let version = row.version.trim_start_matches("go").to_string();
        list.push(Available {
            kind: "go".to_string(),
            version: version.clone(),
            series: series_of(&version),
            channel: "stable".to_string(),
            url: format!("https://dl.google.com/go/{}", file.filename),
            sha256: Some(file.sha256.to_lowercase()),
            bytes: if file.size > 0 { Some(file.size) } else { None },
            published: None,
            executable: if cfg!(windows) { "go.exe" } else { "go" }.to_string(),
        });
    }
    sort_newest_first(&mut list);
    Ok(list)
}

/// Node's per-release digest file, for the platform artifact of one version.
///
/// Node publishes `SHASUMS256.txt` per release rather than in its index, so the
/// digest is fetched when a version is actually chosen for install.
pub fn node_digest(version: &str, filename: &str) -> Result<Option<String>> {
    let url = format!("{NODE_BASE}v{version}/SHASUMS256.txt");
    let body = download::fetch_text(&url, CATALOGUE_TIMEOUT)?;
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name == filename {
            return Ok(Some(digest.to_lowercase()));
        }
    }
    Ok(None)
}

/// Resolve a user-supplied version spec against a catalogue.
///
/// Accepts a channel word (`lts`, `current`, `stable`, `nts`, `ts`), `latest`,
/// a full version (`8.3.35`) or a release series (`8.3`), and prefers the
/// requested flavour when a runtime publishes more than one build per version,
/// as PHP does for thread-safe and non-thread-safe.
pub fn resolve(list: &[Available], spec: &str, channel: Option<&str>) -> Result<Available> {
    let spec = spec.trim();
    if list.is_empty() {
        return Err(DevdError::unsupported(
            "no builds are published upstream for this platform",
        ));
    }
    let wanted = spec.trim_start_matches('v');
    let lowered = wanted.to_ascii_lowercase();

    // A channel word names a build stream rather than a version: `node@lts`
    // should pick the newest LTS release, not look for a series called "lts".
    const CHANNEL_WORDS: &[&str] = &["lts", "current", "stable", "nts", "ts"];
    let spec_is_channel = CHANNEL_WORDS.contains(&lowered.as_str());

    let candidates: Vec<&Available> = if wanted.is_empty() || lowered == "latest" {
        list.iter().collect()
    } else if spec_is_channel {
        list.iter()
            .filter(|a| a.channel.eq_ignore_ascii_case(&lowered))
            .collect()
    } else if wanted.matches('.').count() >= 2 {
        list.iter()
            .filter(|a| a.version.trim_start_matches('v') == wanted)
            .collect()
    } else {
        list.iter().filter(|a| a.series == wanted).collect()
    };

    if candidates.is_empty() {
        let series: Vec<String> = {
            let mut seen: Vec<String> = list.iter().map(|a| a.series.clone()).collect();
            seen.sort_unstable();
            seen.dedup();
            seen.truncate(40);
            seen
        };
        return Err(DevdError::not_found(format!(
            "no build matches `{spec}`; available series: {}",
            series.join(", ")
        )));
    }

    let picked: &Available = match channel.filter(|_| !spec_is_channel) {
        Some(want) => candidates
            .iter()
            .find(|a| a.channel.eq_ignore_ascii_case(want))
            .copied()
            .ok_or_else(|| DevdError::not_found(format!("no `{want}` build matches `{spec}`")))?,
        None => candidates
            .iter()
            .max_by_key(|a| version_key(&a.version))
            .ok_or_else(|| DevdError::not_found(format!("no build matches `{spec}`")))?,
    };
    Ok(picked.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn python_comes_from_the_checksum_file_for_this_platform_only() {
        let sums = "136bbb2e50fe037fd5e32126e70e95b35d1fa5dac5bd04a55b8088415ee99540  cpython-3.11.17+20261021-x86_64-pc-windows-msvc-install_only.tar.gz
be7f14464d1d66f84429ad27e01f6d1482963afff64a2fb5256b2a51c7b5d398  cpython-3.12.15+20261021-x86_64-pc-windows-msvc-install_only.tar.gz
490fd9caa3ed15db752379d6fdd0d9a8e8e44e392c3c9c0f274eface439c03ae  cpython-3.12.15+20261021-i686-pc-windows-msvc-install_only.tar.gz
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  cpython-3.12.15+20261021-x86_64-pc-windows-msvc-install_only_stripped.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  cpython-3.13.0rc1+20261021-x86_64-pc-windows-msvc-install_only.tar.gz
short  cpython-3.10.1+20261021-x86_64-pc-windows-msvc-install_only.tar.gz
";
        let found = super::python_from(sums, "x86_64-pc-windows-msvc");
        let versions: Vec<&str> = found.iter().map(|a| a.version.as_str()).collect();
        assert_eq!(versions, vec!["3.11.17", "3.12.15"], "other targets, stripped variants, pre-releases and bad digests are left out");
        assert_eq!(found[1].url, "https://github.com/astral-sh/python-build-standalone/releases/download/20261021/cpython-3.12.15%2B20261021-x86_64-pc-windows-msvc-install_only.tar.gz");
        assert_eq!(found[1].sha256.as_deref(), Some("be7f14464d1d66f84429ad27e01f6d1482963afff64a2fb5256b2a51c7b5d398"));
    }

    #[test]
    fn ruby_comes_from_the_7z_assets_that_carry_a_digest() {
        let json = r#"[
          {"prerelease":false,"published_at":"2026-09-01T00:00:00Z","assets":[
            {"name":"rubyinstaller-3.3.5-2-x64.7z","size":100,"browser_download_url":"https://github.com/oneclick/rubyinstaller2/releases/download/RubyInstaller-3.3.5-2/rubyinstaller-3.3.5-2-x64.7z","digest":"sha256:AAAA000000000000000000000000000000000000000000000000000000000000"},
            {"name":"rubyinstaller-devkit-3.3.5-2-x64.exe","size":1,"browser_download_url":"https://x/y","digest":"sha256:bb"},
            {"name":"rubyinstaller-3.2.6-1-x64.7z","size":1,"browser_download_url":"https://x/z","digest":null}]},
          {"prerelease":false,"assets":[
            {"name":"rubyinstaller-3.3.5-1-x64.7z","size":1,"browser_download_url":"https://x/old","digest":"sha256:cccc"}]}]"#;
        let found = super::ruby_from(json).unwrap();
        assert_eq!(found.len(), 1, "no digest, no install; the older build of the same Ruby is not offered twice");
        assert_eq!(found[0].version, "3.3.5");
        assert!(found[0].url.ends_with("rubyinstaller-3.3.5-2-x64.7z"));
        assert_eq!(found[0].sha256.as_deref(), Some("aaaa000000000000000000000000000000000000000000000000000000000000"));
    }

    use super::*;

    #[test]
    fn node_and_go_name_each_platform_the_way_their_publishers_do() {
        // macOS used to be treated as Linux, which downloads a program that
        // cannot run there.
        assert_eq!(node_platform(Os::Windows), ("win", "zip"));
        assert_eq!(node_platform(Os::Linux), ("linux", "tar.gz"));
        assert_eq!(node_platform(Os::MacOs), ("darwin", "tar.gz"));
        assert_eq!(go_os(Os::Windows), "windows");
        assert_eq!(go_os(Os::Linux), "linux");
        assert_eq!(go_os(Os::MacOs), "darwin");
    }

    fn sample() -> Vec<Available> {
        ["8.4.2", "8.3.35", "8.3.14", "8.2.29"]
            .iter()
            .map(|v| Available {
                kind: "php".to_string(),
                version: v.to_string(),
                series: series_of(v),
                channel: "nts".to_string(),
                url: format!("https://example.invalid/php-{v}.zip"),
                sha256: None,
                bytes: None,
                published: None,
                executable: "php.exe".to_string(),
            })
            .collect()
    }

    #[test]
    fn versions_sort_numerically_not_lexically() {
        let mut list = sample();
        let template = list[0].clone();
        list.push(Available {
            version: "8.10.0".to_string(),
            series: "8.10".to_string(),
            ..template
        });
        sort_newest_first(&mut list);
        assert_eq!(list[0].version, "8.10.0", "8.10 must outrank 8.9");
        assert_eq!(list[1].version, "8.4.2");
    }

    #[test]
    fn series_spec_picks_the_newest_patch() {
        let list = sample();
        let picked = resolve(&list, "8.3", None).unwrap();
        assert_eq!(picked.version, "8.3.35");
    }

    #[test]
    fn exact_spec_is_honoured() {
        let list = sample();
        let picked = resolve(&list, "8.3.14", None).unwrap();
        assert_eq!(picked.version, "8.3.14");
    }

    #[test]
    fn latest_spec_takes_the_top_of_the_list() {
        let list = sample();
        assert_eq!(resolve(&list, "latest", None).unwrap().version, "8.4.2");
        assert_eq!(resolve(&list, "", None).unwrap().version, "8.4.2");
    }

    #[test]
    fn an_unknown_series_lists_what_exists() {
        let list = sample();
        let err = resolve(&list, "9.9", None).unwrap_err();
        assert_eq!(err.code(), "not_found");
        assert!(err.to_string().contains("8.3"), "{err}");
    }

    #[test]
    fn a_channel_word_selects_that_stream() {
        let mut list = sample();
        for entry in list.iter_mut() {
            entry.channel = "nts".to_string();
        }
        list.push(Available {
            version: "8.3.35".to_string(),
            series: "8.3".to_string(),
            channel: "ts".to_string(),
            url: "https://example.invalid/php-8.3.35-ts.zip".to_string(),
            ..list[0].clone()
        });
        let ts = resolve(&list, "ts", None).unwrap();
        assert_eq!(ts.channel, "ts");
        let nts = resolve(&list, "nts", None).unwrap();
        assert_eq!(nts.channel, "nts");
        assert_eq!(nts.version, "8.4.2");
    }

    #[test]
    fn unknown_series_message_is_bounded() {
        let mut list = sample();
        for i in 0..100 {
            list.push(Available {
                version: format!("1.0.{i}"),
                series: format!("1.{i}"),
                ..list[0].clone()
            });
        }
        let err = resolve(&list, "99.99", None).unwrap_err();
        assert!(err.to_string().len() < 400, "{err}");
    }

    #[test]
    fn build_keys_are_split_or_rejected() {
        assert_eq!(
            split_build_key("nts-vs17-x64"),
            Some(("nts", "vs17", "x64"))
        );
        assert_eq!(split_build_key("ts-vc15-x86"), Some(("ts", "vc15", "x86")));
        assert_eq!(split_build_key("source"), None);
        assert_eq!(split_build_key("nts-vs17"), None);
        assert_eq!(split_build_key("nts-vs17-x64-extra"), None);
    }

    #[test]
    fn php_publishes_sizes_in_human_units() {
        assert_eq!(parse_size("31.25MB"), Some(32_768_000));
        assert_eq!(parse_size("1.26MB"), Some(1_321_205)); // truncated, not rounded
        assert_eq!(parse_size("512KB"), Some(524_288));
        assert_eq!(parse_size("nonsense"), None);
    }

    #[test]
    fn live_php_catalogue_resolves_and_carries_digests() {
        // A live check, deliberately: the whole module is a promise that devd
        // follows the vendor's own catalogue, and a fixture would not test that.
        // Skipped when the network is unavailable so the suite stays runnable
        // offline; `cargo test -- --ignored` runs it explicitly.
        if std::env::var("DEVD_SKIP_NETWORK_TESTS").is_ok() {
            return;
        }
        let Ok(list) = php(false) else {
            eprintln!("skipping: the PHP catalogue was unreachable");
            return;
        };
        if list.is_empty() {
            return;
        }
        let picked = resolve(&list, "8.3", None).unwrap();
        assert_eq!(picked.kind, "php");
        assert!(
            picked.sha256.is_some(),
            "php builds must publish a digest: {picked:?}"
        );
        assert!(picked.url.starts_with("https://windows.php.net/"));
    }
}
