//! `devd-core` - the domain model, capability registry, state store and process
//! supervision primitives shared by every `devd` interface.
//!
//! The central design rule of `devd` lives here: **every capability is declared
//! once** in a [`Registry`], and the CLI, TUI, GUI, HTTP API and MCP server are
//! all thin projections of that registry. No interface re-implements business
//! logic, and no interface can bypass the permission checks enforced by
//! [`Registry::dispatch`].

pub mod capability;
pub mod config;
pub mod events;
pub mod health;
pub mod paths;
pub mod ports;
pub mod process;
pub mod registry;
pub mod state;
pub mod util;

pub use capability::{
    Actor, Approval, Capability, Ctx, Effect, Handler, InvokeRequest, InvokeResult, Outcome,
    Plan, Surface, Tier,
};
pub use config::Settings;
pub use error::{DevdError, Result};
pub use events::{Event, EventBus};
pub use paths::Paths;
pub use registry::Registry;
pub use state::Store;

/// Error type shared across the workspace.
pub mod error {
    /// Errors produced by `devd`. Every variant carries a stable
    /// [`DevdError::code`] so that machine interfaces (HTTP, MCP) can map
    /// failures to structured responses without string matching.
    #[derive(Debug, thiserror::Error)]
    pub enum DevdError {
        #[error("io error: {0}")]
        Io(#[from] std::io::Error),
        #[error("json error: {0}")]
        Json(#[from] serde_json::Error),
        #[error("toml decode error: {0}")]
        TomlDecode(#[from] toml::de::Error),
        #[error("toml encode error: {0}")]
        TomlEncode(#[from] toml::ser::Error),
        #[error("state store error: {0}")]
        Db(#[from] rusqlite::Error),
        #[error("not found: {0}")]
        NotFound(String),
        #[error("invalid input: {0}")]
        Invalid(String),
        #[error("unsupported: {0}")]
        Unsupported(String),
        #[error("conflict: {0}")]
        Conflict(String),
        #[error("permission denied: {0}")]
        Denied(String),
        #[error("approval required for '{capability}': {reason}")]
        ApprovalRequired {
            /// Capability that was blocked.
            capability: String,
            /// Why approval is required.
            reason: String,
        },
        #[error("`{program}` failed with exit code {code}: {stderr}")]
        Command {
            /// Program that was executed.
            program: String,
            /// Process exit code (`-1` when terminated by a signal).
            code: i32,
            /// Captured standard error, truncated.
            stderr: String,
        },
        #[error("timed out: {0}")]
        Timeout(String),
        #[error("internal error: {0}")]
        Internal(String),
    }

    /// Convenience alias.
    pub type Result<T> = std::result::Result<T, DevdError>;

    impl DevdError {
        /// Construct an [`DevdError::Invalid`].
        pub fn invalid(msg: impl Into<String>) -> Self {
            Self::Invalid(msg.into())
        }
        /// Construct an [`DevdError::NotFound`].
        pub fn not_found(msg: impl Into<String>) -> Self {
            Self::NotFound(msg.into())
        }
        /// Construct an [`DevdError::Denied`].
        pub fn denied(msg: impl Into<String>) -> Self {
            Self::Denied(msg.into())
        }
        /// Construct an [`DevdError::Internal`].
        pub fn internal(msg: impl Into<String>) -> Self {
            Self::Internal(msg.into())
        }
        /// Construct an [`DevdError::Unsupported`].
        pub fn unsupported(msg: impl Into<String>) -> Self {
            Self::Unsupported(msg.into())
        }
        /// Construct an [`DevdError::Conflict`].
        pub fn conflict(msg: impl Into<String>) -> Self {
            Self::Conflict(msg.into())
        }

        /// Stable machine-readable error code, used by the HTTP and MCP
        /// surfaces.
        pub fn code(&self) -> &'static str {
            match self {
                Self::Io(_) => "io_error",
                Self::Json(_) => "json_error",
                Self::TomlDecode(_) | Self::TomlEncode(_) => "config_error",
                Self::Db(_) => "state_error",
                Self::NotFound(_) => "not_found",
                Self::Invalid(_) => "invalid_input",
                Self::Unsupported(_) => "unsupported",
                Self::Conflict(_) => "conflict",
                Self::Denied(_) => "permission_denied",
                Self::ApprovalRequired { .. } => "approval_required",
                Self::Command { .. } => "command_failed",
                Self::Timeout(_) => "timeout",
                Self::Internal(_) => "internal_error",
            }
        }

        /// Whether this error means "the caller must obtain approval".
        pub fn is_approval_required(&self) -> bool {
            matches!(self, Self::ApprovalRequired { .. })
        }
    }
}
