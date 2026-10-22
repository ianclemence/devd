//! "Open a terminal here": the operating system's own terminal window, in a project folder,
//! with the project's pinned runtimes and the `devd` command first on `PATH`.
//!
//! devd does not embed a terminal. A terminal is the person's, and the one they already like
//! does everything better than a widget would; what devd adds is the environment.

use std::path::Path;
use std::process::Command;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

use crate::toolchain;

/// The `PATH` a terminal in `dir` starts with.
pub fn path_for(ctx: &Ctx, dir: &Path) -> Result<std::ffi::OsString> {
    let mut entries = toolchain::managed_dirs_for(ctx, dir);
    if let Ok(cli) = devd_core::util::cli_exe() {
        if let Some(parent) = cli.parent() {
            entries.push(parent.to_path_buf());
        }
    }
    if let Some(current) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(entries).map_err(|e| DevdError::invalid(format!("a runtime directory is not usable in PATH: {e}")))
}

/// Open a terminal in `dir`.
pub fn open(ctx: &Ctx, dir: &Path) -> Result<String> {
    if !dir.is_dir() {
        return Err(DevdError::not_found(format!("{} is not a folder", dir.display())));
    }
    let path = path_for(ctx, dir)?;
    let mut cmd = launcher(dir)?;
    cmd.env("PATH", path).env("DEVD_PROJECT", dir).current_dir(dir);
    cmd.spawn().map_err(|e| DevdError::Io(std::io::Error::new(e.kind(), format!("could not open a terminal: {e}"))))?;
    Ok(format!("opened a terminal in {}", dir.display()))
}

#[cfg(windows)]
fn launcher(dir: &Path) -> Result<Command> {
    // `start` gives the new console its own window; it inherits this environment, which a new
    // tab in an already-running Windows Terminal would not.
    let mut cmd = Command::new(devd_core::util::shell_path());
    cmd.args(["/C", "start", "devd"]).arg("/D").arg(dir).args(["cmd", "/K"]);
    Ok(cmd)
}

#[cfg(target_os = "macos")]
fn launcher(dir: &Path) -> Result<Command> {
    // Terminal.app starts its own login shell, so the pinned PATH is exported by the command it runs.
    let script = format!("tell application \"Terminal\" to do script \"cd '{}' && export PATH=\\\"$PATH\\\" && clear\"", dir.display().to_string().replace('\'', "'\''"));
    let mut cmd = Command::new("osascript");
    cmd.args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]);
    Ok(cmd)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launcher(dir: &Path) -> Result<Command> {
    for (program, args) in [
        ("x-terminal-emulator", vec![]),
        ("gnome-terminal", vec!["--working-directory".to_string(), dir.display().to_string()]),
        ("konsole", vec!["--workdir".to_string(), dir.display().to_string()]),
        ("xfce4-terminal", vec!["--working-directory".to_string(), dir.display().to_string()]),
        ("xterm", vec![]),
    ] {
        if devd_core::util::which(program).is_some() {
            let mut cmd = Command::new(program);
            cmd.args(args);
            return Ok(cmd);
        }
    }
    Err(DevdError::not_found("no terminal program was found (x-terminal-emulator, gnome-terminal, konsole, xfce4-terminal or xterm)"))
}
