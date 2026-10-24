//! Commands devd's runtimes make available on the command line.
//!
//! `php` and `composer` that devd installs live under its own home, which is not
//! on `PATH`, so `composer` from a terminal (or a tool that shells out to `php`)
//! says "'php' is not recognized". Putting each PHP version's folder on `PATH`
//! would fix one terminal and break the moment the default version changes or a
//! project needs another one.
//!
//! devd puts **one** folder on `PATH`, `<home>/bin`, holding a small launcher for
//! each command. A launcher does nothing but hand over to `devd shim <name>`,
//! which decides at that moment which program to run: the PHP the site in the
//! current folder is served with, else the folder's pin, else the default. The
//! folder never changes when versions do, so it is added to `PATH` once.
//!
//! This module writes the launchers and edits the user's `PATH`. Which program a
//! launcher runs is decided in `devd-ops`, which knows the sites.

use std::path::{Path, PathBuf};

use crate::paths::Paths;
use crate::util;
use crate::{DevdError, Result};

/// The commands that get a launcher.
pub const COMMANDS: &[&str] = &["php", "composer"];

const BEGIN: &str = "# >>> devd bin >>>";
const END: &str = "# <<< devd bin <<<";

/// The folder holding the launchers, which is the one folder added to `PATH`.
pub fn bin_dir(paths: &Paths) -> PathBuf {
    paths.root().join("bin")
}

/// Where devd keeps `composer.phar`.
pub fn composer_phar(paths: &Paths) -> PathBuf {
    paths.root().join("tools").join("composer.phar")
}

/// The launcher's file name for a command.
pub fn launcher_name(command: &str) -> String {
    if cfg!(windows) {
        format!("{command}.cmd")
    } else {
        command.to_string()
    }
}

/// The text of one launcher. It names the devd program by its full path, so it
/// works from any folder and does not depend on devd being on `PATH` itself.
pub fn launcher_text(exe: &Path, command: &str) -> String {
    if cfg!(windows) {
        format!("@echo off\r\n\"{}\" shim {command} %*\r\nexit /b %errorlevel%\r\n", exe.display())
    } else {
        format!("#!/bin/sh\nexec \"{}\" shim {command} \"$@\"\n", exe.display())
    }
}

/// Whether any PHP runtime is installed. A `php` launcher is only written then,
/// so devd never puts a command on `PATH` that can only fail, in front of a PHP
/// the person installed some other way.
pub fn has_php(paths: &Paths) -> bool {
    std::fs::read_dir(paths.runtimes().join("php"))
        .map(|rd| rd.flatten().any(|e| e.path().is_dir()))
        .unwrap_or(false)
}

/// Write the launchers that apply right now, and rewrite one whose text is stale
/// (devd moved, or was reinstalled elsewhere). Returns the commands now present.
///
/// It is cheap and safe to call often: a launcher that is already right is not
/// touched.
pub fn ensure(paths: &Paths, exe: &Path) -> Result<Vec<String>> {
    let dir = bin_dir(paths);
    let mut present = Vec::new();
    for command in COMMANDS {
        let wanted = match *command {
            "php" => has_php(paths),
            "composer" => composer_phar(paths).is_file(),
            _ => false,
        };
        let file = dir.join(launcher_name(command));
        if !wanted {
            // The thing it launched is gone; a launcher that can only fail goes too.
            if file.is_file() {
                let _ = std::fs::remove_file(&file);
            }
            continue;
        }
        let text = launcher_text(exe, command);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
            std::fs::create_dir_all(&dir)?;
            util::atomic_write(&file, &text)?;
            make_executable(&file)?;
        }
        present.push((*command).to_string());
    }
    Ok(present)
}

#[cfg(unix)]
fn make_executable(file: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_file: &Path) -> Result<()> {
    Ok(())
}

// ------------------------------------------------------------------ PATH text

fn same_entry(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let t = s.trim().trim_end_matches(['\\', '/']).to_string();
        if cfg!(windows) { t.to_lowercase() } else { t }
    };
    !a.trim().is_empty() && norm(a) == norm(b)
}

fn separator() -> char {
    if cfg!(windows) { ';' } else { ':' }
}

