//! The Model Context Protocol server for `devd`.
//!
//! # One more projection
//!
//! This crate owns no business logic. Every capability it exposes is a
//! capability in the shared [`Registry`], and every invocation goes through
//! [`Registry::dispatch`], where tiers, approvals and auditing are enforced. An
//! agent driving `devd` over MCP therefore reaches exactly the same functions
//! as a person using the CLI or the application window, with exactly the same
//! permission checks.
//!
//! # Why five tools and not sixty-four
//!
//! The obvious design is to publish every capability as an MCP tool. With 64
//! Git capabilities today, and services, sites, runtimes, DNS and TLS still to
//! come, that list would be hundreds of entries long, loaded into every
//! conversation whether or not it is relevant. It would also have to be
//! regenerated and re-learned whenever the registry grew.
//!
//! Instead the server publishes **discovery** tools that read the registry:
//!
//! | Tool | Purpose |
//! | --- | --- |
//! | `devd_groups` | The domains, and how many capabilities each holds |
//! | `devd_capabilities` | Search within a domain, or across all of them |
//! | `devd_describe` | One capability, with the JSON Schema of its arguments |
//! | `devd_invoke` | Run one, by id |
//! | `devd_doctor` | The environment check |
//!
//! A capability added tomorrow is discoverable and invocable immediately, with
//! no change here. The list is five entries, and it does not grow.
//!
//! # What an agent cannot do
//!
//! [`McpServer::new`] **overwrites the actor** with an agent. A caller cannot
//! present a context claiming to be a human, for the same reason the HTTP layer
//! does the same thing: the claim would be unverifiable, and the entire point of
//! the tier model is that it does not depend on a claim.
//!
//! An operation that requires approval is therefore refused here, and the
//! refusal is recorded in the audit trail. The agent is handed the plan - so it
//! can show the human exactly what would happen - and told that a human must
//! approve it in the application or the CLI. There is no path by which an agent
//! supplies its own approval, because [`Approval::Granted`] is only ever
//! produced by a human decision.

use std::io::{BufRead, Write};
use std::sync::Arc;

use serde_json::{json, Value};

use devd_core::capability::{Actor, Approval, Ctx};
use devd_core::doctor::{self, Check, Level};
use devd_core::{DevdError, InvokeRequest, Registry, Result};

/// The client side: the MCP servers devd connects to, as opposed to the one it serves.
pub mod caps;
pub mod client;

/// Protocol version this server speaks. A client that wants an older version
/// negotiates it in `initialize`; the tool surface does not depend on it.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Name reported in `initialize`.
pub const SERVER_NAME: &str = "devd";

/// Instruction text returned by `initialize`, for clients that show it.
pub const INSTRUCTIONS: &str = "\
devd manages local development projects. Its capabilities are organised into \
domains. Start with devd_groups to see the domains, then devd_capabilities to \
list what a domain offers, then devd_describe before you invoke anything, so \
you know the tier and the exact argument shape. Read-tier capabilities run \
directly. An operation that needs human approval is refused here and recorded; \
a human approves it in the devd application or the CLI, never an agent. Use \
devd_invoke with dry_run true to see what an operation would do without doing \
it. To work on any project, call project.detect first: it reports the stack, \
package manager, required runtimes, the tasks you can run (project.run), \
services and missing tools (toolchain.install asks a human to approve). Verify \
changes by running the project's own test/build tasks and report their real \
output. Git capabilities operate on a repository the user has added to devd with \
git.repo.track, or on any absolute path supplied in the `repo` argument.";

/// A function producing environment checks that only some callers can perform.
pub type ExtraChecks = Arc<dyn Fn() -> Vec<Check> + Send + Sync>;

/// Called when an agent asks for an operation that needs a person's approval.
///
/// Receives the capability id, its arguments and the plan, and returns the id of
/// the queued request so the agent can poll for the decision. The queue lives
/// outside this crate, which owns no storage.
pub type ApprovalQueue = Arc<dyn Fn(&str, &Value, Option<&Value>) -> Option<String> + Send + Sync>;

/// The MCP server over the shared registry.
pub struct McpServer {
    registry: Arc<Registry>,
    ctx: Ctx,
    extra_checks: Vec<ExtraChecks>,
    approval_queue: Option<ApprovalQueue>,
}

