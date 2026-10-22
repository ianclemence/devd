//! `.htaccess` files written for `mod_php`.
//!
//! PHP runs beside Apache as FastCGI, not inside it, so Apache has no `php_value`,
//! `php_flag`, `php_admin_value` or `php_admin_flag` directive. A `.htaccess` that
//! uses one without a guard answers every request with a 500 ("Invalid command
//! `php_value`"). The portable form, and the one hosts that do run `mod_php` read
//! the same way, wraps them in `<IfModule mod_php.c>`.
//!
//! [`fix`] makes that change, keeps the original beside it, and carries the values
//! over into `.user.ini`, which PHP's FastCGI reads for everything a `.htaccess`
//! may set (the per-directory and per-user settings). `php_admin_*` values are
//! system settings that `.user.ini` cannot change; they are listed, not applied.

use std::path::Path;

use serde::Serialize;

use devd_core::util;
use devd_core::{DevdError, Result};

/// One directive found outside a `mod_php` guard.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PhpDirective {
    /// 1-based line number.
    pub line: usize,
    /// `php_value`, `php_flag`, `php_admin_value` or `php_admin_flag`.
    pub kind: String,
    /// The setting.
    pub name: String,
    /// What it is set to, unquoted.
    pub value: String,
}

const KINDS: &[&str] = &["php_value", "php_flag", "php_admin_value", "php_admin_flag"];
const BEGIN: &str = "; ---- devd: translated from .htaccess ----";
const END: &str = "; ---- end devd ----";

fn directive(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim();
    let mut words = trimmed.splitn(3, char::is_whitespace);
    let kind = words.next()?.to_ascii_lowercase();
    if !KINDS.contains(&kind.as_str()) {
        return None;
    }
    let name = words.next()?.trim().to_string();
    let value = words.next().unwrap_or("").trim().trim_matches('"').trim_matches('\'').to_string();
    (!name.is_empty()).then_some((kind, name, value))
}

fn opens_php_guard(line: &str) -> bool {
    let l = line.trim().to_ascii_lowercase();
    l.starts_with("<ifmodule") && !l.contains('!') && l.contains("php")
}

fn opens_block(line: &str) -> bool {
    line.trim().to_ascii_lowercase().starts_with("<ifmodule")
}

fn closes_block(line: &str) -> bool {
    line.trim().to_ascii_lowercase().starts_with("</ifmodule")
}

/// The unguarded `php_*` directives in an `.htaccess`.
pub fn scan(text: &str) -> Vec<PhpDirective> {
    let mut out = Vec::new();
    // Each open <IfModule> block, and whether it is a php guard.
    let mut stack: Vec<bool> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if opens_block(line) {
            stack.push(opens_php_guard(line));
        } else if closes_block(line) {
            stack.pop();
        } else if !stack.iter().any(|g| *g) {
            if let Some((kind, name, value)) = directive(line) {
                out.push(PhpDirective { line: i + 1, kind, name, value });
            }
        }
    }
    out
}

/// The same text with every unguarded run of `php_*` lines wrapped in
/// `<IfModule mod_php.c>`. Everything else is untouched.
pub fn wrap(text: &str) -> String {
    let flagged: Vec<usize> = scan(text).iter().map(|d| d.line).collect();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut open = false;
    for (i, line) in lines.iter().enumerate() {
        let hit = flagged.contains(&(i + 1));
        if hit && !open {
            out.push("<IfModule mod_php.c>".to_string());
            open = true;
        }
        if !hit && open {
            out.push("</IfModule>".to_string());
            open = false;
        }
        out.push((*line).to_string());
    }
    if open {
        out.push("</IfModule>".to_string());
    }
    let mut joined = out.join(newline);
    if text.ends_with('\n') {
        joined.push_str(newline);
    }
    joined
}

fn flag_value(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => "On",
        _ => "Off",
    }
}

/// The `.user.ini` lines for the directives `.user.ini` can carry, and the names
/// of the ones it cannot.
pub fn user_ini(directives: &[PhpDirective]) -> (String, Vec<String>) {
    let mut lines = Vec::new();
    let mut skipped = Vec::new();
    for d in directives {
        match d.kind.as_str() {
            "php_value" => lines.push(format!("{} = {}", d.name, d.value)),
            "php_flag" => lines.push(format!("{} = {}", d.name, flag_value(&d.value))),
            _ => skipped.push(d.name.clone()),
        }
    }
    (lines.join("\n"), skipped)
}