/// Whether `list` (a `PATH` value) already names `dir`.
pub fn path_has(list: &str, dir: &Path) -> bool {
    let dir = dir.display().to_string();
    list.split(separator()).any(|e| same_entry(e, &dir))
}

/// `list` with `dir` first, and not repeated. The rest is kept byte for byte, so
/// an entry such as `%USERPROFILE%\bin` is not expanded and flattened.
pub fn path_with(list: &str, dir: &Path) -> String {
    let dir = dir.display().to_string();
    let rest: Vec<&str> = list.split(separator()).filter(|e| !e.trim().is_empty() && !same_entry(e, &dir)).collect();
    std::iter::once(dir.as_str()).chain(rest).collect::<Vec<_>>().join(&separator().to_string())
}

/// `list` without `dir`.
pub fn path_without(list: &str, dir: &Path) -> String {
    let dir = dir.display().to_string();
    list.split(separator()).filter(|e| !e.trim().is_empty() && !same_entry(e, &dir)).collect::<Vec<_>>().join(&separator().to_string())
}

// -------------------------------------------------------------- the user PATH

/// What the person's own `PATH` says about devd's folder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PathState {
    /// The folder of launchers.
    pub dir: String,
    /// Whether the user's `PATH` names it.
    pub on_path: bool,
    /// The commands whose launcher exists.
    pub commands: Vec<String>,
}

/// The registry key (under `HKEY_CURRENT_USER`) that holds the user's environment.
const USER_ENV: &str = "Environment";

/// Report the state without changing anything.
pub fn path_state(paths: &Paths) -> Result<PathState> {
    path_state_in(paths, USER_ENV)
}

fn path_state_in(paths: &Paths, key: &str) -> Result<PathState> {
    let dir = bin_dir(paths);
    let on_path = if cfg!(windows) { path_has(&user_path_windows(key)?, &dir) } else { profile_has_block(&profile_file()?) };
    let commands = COMMANDS
        .iter()
        .filter(|c| dir.join(launcher_name(c)).is_file())
        .map(|c| (*c).to_string())
        .collect();
    Ok(PathState { dir: dir.display().to_string(), on_path, commands })
}

/// Put the folder first on the user's `PATH`. Only the user's own `PATH` is
/// touched, so no administrator rights are needed. A terminal that is already
/// open does not see it; a new one does.
pub fn path_enable(paths: &Paths) -> Result<PathState> {
    path_enable_in(paths, USER_ENV)
}

fn path_enable_in(paths: &Paths, key: &str) -> Result<PathState> {
    let dir = bin_dir(paths);
    std::fs::create_dir_all(&dir)?;
    if cfg!(windows) {
        let now = user_path_windows(key)?;
        if !now.starts_with(&dir.display().to_string()) {
            set_user_path_windows(key, &path_with(&now, &dir))?;
        }
    } else {
        let file = profile_file()?;
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, with_profile_block(&text, &dir))?;
    }
    path_state_in(paths, key)
}

/// Take the folder off the user's `PATH`. The launchers stay where they are.
pub fn path_disable(paths: &Paths) -> Result<PathState> {
    path_disable_in(paths, USER_ENV)
}

fn path_disable_in(paths: &Paths, key: &str) -> Result<PathState> {
    let dir = bin_dir(paths);
    if cfg!(windows) {
        let now = user_path_windows(key)?;
        if path_has(&now, &dir) {
            set_user_path_windows(key, &path_without(&now, &dir))?;
        }
    } else {
        let file = profile_file()?;
        if let Ok(text) = std::fs::read_to_string(&file) {
            std::fs::write(&file, without_profile_block(&text))?;
        }
    }
    path_state_in(paths, key)
}

// --------------------------------------------------------------------- Windows

// The key name and the value are handed over in environment variables, never spliced into the
// script, so a path with a quote or a dollar sign cannot change what runs. The registry value
// keeps its type (REG_EXPAND_SZ) so `%VARIABLES%` in other entries still expand. For the real
// environment key, two throwaway variables make Windows tell running programs that it changed.
const READ_USER_PATH: &str = "$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:DEVD_ENV_KEY);      if($k){[Console]::Out.Write($k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames))}";

