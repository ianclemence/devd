//! The loop that runs cron jobs while devd is open.
//!
//! Exactly one devd process runs it: the first long-lived one to claim a file holding its process
//! id. A one-shot command such as `devd site list` builds the same registry and must not also
//! start every due job. The claim is stale when the process in it is gone.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use devd_core::capability::Ctx;
use devd_core::Paths;

static STARTED: AtomicBool = AtomicBool::new(false);

fn claim_file(paths: &Paths) -> PathBuf {
    paths.root().join("ops-host.pid")
}

/// Claim the right to run the jobs. False when another live devd process holds it.
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

/// Start the loop in this process, once, if it may. Returns whether this process runs the jobs.
pub fn start(ctx: Ctx) -> bool {
    if STARTED.swap(true, Ordering::SeqCst) {
        return true;
    }
    if !claim(&ctx.paths) {
        STARTED.store(false, Ordering::SeqCst);
        return false;
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(15));
        // Keep the claim fresh in case the file was removed, and give it up if another process took it.
        if !claim(&ctx.paths) {
            STARTED.store(false, Ordering::SeqCst);
            return;
        }
        crate::cron::tick(&ctx);
        crate::dbadmin::tick(&ctx);
        crate::deploy::tick_scheduled(&ctx);
        crate::health::tick(&ctx);
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_owner_is_replaced_and_the_owner_may_claim_again() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(d.path());
        assert!(claim(&paths));
        assert!(claim(&paths));
        std::fs::write(claim_file(&paths), "4294967").unwrap();
        assert!(claim(&paths));
    }
}