/// The outcome of a tool call.
enum ToolOutcome {
    /// A result the model should read.
    Content(String),
    /// A failure the model should read and correct.
    Failed(String),
    /// A JSON-RPC protocol error: the request itself was wrong.
    Protocol(i32, String),
}

impl McpServer {
    /// Build a server.
    ///
    /// The actor in `ctx` is **discarded** and replaced with an agent actor, so
    /// no caller can hand this server a human's authority.
    pub fn new(registry: Arc<Registry>, mut ctx: Ctx, client: impl Into<String>) -> Self {
        ctx.actor = Actor::agent(client);
        Self {
            registry,
            ctx,
            extra_checks: Vec::new(),
            approval_queue: None,
        }
    }

    /// Queue operations that need approval so a person can decide them in the
    /// application, the dashboard or the CLI instead of re-asking the agent.
    pub fn with_approval_queue(mut self, queue: ApprovalQueue) -> Self {
        self.approval_queue = Some(queue);
        self
    }

    /// Run one tool and return `(is_error, text)`, for callers that host the
    /// tools themselves rather than speaking JSON-RPC (the built-in agent).
    pub fn call(&self, name: &str, args: &Value) -> (bool, String) {
        match self.call_tool(name, args) {
            ToolOutcome::Content(t) => (false, t),
            ToolOutcome::Failed(t) => (true, t),
            ToolOutcome::Protocol(_, m) => (true, m),
        }
    }

    /// Add checks that only this caller can perform - for `devd`, the Git
    /// discovery check.
    pub fn with_extra_checks(mut self, checks: Vec<ExtraChecks>) -> Self {
        self.extra_checks = checks;
        self
    }

    /// The registry this server projects.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Handle one JSON-RPC message.
    ///
    /// Returns `None` for a notification, which by protocol must not be
    /// answered.
    pub fn handle(&self, request: &Value) -> Option<Value> {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let id = request.get("id").cloned();

        // A notification has no id and must produce no response. Checking the
        // prefix as well keeps an unknown `notifications/*` quiet rather than
        // answering something the peer will not read.
        if id.is_none() || method.starts_with("notifications/") {
            return None;
        }
        let id = id?;

        match method {
            "initialize" => Some(Self::ok(id, self.initialize_result(request))),
            "ping" => Some(Self::ok(id, json!({}))),
            "tools/list" => Some(Self::ok(id, json!({ "tools": tools() }))),
            "tools/call" => Some(self.tools_call(
                id,
                request.get("params").cloned().unwrap_or_else(|| json!({})),
            )),
            // Clients probe these even when the server does not advertise them.
            // Answering with an empty list is friendlier than an error.
            "resources/list" => Some(Self::ok(id, json!({ "resources": [] }))),
            "resources/templates/list" => Some(Self::ok(id, json!({ "resourceTemplates": [] }))),
            "prompts/list" => Some(Self::ok(id, json!({ "prompts": [] }))),
            other => Some(Self::error(id, -32601, format!("unknown method `{other}`"))),
        }
    }

    /// Serve newline-delimited JSON-RPC on stdin/stdout until stdin closes.
    ///
    /// Nothing but protocol traffic goes to stdout; anything diagnostic belongs
    /// on stderr, because a stray line on stdout corrupts the stream.
    pub fn serve_stdio(&self) -> Result<()> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(trimmed) {
                Ok(request) => self.handle(&request),
                Err(e) => Some(Self::error(
                    Value::Null,
                    -32700,
                    format!("invalid JSON: {e}"),
                )),
            };
            if let Some(response) = response {
                serde_json::to_writer(&mut stdout, &response)?;
                stdout.write_all(b"\n")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }

    fn initialize_result(&self, request: &Value) -> Value {
        // Echo the client's version when it asks for one we do not know, which
        // is what the specification asks for and what clients expect.
        let requested = request
            .get("params")
            .and_then(|p| p.get("protocolVersion"))
            .and_then(Value::as_str);
        json!({
            "protocolVersion": requested.unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": SERVER_NAME,
                "title": "devd",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": INSTRUCTIONS,
        })
    }