const WRITE_USER_PATH: &str = "$k=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($env:DEVD_ENV_KEY);      $k.SetValue('Path',$env:DEVD_NEW_PATH,[Microsoft.Win32.RegistryValueKind]::ExpandString);      if($env:DEVD_ENV_KEY -eq 'Environment'){        [Environment]::SetEnvironmentVariable('DEVD_PATH_REFRESH','1','User');        [Environment]::SetEnvironmentVariable('DEVD_PATH_REFRESH',$null,'User') }";

fn powershell(script: &str, envs: &[(String, String)]) -> Result<String> {
    let out = util::run(
        Path::new("powershell.exe"),
        &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script.into()],
        None,
        envs,
        Some(std::time::Duration::from_secs(30)),
    )?;
    if out.code != 0 {
        return Err(DevdError::Command { program: "powershell".into(), code: out.code, stderr: out.stderr.trim().to_string() });
    }
    Ok(out.stdout)
}

fn user_path_windows(key: &str) -> Result<String> {
    powershell(READ_USER_PATH, &[("DEVD_ENV_KEY".to_string(), key.to_string())]).map(|s| s.trim().to_string())
}

fn set_user_path_windows(key: &str, value: &str) -> Result<()> {
    powershell(WRITE_USER_PATH, &[("DEVD_ENV_KEY".to_string(), key.to_string()), ("DEVD_NEW_PATH".to_string(), value.to_string())]).map(|_| ())
}

// ------------------------------------------------------------- Linux and macOS

fn profile_file() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| DevdError::not_found("HOME is not set"))?;
    Ok(home.join(".profile"))
}

fn profile_has_block(file: &Path) -> bool {
    std::fs::read_to_string(file).map(|t| t.contains(BEGIN)).unwrap_or(false)
}

/// `text` with the block that puts `dir` on `PATH`, replacing an earlier one.
pub fn with_profile_block(text: &str, dir: &Path) -> String {
    let base = without_profile_block(text);
    let sep = if base.is_empty() || base.ends_with('\n') { "" } else { "\n" };
    format!("{base}{sep}{BEGIN}\nexport PATH=\"{}:$PATH\"\n{END}\n", dir.display())
}

