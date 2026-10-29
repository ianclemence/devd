//! Starting the stack when the person signs in.
//!
//! "Start with Windows". devd does it with what each platform already
//! offers a user without administrator rights: a `Run` registry value on Windows,
//! a `systemd --user` unit (an XDG autostart entry where there is no user
//! manager) on Linux, a LaunchAgent on macOS. What runs is the resident process,
//! `devd daemon`: it brings the enabled services up, then stays to run scheduled
//! work and to serve the window, the dashboard and the command line. The services
//! it starts outlive it, as they do under `devd stack up`.

use std::path::{Path, PathBuf};

use devd_core::util;
use devd_core::{DevdError, Result};

const NAME: &str = "devd";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The command that runs at sign-in.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" daemon start", exe.display())
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
    if let Some(unit) = systemd_unit_path().filter(|p| p.is_file()) {
        return Ok(Some(unit.display().to_string()));
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
    if let Some(unit) = systemd_unit_path().filter(|_| systemd_user_available()) {
        if let Some(dir) = unit.parent() {
            std::fs::create_dir_all(dir)?;
        }
        util::atomic_write(&unit, &unit_text(exe))?;
        for args in [["--user", "daemon-reload"], ["--user", "enable"]] {
            let mut argv: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            if args[1] == "enable" {
                argv.push("devd.service".into());
            }
            let _ = util::run(
                Path::new("systemctl"),
                &argv,
                None,
                &[],
                Some(std::time::Duration::from_secs(15)),
            );
        }
        // One way of starting at sign-in, not two.
        let _ = std::fs::remove_file(entry_path()?);
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
    let mut existed = file.is_file();
    if existed {
        std::fs::remove_file(&file)?;
    }
    if let Some(unit) = systemd_unit_path().filter(|p| p.is_file()) {
        let _ = util::run(
            Path::new("systemctl"),
            &["--user".into(), "disable".into(), "devd.service".into()],
            None,
            &[],
            Some(std::time::Duration::from_secs(15)),
        );
        std::fs::remove_file(&unit)?;
        existed = true;
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

/// Where the `systemd --user` unit lives. `None` off Linux.
fn systemd_unit_path() -> Option<PathBuf> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("systemd/user/devd.service"))
}

/// Whether there is a user service manager to hand the daemon to.
fn systemd_user_available() -> bool {
    util::run(
        Path::new("systemctl"),
        &["--user".into(), "show-environment".into()],
        None,
        &[],
        Some(std::time::Duration::from_secs(10)),
    )
    .map(|out| out.code == 0)
    .unwrap_or(false)
}

/// The unit that runs the resident process. `run` stays in the foreground so
/// systemd supervises it.
fn unit_text(exe: &Path) -> String {
    format!(
        "[Unit]
Description=devd resident process
After=network.target

[Service]
Type=simple
ExecStart=\"{}\" daemon run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
",
        exe.display()
    )
}

fn entry_text(exe: &Path) -> String {
    if cfg!(target_os = "macos") {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\"><dict>\n\
             <key>Label</key><string>dev.devd.stack</string>\n\
             <key>ProgramArguments</key><array><string>{}</string><string>daemon</string><string>run</string></array>\n\
             <key>RunAtLoad</key><true/>\n\
             </dict></plist>\n",
            exe.display()
        )
    } else {
        format!(
            "[Desktop Entry]\nType=Application\nName=devd stack\nComment=Start devd's services\nExec=\"{}\" daemon start\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_starts_the_resident_process() {
        assert_eq!(command_line(Path::new(r"C:\Users\Me\AppData\Local\Programs\devd\devd.exe")), "\"C:\\Users\\Me\\AppData\\Local\\Programs\\devd\\devd.exe\" daemon start");
    }

    #[test]
    fn the_registry_value_is_read_from_reg_query_output() {
        let out = "\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\n    devd    REG_SZ    \"C:\\devd\\devd.exe\" daemon start\n\n";
        assert_eq!(parse_reg_value(out).as_deref(), Some("\"C:\\devd\\devd.exe\" daemon start"));
        assert_eq!(parse_reg_value("    other    REG_SZ    x\n"), None);
    }

    #[test]
    fn the_unix_entries_run_the_same_command() {
        let text = entry_text(Path::new("/usr/local/bin/devd"));
        assert!(text.contains("daemon start") || text.contains("<string>daemon</string><string>run</string>"));
        let unit = unit_text(Path::new("/usr/local/bin/devd"));
        assert!(unit.contains("ExecStart=\"/usr/local/bin/devd\" daemon run"));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