fn with_user_ini_block(existing: &str, block: &str) -> String {
    let existing = existing.replace("\r\n", "\n");
    let base = match (existing.find(BEGIN), existing.find(END)) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &existing[..a], existing[b + END.len()..].trim_start_matches('\n')),
        _ => existing.clone(),
    };
    let mut out = base;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{BEGIN}\n{block}\n{END}\n"));
    out
}

/// What [`fix`] did.
#[derive(Debug, Clone, Serialize)]
pub struct Fixed {
    /// The directives that were guarded.
    pub directives: Vec<PhpDirective>,
    /// Names written to `.user.ini`.
    pub translated: Vec<String>,
    /// Names `.user.ini` cannot carry.
    pub not_applied: Vec<String>,
    /// Where the original `.htaccess` was kept.
    pub backup: Option<String>,
}

/// Guard the `php_*` directives in `<docroot>/.htaccess`, keep the original as
/// `.htaccess.devd-backup`, and carry the values into `.user.ini`.
pub fn fix(docroot: &Path) -> Result<Fixed> {
    let file = docroot.join(".htaccess");
    let text = std::fs::read_to_string(&file).map_err(|_| DevdError::not_found(format!("{} has no .htaccess", docroot.display())))?;
    let directives = scan(&text);
    if directives.is_empty() {
        return Err(DevdError::conflict("the .htaccess has no unguarded php_value, php_flag or php_admin directive"));
    }
    let backup = docroot.join(".htaccess.devd-backup");
    let kept = if backup.exists() {
        None
    } else {
        std::fs::copy(&file, &backup)?;
        Some(backup.display().to_string())
    };
    util::atomic_write(&file, &wrap(&text))?;
    let (block, not_applied) = user_ini(&directives);
    let translated: Vec<String> = block.lines().filter_map(|l| l.split(" = ").next().map(str::to_string)).collect();
    if !block.is_empty() {
        let ini = docroot.join(".user.ini");
        let existing = std::fs::read_to_string(&ini).unwrap_or_default();
        util::atomic_write(&ini, &with_user_ini_block(&existing, &block))?;
    }
    Ok(Fixed { directives, translated, not_applied, backup: kept })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "RewriteEngine On\nphp_value upload_max_filesize 64M\nphp_flag display_errors on\n<IfModule mod_php.c>\nphp_value memory_limit 256M\n</IfModule>\nphp_admin_value open_basedir \"/var/www\"\nRewriteRule ^ index.php [L]\n";

    #[test]
    fn only_unguarded_directives_are_found() {
        let found = scan(SAMPLE);
        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["upload_max_filesize", "display_errors", "open_basedir"]);
        assert_eq!(found[2].value, "/var/www", "quotes are removed");
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn runs_of_directives_are_guarded_and_nothing_else_changes() {
        let wrapped = wrap(SAMPLE);
        assert!(wrapped.contains("<IfModule mod_php.c>\nphp_value upload_max_filesize 64M\nphp_flag display_errors on\n</IfModule>\n"), "{wrapped}");
        assert!(wrapped.contains("RewriteEngine On\n<IfModule mod_php.c>"));
        assert!(wrapped.contains("</IfModule>\nRewriteRule ^ index.php [L]\n"));
        assert!(scan(&wrapped).is_empty(), "nothing is left to fail");
        assert_eq!(wrap(&wrapped), wrapped, "guarding twice changes nothing");
        assert!(wrap("a\r\nphp_flag x on\r\nb\r\n").contains("\r\n"), "line endings are kept");
    }

    #[test]
    fn values_that_php_can_read_per_directory_are_translated_and_the_rest_are_listed() {
        let (block, skipped) = user_ini(&scan(SAMPLE));
        assert_eq!(block, "upload_max_filesize = 64M\ndisplay_errors = On");
        assert_eq!(skipped, vec!["open_basedir"]);
    }

    #[test]
    fn fixing_a_site_keeps_the_original_and_is_repeatable() {
        let dir = std::env::temp_dir().join(format!("devd-htaccess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".htaccess"), SAMPLE).unwrap();
        std::fs::write(dir.join(".user.ini"), "date.timezone = UTC\n").unwrap();
        let done = fix(&dir).unwrap();
        assert_eq!(done.translated, vec!["upload_max_filesize", "display_errors"]);
        assert_eq!(std::fs::read_to_string(dir.join(".htaccess.devd-backup")).unwrap(), SAMPLE);
        let ini = std::fs::read_to_string(dir.join(".user.ini")).unwrap();
        assert!(ini.starts_with("date.timezone = UTC\n") && ini.contains("upload_max_filesize = 64M"), "{ini}");
        assert_eq!(fix(&dir).unwrap_err().code(), "conflict", "a second run has nothing to do");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
