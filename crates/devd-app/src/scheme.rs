//! Making `devd://` links open this app.
//!
//! A link like `devd://runs/9f3a21` in a terminal, a pull request, a chat or an email
//! should open that run in devd. That needs the operating system to know which program
//! owns the `devd` scheme. This writes that association, for the current person only
//! (no administrator rights), and says what it did.
//!
//! The pieces that build the commands and the file are plain functions, so what gets
//! written is tested without touching the registry or the disk.

use std::path::Path;

/// The scheme.
pub const SCHEME: &str = "devd";

/// The `reg add` arguments that make Windows open `devd://` links with `exe`.
///
/// Under the current user's own classes, so no elevation is needed and nothing else on the
/// machine changes. `%1` is the link, passed as one quoted argument.
pub fn windows_registry(exe: &str) -> Vec<Vec<String>> {
    let base = format!(r"HKCU\Software\Classes\{SCHEME}");
    let add = |key: &str, name: Option<&str>, value: &str| -> Vec<String> {
        let mut args = vec!["add".to_string(), key.to_string()];
        match name {
            Some(n) => args.extend(["/v".to_string(), n.to_string()]),
            None => args.push("/ve".to_string()),
        }
        args.extend(["/d".to_string(), value.to_string(), "/f".to_string()]);
        args
    };
    vec![
        add(&base, None, "URL:devd Protocol"),
        add(&base, Some("URL Protocol"), ""),
        add(&format!(r"{base}\DefaultIcon"), None, &format!("\"{exe}\",0")),
        add(&format!(r"{base}\shell\open\command"), None, &format!("\"{exe}\" \"%1\"")),
    ]
}

/// The desktop entry that makes a Linux desktop open `devd://` links with `exe`.
pub fn desktop_entry(exe: &str) -> String {
    // `%u` is the one link. Quoting follows the Desktop Entry specification: quote the program,
    // escape what would end the quotation.
    let quoted = format!("\"{}\"", exe.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"));
    format!("[Desktop Entry]\nType=Application\nName=devd\nComment=Open devd links\nExec={quoted} %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/{SCHEME};\n")
}

/// Register this program for `devd://` links. Returns what was done, in words.
pub fn register(exe: &Path) -> Result<String, String> {
    let exe_text = exe.to_string_lossy().to_string();
    if cfg!(windows) {
        for args in windows_registry(&exe_text) {
            let out = std::process::Command::new("reg").args(&args).output().map_err(|e| format!("could not run reg.exe: {e}"))?;
            if !out.status.success() {
                return Err(format!("Windows refused: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        Ok(format!("devd:// links now open {exe_text} (for this Windows user)."))
    } else if cfg!(target_os = "linux") {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let dir = Path::new(&home).join(".local/share/applications");
        std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let file = dir.join("devd-link.desktop");
        std::fs::write(&file, desktop_entry(&exe_text)).map_err(|e| format!("could not write {}: {e}", file.display()))?;
        // Best effort: not every desktop has these, and the entry works without them on most.
        let _ = std::process::Command::new("xdg-mime").args(["default", "devd-link.desktop", &format!("x-scheme-handler/{SCHEME}")]).status();
        let _ = std::process::Command::new("update-desktop-database").arg(&dir).status();
        Ok(format!("devd:// links now open {exe_text} (wrote {}).", file.display()))
    } else {
        Err("macOS ties a link scheme to an app bundle: install the packaged devd app and it registers itself.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_is_told_to_open_the_scheme_with_the_program_for_this_user_only() {
        let cmds = windows_registry(r"C:\Program Files\devd\devd-app.exe");
        assert_eq!(cmds.len(), 4);
        assert!(cmds.iter().all(|c| c[1].starts_with(r"HKCU\Software\Classes\devd")), "nothing outside this user's own classes");
        assert!(cmds.iter().all(|c| c.last().map(String::as_str) == Some("/f")));
        let open = cmds.iter().find(|c| c[1].ends_with(r"shell\open\command")).unwrap();
        assert_eq!(open[3], "/d");
        assert_eq!(open[4], r#""C:\Program Files\devd\devd-app.exe" "%1""#, "a path with a space stays one program, the link one argument");
        let protocol = cmds.iter().find(|c| c.contains(&"URL Protocol".to_string())).unwrap();
        assert_eq!(protocol[4], "");
    }

    #[test]
    fn a_linux_desktop_entry_names_the_scheme_and_quotes_the_program() {
        let entry = desktop_entry("/home/me/my apps/devd-app");
        assert!(entry.contains("MimeType=x-scheme-handler/devd;"), "{entry}");
        assert!(entry.contains("Exec=\"/home/me/my apps/devd-app\" %u"), "{entry}");
        assert!(entry.contains("NoDisplay=true"), "it is a handler, not a launcher");
        let nasty = desktop_entry("/x/a\"b$c");
        assert!(nasty.contains(r#"Exec="/x/a\"b\$c" %u"#), "{nasty}");
    }
}
