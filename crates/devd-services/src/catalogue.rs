//! Upstream catalogues for services.
//!
//! Services are found the same way runtimes are: from the vendor's own listing,
//! with a digest where the vendor publishes one.
//!
//! | Service | Catalogue | Digest |
//! |---------|-----------|--------|
//! | nginx | `nginx.org/en/download.html` | not published as a file |
//! | MariaDB | `downloads.mariadb.org/rest-api` | per-file `sha256sum` in the API |
//! | Apache httpd | `apachelounge.com/download` (Windows) | `<zip>.txt` beside each archive |
//! | Mailpit | GitHub releases of `axllent/mailpit` | the `digest` GitHub records for each asset |
//! | Redis (Windows) | GitHub releases of `redis-windows/redis-windows` (Redis 8, MSYS2 build) | the `digest` GitHub records for each asset |
//! | memcached (Windows) | GitHub releases of `jefyt/memcached-windows` | the `digest` GitHub records for each asset |
//!
//! nginx is the one honest gap. nginx.org publishes a PGP signature
//! (`nginx-1.28.3.zip.asc`) and no checksum file, so `devd` cannot check the
//! bytes against anything the vendor states in machine-readable form. The
//! download still goes over HTTPS and the install reports, in its own output,
//! that no digest was available - rather than implying a verification that did
//! not happen. See `docs/limitations.md`.

use std::time::Duration;

use devd_core::download;
use devd_core::{DevdError, Result};
use devd_runtime::catalogue::Available;

/// Where nginx lists its builds.
const NGINX_DOWNLOAD: &str = "https://nginx.org/en/download.html";
const NGINX_BASE: &str = "https://nginx.org/download/";

/// Apache Lounge, the Windows builds the Apache project itself points to.
const APACHE_PAGE: &str = "https://www.apachelounge.com/download/";
const APACHE_HOST: &str = "https://www.apachelounge.com";

/// Mailpit's releases, with the SHA-256 GitHub records for each file.
const MAILPIT_API: &str = "https://api.github.com/repos/axllent/mailpit/releases?per_page=10";

/// Redis builds for Windows. Redis itself publishes none; this project builds each release.
const REDIS_API: &str = "https://api.github.com/repos/redis-windows/redis-windows/releases?per_page=10";

/// memcached builds for Windows. Only the release that records digests is offered.
const MEMCACHED_API: &str = "https://api.github.com/repos/jefyt/memcached-windows/releases?per_page=10";

/// Where MariaDB keeps every release's files.
const MARIADB_BASE: &str = "https://archive.mariadb.org/";

/// MariaDB's release API. It is the only machine-readable source that states
/// which series are Stable and, per release, whether a Windows package was
/// actually built. The archive directory listing cannot answer either question:
/// it holds directories for preview series (`mariadb-13.2.0/`) and for stable
/// releases that shipped no Windows build at all (`mariadb-13.0.0/`), and the
/// published zip name is not derivable from the directory alone.
const MARIADB_API: &str = "https://downloads.mariadb.org/rest-api/mariadb/";

/// How long to wait for a catalogue document.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The oldest MariaDB series `devd` will offer. Earlier releases predate the
/// current Windows package layout.
const MARIADB_FLOOR: (u32, u32) = (10, 4);

/// Every nginx build nginx.org currently lists.
///
/// nginx's own release scheme is used to label the channels: an even minor
/// version is a stable branch, an odd one is mainline.
/// The digests of the nginx archives devd installs.
///
/// nginx.org publishes a PGP signature for each archive and no checksum, so
/// devd records the digest of an archive when it is first fetched and checks
/// every later download against it. The table is shipped with devd; an archive
/// that does not match is deleted and nothing is installed.
const NGINX_DIGESTS: &str = include_str!("../defs/nginx-digests.txt");