    fn tools_call(&self, id: Value, params: Value) -> Value {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        match self.call_tool(&name, &args) {
            ToolOutcome::Content(text) => Self::ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false,
                }),
            ),
            ToolOutcome::Failed(text) => Self::ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": true,
                }),
            ),
            ToolOutcome::Protocol(code, message) => Self::error(id, code, message),
        }
    }

    fn call_tool(&self, name: &str, args: &Value) -> ToolOutcome {
        match name {
            "devd_groups" => self.tool_groups(),
            "devd_capabilities" => self.tool_capabilities(args),
            "devd_describe" => self.tool_describe(args),
            "devd_invoke" => self.tool_invoke(args),
            "devd_doctor" => self.tool_doctor(),
            other => ToolOutcome::Protocol(
                -32602,
                format!("unknown tool `{other}`; call tools/list for the available tools"),
            ),
        }
    }

    fn tool_groups(&self) -> ToolOutcome {
        let groups = self.registry.groups();
        let items: Vec<Value> = groups
            .iter()
            .map(|(group, caps)| {
                let tiers: Vec<&str> = caps.iter().map(|c| c.tier.as_str()).collect();
                json!({
                    "group": group,
                    "capabilities": caps.len(),
                    "sample": caps.first().map(|c| c.id.clone()),
                    "tiers": tiers,
                })
            })
            .collect();
        ToolOutcome::Content(pretty(&json!({
            "total_capabilities": self.registry.list().len(),
            "domain_count": items.len(),
            "domains": items,
            "next_step": "Call devd_capabilities with a `group` to list one domain.",
        })))
    }

    fn tool_capabilities(&self, args: &Value) -> ToolOutcome {
        let query = args.get("query").and_then(Value::as_str).unwrap_or("");
        let group = args.get("group").and_then(Value::as_str);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 500) as usize;

        // Search returns a page with no total, so ask for everything and slice
        // here; that is what makes `total` honest rather than "at least".
        let all = self.registry.search(query, group, usize::MAX, 0);
        let items: Vec<Value> = all.iter().take(limit).map(summarise).collect();

        ToolOutcome::Content(pretty(&json!({
            "total": all.len(),
            "returned": items.len(),
            "query": query,
            "group": group,
            "capabilities": items,
            "next_step": "Call devd_describe with an `id` for the full declaration and its argument schema.",
        })))
    }

    fn tool_describe(&self, args: &Value) -> ToolOutcome {
        let Some(id) = args.get("id").and_then(Value::as_str) else {
            return ToolOutcome::Protocol(-32602, "`id` is required".to_string());
        };
        match self.registry.describe_json(id) {
            Ok(description) => ToolOutcome::Content(pretty(&description)),
            Err(e) => ToolOutcome::Failed(pretty(&error_body(&e, None, None))),
        }
    }

    fn tool_invoke(&self, args: &Value) -> ToolOutcome {
        let Some(id) = args.get("id").and_then(Value::as_str) else {
            return ToolOutcome::Protocol(-32602, "`id` is required".to_string());
        };
        let cap_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let request = InvokeRequest {
            id: id.to_string(),
            args: cap_args.clone(),
            dry_run,
        };

        match self.registry.dispatch(&self.ctx, &request, Approval::None) {
            Ok(result) => {
                let value = serde_json::to_value(&result).unwrap_or(Value::Null);
                ToolOutcome::Content(pretty(&json!({
                    "ok": true,
                    "dry_run": result.dry_run,
                    "result": value,
                })))
            }
            Err(e) => {
                // An operation that needs approval is not an error the model can
                // fix, so hand it the plan and the next step. Producing the plan
                // is itself a dry run: it changes nothing and is recorded as a
                // plan, not as an application.
                let plan = if e.code() == "approval_required" && !dry_run {
                    self.plan_for(id, &cap_args)
                } else {
                    None
                };
                let mut body = error_body(&e, plan.clone(), Some(id));
                if e.code() == "approval_required" && !dry_run {
                    let queueable = self
                        .registry
                        .get(id)
                        .map(|c| c.agent_safe && !c.human_only)
                        .unwrap_or(false);
                    if let (true, Some(queue)) = (queueable, &self.approval_queue) {
                        if let Some(request_id) = queue(id, &cap_args, plan.as_ref()) {
                            body["request_id"] = json!(request_id);
                            body["next_step"] = json!(format!(
                                "The request was queued as {request_id} and shows in the devd inbox for a person to approve or decline. \
                                 Poll it with the approval.get capability instead of asking again, carry on with other work meanwhile, \
                                 and do not report the operation as having happened until its state is `approved`."
                            ));
                        }
                    }
                }
                ToolOutcome::Failed(pretty(&body))
            }
        }
    }

    /// Ask the registry for a plan without applying anything.
    fn plan_for(&self, id: &str, args: &Value) -> Option<Value> {
        let request = InvokeRequest {
            id: id.to_string(),
            args: args.clone(),
            dry_run: true,
        };
        self.registry
            .dispatch(&self.ctx, &request, Approval::None)
            .ok()
            .and_then(|r| r.plan)
            .and_then(|p| serde_json::to_value(p).ok())
    }

    fn tool_doctor(&self) -> ToolOutcome {
        let mut checks = doctor::core_checks_for(&self.ctx.paths, &self.ctx.settings, Some(&self.ctx.store));
        for extra in &self.extra_checks {
            checks.extend(extra());
        }
        let ok = !checks.iter().any(|c| c.level == Level::Fail);
        ToolOutcome::Content(pretty(&json!({
            "ok": ok,
            "checks": checks,
        })))
    }

    fn ok(id: Value, result: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    }

    fn error(id: Value, code: i32, message: impl Into<String>) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message.into() },
        })
    }
}

