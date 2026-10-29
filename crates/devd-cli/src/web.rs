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

/// Start the application, or attach to the instance that already runs it.
///
/// The instance record says where the resident process is, so a second
/// `devd ui` after a closed window reopens it instead of failing with "address
/// in use". Nothing scans ports to find it: a process that holds a port but not
/// the record is never mistaken for devd, and a devd that holds the record is
/// never mistaken for something else.
///
/// `role` is `daemon` for the resident process and `ui` for a window's own
/// server. Either way this process becomes the owner of the background work
/// only after it has claimed the record.
pub fn serve(
    app: App,
    requested: Option<u16>,
    open: bool,
    role: &str,
) -> Result<Started, DevdError> {
    let claim = match devd_server::attach_or_claim(&app.paths, role)? {
        devd_server::Attach::Client { port, version } => {
            let url = format!("http://127.0.0.1:{port}/");
            println!("devd {version} is already serving at {url}");
            if open {
                window::open_window(&url);
            }
            return Ok(Started { port, reused: true });
        }
        devd_server::Attach::Own(claim) => claim,
    };
    let claim = Arc::new(std::sync::Mutex::new(claim));

    app.resume_agents();
    let candidates = port_candidates(&app, requested);
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

    let recorded = claim.clone();
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
            if let Ok(mut guard) = recorded.lock() {
                let _ = guard.set_port(bound.port);
            }
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

    drop(claim);
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
