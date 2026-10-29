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

/// How often logs are rotated and old audit rows removed.
const MAINTENANCE_EVERY: Duration = Duration::from_secs(3600);

fn claim_file(paths: &Paths) -> PathBuf {
    paths.root().join("ops-host.pid")
}

/// Claim the right to run the jobs. False when another live devd process holds it.
pub fn claim(paths: &Paths) -> bool {
    let file = claim_file(paths);
    let me = std::process::id();
    if devd_core::process::live_devd_holder(&file).is_some_and(|owner| owner != me) {
        return false;
    }
    std::fs::write(&file, me.to_string()).is_ok()
}

/// The devd process that is running the jobs, if any is. Cron jobs, scheduled backups and deploys
/// and health checks run in a long-lived devd process (the window, `devd ui` or `devd serve`);
/// when none is open nothing runs them, which is worth saying out loud.
pub fn holder(paths: &Paths) -> Option<u32> {
    devd_core::process::live_devd_holder(&claim_file(paths))
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
    std::thread::spawn(move || {
        let mut last_maintenance: Option<std::time::Instant> = None;
        let mut first = true;
        loop {
            // The first pass is immediate: a restarted daemon should reconcile
            // what it finds before the first quarter minute has passed.
            if !std::mem::take(&mut first) {
                std::thread::sleep(Duration::from_secs(15));
            }
            // Keep the claim fresh in case the file was removed, and give it up if another process took it.
            if !claim(&ctx.paths) {
                STARTED.store(false, Ordering::SeqCst);
                return;
            }
            crate::cron::tick(&ctx);
            crate::dbadmin::tick(&ctx);
            crate::deploy::tick_scheduled(&ctx);
            crate::health::tick(&ctx);
            // Rotation and retention are slow to matter, so once an hour is plenty.
            if last_maintenance.is_none_or(|t| t.elapsed() >= MAINTENANCE_EVERY) {
                last_maintenance = Some(std::time::Instant::now());
                for line in devd_core::maintenance::run(&ctx) {
                    ctx.events.emit("maintenance", line);
                }
                for report in devd_services::reconcile::reconcile(&ctx) {
                    ctx.events.emit("reconcile", report.detail);
                }
            }
        }
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
