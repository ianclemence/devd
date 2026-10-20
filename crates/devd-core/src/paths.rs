//! Filesystem layout for `devd`.
//!
//! Everything `devd` owns lives under a single root so that the "move the
//! folder and the whole stack moves with it" property Laragon users expect is
//! preserved. The root is `%LOCALAPPDATA%\devd` on Windows and
//! `$XDG_DATA_HOME/devd` (or `~/.local/share/devd`) on Linux, and can be
//! overridden with `DEVD_HOME` for portable or test use.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// Resolved locations of every directory and file `devd` owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Resolve the layout from the environment.
    ///
    /// Order of preference:
    /// 1. `DEVD_HOME` - explicit override, used for portable installs and tests.
    /// 2. Windows: `%LOCALAPPDATA%\devd`.
    /// 3. Linux: `$XDG_DATA_HOME/devd`, else `~/.local/share/devd`.
    /// 4. Current directory fallback (`.devd`) if none of the above exist.
    pub fn discover() -> Result<Self> {
        if let Some(v) = std::env::var_os("DEVD_HOME") {
            let p = PathBuf::from(v);
            if !p.as_os_str().is_empty() {
                return Ok(Self { root: p });
            }
        }

        let root = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("devd"))
                .unwrap_or_else(|| PathBuf::from(".devd"))
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .map(|p| p.join("devd"))
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/devd"))
                })
                .unwrap_or_else(|| PathBuf::from(".devd"))
        };

        Ok(Self { root })
    }

    /// Build a layout rooted at an explicit directory.
    pub fn rooted(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The layout root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Installed runtime builds: `runtimes/<kind>/<version>/`.
    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    /// Directory holding one folder per runtime kind.
    pub fn runtime_kind(&self, kind: &str) -> PathBuf {
        self.runtimes().join(kind)
    }

    /// Directory holding one folder per installed version of a runtime kind.
    pub fn runtime_version(&self, kind: &str, version: &str) -> PathBuf {
        self.runtime_kind(kind).join(version)
    }

    /// Generated service configuration and supervision files.
    pub fn services(&self) -> PathBuf {
        self.root.join("services")
    }

    /// Generated web-server and interpreter configuration.
    pub fn etc(&self) -> PathBuf {
        self.root.join("etc")
    }

    /// Local certificate authority and issued leaves.
    pub fn certs(&self) -> PathBuf {
        self.root.join("certs")
    }

    /// The directory projects are dropped into (Laragon's `www`).
    pub fn www(&self) -> PathBuf {
        self.root.join("www")
    }

    /// Log output.
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Bounded transcripts for agent sessions, one file per session.
    pub fn sessions(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Supervision data for project processes: logs and pid records.
    pub fn projects(&self) -> PathBuf {
        self.root.join("projects")
    }

    /// Download staging area; safe to delete at any time.
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Installation receipts used by uninstall and rollback.
    pub fn receipts(&self) -> PathBuf {
        self.root.join("receipts")
    }

    /// Snapshots taken before sensitive operations.
    pub fn snapshots(&self) -> PathBuf {
        self.root.join("snapshots")
    }

    /// The SQLite state database.
    pub fn state_db(&self) -> PathBuf {
        self.root.join("state.db")
    }

    /// The user settings file.
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    /// Create every directory in the layout, if missing.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.root.clone(),
            self.runtimes(),
            self.services(),
            self.etc(),
            self.certs(),
            self.www(),
            self.logs(),
            self.cache(),
            self.receipts(),
            self.snapshots(),
            self.sessions(),
            self.projects(),
        ] {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_paths_are_under_root() {
        let p = Paths::rooted("C:/tmp/devd-home");
        assert_eq!(
            p.state_db(),
            PathBuf::from("C:/tmp/devd-home").join("state.db")
        );
        assert!(p.runtimes().starts_with(p.root()));
        assert!(
            p.runtime_version("php", "8.3.14")
                .ends_with("php/8.3.14".replace('/', std::path::MAIN_SEPARATOR_STR))
                || p.runtime_version("php", "8.3.14")
                    .to_string_lossy()
                    .contains("php")
        );
    }

    #[test]
    fn env_override_wins() {
        std::env::set_var("DEVD_HOME", "D:/portable/devd");
        let p = Paths::discover().unwrap();
        assert_eq!(p.root(), Path::new("D:/portable/devd"));
        std::env::remove_var("DEVD_HOME");
    }
}
