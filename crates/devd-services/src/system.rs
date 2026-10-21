//! nginx and MariaDB, adopted from the system's package manager.
//!
//! See [`devd_core::platform`] for why devd adopts rather than downloads on Linux
//! and macOS. This module knows which programs each service needs, how to read
//! their versions, and what has to exist next to them before devd can run them
//! as an ordinary user.

use std::path::{Path, PathBuf};

use devd_core::platform::{self, Adoption, Host, Want};
use devd_core::util;
use devd_core::{DevdError, Result};

/// nginx is one program.
const NGINX: &[Want] = &[Want {
    link: "sbin/nginx",
    names: &["nginx"],
}];

/// MariaDB is the server, the client devd uses to create databases, and the
/// program that creates the system tables. Distributions have renamed all three
/// over the years (`mysqld` became `mariadbd`), so each lists both names.
const MARIADB: &[Want] = &[
    Want {
        link: "bin/mariadbd",
        names: &["mariadbd", "mysqld"],
    },
    Want {
        link: "bin/mysql",
        names: &["mariadb", "mysql"],
    },
    Want {
        link: "bin/mariadb-install-db",
        names: &["mariadb-install-db", "mysql_install_db"],
    },
];

/// The standard `fastcgi_params`, used when the system's copy cannot be found.
const FASTCGI_PARAMS: &str = "\
fastcgi_param  QUERY_STRING       $query_string;
fastcgi_param  REQUEST_METHOD     $request_method;
fastcgi_param  CONTENT_TYPE       $content_type;
fastcgi_param  CONTENT_LENGTH     $content_length;

fastcgi_param  SCRIPT_NAME        $fastcgi_script_name;
fastcgi_param  REQUEST_URI        $request_uri;
fastcgi_param  DOCUMENT_URI       $document_uri;
fastcgi_param  DOCUMENT_ROOT      $document_root;
fastcgi_param  SERVER_PROTOCOL    $server_protocol;
fastcgi_param  REQUEST_SCHEME     $scheme;
fastcgi_param  HTTPS              $https if_not_empty;

fastcgi_param  GATEWAY_INTERFACE  CGI/1.1;
fastcgi_param  SERVER_SOFTWARE    nginx/$nginx_version;

fastcgi_param  REMOTE_ADDR        $remote_addr;
fastcgi_param  REMOTE_PORT        $remote_port;
fastcgi_param  SERVER_ADDR        $server_addr;
fastcgi_param  SERVER_PORT        $server_port;
fastcgi_param  SERVER_NAME        $server_name;

fastcgi_param  REDIRECT_STATUS    200;
";

/// A small `mime.types`, used when the system's copy cannot be found.
const MIME_TYPES: &str = "\
types {
    text/html                             html htm shtml;
    text/css                              css;
    text/plain                            txt;
    text/xml                              xml;
    application/javascript                js mjs;
    application/json                      json map;
    application/wasm                      wasm;
    application/pdf                       pdf;
    application/zip                       zip;
    image/gif                             gif;
    image/jpeg                            jpeg jpg;
    image/png                             png;
    image/svg+xml                         svg svgz;
    image/webp                            webp;
    image/avif                            avif;
    image/x-icon                          ico;
    font/woff                             woff;
    font/woff2                            woff2;
    font/ttf                              ttf;
    font/otf                              otf;
    audio/mpeg                            mp3;
    video/mp4                             mp4;
    video/webm                            webm;
}
";