/// `text` without devd's block.
pub fn without_profile_block(text: &str) -> String {
    let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END)) else {
        return text.to_string();
    };
    if end < start {
        return text.to_string();
    }
    let after = &text[end + END.len()..];
    format!("{}{}", &text[..start], after.strip_prefix('\n').unwrap_or(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(tag: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("devd-shims-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Paths::rooted(dir)
    }

    fn install_php(paths: &Paths) {
        std::fs::create_dir_all(paths.runtimes().join("php").join("8.4.1")).unwrap();
    }

    #[test]
    fn a_launcher_hands_over_to_devd_by_its_full_path() {
        let text = launcher_text(Path::new("D:\\devd\\devd.exe"), "php");
        assert!(text.contains("shim php"), "{text}");
        if cfg!(windows) {
            assert!(text.contains("\"D:\\devd\\devd.exe\" shim php %*"), "{text}");
            assert!(text.ends_with("exit /b %errorlevel%\r\n"), "the exit code is passed on: {text}");
        } else {
            assert!(text.starts_with("#!/bin/sh\nexec "), "{text}");
        }
    }

    #[test]
    fn no_php_launcher_is_written_until_a_php_is_installed() {
        let p = home("nophp");
        assert!(ensure(&p, Path::new("devd")).unwrap().is_empty());
        assert!(!bin_dir(&p).join(launcher_name("php")).exists(), "a launcher that can only fail would hide a PHP installed elsewhere");
        install_php(&p);
        assert_eq!(ensure(&p, Path::new("devd")).unwrap(), vec!["php".to_string()]);
        assert!(bin_dir(&p).join(launcher_name("php")).is_file());
        let _ = std::fs::remove_dir_all(p.root());
    }

    #[test]
    fn composer_gets_a_launcher_once_its_phar_is_there_and_loses_it_when_it_goes() {
        let p = home("composer");
        install_php(&p);
        assert_eq!(ensure(&p, Path::new("devd")).unwrap(), vec!["php".to_string()]);
        std::fs::create_dir_all(composer_phar(&p).parent().unwrap()).unwrap();
        std::fs::write(composer_phar(&p), b"phar").unwrap();
        assert_eq!(ensure(&p, Path::new("devd")).unwrap(), vec!["php".to_string(), "composer".to_string()]);
        std::fs::remove_file(composer_phar(&p)).unwrap();
        assert_eq!(ensure(&p, Path::new("devd")).unwrap(), vec!["php".to_string()]);
        assert!(!bin_dir(&p).join(launcher_name("composer")).exists());
        let _ = std::fs::remove_dir_all(p.root());
    }

    #[test]
    fn a_launcher_is_rewritten_when_devd_moves_and_left_alone_otherwise() {
        let p = home("moved");
        install_php(&p);
        ensure(&p, Path::new("one")).unwrap();
        let file = bin_dir(&p).join(launcher_name("php"));
        let first = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure(&p, Path::new("one")).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), first, "an unchanged launcher is not rewritten");
        ensure(&p, Path::new("two")).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("two"));
        let _ = std::fs::remove_dir_all(p.root());
    }

    #[test]
    fn path_text_is_edited_without_touching_the_other_entries() {
        let sep = separator();
        let dir = Path::new(if cfg!(windows) { "D:\\devd\\bin" } else { "/opt/devd/bin" });
        let list = format!("%USERPROFILE%\\bin{sep}C:\\tools{sep}");
        assert!(!path_has(&list, dir));
        let with = path_with(&list, dir);
        assert!(with.starts_with(&dir.display().to_string()), "devd's folder goes first: {with}");
        assert!(with.contains("%USERPROFILE%\\bin"), "other entries are kept as written: {with}");
        assert_eq!(path_with(&with, dir), with, "adding twice changes nothing");
        assert_eq!(path_without(&with, dir), format!("%USERPROFILE%\\bin{sep}C:\\tools"));
        if cfg!(windows) {
            assert!(path_has("d:\\DEVD\\bin\\;C:\\x", dir), "Windows paths ignore case and a trailing slash");
        }
    }

    /// The same scripts the real call uses, run against a throwaway key, so the registry code is
    /// exercised without touching the person's own environment.
    #[cfg(windows)]
    #[test]
    fn the_user_path_is_edited_in_the_registry_and_nothing_else_changes() {
        let p = home("registry");
        let key = format!("Software\\devd-test-{}", std::process::id());
        // A value with an unexpanded variable, a quote and a dollar sign.
        let seed = "%USERPROFILE%\\bin;C:\\a $b \"c\";C:\\tools";
        set_user_path_windows(&key, seed).unwrap();
        assert_eq!(user_path_windows(&key).unwrap(), seed, "stored as written, not expanded");

        let state = path_state_in(&p, &key).unwrap();
        assert!(!state.on_path);
        let enabled = path_enable_in(&p, &key).unwrap();
        assert!(enabled.on_path);
        let after = user_path_windows(&key).unwrap();
        assert!(after.starts_with(&bin_dir(&p).display().to_string()), "first: {after}");
        assert!(after.ends_with(seed), "the other entries are exactly as they were: {after}");
        assert_eq!(path_enable_in(&p, &key).unwrap(), enabled, "enabling twice changes nothing");
        assert_eq!(user_path_windows(&key).unwrap(), after);

        let disabled = path_disable_in(&p, &key).unwrap();
        assert!(!disabled.on_path);
        assert_eq!(user_path_windows(&key).unwrap(), seed, "disable restores the original");

        let _ = util::run(
            Path::new("reg.exe"),
            &["delete".into(), format!("HKCU\\{key}"), "/f".into()],
            None,
            &[],
            Some(std::time::Duration::from_secs(15)),
        );
        let _ = std::fs::remove_dir_all(p.root());
    }

    #[test]
    fn the_profile_block_is_added_replaced_and_removed_cleanly() {
        let dir = Path::new("/opt/devd/bin");
        let original = "export A=1\nalias ll='ls -l'\n";
        let added = with_profile_block(original, dir);
        assert!(added.starts_with(original) && added.contains("export PATH=\"/opt/devd/bin:$PATH\""));
        assert_eq!(with_profile_block(&added, Path::new("/other/bin")).matches(BEGIN).count(), 1, "replaced, not stacked");
        assert_eq!(without_profile_block(&added), original);
        assert_eq!(without_profile_block(original), original);
        assert_eq!(with_profile_block("no newline", dir).lines().next(), Some("no newline"));
    }
}
