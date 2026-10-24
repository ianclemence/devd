//! Asking devd a question must not leave a devd home behind.
//!
//! `DEVD_HOME` can name a folder nobody made: a variable that outlived the folder it was set for,
//! a typo, a drive that is not there. A command that only inspects (`doctor`) or that is about to
//! run somebody else's program (`shim`, behind every `php` and `composer` launcher) must say so
//! and leave the disk as it found it, not create a second, empty devd.

use std::path::{Path, PathBuf};
use std::process::Command;

fn devd(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_devd")).args(args).env("DEVD_HOME", home).output().expect("devd runs")
}

fn absent(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("devd-absent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    home
}

#[test]
fn doctor_reports_a_missing_home_and_does_not_create_it() {
    let home = absent("doctor");
    let out = devd(&home, &["doctor"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("does not exist yet"), "it says why there is nothing to check: {text}");
    assert!(!home.exists(), "doctor created {}", home.display());
}

#[test]
fn a_launcher_with_no_home_says_where_it_looked_and_creates_nothing() {
    let home = absent("shim");
    let out = devd(&home, &["shim", "php", "-v"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("DEVD_HOME") && err.contains(&home.display().to_string()), "{err}");
    assert!(!home.exists(), "shim created {}", home.display());
}

#[test]
fn a_launcher_without_a_command_is_a_usage_error_not_a_crash() {
    let home = absent("usage");
    let out = devd(&home, &["shim"]);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!home.exists());
}
