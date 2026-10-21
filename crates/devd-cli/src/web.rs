//! `devd ui` and `devd serve`: the application, from the command line.
//!
//! Both start the same server. They differ only in whether a window is opened,
//! which is why they share this module rather than each having their own entry
//! point - a second entry point is a second place for the token policy, the
//! check list or the actor to drift.

use std::sync::Arc;

use devd_core::DevdError;
use devd_server::{ExtraChecks, Options};

use crate::app::{git_check, App};
use crate::window;

/// Where the interface ended up, for reporting.
pub struct Started {
    /// Port that is now serving.
    pub port: u16,
    /// Whether an already-running instance was reused instead of starting one.
    pub reused: bool,
}

/// Start the application, or attach to one that is already running.
///
/// Attaching matters more than it sounds. Closing a browser window closes a
/// window, not a process, so a second `devd ui` after an accidental close would
/// otherwise fail with "address in use" - which reads as a bug even though
/// nothing is wrong. Instead the port is probed for a `devd` instance, and one
/// that answers is simply reopened.
///
/// The probe is not a blind port check: it requires a `devd`-specific response
/// header, so an unrelated program on the port is never mistaken for ours and
/// never opened in a window.
pub fn serve(app: App, requested: Option<u16>, open: bool) -> Result<Started, DevdError> {
    let candidates = port_candidates(&app, requested);

    for port in &candidates {
        if let Some(version) = devd_server::probe_instance(*port) {
            let url = format!("http://127.0.0.1:{port}/");
            println!("devd {version} is already serving at {url}");
            if open {
                window::open_window(&url);
            }
            return Ok(Started {
                port: *port,
                reused: true,
            });
        }
    }

    let port = candidates
        .iter()
        .copied()
        .find(|p| devd_core::ports::is_free(*p))
        .ok_or_else(|| {
            DevdError::conflict(
                "no free port for the interface; pass `--port` with a port that is available",
            )
        })?;

    let store = app.store.clone();
    // Context first: `app.ctx()` borrows the application, and the registry is
    // moved out of it immediately afterwards.
    let ctx = app.ctx();
    let git_executable = app.settings.git_executable.clone();
    let registry = app.registry.clone();
    let workspace = app.workspace.clone();

    // The Git probe is contributed from here rather than from `devd-server`
    // because the server must not depend on any particular backend. Each entry
    // point appends what only it knows.
    let extra: Vec<ExtraChecks> =
        vec![Arc::new(move || vec![git_check(git_executable.as_deref())])];

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| DevdError::internal(format!("cannot start the runtime: {e}")))?;

    runtime.block_on(devd_server::serve_with(
        registry,
        ctx,
        Options {
            port,
            extra_checks: extra,
            workspace: Some(workspace),
            shutdown: None,
            // A browser cannot open the operating system's own folder dialog,
            // so the web application falls back to the folder browser it draws
            // itself.
            picker: None,
        },
        move |bound| {
            let url = format!("http://127.0.0.1:{}/", bound.port);
            println!("devd interface at {url}");
            println!("loopback only; the session token is in the page, never on disk");
            let _ = store.set_setting("ui.port", &bound.port.to_string());
            if open {
                if window::open_window(&url) {
                    println!("opened in an application window");
                } else {
                    println!("open the URL above in a browser");
                }
            }
            println!("press Ctrl+C to stop");
        },
    ))?;

    Ok(Started {
        port,
        reused: false,
    })
}

/// Ports to try, in order.
///
/// The remembered port comes first so a user who chose `--port 9000` once keeps
/// it without repeating the flag. The default block follows so the common case
/// needs no argument at all.
fn port_candidates(app: &App, requested: Option<u16>) -> Vec<u16> {
    if let Some(port) = requested {
        return vec![port];
    }
    let mut ports = Vec::new();
    if let Ok(saved) = app.store.get_setting("ui.port") {
        if let Some(port) = saved.and_then(|value| value.parse::<u16>().ok()) {
            ports.push(port);
        }
    }
    for offset in 0..devd_server::PORT_SCAN {
        let port = devd_server::DEFAULT_PORT.saturating_add(offset);
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    ports
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::Paths;

    fn app() -> App {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("devd-web-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        App::bootstrap_at(Paths::rooted(root), true, false, false).unwrap()
    }

    #[test]
    fn an_explicit_port_is_the_only_candidate() {
        assert_eq!(port_candidates(&app(), Some(9123)), vec![9123]);
    }

    #[test]
    fn the_default_block_starts_at_the_default_port_and_has_no_duplicates() {
        let ports = port_candidates(&app(), None);
        assert!(ports.contains(&devd_server::DEFAULT_PORT));
        let mut sorted = ports.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ports.len(), "duplicate candidate: {ports:?}");
    }

    #[test]
    fn a_remembered_port_is_tried_first() {
        let app = app();
        app.store.set_setting("ui.port", "9123").unwrap();
        assert_eq!(port_candidates(&app, None)[0], 9123);
    }
}
