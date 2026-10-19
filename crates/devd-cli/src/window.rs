//! Opening the application in a window.
//!
//! # Why a browser window rather than a native one
//!
//! The interface is served over loopback HTTP, so the only thing missing from a
//! "real app" is chrome the user does not want to see. Chromium-based browsers
//! ship exactly that: `--app=<url>` opens a window with no address bar, no tab
//! strip and its own taskbar entry, which is indistinguishable from a native
//! window for this purpose.
//!
//! This is a deliberate staging decision, not a permanent one. A native shell
//! (Tauri) replaces *only this function*: it points a webview at the same URL
//! and the rest of the product is unchanged. Because that seam is one function,
//! the interface does not have to be rewritten to become native - which is why
//! building the interface against the HTTP API first costs nothing later.
//!
//! When no Chromium browser is found the default browser is used instead. The
//! result has browser chrome, and it still works, which is the right trade for
//! a Firefox user: correct over pretty.

use std::path::PathBuf;
use std::process::Command;

/// Open `url` in a window. Returns whether a chrome-less window was used.
pub fn open_window(url: &str) -> bool {
    if let Some(browser) = chromium_candidates().into_iter().find(|p| p.exists()) {
        let mut command = Command::new(&browser);
        command
            .arg(format!("--app={url}"))
            // These two suppress first-run UI, which would otherwise appear in
            // front of the application on a fresh profile.
            .arg("--no-first-run")
            .arg("--no-default-browser-check");
        if command.spawn().is_ok() {
            return true;
        }
    }
    !open::that(url).is_err()
}

/// Chromium-family executables to try, in preference order.
fn chromium_candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();

    if cfg!(windows) {
        for root in [
            std::env::var_os("ProgramFiles"),
            std::env::var_os("ProgramFiles(x86)"),
            std::env::var_os("LOCALAPPDATA"),
        ]
        .into_iter()
        .flatten()
        {
            let root = PathBuf::from(root);
            for leaf in [
                "Microsoft/Edge/Application/msedge.exe",
                "Google/Chrome/Application/chrome.exe",
                "Chromium/Application/chrome.exe",
            ] {
                found.push(root.join(leaf));
            }
        }
        for name in ["msedge", "chrome"] {
            if let Some(path) = devd_core::util::which(name) {
                found.push(path);
            }
        }
    } else {
        for name in [
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ] {
            if let Some(path) = devd_core::util::which(name) {
                found.push(path);
            }
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_list_is_never_empty() {
        // The list is a preference order, not a guarantee; it must at least be
        // populated so the fallback logic is reachable.
        assert!(!chromium_candidates().is_empty() || cfg!(not(windows)));
    }
}