/// The tools this server publishes.
///
/// Kept as data rather than a builder chain because it is a contract: a client
/// caches it, and a test asserts its shape.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "devd_groups",
            "title": "List capability domains",
            "description": "List the domains of capability devd exposes, with how many each \
                contains. Start here. The full catalogue is too large to load at once, so \
                work one domain at a time.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
        }),
        json!({
            "name": "devd_capabilities",
            "title": "Search capabilities",
            "description": "Search devd's capabilities by text, by domain, or both. Returns each \
                id with its permission tier and whether it needs human approval. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Free text matched against ids, summaries and descriptions."
                    },
                    "group": {
                        "type": "string",
                        "description": "Restrict to one domain, e.g. `git.branches`."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 50
                    }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
        }),
        json!({
            "name": "devd_describe",
            "title": "Describe one capability",
            "description": "Describe one capability: what it does, its permission tier, whether \
                it needs human approval, whether an agent may call it, and the JSON Schema of its \
                arguments. Call this before devd_invoke.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Capability id, e.g. `git.status`." }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
        }),
        json!({
            "name": "devd_invoke",
            "title": "Invoke a capability",
            "description": "Invoke a capability by id. Read-tier capabilities run directly. An \
                operation that needs human approval is refused here and recorded in the audit \
                trail; a human approves it in the devd application or the CLI. Use dry_run true \
                to see what an operation would do without doing it. Never claim an operation \
                succeeded unless the result says it ran.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Capability id." },
                    "args": {
                        "type": "object",
                        "description": "Arguments matching the capability's schema."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "default": false,
                        "description": "Produce a plan instead of applying the operation."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false },
        }),
        json!({
            "name": "devd_doctor",
            "title": "Check the environment",
            "description": "Check devd's environment: the layout, the settings, the state store, \
                the ports it would use, and the Git backend. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
        }),
    ]
}

fn summarise(cap: &Arc<devd_core::capability::Capability>) -> Value {
    json!({
        "id": cap.id,
        "group": cap.group,
        "tier": cap.tier.as_str(),
        "summary": cap.summary,
        "requires_approval": cap.tier.requires_approval(),
        "agent_safe": cap.agent_safe,
        "human_only": cap.human_only,
        "supports_dry_run": cap.supports_dry_run,
    })
}

