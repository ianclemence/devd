//! The operating system's own folder dialog.
//!
//! This is the one capability a browser page cannot have for itself, and the
//! one thing the native shell adds to the shared backend. It returns a path
//! and nothing else: no file is read, no program is run, and the path is
//! treated as untrusted input by everything that acts on it afterwards.

use std::path::PathBuf;

use devd_server::picker::FolderPicker;

/// A folder dialog backed by the platform.
pub struct NativePicker;

impl FolderPicker for NativePicker {
    fn pick(&self, title: &str, start: Option<PathBuf>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new();
        dialog = dialog.set_title(title);
        if let Some(dir) = start {
            // A directory that has gone away should not stop the dialog from
            // opening; it simply starts somewhere else.
            if dir.is_dir() {
                dialog = dialog.set_directory(dir);
            }
        }
        dialog.pick_folder()
    }
}