/// The recorded digest of one nginx archive.
pub fn nginx_digest(version: &str) -> Option<String> {
    for line in NGINX_DIGESTS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let v = parts.next()?;
        let digest = parts.next()?;
        if v == version {
            return Some(digest.to_string());
        }
    }
    None
}

/// Every Apache build Apache Lounge currently lists for 64-bit Windows, with the
/// SHA-256 it publishes in the `.txt` file beside each archive.
pub fn apache() -> Result<Vec<Available>> {
    let html = download::fetch_text(APACHE_PAGE, TIMEOUT)?;
    let mut list = Vec::new();
    for (version, path) in scan_apache(&html) {
        let url = format!("{APACHE_HOST}{path}");
        let sums = download::fetch_text(&format!("{url}.txt"), TIMEOUT)?;
        let sha256 = apache_sha256(&sums).ok_or_else(|| {
            DevdError::internal(format!("{url}.txt does not contain a SHA256 line devd can read"))
        })?;
        list.push(Available {
            kind: "apache".to_string(),
            series: devd_runtime::catalogue::series_of(&version),
            version,
            channel: "stable".to_string(),
            url,
            sha256: Some(sha256),
            bytes: None,
            published: None,
            executable: "Apache24/bin/httpd.exe".to_string(),
        });
    }
    sort_newest_first(&mut list);
    Ok(list)
}

/// The 64-bit Windows archives linked from Apache Lounge's download page, as
/// (version, path). A file name is `httpd-2.4.69-261002-Win64-VS18.zip`.
fn scan_apache(document: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for piece in document.split("href=\"").skip(1) {
        let Some(path) = piece.split('"').next() else { continue };
        let Some(name) = path.rsplit('/').next() else { continue };
        let lower = name.to_ascii_lowercase();
        if !(lower.starts_with("httpd-") && lower.contains("-win64-vs") && lower.ends_with(".zip")) {
            continue;
        }
        let version: String = name["httpd-".len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let version = version.trim_end_matches('.').to_string();
        if version.split('.').count() != 3 || out.iter().any(|(v, _)| v == &version) {
            continue;
        }
        out.push((version, path.to_string()));
    }
    out
}

/// The SHA-256 line of an Apache Lounge checksum file, lower-cased.
///
/// Apache Lounge has used two layouts: `SHA256: <digest>`, and (since late
/// 2026) `SHA256-Checksum for: <file>:` with the digest on the next line. Read
/// the digest itself rather than the label, which is the part that changes.
fn apache_sha256(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .find(|token| token.len() == 64)
        .map(str::to_ascii_lowercase)
}

/// The Mailpit asset name for this platform, e.g. `mailpit-windows-amd64.zip`.
fn mailpit_asset() -> Option<String> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return None;
    };
    Some(format!("mailpit-{os}-{arch}.{}", if cfg!(windows) { "zip" } else { "tar.gz" }))
}

/// Every recent Mailpit release that has a build for this platform and a digest.
pub fn mailpit() -> Result<Vec<Available>> {
    let asset = mailpit_asset().ok_or_else(|| DevdError::unsupported("Mailpit publishes no build for this platform"))?;
    let body = download::fetch_text(MAILPIT_API, TIMEOUT)?;
    mailpit_from(&body, &asset)
}