/// Where the distributions keep nginx's stock configuration files.
fn system_conf_dirs() -> Vec<PathBuf> {
    [
        "/etc/nginx",
        "/usr/local/etc/nginx",
        "/opt/homebrew/etc/nginx",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// How to get a program that is missing.
fn hint(tool: &str) -> String {
    format!(
        "Install it with `devd toolchain install {tool}` (it shows what it will do and asks \
         first) or with your package manager, then run this again."
    )
}

/// Find the programs a service needs, and decide whether the package manager
/// still vouches for them.
pub fn adopt(host: &dyn Host, kind: &str) -> Result<Adoption> {
    match kind {
        "nginx" => platform::adopt(host, "nginx", &hint("nginx"), NGINX, &["-v"]),
        "mariadb" => {
            let adoption =
                platform::adopt(host, "MariaDB", &hint("mariadb"), MARIADB, &["--version"])?;
            // `mysqld` is MariaDB on some systems and Oracle's MySQL on others.
            // Running one while the person believes it is the other is how a
            // data directory gets initialised by the wrong server.
            if !adoption.banner.to_lowercase().contains("mariadb") {
                return Err(DevdError::invalid(format!(
                    "the database server on this system is not MariaDB (`{}`). Install \
                     MariaDB, or leave the database to the system.",
                    adoption.banner.trim()
                )));
            }
            Ok(adoption)
        }
        other => Err(DevdError::unsupported(format!(
            "there is no system package for the service `{other}`"
        ))),
    }
}

/// The version a program printed.
pub fn version_of(kind: &str, banner: &str) -> Option<String> {
    let pattern = match kind {
        "nginx" => r"nginx/(\d+\.\d+\.\d+)",
        // `mariadbd  Ver 11.4.2-MariaDB ...` and `mariadbd from 11.4.2-MariaDB ...`
        "mariadb" => r"(?:Ver|from) (\d+\.\d+\.\d+)",
        _ => return None,
    };
    util::extract_version(banner, Some(pattern))
}

/// Create what the adopted program needs beside it.
pub fn prepare(kind: &str, dir: &Path, adoption: &Adoption) -> Result<()> {
    match kind {
        "nginx" => prepare_nginx(dir, &system_conf_dirs()),
        "mariadb" => {
            // The server finds its message files and plugins relative to its
            // base directory, which is wherever the package put it - not devd's
            // install directory, which only holds links.
            util::atomic_write(&dir.join(".devd-system"), &adoption.prefix.to_string_lossy())?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The directories and files nginx needs under its prefix.
///
/// nginx on Windows ships these in its archive. A package keeps them in `/etc`,
/// where devd neither reads nor writes, so devd gives nginx its own: the
/// system's `mime.types` and `fastcgi_params` when they can be found, and
/// standard ones when they cannot.
pub fn prepare_nginx(dir: &Path, search: &[PathBuf]) -> Result<()> {
    for sub in [
        "conf",
        "logs",
        "temp/client_body",
        "temp/proxy",
        "temp/fastcgi",
        "temp/uwsgi",
        "temp/scgi",
    ] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    for (name, fallback) in [("mime.types", MIME_TYPES), ("fastcgi_params", FASTCGI_PARAMS)] {
        let wanted = search
            .iter()
            .map(|d| d.join(name))
            .find(|p| p.is_file())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| fallback.to_string());
        util::atomic_write(&dir.join("conf").join(name), &wanted)?;
    }
    Ok(())
}

/// The base directory recorded for a system MariaDB, when this is one.
pub fn system_basedir(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(".devd-system")).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nginx_prints_its_version_on_stderr_in_this_shape() {
        assert_eq!(
            version_of("nginx", "nginx version: nginx/1.24.0 (Ubuntu)\n").as_deref(),
            Some("1.24.0")
        );
        assert_eq!(version_of("nginx", "something else"), None);
    }

    #[test]
    fn both_mariadb_banners_are_read() {
        assert_eq!(
            version_of(
                "mariadb",
                "mariadbd  Ver 11.4.2-MariaDB-ubu2404 for debian-linux-gnu on x86_64\n"
            )
            .as_deref(),
            Some("11.4.2")
        );
        assert_eq!(
            version_of("mariadb", "mariadbd from 10.11.6-MariaDB, client 15.2\n").as_deref(),
            Some("10.11.6")
        );
        assert_eq!(
            version_of("mariadb", "mysqld  Ver 10.5.23-MariaDB-0+deb11u1 for debian-linux-gnu\n")
                .as_deref(),
            Some("10.5.23")
        );
    }

    #[test]
    fn nginx_gets_its_own_files_when_the_system_has_none() {
        let dir = tempfile::tempdir().unwrap();
        prepare_nginx(dir.path(), &[]).unwrap();
        let fastcgi = std::fs::read_to_string(dir.path().join("conf/fastcgi_params")).unwrap();
        assert!(fastcgi.contains("fastcgi_param  REDIRECT_STATUS    200;"));
        let mime = std::fs::read_to_string(dir.path().join("conf/mime.types")).unwrap();
        assert!(mime.contains("application/wasm"));
        for sub in ["logs", "temp/client_body", "temp/fastcgi"] {
            assert!(dir.path().join(sub).is_dir(), "{sub} was not created");
        }
    }

    #[test]
    fn nginx_prefers_the_systems_own_files() {
        let system = tempfile::tempdir().unwrap();
        std::fs::write(system.path().join("mime.types"), "types { x/y z; }").unwrap();
        let dir = tempfile::tempdir().unwrap();
        prepare_nginx(dir.path(), &[system.path().to_path_buf()]).unwrap();
        let mime = std::fs::read_to_string(dir.path().join("conf/mime.types")).unwrap();
        assert_eq!(mime, "types { x/y z; }");
        // The file the system does not have still gets the standard one.
        assert!(dir.path().join("conf/fastcgi_params").is_file());
    }

    #[test]
    fn preparing_again_does_not_fail_or_change_anything_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        prepare_nginx(dir.path(), &[]).unwrap();
        let first = std::fs::read(dir.path().join("conf/mime.types")).unwrap();
        prepare_nginx(dir.path(), &[]).unwrap();
        assert_eq!(std::fs::read(dir.path().join("conf/mime.types")).unwrap(), first);
    }

    #[test]
    fn the_system_base_directory_is_recorded_for_mariadb_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(system_basedir(dir.path()).is_none());
        std::fs::write(dir.path().join(".devd-system"), "/usr\n").unwrap();
        assert_eq!(system_basedir(dir.path()), Some(PathBuf::from("/usr")));
    }
}
