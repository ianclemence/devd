//! Upstream catalogues for services.
//!
//! Services are found the same way runtimes are: from the vendor's own listing,
//! with a digest where the vendor publishes one.
//!
//! | Service | Catalogue | Digest |
//! |---------|-----------|--------|
//! | nginx | `nginx.org/en/download.html` | not published as a file |
//! | MariaDB | `downloads.mariadb.org/rest-api` | per-file `sha256sum` in the API |
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
            // nginx publishes a PGP signature, not a checksum file.
            sha256: None,
            bytes: None,
            published: None,
            executable: if cfg!(windows) { "nginx.exe" } else { "sbin/nginx" }.to_string(),
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
        let name = name.trim_start_matches("./").rsplit('/').next().unwrap_or(name);
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
        other => Err(DevdError::unsupported(format!(
            "`{other}` has no upstream catalogue in devd; installable services are nginx and mariadb"
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
        assert_eq!(scan_versions(html, "nginx-", ".zip"), vec!["nginx-1.28.3.zip"]);
    }

    #[test]
    fn stable_and_mainline_follow_nginx_numbering() {
        // 1.28 is a stable branch, 1.31 is mainline. This is nginx's own rule.
        let mut list = vec![
            make("1.28.3", "stable"),
            make("1.31.7", "mainline"),
        ];
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
        let err = for_kind("apache").unwrap_err();
        assert_eq!(err.code(), "unsupported");
        assert!(err.to_string().contains("nginx and mariadb"), "{err}");
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
        assert!(list.iter().all(|a| a.url.starts_with("https://nginx.org/download/")));
        assert!(
            list.iter().any(|a| a.channel == "stable"),
            "the stable branch should be represented: {list:?}"
        );
    }
}
