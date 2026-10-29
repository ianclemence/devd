//! Which devd process is the resident one, and how anything else finds it.
//!
//! One long-lived process per user owns the background work: cron, backups,
//! scheduled deploys, health checks, log rotation, and the local API the window,
//! the terminal dashboard and the command line talk to. Everything else is a
//! client. This module is the single answer to "is that process running, and
//! where do I reach it".
//!
//! The answer is a record file written with an exclusive create, so two
//! processes cannot both believe they are the resident one. A record is trusted
//! only when the process it names is alive, is still the process that wrote it
//! (the same image and the same start time), so a recycled process id or a
//! crashed daemon's leftover file is never mistaken for a live one. No port is
//! probed to find it: the record says which port it listens on.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::process;
use crate::{util, DevdError, Result};

/// What a running devd process records about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// Process id.
    pub pid: u32,
    /// Opaque start token of the process; see [`process::process_start_token`].
    pub started: String,
    /// Image name of the process.
    pub image: String,
    /// Loopback port of its local API. `0` until the listener is bound.
    pub port: u16,
    /// Version of the program.
    pub version: String,
    /// `daemon` for the resident process, `ui` for a window's own server.
    pub role: String,
    /// When the record was written, RFC 3339.
    pub since: String,
}

impl Instance {
    /// The record for this process.
    pub fn this(port: u16, version: &str, role: &str) -> Result<Self> {
        let pid = std::process::id();
        Ok(Self {
            pid,
            started: process::process_start_token(pid)
                .ok_or_else(|| DevdError::internal("cannot read this process's start time"))?,
            image: process::process_image(pid).unwrap_or_default(),
            port,
            version: version.to_string(),
            role: role.to_string(),
            since: util::now(),
        })
    }

    /// Whether the process this record names is still the one that wrote it.
    pub fn is_live(&self) -> bool {
        process::pid_alive(self.pid)
            && process::process_start_token(self.pid).as_deref() == Some(self.started.as_str())
            && process::process_image(self.pid).unwrap_or_default() == self.image
    }
}

fn record_path(paths: &Paths) -> PathBuf {
    paths.root().join("instance.json")
}

fn read_record(paths: &Paths) -> Option<Instance> {
    let text = std::fs::read_to_string(record_path(paths)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The live resident process, if there is one.
///
/// A record whose process is gone, or whose process id now belongs to something
/// else, is not an instance. It is left on disk for the next [`claim`] to
/// replace.
pub fn current(paths: &Paths) -> Option<Instance> {
    read_record(paths).filter(Instance::is_live)
}

/// Proof that this process holds the instance record. Dropping it releases the
/// record, but only if the record still names this process.
#[derive(Debug)]
pub struct Claim {
    paths: Paths,
    instance: Instance,
}

impl Claim {
    /// Record the port once the listener is bound.
    pub fn set_port(&mut self, port: u16) -> Result<()> {
        self.instance.port = port;
        write_record(&self.paths, &self.instance)
    }

    /// What was recorded.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if read_record(&self.paths).is_some_and(|r| r.pid == self.instance.pid) {
            let _ = std::fs::remove_file(record_path(&self.paths));
        }
    }
}

fn write_record(paths: &Paths, instance: &Instance) -> Result<()> {
    let text =
        serde_json::to_string_pretty(instance).map_err(|e| DevdError::internal(e.to_string()))?;
    util::atomic_write(&record_path(paths), &text)
}

/// Become the instance, or learn who already is.
///
/// The create is exclusive, so of two simultaneous callers exactly one wins. A
/// stale record is removed and the create retried once; a live one is returned
/// as a conflict that names the holder.
pub fn claim(paths: &Paths, port: u16, version: &str, role: &str) -> Result<Claim> {
    use std::io::Write;

    let instance = Instance::this(port, version, role)?;
    let path = record_path(paths);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let text = serde_json::to_string_pretty(&instance)
                    .map_err(|e| DevdError::internal(e.to_string()))?;
                file.write_all(text.as_bytes())?;
                return Ok(Claim {
                    paths: paths.clone(),
                    instance,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some(held) = current(paths) {
                    return Err(DevdError::conflict(format!(
                        "devd {} (pid {}) is already running as the {} on port {}",
                        held.version, held.pid, held.role, held.port
                    )));
                }
                // Unreadable or stale. Replace it and try again exactly once.
                if attempt == 0 {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                return Err(DevdError::conflict(
                    "another devd process is claiming the instance record; try again",
                ));
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(DevdError::internal("could not claim the instance record"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path());
        (dir, paths)
    }

    #[test]
    fn one_process_holds_the_record_and_a_second_claim_names_it() {
        let (_dir, paths) = paths();
        let mut first = claim(&paths, 0, "9.9.9", "daemon").unwrap();
        first.set_port(4242).unwrap();
        let seen = current(&paths).expect("the claim is visible");
        assert_eq!(seen.port, 4242);
        assert_eq!(seen.pid, std::process::id());

        let err = claim(&paths, 0, "9.9.9", "ui").unwrap_err();
        assert!(matches!(err, DevdError::Conflict(_)));
        assert!(err.to_string().contains("4242"), "{err}");
    }

    #[test]
    fn dropping_the_claim_releases_it() {
        let (_dir, paths) = paths();
        drop(claim(&paths, 1, "1", "daemon").unwrap());
        assert!(current(&paths).is_none());
        assert!(claim(&paths, 2, "1", "daemon").is_ok());
    }

    #[test]
    fn a_record_of_a_dead_or_recycled_process_is_replaced() {
        let (_dir, paths) = paths();
        // A crashed daemon: right pid shape, wrong start time.
        let mut ghost = Instance::this(5000, "0", "daemon").unwrap();
        ghost.started = "not-the-start-time".into();
        write_record(&paths, &ghost).unwrap();
        assert!(
            current(&paths).is_none(),
            "a recycled pid is not an instance"
        );
        let taken = claim(&paths, 6000, "1", "daemon").unwrap();
        assert_eq!(taken.instance().port, 6000);

        // A record naming a pid that does not exist is stale too.
        drop(taken);
        let mut gone = Instance::this(5000, "0", "daemon").unwrap();
        gone.pid = u32::MAX - 3;
        write_record(&paths, &gone).unwrap();
        assert!(current(&paths).is_none());
        assert!(claim(&paths, 7000, "1", "daemon").is_ok());
    }

    #[test]
    fn a_corrupt_record_does_not_block_a_new_instance() {
        let (_dir, paths) = paths();
        std::fs::write(record_path(&paths), "{ not json").unwrap();
        assert!(current(&paths).is_none());
        assert!(claim(&paths, 1, "1", "daemon").is_ok());
    }
}
