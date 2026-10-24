//! Capabilities for the MCP servers devd connects to as a client.
//!
//! The separation is deliberate:
//!
//! * **Configuration** (`mcp.add`, `mcp.update`, `mcp.remove`, `mcp.enable`, `mcp.trust`) is a
//!   person's. An agent cannot add a program for devd to run, and cannot widen its own access.
//! * **Observation** (`mcp.list`, `mcp.test`) is free.
//! * **Use** (`mcp.call`) is an Exec operation for a person and waits for a person's approval when an
//!   agent calls it. A person who decided a server is trustworthy records that with `mcp.trust`; the
//!   assistant then calls it through `mcp.call_trusted`, which checks that decision in its handler so
//!   the permission is enforced in the execution layer, not in the prompt.
//!
//! Importing the servers another program already configured is one human-only operation
//! (`mcp.import`). It reads the file and reports names; it never returns environment values.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use devd_core::capability::{Ctx, Effect, Outcome, Plan, Tier};
use devd_core::registry::redact_value;
use devd_core::{DevdError, McpServer, McpTrust, Registry, Result};

use crate::client;

const MAX_TIMEOUT: u64 = 600;
const MIN_TIMEOUT: u64 = 5;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct NoArgs {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ServerArgs {
    /// The server's name.
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddArgs {
    /// A lower-case name; it is also the prefix of the tools (`mcp__<name>__…`).
    pub name: String,
    /// For a stdio server: the program to start.
    #[serde(default)]
    pub command: Option<String>,
    /// For a stdio server: its arguments.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// For an http server: the Streamable HTTP endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Environment values for a stdio server. Stored in an owner-only file, never in the database.
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    /// For a stdio server: the directory to start it in.
    #[serde(default)]
    pub cwd: Option<String>,
    /// How long one request may take, in seconds (5-600).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// `ask` (default), `allow`, or `deny`. Only a person can set it.
    #[serde(default)]
    pub trust: Option<String>,
    /// Defaults to true.
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnableArgs {
    /// The server's name.
    pub name: String,
    /// Whether devd may use it.
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TrustArgs {
    /// The server's name.
    pub name: String,
    /// `ask`, `allow` or `deny`.
    pub trust: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CallArgs {
    /// The server's name.
    pub server: String,
    /// The tool's name, as the server publishes it.
    pub tool: String,
    /// The arguments object the tool expects.
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ImportArgs {
    /// A configuration file to read. Without it, devd checks the usual places.
    #[serde(default)]
    pub path: Option<String>,
}

fn now() -> String {
    devd_core::util::now()
}

fn server_named(ctx: &Ctx, name: &str) -> Result<McpServer> {
    ctx.store.mcp_server(name)?.ok_or_else(|| DevdError::not_found(format!("no MCP server named `{name}`")))
}

fn describe(s: &McpServer) -> String {
    match s.transport.as_str() {
        "http" => s.url.clone().unwrap_or_else(|| "(no address)".into()),
        _ => {
            let mut line = s.command.clone().unwrap_or_else(|| "(no command)".into());
            for a in &s.args {
                line.push(' ');
                line.push_str(a);
            }
            line
        }
    }
}

fn valid_env_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn env_from(input: Option<BTreeMap<String, String>>) -> Result<BTreeMap<String, String>> {
    let env = input.unwrap_or_default();
    if env.len() > 64 {
        return Err(DevdError::invalid("a server may be given at most 64 environment values"));
    }
    for (k, v) in &env {
        if !valid_env_key(k) {
            return Err(DevdError::invalid(format!("`{k}` is not a valid environment variable name")));
        }
        if v.len() > 8192 {
            return Err(DevdError::invalid(format!("`{k}` is longer than 8,192 characters")));
        }
        if v.contains('\0') {
            return Err(DevdError::invalid(format!("`{k}` contains a NUL character")));
        }
    }
    Ok(env)
}

fn trust_from(s: Option<&str>) -> Result<McpTrust> {
    match s {
        None | Some("ask") => Ok(McpTrust::Ask),
        Some("allow") => Ok(McpTrust::Allow),
        Some("deny") => Ok(McpTrust::Deny),
        Some(other) => Err(DevdError::invalid(format!("`{other}` is not a trust value (ask, allow, deny)"))),
    }
}

/// Turn the arguments into a stored row, validating everything first.
fn definition(a: &AddArgs, id: String, created_at: String) -> Result<McpServer> {
    if !client::valid_name(&a.name) {
        return Err(DevdError::invalid("a server name is 1-32 lower-case letters, digits and `-`, and does not start with `-`"));
    }
    let args = a.args.clone().unwrap_or_default();
    if args.len() > 64 {
        return Err(DevdError::invalid("a server may be given at most 64 arguments"));
    }
    for arg in &args {
        if arg.len() > 4096 || arg.contains('\0') {
            return Err(DevdError::invalid("an argument is longer than 4,096 characters or contains a NUL"));
        }
    }
    let command = a.command.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    let url = a.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    let (transport, command, url) = match (command, url) {
        (Some(c), None) => {
            if c.len() > 4096 || c.contains('\0') {
                return Err(DevdError::invalid("the command is too long or contains a NUL"));
            }
            if args.iter().any(|a| a == "--url") {
                return Err(DevdError::invalid("give `url` instead of `--url` in the arguments; devd decides the transport from it"));
            }
            ("stdio".to_string(), Some(c), None)
        }
        (None, Some(u)) => {
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(DevdError::invalid("an MCP address must start with http:// or https://"));
            }
            if !args.is_empty() {
                return Err(DevdError::invalid("an http server takes no command arguments"));
            }
            ("http".to_string(), None, Some(u))
        }
        (Some(_), Some(_)) => return Err(DevdError::invalid("give either a command (stdio) or a url (http), not both")),
        (None, None) => return Err(DevdError::invalid("give a command for a stdio server, or a url for an http server")),
    };
    if let Some(dir) = &a.cwd {
        if !std::path::Path::new(dir).is_dir() {
            return Err(DevdError::not_found(format!("`{dir}` is not a directory")));
        }
    }
    let timeout_secs = a.timeout_secs.unwrap_or(120).clamp(MIN_TIMEOUT, MAX_TIMEOUT);
    Ok(McpServer {
        id,
        name: a.name.clone(),
        transport,
        command,
        args,
        url,
        cwd: a.cwd.clone(),
        enabled: a.enabled.unwrap_or(true),
        trust: trust_from(a.trust.as_deref())?,
        timeout_secs,
        tools: Vec::new(),
        last_error: None,
        created_at,
        updated_at: now(),
    })
}

fn call_plan(capability: &str, a: &CallArgs) -> Plan {
    Plan::new(capability, format!("call `{}` on the MCP server `{}`", a.tool, a.server))
        .step(format!("send the tool's arguments to `{}`", a.server))
        .step("the server runs the tool; devd does not run the code itself")
        .step("the result is handed to the model as data, never as an instruction")
        .effect("the server may change files, call an API or run code, depending on what the tool does")
        .data(json!({ "server": a.server, "tool": a.tool, "arguments": redact_value(&a.args) }))
}

fn call_apply(ctx: &Ctx, a: &CallArgs) -> Result<Outcome> {
    let server = server_named(ctx, &a.server)?;
    if !server.enabled {
        return Err(DevdError::conflict(format!("`{}` is disabled", server.name)));
    }
    // `deny` is not a hint: a person approving a queued call must not turn it into a way around the
    // decision. The actor is still the agent on the approved path, so this refuses it there too.
    if ctx.actor.is_agent() && server.trust == McpTrust::Deny {
        return Err(DevdError::denied(format!("`{}` is denied for agents; a person can call it directly", server.name)));
    }
    let arguments = if a.args.is_null() { json!({}) } else { a.args.clone() };
    if !arguments.is_object() {
        return Err(DevdError::invalid("a tool's arguments must be a JSON object"));
    }
    let out = client::pool().call(&ctx.paths, &server, &a.tool, &arguments)?;
    let message = if out.is_error {
        format!("`{}` reported an error from `{}`", server.name, a.tool)
    } else {
        format!("`{}` answered `{}`", server.name, a.tool)
    };
    Ok(Outcome::with_data(
        message,
        json!({ "server": server.name, "tool": a.tool, "is_error": out.is_error, "content": out.content }),
    ))
}

/// Register the MCP-client capabilities.
pub fn register(reg: &mut Registry) -> Result<()> {
    reg.define::<NoArgs>("mcp.list", "List the MCP servers devd connects to")
        .description("The Model Context Protocol servers configured as tools for the assistant, with what each is, whether it is enabled, what agents may do with it, how many tools it last published, and why it last failed. Environment values are never returned, only their names.")
        .group("mcp")
        .tier(Tier::Read)
        .effect(Effect::Read)
        .agent_safe()
        .handlers(
            |_ctx, _a| Ok(Plan::new("mcp.list", "read the configured MCP servers")),
            |ctx, _a| {
                let servers = ctx.store.mcp_servers()?;
                let rows: Vec<Value> = servers
                    .iter()
                    .map(|s| {
                        json!({
                            "id": s.id,
                            "name": s.name,
                            "transport": s.transport,
                            "command": s.command,
                            "args": s.args,
                            "url": s.url,
                            "cwd": s.cwd,
                            "enabled": s.enabled,
                            "trust": s.trust.as_str(),
                            "timeout_secs": s.timeout_secs,
                            "tool_count": s.tools.len(),
                            "tools": s.tools.iter().map(|t| json!({ "name": t.name, "description": t.description })).collect::<Vec<_>>(),
                            "last_error": s.last_error,
                            "env_keys": client::env_keys(&ctx.paths, &s.id),
                        })
                    })
                    .collect();
                Ok(Outcome::with_data(format!("{} MCP server(s)", rows.len()), json!({ "servers": rows })))
            },
        )?;

    reg.define::<AddArgs>("mcp.add", "Connect to another program's MCP server")
        .description("Adds a Model Context Protocol server for the assistant to use, the way Claude Code connects to the servers configured for it. A stdio server is a program devd starts and speaks JSON-RPC to; an http server is a Streamable HTTP endpoint. The environment is stored in an owner-only file, never in the database. `trust` decides what agents may do with its tools: ask (default) makes every agent call wait for a person, allow lets them run, deny hides them. Human only: this decides which programs devd runs.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .handlers(
            |_ctx, a| {
                Ok(Plan::new("mcp.add", format!("add the MCP server `{}`", a.name))
                    .step(match (&a.command, &a.url) {
                        (Some(c), _) => format!("start `{c}` and speak MCP to it"),
                        (_, Some(u)) => format!("connect to {u} over Streamable HTTP"),
                        _ => "give a command or a url".to_string(),
                    })
                    .step("store any environment values in an owner-only file")
                    .step(format!("agents: {}", a.trust.as_deref().unwrap_or("ask")))
                    .effect("the assistant may offer this server's tools"))
            },
            |ctx, a| {
                if ctx.store.mcp_server(&a.name)?.is_some() {
                    return Err(DevdError::conflict(format!("there is already an MCP server named `{}`; remove it or use mcp.update", a.name)));
                }
                let env = env_from(a.env.clone())?;
                let server = definition(a, devd_core::util::new_id(), now())?;
                ctx.store.mcp_put(&server)?;
                if let Err(e) = client::save_env(&ctx.paths, &server.id, &env) {
                    let _ = ctx.store.mcp_delete(&server.id);
                    return Err(e);
                }
                Ok(Outcome::with_data(
                    format!("added `{}` ({})", server.name, server.transport),
                    json!({ "id": server.id, "name": server.name, "transport": server.transport, "trust": server.trust.as_str() }),
                ))
            },
        )?;

    reg.define::<AddArgs>("mcp.update", "Change an MCP server")
        .description("Replaces an MCP server's definition: its command or url, arguments, working directory, environment, timeout, trust and whether it is enabled. Omitted environment values are removed. Human only.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .handlers(
            |ctx, a| {
                let existing = server_named(ctx, &a.name)?;
                let after = match (&a.command, &a.url) {
                    (Some(c), _) => c.clone(),
                    (_, Some(u)) => u.clone(),
                    _ => existing.command.clone().unwrap_or_default(),
                };
                Ok(Plan::new("mcp.update", format!("change the MCP server `{}`", existing.name))
                    .step(format!("now: {}", describe(&existing)))
                    .step(format!("after: {after}"))
                    .step("the next call reconnects with the new definition"))
            },
            |ctx, a| {
                let existing = server_named(ctx, &a.name)?;
                let env = env_from(a.env.clone())?;
                let mut server = definition(a, existing.id.clone(), existing.created_at.clone())?;
                server.tools = existing.tools.clone();
                ctx.store.mcp_put(&server)?;
                client::save_env(&ctx.paths, &server.id, &env)?;
                client::pool().forget(&server.id);
                Ok(Outcome::with_data(format!("changed `{}`", server.name), json!({ "id": server.id, "name": server.name })))
            },
        )?;

    reg.define::<ServerArgs>("mcp.remove", "Forget an MCP server")
        .description("Removes an MCP server from devd, its owner-only environment file and its live connection. It does not uninstall the program the server runs. Human only.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .handlers(
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                Ok(Plan::new("mcp.remove", format!("remove the MCP server `{}`", s.name)).step("its tools stop being offered to the assistant").step("its environment file is deleted").step("the program itself is not uninstalled"))
            },
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                ctx.store.mcp_delete(&s.id)?;
                client::delete_env(&ctx.paths, &s.id);
                client::pool().forget(&s.id);
                Ok(Outcome::ok(format!("removed `{}`", s.name)))
            },
        )?;

    reg.define::<EnableArgs>("mcp.enable", "Turn an MCP server on or off")
        .description("Enables or disables an MCP server without forgetting it. A disabled server's tools are not offered and cannot be called. Human only.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .idempotent()
        .handlers(
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                Ok(Plan::new("mcp.enable", format!("{} `{}`", if a.enabled { "enable" } else { "disable" }, s.name)))
            },
            |ctx, a| {
                let mut s = server_named(ctx, &a.name)?;
                s.enabled = a.enabled;
                s.updated_at = now();
                ctx.store.mcp_put(&s)?;
                if !a.enabled {
                    client::pool().forget(&s.id);
                }
                Ok(Outcome::ok(format!("{} `{}`", if a.enabled { "enabled" } else { "disabled" }, s.name)))
            },
        )?;

    reg.define::<TrustArgs>("mcp.trust", "Decide what agents may do with an MCP server")
        .description("Sets whether an agent's call of this server's tools waits for a person (`ask`, the default), may run without asking (`allow`), or is refused (`deny`). This is a security decision: `allow` lets a model use whatever the server can do without a person in the loop. Human only.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .idempotent()
        .handlers(
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                let trust = trust_from(Some(&a.trust))?;
                Ok(Plan::new("mcp.trust", format!("set `{}` to {}", s.name, trust.as_str()))
                    .step(match trust {
                        McpTrust::Ask => "an agent's call waits for a person, every time",
                        McpTrust::Allow => "an agent may call these tools without asking; the server acts with your permissions",
                        McpTrust::Deny => "agents cannot see or call these tools",
                    }))
            },
            |ctx, a| {
                let mut s = server_named(ctx, &a.name)?;
                s.trust = trust_from(Some(&a.trust))?;
                s.updated_at = now();
                ctx.store.mcp_put(&s)?;
                Ok(Outcome::ok(format!("`{}` is now `{}` for agents", s.name, s.trust.as_str())))
            },
        )?;

    reg.define::<ServerArgs>("mcp.test", "Connect to an MCP server and list its tools")
        .description("Starts or connects to an MCP server, performs the initialise handshake, and asks for its tools. Records what it found (or the error) on the server row. Changes no files and calls no tool.")
        .group("mcp")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .dry_run()
        .handlers(
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                Ok(Plan::new("mcp.test", format!("connect to `{}`", s.name)).step(format!("run or reach {}", describe(&s))).step("ask for its tool list").step("change nothing else"))
            },
            |ctx, a| {
                let s = server_named(ctx, &a.name)?;
                match client::pool().test(&ctx.paths, &s) {
                    Ok(tools) => {
                        ctx.store.mcp_set_catalogue(&s.id, &tools, None)?;
                        Ok(Outcome::with_data(
                            format!("`{}` published {} tool(s)", s.name, tools.len()),
                            json!({ "server": s.name, "tools": tools }),
                        ))
                    }
                    Err(e) => {
                        ctx.store.mcp_set_catalogue(&s.id, &[], Some(&e.to_string()))?;
                        Err(e)
                    }
                }
            },
        )?;

    reg.define::<CallArgs>("mcp.call", "Call a tool on an MCP server")
        .description("Calls one tool of one configured MCP server and returns its result as data. An agent's call waits for a person's approval unless `mcp.trust` allowed the server. A person calling it directly is not asked, because they chose the server.")
        .group("mcp")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .agent_needs_approval()
        .dry_run()
        .handlers(
            |_ctx, a| Ok(call_plan("mcp.call", a)),
            call_apply,
        )?;

    reg.define::<CallArgs>("mcp.call_trusted", "Call a trusted MCP server's tool without asking")
        .description("The path the assistant uses for a server a person marked `allow`. The handler itself refuses an agent unless the stored trust is exactly `allow`, so the decision is enforced where the call runs and not in a prompt. Not for other callers; use mcp.call.")
        .group("mcp")
        .tier(Tier::Exec)
        .effect(Effect::Process)
        .agent_safe()
        .dry_run()
        .handlers(
            |_ctx, a| Ok(call_plan("mcp.call_trusted", a)),
            |ctx, a| {
                if ctx.actor.is_agent() {
                    let server = server_named(ctx, &a.server)?;
                    if server.trust != McpTrust::Allow {
                        return Err(DevdError::denied(format!(
                            "`{}` is not trusted for agents (it is `{}`); use mcp.call, which waits for a person",
                            server.name,
                            server.trust.as_str()
                        )));
                    }
                }
                call_apply(ctx, a)
            },
        )?;

    reg.define::<ImportArgs>("mcp.import", "Import the MCP servers another program configured")
        .description("Reads an MCP configuration file and adds the servers it defines. Without a path, devd checks Claude Code's ~/.claude.json, Claude Desktop's claude_desktop_config.json, Cursor's ~/.cursor/mcp.json and a project's .mcp.json. Existing names are left alone and reported. Environment values are written to owner-only files and never returned. Human only.")
        .group("mcp")
        .tier(Tier::Write)
        .effect(Effect::Config)
        .human_only()
        .dry_run()
        .handlers(
            |_ctx, a| {
                let found = discover_configs(a.path.as_deref())?;
                let names: Vec<String> = found.entries.iter().map(|e| e.name.clone()).collect();
                Ok(Plan::new("mcp.import", format!("import {} MCP server(s) from {} file(s)", names.len(), found.files.len()))
                    .step(if found.files.is_empty() { "no configuration file was found".to_string() } else { format!("read {}", found.files.join(", ")) })
                    .step(if names.is_empty() { "nothing to import".to_string() } else { format!("add: {}", names.join(", ")) })
                    .step("servers already configured with the same name are left alone")
                    .data(json!({ "names": names, "files": found.files })))
            },
            |ctx, a| {
                let found = discover_configs(a.path.as_deref())?;
                let mut imported = Vec::new();
                let mut skipped = Vec::new();
                for entry in found.entries {
                    if ctx.store.mcp_server(&entry.name)?.is_some() {
                        skipped.push(json!({ "name": entry.name, "reason": "already configured" }));
                        continue;
                    }
                    if !client::valid_name(&entry.name) {
                        skipped.push(json!({ "name": entry.name, "reason": "not a usable name" }));
                        continue;
                    }
                    let name = entry.name.clone();
                    let env = match env_from(entry.env.clone()) {
                        Ok(e) => e,
                        Err(e) => {
                            skipped.push(json!({ "name": name, "reason": e.to_string() }));
                            continue;
                        }
                    };
                    let server = match entry.into_server(devd_core::util::new_id()) {
                        Ok(s) => s,
                        Err(e) => {
                            skipped.push(json!({ "name": name, "reason": e.to_string() }));
                            continue;
                        }
                    };
                    ctx.store.mcp_put(&server)?;
                    client::save_env(&ctx.paths, &server.id, &env)?;
                    imported.push(server.name);
                }
                Ok(Outcome::with_data(
                    format!("imported {} server(s)", imported.len()),
                    json!({ "imported": imported, "skipped": skipped, "files": found.files }),
                ))
            },
        )?;

    Ok(())
}

// ----------------------------------------------------------------------------------------- import

/// A server found in another program's configuration.
struct Found {
    name: String,
    command: Option<String>,
    args: Vec<String>,
    env: Option<BTreeMap<String, String>>,
    url: Option<String>,
}

impl Found {
    fn into_server(self, id: String) -> Result<McpServer> {
        let args = AddArgs {
            name: self.name,
            command: self.command,
            args: Some(self.args),
            url: self.url,
            env: None,
            cwd: None,
            timeout_secs: None,
            trust: None,
            enabled: Some(true),
        };
        definition(&args, id, now())
    }
}

struct Discovered {
    files: Vec<String>,
    entries: Vec<Found>,
}

fn discover_configs(explicit: Option<&str>) -> Result<Discovered> {
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Some(p) = explicit {
        let path = std::path::PathBuf::from(p);
        if !path.is_file() {
            return Err(DevdError::not_found(format!("`{p}` is not a file")));
        }
        paths.push(path);
    } else {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            let home = std::path::PathBuf::from(home);
            paths.push(home.join(".claude.json"));
            paths.push(home.join(".cursor").join("mcp.json"));
        }
        if let Some(app) = std::env::var_os("APPDATA") {
            paths.push(std::path::PathBuf::from(app).join("Claude").join("claude_desktop_config.json"));
        }
        if let Ok(cwd) = std::env::current_dir() {
            paths.push(cwd.join(".mcp.json"));
        }
    }
    let mut files = Vec::new();
    let mut entries: Vec<Found> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        files.push(path.display().to_string());
        let mut blocks = Vec::new();
        if let Some(map) = value.get("mcpServers") {
            blocks.push(map.clone());
        }
        if let Some(projects) = value.get("projects").and_then(Value::as_object) {
            for (_, project) in projects {
                if let Some(map) = project.get("mcpServers") {
                    blocks.push(map.clone());
                }
            }
        }
        for block in blocks {
            let Some(map) = block.as_object() else { continue };
            for (name, spec) in map {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let command = spec.get("command").and_then(Value::as_str).map(str::to_string);
                let url = spec
                    .get("url")
                    .or_else(|| spec.get("serverUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let args: Vec<String> = spec
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str()).map(str::to_string).collect())
                    .unwrap_or_default();
                let env: Option<BTreeMap<String, String>> = spec.get("env").and_then(Value::as_object).map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                });
                entries.push(Found { name: name.clone(), command, args, env, url });
            }
        }
    }
    Ok(Discovered { files, entries })
}
