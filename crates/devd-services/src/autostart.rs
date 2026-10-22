//! Starting the stack when the person signs in.
//!
//! Laragon's "start with Windows". devd does it with what each platform already
//! offers a user without administrator rights: a `Run` registry value on Windows,
//! an XDG autostart entry on Linux, a LaunchAgent on macOS. What runs is
//! `devd stack up`: it starts the enabled services detached and exits, so nothing
//! of devd stays resident, and the services are the same ones `devd stack up`
//! starts by hand.

use std::path::{Path, PathBuf};

use devd_core::util;
use devd_core::{DevdError, Result};

const NAME: &str = "devd";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The command that runs at sign-in.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" stack up", exe.display())
}

/// Whether sign-in starts the stack, and the command it runs.
pub fn status() -> Result<Option<String>> {
    if cfg!(windows) {
        let out = util::run(
            Path::new("reg.exe"),
            &["query".into(), RUN_KEY.into(), "/v".into(), NAME.into()],
            None,
            &[],
            Some(std::time::Duration::from_secs(15)),
        )?;
        if out.code != 0 {
            return Ok(None);
        }
        return Ok(parse_reg_value(&out.stdout));
    }
    let file = entry_path()?;
    Ok(std::fs::read_to_string(&file).ok().map(|_| file.display().to_string()))
}

/// Start the stack at sign-in.
pub fn enable(exe: &Path) -> Result<String> {
    let command = command_line(exe);
    if cfg!(windows) {
        let out = util::run(
            Path::new("reg.exe"),
            &["add".into(), RUN_KEY.into(), "/v".into(), NAME.into(), "/t".into(), "REG_SZ".into(), "/d".into(), command.clone(), "/f".into()],
            None,
            &[],
            Some(std::time::Duration::from_secs(15)),
        )?;
        if out.code != 0 {
            return Err(DevdError::Command { program: "reg add".into(), code: out.code, stderr: out.stderr.trim().to_string() });
        }
        return Ok(command);
    }
    let file = entry_path()?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    util::atomic_write(&file, &entry_text(exe))?;
    Ok(command)
}

/// Stop starting the stack at sign-in. Nothing already running is stopped.
pub fn disable() -> Result<bool> {
    if cfg!(windows) {
        let was = status()?.is_some();
        let out = util::run(
            Path::new("reg.exe"),
            &["delete".into(), RUN_KEY.into(), "/v".into(), NAME.into(), "/f".into()],
            None,
            &[],
            Some(std::time::Duration::from_secs(15)),
        )?;
        if out.code != 0 && was {
            return Err(DevdError::Command { program: "reg delete".into(), code: out.code, stderr: out.stderr.trim().to_string() });
        }
        return Ok(was);
    }
    let file = entry_path()?;
    let existed = file.is_file();
    if existed {
        std::fs::remove_file(&file)?;
    }
    Ok(existed)
}

/// The data of the `devd` value in `reg query` output:
/// `    devd    REG_SZ    "C:\...\devd.exe" stack up`.
fn parse_reg_value(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(NAME)?.trim_start();
        let rest = rest.strip_prefix("REG_SZ")?.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    })
}

/// Where the autostart entry lives on Linux and macOS.
fn entry_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| DevdError::not_found("HOME is not set"))?;
    Ok(if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/dev.devd.stack.plist")
    } else {
        home.join(".config/autostart/devd-stack.desktop")
    })
}

fn entry_text(exe: &Path) -> String {
    if cfg!(target_os = "macos") {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>dev.devd.stack</string>\n\
             <key>ProgramArguments</key><array><string>{}</string><string>stack</string><string>up</string></array>\n\
             <key>RunAtLoad</key><true/>\n\
             </dict></plist>\n",
            exe.display()
        )
    } else {
        format!(
            "[Desktop Entry]\nType=Application\nName=devd stack\nComment=Start devd's services\nExec=\"{}\" stack up\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_the_stack_up_the_person_would_type() {
        assert_eq!(command_line(Path::new(r"C:\Users\Me\AppData\Local\Programs\devd\devd.exe")), "\"C:\\Users\\Me\\AppData\\Local\\Programs\\devd\\devd.exe\" stack up");
    }

    #[test]
    fn the_registry_value_is_read_from_reg_query_output() {
        let out = "\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\n    devd    REG_SZ    \"C:\\devd\\devd.exe\" stack up\n\n";
        assert_eq!(parse_reg_value(out).as_deref(), Some("\"C:\\devd\\devd.exe\" stack up"));
        assert_eq!(parse_reg_value("    other    REG_SZ    x\n"), None);
    }

    #[test]
    fn the_unix_entries_run_the_same_command() {
        let text = entry_text(Path::new("/usr/local/bin/devd"));
        assert!(text.contains("stack up") || text.contains("<string>stack</string><string>up</string>"));
    }
}
