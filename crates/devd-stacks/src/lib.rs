//! Stacks: the registry of supported frameworks and tools, the project
//! initialisation engine, project detection, task execution and toolchain
//! management that the CLI, the TUI, the window, the HTTP API and the MCP
//! server all share.

pub mod authz;
pub mod caps;
pub mod detect;
pub mod env;
pub mod exec;
pub mod init;
pub mod matrix;
pub mod model;
pub mod reconcile;
pub mod registry;
pub mod tasks;
pub mod terminal;
pub mod template;
pub mod toolchain;
pub mod tools;

pub use registry::StackRegistry;
