//! Archive extraction.
//!
//! Every distribution `devd` installs - the nginx Windows build, the PHP zip,
//! the MariaDB zip, the Linux Node and Go tarballs - is a single archive with a
//! single version-stamped top-level directory. Unpacking that directly onto the
//! target would give `runtimes/php/8.3.35/php-8.3.35-nts-Win32-vs16-x64/php.exe`,
//! so this module does two things the standard libraries do not:
//!
//! 1. **Strips the single common root.** `nginx-1.28.0/html` becomes `html`.
//! 2. **Installs atomically.** The archive is unpacked into a staging directory
//!    next to the target and moved into place only once it is complete, so an
//!    interrupted install cannot leave a half-populated runtime that `devd`
//!    would later report as present.
//!
//! Zip-slip is handled by the archive crates themselves - `zip` through
//! `enclosed_name`, `tar` through `unpack_in` - and a refused entry is treated
//! as a failed install rather than skipped, because silently omitting files from
//! a distribution is how you get a runtime that half-works.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use serde::Serialize;

use crate::error::{DevdError, Result};

/// Which container an archive uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A `.zip` archive.
    Zip,
    /// A gzip-compressed tarball.
    TarGz,
    /// An uncompressed tarball.
    Tar,
}

impl Format {
    /// Detect the format from a file name.
    pub fn detect(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(".zip") {
            Ok(Self::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Ok(Self::TarGz)
        } else if name.ends_with(".tar") {
            Ok(Self::Tar)
        } else {
            Err(DevdError::unsupported(format!(
                "`{}` is not an archive devd can unpack (expected .zip, .tar.gz or .tar)",
                path.display()
            )))
        }
    }
}

/// What an extraction produced.
#[derive(Debug, Clone, Serialize)]
pub struct ExtractReport {
    /// Files written.
    pub files: usize,
    /// Directories created.
    pub directories: usize,
    /// Bytes written.
    pub bytes: u64,
    /// The stripped top-level directory, when there was exactly one.
    pub stripped_root: Option<String>,
}

/// Unpack `archive` so that its contents land directly in `dest`.
///
/// When `replace` is false an existing `dest` is a conflict, not something to
/// overwrite - removing a runtime the user depends on is a decision, not a side
/// effect of installing a new one.
pub fn extract(archive: &Path, dest: &Path, replace: bool) -> Result<ExtractReport> {
    if !archive.is_file() {
        return Err(DevdError::not_found(format!(
            "archive does not exist: {}",
            archive.display()
        )));
    }
    if dest.exists() {
        if !replace {
            return Err(DevdError::conflict(format!(
                "{} already exists",
                dest.display()
            )));
        }
        std::fs::remove_dir_all(dest)?;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Staging lives beside the target so that the final move is a rename on the
    // same volume, which is atomic; a temp directory on another drive would turn
    // the install into a copy that can be interrupted half way.
    let parent = dest
        .parent()
        .ok_or_else(|| DevdError::invalid("install target has no parent directory"))?;
    let staging = parent.join(format!(
        ".devd-extract-{}-{}",
        std::process::id(),
        crate::util::new_token().get(..8).unwrap_or("staging")
    ));
    std::fs::create_dir_all(&staging)?;

    let outcome = (|| -> Result<ExtractReport> {
        let format = Format::detect(archive)?;
        let mut report = match format {
            Format::Zip => extract_zip(archive, &staging)?,
            Format::TarGz => {
                let file = BufReader::new(File::open(archive)?);
                extract_tar(GzDecoder::new(file), &staging)?
            }
            Format::Tar => {
                let file = BufReader::new(File::open(archive)?);
                extract_tar(file, &staging)?
            }
        };

        let source = common_root(&staging)?;
        report.stripped_root = source
            .file_name()
            .filter(|_| source != staging)
            .map(|s| s.to_string_lossy().to_string());

        std::fs::rename(&source, dest)?;
        Ok(report)
    })();

    // Staging is removed whether or not the extraction worked; a failed install
    // must not leave a directory tree behind that a later run might trip over.
    let _ = std::fs::remove_dir_all(&staging);
    outcome
}

/// If the staging directory holds exactly one entry and it is a directory, that
/// directory is the distribution's own root and everything inside it belongs at
/// the target. Otherwise the whole staging directory is the content.
fn common_root(staging: &Path) -> Result<PathBuf> {
    let mut entries = std::fs::read_dir(staging)?.collect::<std::io::Result<Vec<_>>>()?;
    if entries.len() == 1 {
        let only = entries.remove(0);
        let path = only.path();
        if path.is_dir() {
            return Ok(path);
        }
    }
    Ok(staging.to_path_buf())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<ExtractReport> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        DevdError::invalid(format!("{} is not a readable zip: {e}", archive.display()))
    })?;

