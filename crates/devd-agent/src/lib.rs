//! The built-in agent.
//!
//! devd already lets outside agents (Claude Code, OpenCode, anything speaking
//! MCP) drive it. This crate adds the other half: an agent that lives inside
//! devd, with the same permission model, so the server is useful without any
//! other tool installed.
//!
//! The design follows Anthropic's engineering guidance on agents:
//!
//! * **Simple, composable pieces** ("Building effective agents"): one loop that
//!   calls a model, runs the tools it asks for and feeds the results back. No
//!   framework, no hidden state.
//! * **Brain, hands and session decoupled** ("Scaling Managed Agents"): the
//!   conversation is an append-only log in SQLite. The loop holds nothing, so a
//!   crash is recovered by reading the log; tools are called as `execute(name,
//!   input) -> string` through the capability registry; credentials never enter
//!   the model's context.
//! * **Few, well-described tools with progressive disclosure** ("Writing tools
//!   for agents", "Advanced tool use"): the model gets the same five discovery
//!   and invocation tools the MCP server publishes and loads capability schemas
//!   on demand, instead of hundreds of tool definitions up front.
//! * **Context engineering**: tool results are size-capped, old results are
//!   cleared from the working context while staying in the log, and the project
//!   brief is a short summary, not a dump.
//! * **Human approval where it matters** ("Claude Code auto mode", "sandboxing"):
//!   operations above the agent tier are queued for a person with their plan, and
//!   the agent is told plainly that it must not claim they happened.

pub mod caps;
pub mod chat;
pub mod dictation;
pub mod docs;
pub mod fs;
pub mod health;
pub mod host;
pub mod judge;
pub mod providers;
pub mod requirements;
pub mod search;
pub mod roles;
pub mod secrets;
pub mod skills;
pub mod skills_data;
pub mod skills_install;
pub mod team;

pub use chat::Hub;
