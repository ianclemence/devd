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

/// Claim the right to recover interrupted conversations and runs. False when
/// another live devd process already holds it.
pub fn claim(paths: &Paths) -> bool {
    let file = claim_file(paths);
    let me = std::process::id();
    if let Some(owner) = std::fs::read_to_string(&file).ok().and_then(|s| s.trim().parse::<u32>().ok()) {
        if owner != me && devd_core::process::pid_alive(owner) && devd_core::process::process_image(owner).map(|i| i.to_ascii_lowercase().contains("devd")).unwrap_or(false) {
            return false;
        }
    }
    std::fs::write(&file, me.to_string()).is_ok()
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
        assert_eq!(std::fs::read_to_string(claim_file(&paths)).unwrap().trim(), std::process::id().to_string());
    }
}