    let mut report = ExtractReport {
        files: 0,
        directories: 0,
        bytes: 0,
        stripped_root: None,
    };

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| {
            DevdError::invalid(format!("{}: entry {index}: {e}", archive.display()))
        })?;

        // `enclosed_name` returns `None` for absolute paths and paths that
        // escape the destination through `..`, which is the zip-slip guard.
        let Some(relative) = entry.enclosed_name() else {
            return Err(DevdError::invalid(format!(
                "{} contains an entry that escapes the install directory: {}",
                archive.display(),
                entry.name()
            )));
        };
        let target = dest.join(&relative);
        debug_assert!(target.starts_with(dest));

        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            report.directories += 1;
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        report.bytes += std::io::copy(&mut entry, &mut out)?;
        report.files += 1;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode));
            }
        }
    }

    Ok(report)
}

fn extract_tar<R: Read>(reader: R, dest: &Path) -> Result<ExtractReport> {
    let mut archive = tar::Archive::new(reader);
    let mut report = ExtractReport {
        files: 0,
        directories: 0,
        bytes: 0,
        stripped_root: None,
    };

    for entry in archive
        .entries()
        .map_err(|e| DevdError::invalid(format!("unreadable tarball: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| DevdError::invalid(format!("unreadable tarball: {e}")))?;
        let size = entry.header().size().unwrap_or(0);
        let is_dir = entry.header().entry_type().is_dir();

        // `unpack_in` validates the path and returns `false` when it refuses to
        // write outside `dest`. A distribution that contains such an entry is
        // not one to install quietly.
        let unpacked = entry
            .unpack_in(dest)
            .map_err(|e| DevdError::invalid(format!("could not unpack the tarball: {e}")))?;
        if !unpacked {
            return Err(DevdError::invalid(
                "the tarball contains an entry that escapes the install directory",
            ));
        }

        if is_dir {
            report.directories += 1;
        } else {
            report.files += 1;
            report.bytes += size;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devd-archive-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_with(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join(name);
        let file = File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (entry_name, body) in entries {
            writer.start_file(*entry_name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn detects_formats_by_name() {
        assert_eq!(
            Format::detect(Path::new("a/b/nginx-1.28.0.zip")).unwrap(),
            Format::Zip
        );
        assert_eq!(
            Format::detect(Path::new("node.tar.gz")).unwrap(),
            Format::TarGz
        );
        assert_eq!(
            Format::detect(Path::new("node.tgz")).unwrap(),
            Format::TarGz
        );
        assert_eq!(Format::detect(Path::new("go.tar")).unwrap(), Format::Tar);
        assert!(Format::detect(Path::new("php.exe")).is_err());
    }

    #[test]
    fn strips_the_single_top_level_directory() {
        let dir = scratch("strip");
        let archive = zip_with(
            &dir,
            "php.zip",
            &[
                ("php-8.3.35/php.exe", b"binary"),
                ("php-8.3.35/ext/php_curl.dll", b"extension"),
            ],
        );
        let dest = dir.join("install");
        let report = extract(&archive, &dest, false).unwrap();
        assert_eq!(report.stripped_root.as_deref(), Some("php-8.3.35"));
        assert_eq!(report.files, 2);
        assert!(dest.join("php.exe").is_file());
        assert!(dest.join("ext/php_curl.dll").is_file());
        assert!(!dest.join("php-8.3.35").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_the_tree_when_there_is_no_single_root() {
        let dir = scratch("multiroot");
        let archive = zip_with(
            &dir,
            "flat.zip",
            &[("bin/tool", b"x"), ("share/doc.txt", b"y")],
        );
        let dest = dir.join("install");
        let report = extract(&archive, &dest, false).unwrap();
        assert_eq!(report.stripped_root, None);
        assert!(dest.join("bin/tool").is_file());
        assert!(dest.join("share/doc.txt").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_target_is_a_conflict_unless_replacing() {
        let dir = scratch("replace");
        let archive = zip_with(&dir, "a.zip", &[("pkg/file.txt", b"one")]);
        let dest = dir.join("install");
        extract(&archive, &dest, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "one"
        );

        let err = extract(&archive, &dest, false).unwrap_err();
        assert_eq!(err.code(), "conflict");

        let archive2 = zip_with(&dir, "b.zip", &[("pkg/file.txt", b"two")]);
        extract(&archive2, &dest, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "two"
        );
        // Nothing from the previous contents survived a replace.
        assert!(!dest.join("file.txt").exists() || std::fs::read_dir(&dest).unwrap().count() == 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_slip_is_refused() {
        let dir = scratch("slip");
        let archive = zip_with(&dir, "evil.zip", &[("pkg/../../escaped.txt", b"no")]);
        let dest = dir.join("install");
        let err = extract(&archive, &dest, false).unwrap_err();
        assert!(err.to_string().contains("escapes"), "{err}");
        assert!(!dir.join("escaped.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_archive_is_not_found() {
        let dir = scratch("missing");
        let err = extract(&dir.join("nope.zip"), &dir.join("out"), false).unwrap_err();
        assert_eq!(err.code(), "not_found");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
