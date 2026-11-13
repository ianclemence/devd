//! Which process owns the agents.
//!
//! Conversations and team runs are driven by threads in one process, and their
//! state lives in the shared database. Any devd command builds the same
//! registry, so a one-line `devd provider list` run beside the app must not treat
//! the app's live conversations as orphans and restart them. Exactly one process,
//! the first long-lived one to claim it, recovers what a previous process left
//! behind. The claim is a file holding the owner's process id; it is stale when
//! that process is gone.

use std::path::PathBuf;

use devd_core::Paths;

fn claim_file(paths: &Paths) -> PathBuf {
    paths.root().join("agent-host.pid")
}

/// Whether a live devd process holds the agents right now: the window, the daemon, or `devd serve`.
pub fn live(paths: &Paths) -> bool {
    devd_core::process::live_devd_holder(&claim_file(paths)).is_some()
}

/// Claim the right to recover interrupted conversations and runs. False when
/// another live devd process already holds it.
pub fn claim(paths: &Paths) -> bool {
    let file = claim_file(paths);
    let me = std::process::id();
    // Only the process that wrote the file holds it. A bare pid cannot say that:
    // the operating system reuses ids, so the file now carries the writer's start
    // token as well (see `live_devd_holder`).
    if devd_core::process::live_devd_holder(&file).is_some_and(|owner| owner != me) {
        return false;
    }
    std::fs::write(&file, devd_core::process::holder_line()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_claim_wins_and_a_dead_owner_is_replaced() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        assert!(claim(&paths), "nobody holds it");
        assert!(claim(&paths), "the owner may claim again");
        // A process id that is not running is not an owner.
        std::fs::write(claim_file(&paths), "4294967").unwrap();
        assert!(claim(&paths));
        let written = std::fs::read_to_string(claim_file(&paths)).unwrap();
        let mut parts = written.split_whitespace();
        assert_eq!(parts.next(), Some(std::process::id().to_string().as_str()), "the pid is first");
        assert!(
            parts.next().is_some(),
            "the start token is written beside it, so a recycled pid is not mistaken for the owner: {written}"
        );
    }
}
