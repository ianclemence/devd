//! `devd daemon`: the resident process every other surface is a client of.
//!
//! One per user. It owns cron, scheduled backups and deploys, health checks,
//! log rotation and the local API. The window, the terminal dashboard and the
//! command line find it through the instance record (see
//! `devd_core::daemon`), never by scanning ports.

use std::time::{Duration, Instant};

use serde_json::json;

use devd_core::daemon as instance;
use devd_core::{DevdError, Result};

use crate::app::App;
use crate::web;

/// Run the resident process in this terminal until it is stopped.
pub fn run(app: App, port: Option<u16>) -> Result<i32> {
    if let Some(held) = instance::current(&app.paths) {
        println!(
            "devd {} is already running (pid {}, port {})",
            held.version, held.pid, held.port
        );
        return Ok(0);
    }
    // What `devd stack up` did at sign-in before there was a daemon. It is
    // idempotent, and a failure here must not stop the daemon from serving.
    if let Err(e) = app.invoke("stack.up", json!({})) {
        eprintln!("the stack did not start cleanly: {e}");
    }
    web::serve(app, port, false, "daemon")?;
    Ok(0)
}

/// Start the resident process detached from this terminal, and wait until it
/// answers.
pub fn start(app: &App) -> Result<i32> {
    if let Some(held) = instance::current(&app.paths) {
        println!(
            "devd {} is already running (pid {}, port {})",
            held.version, held.pid, held.port
        );
        return Ok(0);
    }
    let exe = std::env::current_exe()
        .map_err(|e| DevdError::internal(format!("cannot find this program: {e}")))?;
    let log = app.paths.root().join("daemon.log");
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    let err = out.try_clone()?;

    let mut cmd = std::process::Command::new(exe);
    cmd.args(["daemon", "run"])
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW: no
        // console, and it survives the shell that started it.
        cmd.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn()
        .map_err(|e| DevdError::internal(format!("cannot start the daemon: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(held) = instance::current(&app.paths) {
            if held.port != 0 {
                println!(
                    "devd {} is running (pid {}) at http://127.0.0.1:{}/",
                    held.version, held.pid, held.port
                );
                return Ok(0);
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(DevdError::internal(format!(
        "the daemon did not become ready in 20 seconds; see {}",
        log.display()
    )))
}

/// Stop the resident process.
///
/// Only the process the record names is stopped, and only after its image and
/// start time have been checked, so a recycled process id is never signalled.
/// Services devd started keep running: they outlive it by design.
pub fn stop(app: &App) -> Result<i32> {
    let Some(held) = instance::current(&app.paths) else {
        println!("devd is not running");
        return Ok(0);
    };
    devd_core::process::kill_tree(held.pid);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !held.is_live() {
            let _ = std::fs::remove_file(app.paths.root().join("instance.json"));
            println!("stopped devd (pid {})", held.pid);
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(DevdError::internal(format!(
        "devd (pid {}) did not stop within 10 seconds",
        held.pid
    )))
}

/// Say whether the resident process is running and where.
pub fn status(app: &App) -> Result<i32> {
    let held = instance::current(&app.paths);
    if app.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "running": held.is_some(), "instance": held }))
                .map_err(|e| DevdError::internal(e.to_string()))?
        );
    } else {
        match &held {
            Some(i) => println!(
                "devd {} is running as the {} (pid {}) at http://127.0.0.1:{}/ since {}",
                i.version, i.role, i.pid, i.port, i.since
            ),
            None => println!("devd is not running"),
        }
    }
    Ok(if held.is_some() { 0 } else { 1 })
}
