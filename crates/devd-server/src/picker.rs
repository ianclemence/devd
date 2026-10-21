//! The native folder picker.
//!
//! Choosing a project folder is a *user interface* act, not a domain
//! operation: it shows a dialog, the person moves around their disk, and the
//! result is a path. It therefore does not belong in the capability registry,
//! where every entry is something an agent may also do.
//!
//! The desktop shell supplies an implementation of [`FolderPicker`]. A plain
//! `devd ui` in a browser has no native dialog to show, so the endpoint
//! answers with a clear "not available here" and the web application falls
//! back to its own folder browser, which lists directories through
//! `fs.browse`.
//!
//! Either way the path that comes back is treated as untrusted input: it is
//! inspected, validated and registered through the ordinary capabilities, and
//! nothing is read from it until it has been.

use std::path::PathBuf;
use std::sync::Arc;

/// A way of asking the person for a folder.
pub trait FolderPicker: Send + Sync {
    /// Show a folder picker and return the chosen directory, if any.
    ///
    /// `title` is the heading of the dialog. `start` is the directory the
    /// dialog should open in, when there is one.
    fn pick(&self, title: &str, start: Option<PathBuf>) -> Option<PathBuf>;
}

/// A shared handle to a folder picker.
pub type Picker = Arc<dyn FolderPicker>;

/// A picker that always declines, for a server with no shell around it.
pub struct NoPicker;

impl FolderPicker for NoPicker {
    fn pick(&self, _title: &str, _start: Option<PathBuf>) -> Option<PathBuf> {
        None
    }
}