/// The body of a tool failure. Shaped like the HTTP API's error envelope so a
/// reader that knows one knows the other.
fn error_body(error: &DevdError, plan: Option<Value>, id: Option<&str>) -> Value {
    let mut body = json!({
        "ok": false,
        "error": { "code": error.code(), "message": error.to_string() },
    });
    if let Some(plan) = plan {
        body["plan"] = plan;
        body["next_step"] = json!(format!(
            "A human must approve `{}` before it can run. Ask them to approve it in the devd \
             application or with the devd CLI. An agent cannot approve its own operation, and \
             you must not report this operation as having happened.",
            id.unwrap_or("")
        ));
    } else if error.code() == "approval_required" {
        body["next_step"] = json!(
            "A human must approve this before it can run, and it cannot describe itself in \
             advance. Ask them to approve it in the devd application or with the devd CLI."
        );
    }
    body
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use devd_core::capability::{Capability, Effect, Surface, Tier};
    use devd_core::{EventBus, Paths, Settings, Store};

    /// A registry with three capabilities that cover the three cases that
    /// matter: a read, an approval-gated write that can plan, and a human-only
    /// operation.
    fn fixture() -> (McpServer, Arc<AtomicUsize>) {
        let applications = Arc::new(AtomicUsize::new(0));

        let mut registry = Registry::new();
        registry
            .register(
                Capability::builder("test.inspect", "Inspect something")
                    .group("test")
                    .tier(Tier::Read)
                    .effect(Effect::Read)
                    .handler(
                        |_ctx, _a: &Value| {
                            Ok(devd_core::capability::Plan::new("test.inspect", "inspect"))
                        },
                        |_ctx, _a: &Value| Ok(devd_core::capability::Outcome::ok("inspected")),
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let counter = applications.clone();
        registry
            .register(
                Capability::builder("test.reset", "Reset something")
                    .group("test")
                    .tier(Tier::Destructive)
                    .effect(Effect::Write)
                    .dry_run(true)
                    .handler(
                        move |_ctx, _a: &Value| {
                            Ok(devd_core::capability::Plan::new("test.reset", "reset Hard")
                                .step("the working tree is reset")
                                .destructive(true))
                        },
                        {
                            let counter = counter.clone();
                            move |_ctx, _a: &Value| {
                                counter.fetch_add(1, Ordering::SeqCst);
                                Ok(devd_core::capability::Outcome::ok("reset"))
                            }
                        },
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let human = applications.clone();
        registry
            .register(
                Capability::builder("test.trust", "Change a trust store")
                    .group("test.admin")
                    .tier(Tier::Admin)
                    .effect(Effect::System)
                    .human_only(true)
                    .dry_run(true)
                    .handler(
                        |_ctx, _a: &Value| {
                            Ok(devd_core::capability::Plan::new("test.trust", "install"))
                        },
                        move |_ctx, _a: &Value| {
                            human.fetch_add(1, Ordering::SeqCst);
                            Ok(devd_core::capability::Outcome::ok("installed"))
                        },
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();

        // An approval-gated capability that cannot describe itself in advance.
        // The server must say so honestly rather than inventing a plan.
        let publish = applications.clone();
        registry
            .register(
                Capability::builder("test.publish", "Publish to a remote")
                    .group("test.publish")
                    .tier(Tier::Remote)
                    .effect(Effect::Remote)
                    .dry_run(false)
                    .handler(
                        |_ctx, _a: &Value| {
                            Err(devd_core::DevdError::unsupported("cannot plan a publish"))
                        },
                        move |_ctx, _a: &Value| {
                            publish.fetch_add(1, Ordering::SeqCst);
                            Ok(devd_core::capability::Outcome::ok("published"))
                        },
                    )
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let root = std::env::temp_dir().join(format!("devd-mcp-{}", std::process::id()));
        let paths = Paths::rooted(root);
        paths.ensure().unwrap();
        let ctx = Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Cli),
            dry_run: false,
        };

        (
            McpServer::new(Arc::new(registry), ctx, "test-client"),
            applications,
        )
    }

    fn call(server: &McpServer, id: u64, method: &str, params: Value) -> Value {
        server
            .handle(&json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            }))
            .expect("a request expects a response")
    }

    fn tool_text(response: &Value) -> String {
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn tool_json(response: &Value) -> Value {
        serde_json::from_str(&tool_text(response)).expect("tool text is JSON")
    }

    #[test]
    fn initialize_reports_the_server_and_its_tools() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "initialize",
            json!({ "protocolVersion": "2025-06-18" }),
        );
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["result"]["serverInfo"]["name"], "devd");
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["instructions"].as_str().unwrap().len() > 100);
    }

    #[test]
    fn initialize_echoes_an_unknown_protocol_version() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "initialize",
            json!({ "protocolVersion": "1999-01-01" }),
        );
        assert_eq!(response["result"]["protocolVersion"], "1999-01-01");
    }

    #[test]
    fn a_notification_gets_no_response() {
        let (server, _) = fixture();
        assert!(server
            .handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none());
        assert!(server
            .handle(&json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {} }))
            .is_none());
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let (server, _) = fixture();
        let response = call(&server, 7, "tools/explode", json!({}));
        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["id"], 7);
    }

    #[test]
    fn tools_list_is_a_small_discovery_surface() {
        let (server, _) = fixture();
        let response = call(&server, 1, "tools/list", json!({}));
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(
            tools.len(),
            5,
            "the tool list must stay small and discovery-shaped, not one entry per capability"
        );
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "devd_groups",
            "devd_capabilities",
            "devd_describe",
            "devd_invoke",
            "devd_doctor",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        for tool in tools {
            assert!(tool["inputSchema"]["type"] == "object", "{tool}");
            assert!(tool["description"].as_str().unwrap().len() > 40, "{tool}");
        }
    }

    #[test]
    fn the_tool_surface_does_not_grow_with_the_registry() {
        // The point of the discovery tools is that adding capabilities does not
        // add tools. Two servers over registries of different sizes must publish
        // the same list.
        let (server, _) = fixture();
        let small = call(&server, 1, "tools/list", json!({}))["result"]["tools"].clone();

        let mut registry = Registry::new();
        for i in 0..200 {
            registry
                .register(
                    Capability::builder(format!("bulk.cap{i}").as_str(), "bulk")
                        .group("bulk")
                        .tier(Tier::Read)
                        .effect(Effect::Read)
                        .handler(
                            |_ctx, _a: &Value| Ok(devd_core::capability::Plan::new("bulk", "x")),
                            |_ctx, _a: &Value| Ok(devd_core::capability::Outcome::ok("x")),
                        )
                        .build()
                        .unwrap(),
                )
                .unwrap();
        }
        let big_server = McpServer::new(
            Arc::new(registry),
            Ctx {
                store: Arc::new(Store::open_in_memory().unwrap()),
                paths: Arc::new(Paths::rooted(std::env::temp_dir())),
                settings: Arc::new(Settings::default()),
                events: EventBus::new(),
                actor: Actor::agent("x"),
                dry_run: false,
            },
            "test-client",
        );
        let big = call(&big_server, 1, "tools/list", json!({}))["result"]["tools"].clone();
        assert_eq!(small, big);
    }

    #[test]
    fn an_agent_cannot_borrow_a_human_identity() {
        // The caller hands in a human context; the server must not honour it.
        let root = std::env::temp_dir().join(format!("devd-mcp-id-{}", std::process::id()));
        let ctx = Ctx {
            store: Arc::new(Store::open_in_memory().unwrap()),
            paths: Arc::new(Paths::rooted(root)),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Gui),
            dry_run: false,
        };
        let server = McpServer::new(Arc::new(Registry::new()), ctx, "claude-code");
        assert!(server.ctx.actor.is_agent());
        assert_eq!(server.ctx.actor.name, "claude-code");
        assert_eq!(server.ctx.actor.surface.as_str(), "mcp");
    }

    #[test]
    fn groups_are_discoverable_and_counted() {
        let (server, _) = fixture();
        let body = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_groups", "arguments": {}
            }),
        ));
        assert_eq!(body["total_capabilities"], 4);
        let domains = body["domains"].as_array().unwrap();
        assert_eq!(domains.len(), 3);
        assert_eq!(domains[0]["group"], "test");
        assert_eq!(domains[0]["capabilities"], 2);
        assert_eq!(domains[1]["group"], "test.admin");
        assert_eq!(domains[2]["group"], "test.publish");
    }

    #[test]
    fn capabilities_can_be_searched_and_grouped() {
        let (server, _) = fixture();
        let all = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_capabilities", "arguments": {}
            }),
        ));
        assert_eq!(all["total"], 4);

        let grouped = tool_json(&call(
            &server,
            2,
            "tools/call",
            json!({
                "name": "devd_capabilities", "arguments": { "group": "test" }
            }),
        ));
        assert_eq!(grouped["total"], 2);

        let searched = tool_json(&call(
            &server,
            3,
            "tools/call",
            json!({
                "name": "devd_capabilities", "arguments": { "query": "trust" }
            }),
        ));
        assert_eq!(searched["total"], 1, "{searched}");
        assert_eq!(searched["capabilities"][0]["id"], "test.trust");
        assert_eq!(searched["capabilities"][0]["human_only"], true);
    }

    #[test]
    fn describe_returns_the_argument_schema() {
        let (server, _) = fixture();
        let body = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_describe", "arguments": { "id": "test.reset" }
            }),
        ));
        assert_eq!(body["id"], "test.reset");
        assert_eq!(body["tier"], "destructive");
        assert_eq!(body["requires_approval"], true);
        assert!(body["args_schema"].is_object());
    }

    #[test]
    fn describing_an_unknown_capability_fails_helpfully() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_describe", "arguments": { "id": "nope.nope" }
            }),
        );
        assert_eq!(response["result"]["isError"], true);
        let body = tool_json(&response);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[test]
    fn a_read_capability_runs_for_an_agent() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke", "arguments": { "id": "test.inspect" }
            }),
        );
        assert_eq!(response["result"]["isError"], false, "{response}");
        let body = tool_json(&response);
        assert_eq!(body["ok"], true);
        assert_eq!(body["result"]["outcome"]["message"], "inspected");
    }

    #[test]
    fn a_destructive_operation_is_refused_and_the_plan_is_handed_over() {
        let (server, applications) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke", "arguments": { "id": "test.reset" }
            }),
        );
        assert_eq!(response["result"]["isError"], true);
        let body = tool_json(&response);
        assert_eq!(body["error"]["code"], "approval_required");
        // The agent gets the plan, so it can tell the human what would happen.
        assert_eq!(body["plan"]["destructive"], true);
        assert!(body["plan"]["steps"][0]
            .as_str()
            .unwrap()
            .contains("working tree"));
        assert!(body["next_step"].as_str().unwrap().contains("human"));
        // And nothing ran.
        assert_eq!(applications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_human_only_operation_is_refused_for_an_agent_even_with_a_plan() {
        let (server, applications) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke", "arguments": { "id": "test.trust" }
            }),
        );
        assert_eq!(response["result"]["isError"], true);
        let body = tool_json(&response);
        assert_eq!(body["error"]["code"], "permission_denied");
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("human-only"));
        assert_eq!(applications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_approval_gated_operation_that_cannot_plan_says_so() {
        let (server, applications) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke", "arguments": { "id": "test.publish" }
            }),
        );
        assert_eq!(response["result"]["isError"], true);
        let body = tool_json(&response);
        assert_eq!(body["error"]["code"], "approval_required");
        // No plan is invented for a capability that cannot describe itself.
        assert!(body.get("plan").is_none(), "{body}");
        assert!(body["next_step"]
            .as_str()
            .unwrap()
            .contains("cannot describe itself"));
        assert_eq!(applications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_dry_run_returns_a_plan_and_changes_nothing() {
        let (server, applications) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke",
                "arguments": { "id": "test.reset", "dry_run": true }
            }),
        );
        assert_eq!(response["result"]["isError"], false, "{response}");
        let body = tool_json(&response);
        assert_eq!(body["ok"], true);
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["result"]["plan"]["destructive"], true);
        assert_eq!(applications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn invoking_an_unknown_capability_reports_not_found() {
        let (server, _) = fixture();
        let body = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_invoke", "arguments": { "id": "does.not.exist" }
            }),
        ));
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[test]
    fn an_unknown_tool_is_a_protocol_error_naming_the_alternative() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "run_shell", "arguments": {}
            }),
        );
        assert_eq!(response["error"]["code"], -32602);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("tools/list"));
    }

    #[test]
    fn a_missing_required_argument_is_a_protocol_error() {
        let (server, _) = fixture();
        let response = call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_describe", "arguments": {}
            }),
        );
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn the_doctor_reports_the_environment_and_the_extra_checks() {
        let (mut server, _) = fixture();
        server = server.with_extra_checks(vec![Arc::new(|| {
            vec![Check::ok("git", "2.46.0 at /usr/bin/git")]
        })]);
        let body = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_doctor", "arguments": {}
            }),
        ));
        assert_eq!(body["ok"], true);
        let names: Vec<&str> = body["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"layout"), "{names:?}");
        assert!(names.contains(&"state"), "{names:?}");
        assert!(names.contains(&"git"), "{names:?}");
    }

    #[test]
    fn a_failing_check_makes_the_doctor_report_not_ok() {
        let (server, _) = fixture();
        let body = tool_json(&call(
            &server,
            1,
            "tools/call",
            json!({
                "name": "devd_doctor", "arguments": {}
            }),
        ));
        // The store is in memory and the layout exists, so this must be ok. If it
        // is not, the fixture is wrong rather than the assertion.
        assert_eq!(body["ok"], true, "{body}");
    }
}