fn mailpit_from(json_text: &str, asset: &str) -> Result<Vec<Available>> {
    let releases: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|e| DevdError::internal(format!("the Mailpit release list is not JSON: {e}")))?;
    let mut list = Vec::new();
    for release in releases.as_array().into_iter().flatten() {
        if release["prerelease"].as_bool().unwrap_or(false) || release["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(tag) = release["tag_name"].as_str() else { continue };
        let version = tag.trim_start_matches('v').to_string();
        for file in release["assets"].as_array().into_iter().flatten() {
            if file["name"].as_str() != Some(asset) {
                continue;
            }
            let digest = file["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_ascii_lowercase());
            let (Some(url), Some(sha256)) = (file["browser_download_url"].as_str(), digest) else { continue };
            list.push(Available {
                kind: "mailpit".to_string(),
                series: devd_runtime::catalogue::series_of(&version),
                version: version.clone(),
                channel: "stable".to_string(),
                url: url.to_string(),
                sha256: Some(sha256),
                bytes: file["size"].as_u64(),
                published: release["published_at"].as_str().map(str::to_string),
                executable: if cfg!(windows) { "mailpit.exe" } else { "mailpit" }.to_string(),
            });
        }
    }
    sort_newest_first(&mut list);
    Ok(list)
}

/// The releases of a GitHub project that have an asset the matcher accepts and a recorded digest.
/// `version_of` turns the release tag and the asset name into a version, or rejects the asset.
fn github_assets(
    json_text: &str,
    kind: &str,
    executable: &str,
    accept: &dyn Fn(&str) -> bool,
    version_of: &dyn Fn(&str, &str) -> Option<String>,
) -> Result<Vec<Available>> {
    let releases: serde_json::Value = serde_json::from_str(json_text)
        .map_err(|e| DevdError::internal(format!("the {kind} release list is not JSON: {e}")))?;
    let mut list = Vec::new();
    for release in releases.as_array().into_iter().flatten() {
        if release["prerelease"].as_bool().unwrap_or(false) || release["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(tag) = release["tag_name"].as_str() else { continue };
        for file in release["assets"].as_array().into_iter().flatten() {
            let Some(name) = file["name"].as_str().filter(|n| accept(n)) else { continue };
            let Some(version) = version_of(tag, name) else { continue };
            let digest = file["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_ascii_lowercase());
            let (Some(url), Some(sha256)) = (file["browser_download_url"].as_str(), digest) else { continue };
            list.push(Available {
                kind: kind.to_string(),
                series: devd_runtime::catalogue::series_of(&version),
                version,
                channel: "stable".to_string(),
                url: url.to_string(),
                sha256: Some(sha256),
                bytes: file["size"].as_u64(),
                published: release["published_at"].as_str().map(str::to_string),
                executable: executable.to_string(),
            });
        }
    }
    sort_newest_first(&mut list);
    Ok(list)
}

fn redis_from(json_text: &str) -> Result<Vec<Available>> {
    github_assets(
        json_text,
        "redis",
        "redis-server.exe",
        // The plain MSYS2 build: no Windows service wrapper, which devd's own supervisor replaces.
        &|n| n.starts_with("Redis-") && n.ends_with("-Windows-x64-msys2.zip"),
        &|tag, _| Some(tag.trim_start_matches('v').to_string()),
    )
}

/// Redis for Windows, from the project that builds it.
pub fn redis() -> Result<Vec<Available>> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    redis_from(&download::fetch_text(REDIS_API, TIMEOUT)?)
}

fn memcached_from(json_text: &str) -> Result<Vec<Available>> {
    github_assets(
        json_text,
        "memcached",
        "bin/memcached.exe",
        &|n| n.starts_with("memcached-") && n.ends_with("-win64-mingw.zip"),
        &|_, name| name.strip_prefix("memcached-")?.split("-win64").next().map(str::to_string),
    )
}

/// memcached for Windows.
pub fn memcached() -> Result<Vec<Available>> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    memcached_from(&download::fetch_text(MEMCACHED_API, TIMEOUT)?)
}

pub fn nginx() -> Result<Vec<Available>> {
    let html = download::fetch_text(NGINX_DOWNLOAD, TIMEOUT)?;
    let mut list = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for name in scan_versions(&html, "nginx-", ".zip") {
        if !seen.insert(name.clone()) {
            continue;
        }
        let version = name
            .trim_start_matches("nginx-")
            .trim_end_matches(".zip")
            .to_string();
        let minor = version
            .split('.')
            .nth(1)
            .and_then(|m| m.parse::<u32>().ok())
            .unwrap_or(0);
        list.push(Available {
            kind: "nginx".to_string(),
            version: version.clone(),
            series: devd_runtime::catalogue::series_of(&version),
            channel: if minor % 2 == 0 { "stable" } else { "mainline" }.to_string(),
            url: format!("{NGINX_BASE}{name}"),
            // nginx publishes a PGP signature, not a checksum file. devd
            // checks the archive against the digest it recorded.
            sha256: nginx_digest(&version),
            bytes: None,
            published: None,
            executable: if cfg!(windows) {
                "nginx.exe"
            } else {
                "sbin/nginx"
            }
            .to_string(),
        });
    }

    sort_newest_first(&mut list);
    Ok(list)
}

/// Find `<prefix><major>.<minor>.<patch><suffix>` occurrences in a document.
///
/// The vendor listings are HTML, and the only thing needed from them is the set
/// of release file names, so this scans rather than parses. A regular-expression
/// crate is not worth a dependency for one shape. The version has to be exactly
/// three dot-separated digit groups, which is what keeps `nginx-1.28.3.zip` from
/// being read as a version ending in a dot.
fn scan_versions(document: &str, prefix: &str, suffix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = document;
    while let Some(start) = rest.find(prefix) {
        // Advancing past the prefix first guarantees progress even when the text
        // that follows is not a version.
        let after = &rest[start + prefix.len()..];
        rest = after;

        let candidate: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let version = candidate.trim_end_matches('.');
        let parts: Vec<&str> = version.split('.').collect();
        let is_release = parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        if !is_release || !after[version.len()..].starts_with(suffix) {
            continue;
        }
        out.push(format!("{prefix}{version}{suffix}"));
    }
    out
}

/// Every MariaDB release that has a Windows package, newest first.
///
/// Only Stable series are offered. A preview or release-candidate series is
/// read from the same API and skipped, because MariaDB does not publish a
/// Windows zip for every one of them and an install that can only fail is not a
/// choice worth offering.
pub fn mariadb() -> Result<Vec<Available>> {
    let series = mariadb_stable_series()?;

    // One request per stable series, and MariaDB currently marks six of them
    // Stable. Fetched in sequence that is several seconds of TLS handshakes for
    // a listing, so they are read at the same time; a series that fails fails
    // the catalogue rather than silently shortening it.
    let mut handles = Vec::new();
    for name in series {
        handles.push(std::thread::spawn(move || mariadb_series(&name)));
    }

    let mut list = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for handle in handles {
        let releases = handle
            .join()
            .map_err(|_| DevdError::internal("a MariaDB catalogue reader panicked"))??;
        for release in releases {
            if seen.insert(release.version.clone()) {
                list.push(release);
            }
        }
    }

    sort_newest_first(&mut list);
    Ok(list)
}

/// Every release in one MariaDB series that has a Windows package.
fn mariadb_series(series: &str) -> Result<Vec<Available>> {
    let url = format!("{MARIADB_API}{series}/");
    let body = download::fetch_text(&url, TIMEOUT)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| DevdError::internal(format!("{url} did not return JSON: {e}")))?;

    let mut out = Vec::new();
    let Some(releases) = json.get("releases").and_then(|v| v.as_object()) else {
        return Ok(out);
    };

    for (version, release) in releases {
        let floor: Vec<u32> = version
            .split('.')
            .take(2)
            .filter_map(|p| p.parse::<u32>().ok())
            .collect();
        if floor.len() != 2 || (floor[0], floor[1]) < MARIADB_FLOOR {
            continue;
        }

        // The release exists; the Windows build may not. This is the check the
        // archive's directory listing cannot make.
        let filename = format!("mariadb-{version}-winx64.zip");
        let Some(file) = release
            .get("files")
            .and_then(|v| v.as_array())
            .and_then(|files| {
                files.iter().find(|f| {
                    f.get("file_name").and_then(|v| v.as_str()) == Some(filename.as_str())
                })
            })
        else {
            continue;
        };

        out.push(Available {
            kind: "mariadb".to_string(),
            version: version.clone(),
            series: devd_runtime::catalogue::series_of(version),
            channel: "stable".to_string(),
            url: format!("{MARIADB_BASE}mariadb-{version}/winx64-packages/{filename}"),
            // Published per file, so the digest is known before the download
            // rather than read from a sibling checksum file afterwards.
            sha256: file
                .get("checksum")
                .and_then(|c| c.get("sha256sum"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_lowercase()),
            bytes: None,
            published: release
                .get("date_of_release")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            executable: if cfg!(windows) {
                "bin/mysqld.exe"
            } else {
                "bin/mariadbd"
            }
            .to_string(),
        });
    }

    Ok(out)
}

/// The major series MariaDB currently marks Stable, newest first.
fn mariadb_stable_series() -> Result<Vec<String>> {
    let body = download::fetch_text(MARIADB_API, TIMEOUT)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| DevdError::internal(format!("{MARIADB_API} did not return JSON: {e}")))?;

    let mut series = Vec::new();
    if let Some(releases) = json.get("major_releases").and_then(|v| v.as_array()) {
        for release in releases {
            let status = release
                .get("release_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id = release
                .get("release_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if status.eq_ignore_ascii_case("stable") && !id.is_empty() {
                series.push(id.to_string());
            }
        }
    }

    if series.is_empty() {
        return Err(DevdError::invalid(
            "MariaDB's release list named no stable series, which would make the \
             catalogue silently empty",
        ));
    }
    // The API lists newest first, but sorting here keeps that from being an
    // assumption the rest of the file depends on.
    series.sort_by(|a, b| {
        devd_runtime::catalogue::version_key(b).cmp(&devd_runtime::catalogue::version_key(a))
    });
    Ok(series)
}

/// The digest MariaDB publishes for one Windows package.
pub fn mariadb_digest(version: &str, filename: &str) -> Result<Option<String>> {
    let url = format!("{MARIADB_BASE}mariadb-{version}/winx64-packages/sha256sums.txt");
    let body = download::fetch_text(&url, TIMEOUT)?;
    for line in body.lines() {
        // `b7c11d...  ./mariadb-11.4.5-winx64.zip`
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let name = name
            .trim_start_matches("./")
            .rsplit('/')
            .next()
            .unwrap_or(name);
        if name == filename {
            return Ok(Some(digest.to_lowercase()));
        }
    }
    Ok(None)
}

/// Newest first, using the shared version ordering.
fn sort_newest_first(list: &mut [Available]) {
    list.sort_by(|a, b| {
        devd_runtime::catalogue::version_key(&b.version)
            .cmp(&devd_runtime::catalogue::version_key(&a.version))
            .then_with(|| a.channel.cmp(&b.channel))
    });
}

/// The catalogue for a service kind, with a clear error for anything else.
pub fn for_kind(kind: &str) -> Result<Vec<Available>> {
    match kind {
        "nginx" => {
            if !cfg!(windows) {
                // nginx publishes source for Linux; every distribution builds
                // its own package. `devd` uses what the system has.
                return Ok(Vec::new());
            }
            nginx()
        }
        "mariadb" => {
            if !cfg!(windows) {
                return Ok(Vec::new());
            }
            mariadb()
        }
        "apache" => {
            if !cfg!(windows) {
                return Ok(Vec::new());
            }
            apache()
        }
        "mailpit" => mailpit(),
        "redis" => redis(),
        "memcached" => memcached(),
        other => Err(DevdError::unsupported(format!(
            "`{other}` has no upstream catalogue in devd; installable services are nginx, apache, mariadb, redis, memcached and mailpit"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_names_are_scanned_out_of_html() {
        let html = r#"
            <a href="/download/nginx-1.28.3.zip">nginx/Windows-1.28.3</a>
            <a href="/download/nginx-1.31.7.zip">nginx/Windows-1.31.7</a>
            <a href="/download/nginx-1.28.3.zip">duplicate</a>
            <a href="/download/nginx-1.2.9.zip">old</a>
            <a href="/en/docs/">docs</a>
            <a href="/download/nginx-2.0.zip">not a release</a>
        "#;
        let found = scan_versions(html, "nginx-", ".zip");
        assert_eq!(
            found,
            vec![
                "nginx-1.28.3.zip",
                "nginx-1.31.7.zip",
                "nginx-1.28.3.zip",
                "nginx-1.2.9.zip",
            ]
        );
    }

    #[test]
    fn scan_ignores_a_version_without_the_suffix() {
        let html = "nginx-1.28.3.tar.gz and nginx-1.28.3.zip";
        assert_eq!(
            scan_versions(html, "nginx-", ".zip"),
            vec!["nginx-1.28.3.zip"]
        );
    }

    #[test]
    fn stable_and_mainline_follow_nginx_numbering() {
        // 1.28 is a stable branch, 1.31 is mainline. This is nginx's own rule.
        let mut list = vec![make("1.28.3", "stable"), make("1.31.7", "mainline")];
        sort_newest_first(&mut list);
        assert_eq!(list[0].version, "1.31.7");
        assert_eq!(list[1].channel, "stable");
    }

    fn make(version: &str, channel: &str) -> Available {
        Available {
            kind: "nginx".to_string(),
            version: version.to_string(),
            series: devd_runtime::catalogue::series_of(version),
            channel: channel.to_string(),
            url: format!("https://nginx.org/download/nginx-{version}.zip"),
            sha256: None,
            bytes: None,
            published: None,
            executable: "nginx.exe".to_string(),
        }
    }

    #[test]
    fn unknown_kinds_are_refused() {
        let err = for_kind("lighttpd").unwrap_err();
        assert_eq!(err.code(), "unsupported");
        assert!(err.to_string().contains("nginx, apache, mariadb, redis, memcached and mailpit"), "{err}");
    }

    #[test]
    fn redis_and_memcached_are_offered_only_with_a_recorded_digest() {
        let redis = r#"[{"tag_name":"8.10.2","prerelease":false,"published_at":"2026-09-18T01:28:52Z","assets":[
            {"name":"Redis-8.10.2-Windows-x64-msys2.zip","size":13938657,"browser_download_url":"https://x/r.zip","digest":"sha256:7C8CEBD50347EABA"},
            {"name":"Redis-8.10.2-Windows-x64-cygwin.zip","size":1,"browser_download_url":"https://x/c.zip","digest":"sha256:AA"}]},
            {"tag_name":"5.0.14.1","prerelease":false,"assets":[{"name":"Redis-x64-5.0.14.1.zip","size":1,"browser_download_url":"https://x/o.zip","digest":null}]}]"#;
        let list = redis_from(redis).unwrap();
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].version, "8.10.2");
        assert_eq!(list[0].sha256.as_deref(), Some("7c8cebd50347eaba"));
        let mc = r#"[{"tag_name":"1.6.8_mingw_libressl","prerelease":false,"assets":[
            {"name":"memcached-1.6.8-win64-mingw.zip","size":3540169,"browser_download_url":"https://x/m.zip","digest":"sha256:48EC"},
            {"name":"hashes.txt","size":1,"browser_download_url":"https://x/h","digest":"sha256:67"}]},
            {"tag_name":"1.6.8_mingw","prerelease":false,"assets":[{"name":"memcached-1.6.8-win64-mingw.zip","size":1,"browser_download_url":"https://x/old.zip","digest":null}]}]"#;
        let list = memcached_from(mc).unwrap();
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!(list[0].version, "1.6.8");
        assert_eq!(list[0].executable, "bin/memcached.exe");
    }

    #[test]
    fn mailpit_is_chosen_by_platform_and_only_with_a_recorded_digest() {
        let json = r#"[
          {"tag_name":"v1.31.6","prerelease":false,"published_at":"2026-10-01T00:00:00Z","assets":[
            {"name":"mailpit-windows-amd64.zip","size":10640632,"browser_download_url":"https://github.com/axllent/mailpit/releases/download/v1.31.6/mailpit-windows-amd64.zip","digest":"sha256:5D5AD094BFB3BAC9004F9D1105D2335FEA9F8952C09951DF73AE0A280E34D3B1"},
            {"name":"mailpit-linux-amd64.tar.gz","size":1,"browser_download_url":"https://x/y","digest":null}]},
          {"tag_name":"v1.32.0-rc1","prerelease":true,"assets":[]}]"#;
        let win = mailpit_from(json, "mailpit-windows-amd64.zip").unwrap();
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].version, "1.31.6");
        assert_eq!(win[0].sha256.as_deref(), Some("5d5ad094bfb3bac9004f9d1105d2335fea9f8952c09951df73ae0a280e34d3b1"));
        assert!(mailpit_from(json, "mailpit-linux-amd64.tar.gz").unwrap().is_empty(), "no digest, no install");
    }

    #[test]
    fn the_apache_page_yields_the_64_bit_archive_and_not_its_signature() {
        let page = r#"<a href="/download/VS18/binaries/httpd-2.4.69-261002-Win64-VS18.zip">zip</a>
            <a href="/download/VS18/binaries/httpd-2.4.69-261002-Win64-VS18.zip.asc">sig</a>
            <a href="/download/VS18/binaries/httpd-2.4.69-261002-Win64-VS18.zip.txt">sums</a>
            <a href="/download/VS18/binaries/httpd-2.4.69-261002-win32-vs18.zip">32</a>"#;
        let found = scan_apache(page);
        assert_eq!(found, vec![("2.4.69".to_string(), "/download/VS18/binaries/httpd-2.4.69-261002-Win64-VS18.zip".to_string())]);
    }

    #[test]
    fn the_sha256_is_read_from_apache_lounges_checksum_file() {
        let text = "File name: httpd-2.4.69-261002-Win64-VS18.zip\nSHA1:   D7B557581787727CA222C6A5FF0E9E6FA6029893\nSHA256: CA272E0B75ABAD78B1E4029A0753B20FF2232D2D857E8EEBEEA5D469C5776977\nSHA384: 6C32\n";
        assert_eq!(apache_sha256(text).as_deref(), Some("ca272e0b75abad78b1e4029a0753b20ff2232d2d857e8eebeea5d469c5776977"));
        let current = "Checksums created with GPGHash by SmartJuwel\n\nSHA1-Checksum for: httpd.zip:\nD7B557581787727CA222C6A5FF0E9E6FA6029893\n\nSHA256-Checksum for: httpd.zip:\nCA272E0B75ABAD78B1E4029A0753B20FF2232D2D857E8EEBEEA5D469C5776977\n\nSHA512-Checksum for: httpd.zip:\nBDB569BC13B02E5F5227CA1DC168AA0907FA6D3422314CE242C121647044F67B5A72EA0E15A2FC14F706739A2376B1F7851AABCC7DFD843A881EA8F12B2E71B0\n";
        assert_eq!(apache_sha256(current).as_deref(), Some("ca272e0b75abad78b1e4029a0753b20ff2232d2d857e8eebeea5d469c5776977"));
        assert_eq!(apache_sha256("SHA256: short"), None);
    }

    #[test]
    fn live_nginx_catalogue_lists_windows_builds() {
        if std::env::var("DEVD_SKIP_NETWORK_TESTS").is_ok() {
            return;
        }
        let Ok(list) = nginx() else {
            eprintln!("skipping: nginx.org was unreachable");
            return;
        };
        if list.is_empty() {
            return;
        }
        assert!(list
            .iter()
            .all(|a| a.url.starts_with("https://nginx.org/download/")));
        assert!(
            list.iter().any(|a| a.channel == "stable"),
            "the stable branch should be represented: {list:?}"
        );
    }
}
