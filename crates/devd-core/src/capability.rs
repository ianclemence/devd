//! Capabilities: the single declaration of everything `devd` can do.
//!
//! A [`Capability`] carries its own identity, argument schema, permission tier,
//! side-effect class, dry-run support, undo metadata and handler. The CLI, TUI,
//! GUI, HTTP API and MCP server all read from this structure, which is why they
//! cannot disagree about what is possible or who may do it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Settings;
use crate::error::{DevdError, Result};
use crate::events::EventBus;
use crate::paths::Paths;
use crate::state::Store;

/// Permission tier. Higher tiers require explicit human approval.
///
/// The ordering matters: a surface that is granted one tier is granted every
/// tier below it, and never a tier above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Inspection only; changes nothing.
    Read,
    /// Reversible, project-scoped change.
    Write,
    /// Starts processes or runs project tooling.
    Exec,
    /// Can destroy data; requires approval and is never agent-self-approvable.
    Destructive,
    /// Acts on a machine we do not own.
    Remote,
    /// Changes system-wide security state (trust stores, hosts file).
    Admin,
}

impl Tier {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Exec => "exec",
            Self::Destructive => "destructive",
            Self::Remote => "remote",
            Self::Admin => "admin",
        }
    }

    /// Parse from a tier name.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "exec" => Ok(Self::Exec),
            "destructive" => Ok(Self::Destructive),
            "remote" => Ok(Self::Remote),
            "admin" => Ok(Self::Admin),
            other => Err(DevdError::invalid(format!("unknown tier `{other}`"))),
        }
    }

    /// Whether this tier demands an explicit human approval before applying.
    pub fn requires_approval(self) -> bool {
        matches!(self, Self::Destructive | Self::Remote | Self::Admin)
    }
}

/// The kind of side effect a capability has. Used by the GUI and by agents to
/// render plans intelligibly, and by the HTTP/MCP layers for annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Reads state, changes nothing.
    Read,
    /// Writes files inside the devd root or a project.
    Write,
    /// Renders service configuration.
    Config,
    /// Starts, stops or signals a process.
    Process,
    /// Touches a database or other stored data.
    Data,
    /// Modifies machine-level state (hosts file, trust store, PATH).
    System,
    /// Acts over the network against a remote host.
    Remote,
}

impl Effect {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Config => "config",
            Self::Process => "process",
            Self::Data => "data",
            Self::System => "system",
            Self::Remote => "remote",
        }
    }
}

/// Which interface invoked a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    /// Command line.
    Cli,
    /// Terminal dashboard.
    Tui,
    /// Desktop application.
    Gui,
    /// Loopback HTTP API.
    Http,
    /// Model Context Protocol server.
    Mcp,
    /// The in-app conversational agent.
    Agent,
    /// The daemon itself (scheduled work, reconciliation).
    Daemon,
}

impl Surface {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Tui => "tui",
            Self::Gui => "gui",
            Self::Http => "http",
            Self::Mcp => "mcp",
            Self::Agent => "agent",
            Self::Daemon => "daemon",
        }
    }

    /// Whether this surface is a human at a keyboard.
    ///
    /// Note that this is about *who* is typing, not about whether a question can
    /// be asked; see [`Surface::can_prompt`] for the latter.
    pub fn is_human(self) -> bool {
        matches!(self, Self::Cli | Self::Tui | Self::Gui)
    }

    /// Whether this surface can put a question to a person and wait.
    ///
    /// The local HTTP surface counts, because it *is* the application window:
    /// the same person is looking at it, and the server can hold a request open
    /// until they answer. MCP, the embedded agent and the daemon cannot, which
    /// is what makes this distinction worth having - a capability that needs a
    /// decision can refuse an unattended caller without refusing the app.
    pub fn can_prompt(self) -> bool {
        matches!(self, Self::Cli | Self::Tui | Self::Gui | Self::Http)
    }
}

/// Who is performing an action. Recorded on every audit row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    /// `human`, `agent` or `system`.
    pub kind: String,
    /// Interface the call arrived through.
    pub surface: Surface,
    /// Client identifier, e.g. `claude-code` or `cli`.
    pub name: String,
}

impl Actor {
    /// A human using an interactive surface.
    pub fn human(surface: Surface) -> Self {
        Self {
            kind: "human".to_string(),
            surface,
            name: surface.as_str().to_string(),
        }
    }

    /// An AI agent, optionally naming the client.
    pub fn agent(client: impl Into<String>) -> Self {
        Self {
            kind: "agent".to_string(),
            surface: Surface::Mcp,
            name: client.into(),
        }
    }

    /// The daemon acting on its own behalf.
    pub fn system() -> Self {
        Self {
            kind: "system".to_string(),
            surface: Surface::Daemon,
            name: "daemon".to_string(),
        }
    }

    /// Whether the caller is an AI agent.
    pub fn is_agent(&self) -> bool {
        self.kind == "agent"
    }

    /// Whether the caller is a human at an interactive surface.
    pub fn is_human(&self) -> bool {
        self.kind == "human"
    }

    /// Compact label for logs and the audit trail.
    pub fn label(&self) -> String {
        format!("{}:{}", self.kind, self.name)
    }
}

/// The approval state attached to an invocation.
///
/// Approval is *data*, produced by a human decision, never by the caller's own
/// assertion: the MCP layer, HTTP layer and CLI each have to obtain it from a
/// person before they can pass [`Approval::Granted`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    /// The caller cannot answer an approval prompt (typical for batch CLI use).
    None,
    /// A human approved this specific invocation.
    Granted,
    /// A human declined. The operation must not run.
    Denied,
}

/// Execution context handed to every capability handler.
#[derive(Clone)]
pub struct Ctx {
    /// Authoritative state store.
    pub store: Arc<Store>,
    /// Filesystem layout.
    pub paths: Arc<Paths>,
    /// Effective settings for this invocation.
    pub settings: Arc<Settings>,
    /// Event bus for live updates.
    pub events: EventBus,
    /// Who is calling.
    pub actor: Actor,
    /// When true, handlers must compute a plan and change nothing.
    pub dry_run: bool,
}

impl Ctx {
    /// Parse and validate capability arguments into a typed struct.
    pub fn parse<T: for<'de> Deserialize<'de>>(&self, args: &Value) -> Result<T> {
        serde_json::from_value(args.clone())
            .map_err(|e| DevdError::invalid(format!("invalid arguments: {e}")))
    }

    /// Reject the call unless the caller is a human.
    ///
    /// This is the code-level enforcement behind the "never delegate" list: an
    /// agent cannot satisfy it by describing the action differently.
    pub fn require_human(&self, what: &str) -> Result<()> {
        if self.actor.is_human() {
            Ok(())
        } else {
            Err(DevdError::denied(format!(
                "{what} can only be performed by a human operator"
            )))
        }
    }

    /// Reject the call unless the caller is on a surface that can ask a person
    /// a question and wait for the answer.
    pub fn require_interactive(&self) -> Result<()> {
        if self.actor.surface.can_prompt() {
            Ok(())
        } else {
            Err(DevdError::denied(
                "this operation requires an interactive session",
            ))
        }
    }
}

/// A description of what an operation would do, produced without side effects.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// Capability that produced the plan.
    pub capability: String,
    /// One-line summary.
    pub summary: String,
    /// Ordered human-readable steps.
    pub steps: Vec<String>,
    /// Effects this plan would have.
    pub effects: Vec<String>,
    /// Whether applying requires explicit approval.
    pub requires_approval: bool,
    /// Whether applying can destroy data.
    pub destructive: bool,
    /// Structured plan detail for GUIs and agents.
    pub data: Value,
}

impl Plan {
    /// Start a plan for a capability.
    pub fn new(capability: &str, summary: impl Into<String>) -> Self {
        Self {
            capability: capability.to_string(),
            summary: summary.into(),
            steps: Vec::new(),
            effects: Vec::new(),
            requires_approval: false,
            destructive: false,
            data: Value::Null,
        }
    }

    /// Append a step.
    pub fn step(mut self, step: impl Into<String>) -> Self {
        self.steps.push(step.into());
        self
    }

    /// Record an effect.
    pub fn effect(mut self, effect: impl Into<String>) -> Self {
        self.effects.push(effect.into());
        self
    }

    /// Mark whether approval is required.
    pub fn approval(mut self, required: bool) -> Self {
        self.requires_approval = required;
        self
    }

    /// Mark whether the plan is destructive.
    ///
    /// Declaring a plan destructive also declares that it requires approval,
    /// because in the tier model every destructive capability needs one. That
    /// makes it impossible to describe a plan as destructive while claiming it
    /// needs no approval, which is the contradiction a GUI or an agent would
    /// otherwise have to guess about.
    pub fn destructive(mut self, yes: bool) -> Self {
        self.destructive = yes;
        if yes {
            self.requires_approval = true;
        }
        self
    }

    /// Attach structured plan data.
    pub fn data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

/// The result of applying a capability.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    /// Whether the operation succeeded. Failures are normally returned as
    /// `Err`, so this is `true` except for partial successes, which must be
    /// reported honestly rather than as success.
    pub ok: bool,
    /// Human-readable summary.
    pub message: String,
    /// Structured result.
    pub data: Value,
    /// Warnings that do not invalidate the result.
    pub warnings: Vec<String>,
}

impl Outcome {
    /// A successful outcome with a message.
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data: Value::Null,
            warnings: Vec::new(),
        }
    }

    /// A successful outcome with structured data.
    pub fn with_data(message: impl Into<String>, data: Value) -> Self {
        Self {
            ok: true,
            message: message.into(),
            data,
            warnings: Vec::new(),
        }
    }

    /// Attach a warning.
    pub fn warn(mut self, msg: impl Into<String>) -> Self {
        self.warnings.push(msg.into());
        self
    }
}

/// A capability's implementation.
pub trait Handler: Send + Sync {
    /// Compute the plan without side effects.
    fn plan(&self, ctx: &Ctx, args: &Value) -> Result<Plan>;
    /// Apply the operation.
    fn apply(&self, ctx: &Ctx, args: &Value) -> Result<Outcome>;
}

type PlanFn = Arc<dyn Fn(&Ctx, &Value) -> Result<Plan> + Send + Sync>;
type ApplyFn = Arc<dyn Fn(&Ctx, &Value) -> Result<Outcome> + Send + Sync>;

/// A [`Handler`] built from two closures.
pub struct FnHandler {
    plan_fn: PlanFn,
    apply_fn: ApplyFn,
}

impl Handler for FnHandler {
    fn plan(&self, ctx: &Ctx, args: &Value) -> Result<Plan> {
        (self.plan_fn)(ctx, args)
    }
    fn apply(&self, ctx: &Ctx, args: &Value) -> Result<Outcome> {
        (self.apply_fn)(ctx, args)
    }
}

/// A fully declared capability.
pub struct Capability {
    /// Unique dotted identifier, e.g. `site.create`.
    pub id: String,
    /// One-line summary used in listings and MCP tool descriptions.
    pub summary: String,
    /// Longer description for help output and agent context.
    pub description: String,
    /// Grouping used for namespacing and discovery, e.g. `git`.
    pub group: String,
    /// Permission tier.
    pub tier: Tier,
    /// Side-effect class.
    pub effect: Effect,
    /// JSON Schema for the arguments object.
    pub args_schema: Value,
    /// Whether a plan can be produced without side effects.
    pub supports_dry_run: bool,
    /// Whether an agent may call this without a human in the loop, assuming it
    /// holds the tier.
    pub agent_safe: bool,
    /// When true, an agent can never satisfy the approval requirement.
    pub human_only: bool,
    /// Whether repeating the call is harmless.
    pub idempotent: bool,
    /// Capability that reverses this one, when a reversal exists.
    pub undo: Option<String>,
    /// Implementation.
    pub handler: Arc<dyn Handler>,
}

impl std::fmt::Debug for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capability")
            .field("id", &self.id)
            .field("group", &self.group)
            .field("tier", &self.tier)
            .finish()
    }
}

impl Capability {
    /// Begin building a capability.
    pub fn builder(id: &str, summary: &str) -> CapabilityBuilder {
        CapabilityBuilder {
            id: id.to_string(),
            summary: summary.to_string(),
            description: String::new(),
            group: id.split('.').next().unwrap_or("general").to_string(),
            tier: Tier::Read,
            effect: Effect::Read,
            args_schema: serde_json::json!({ "type": "object", "properties": {} }),
            supports_dry_run: false,
            agent_safe: true,
            human_only: false,
            idempotent: false,
            undo: None,
            handler: None,
        }
    }
}

/// Fluent builder for [`Capability`].
pub struct CapabilityBuilder {
    id: String,
    summary: String,
    description: String,
    group: String,
    tier: Tier,
    effect: Effect,
    args_schema: Value,
    supports_dry_run: bool,
    agent_safe: bool,
    human_only: bool,
    idempotent: bool,
    undo: Option<String>,
    handler: Option<Arc<dyn Handler>>,
}

impl CapabilityBuilder {
    /// Set the longer description.
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = d.into();
        self
    }
    /// Set the discovery group.
    pub fn group(mut self, g: impl Into<String>) -> Self {
        self.group = g.into();
        self
    }
    /// Set the permission tier.
    pub fn tier(mut self, t: Tier) -> Self {
        self.tier = t;
        self
    }
    /// Set the side-effect class.
    pub fn effect(mut self, e: Effect) -> Self {
        self.effect = e;
        self
    }
    /// Supply a JSON Schema for the arguments.
    pub fn args_schema(mut self, schema: Value) -> Self {
        self.args_schema = schema;
        self
    }
    /// Declare that a plan can be produced safely.
    pub fn dry_run(mut self, yes: bool) -> Self {
        self.supports_dry_run = yes;
        self
    }
    /// Declare whether an agent may call this unattended.
    pub fn agent_safe(mut self, yes: bool) -> Self {
        self.agent_safe = yes;
        self
    }
    /// Declare that only a human may ever authorise this.
    pub fn human_only(mut self, yes: bool) -> Self {
        self.human_only = yes;
        if yes {
            self.agent_safe = false;
        }
        self
    }
    /// Declare the operation idempotent.
    pub fn idempotent(mut self, yes: bool) -> Self {
        self.idempotent = yes;
        self
    }
    /// Name the capability that reverses this one.
    pub fn undo(mut self, id: impl Into<String>) -> Self {
        self.undo = Some(id.into());
        self
    }
    /// Supply the handler as two closures.
    pub fn handler(
        mut self,
        plan: impl Fn(&Ctx, &Value) -> Result<Plan> + Send + Sync + 'static,
        apply: impl Fn(&Ctx, &Value) -> Result<Outcome> + Send + Sync + 'static,
    ) -> Self {
        self.handler = Some(Arc::new(FnHandler {
            plan_fn: Arc::new(plan),
            apply_fn: Arc::new(apply),
        }));
        self
    }
    /// Supply a pre-built handler.
    pub fn handler_arc(mut self, handler: Arc<dyn Handler>) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Finish the capability. Fails when no handler was supplied.
    pub fn build(self) -> Result<Capability> {
        let handler = self.handler.ok_or_else(|| {
            DevdError::internal(format!("capability `{}` has no handler", self.id))
        })?;
        if self.human_only && self.agent_safe {
            return Err(DevdError::internal(format!(
                "capability `{}` is human_only and cannot be agent_safe",
                self.id
            )));
        }
        Ok(Capability {
            id: self.id,
            summary: self.summary,
            description: self.description,
            group: self.group,
            tier: self.tier,
            effect: self.effect,
            args_schema: self.args_schema,
            supports_dry_run: self.supports_dry_run,
            agent_safe: self.agent_safe,
            human_only: self.human_only,
            idempotent: self.idempotent,
            undo: self.undo,
            handler,
        })
    }
}

/// A request to invoke a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvokeRequest {
    /// Capability identifier.
    pub id: String,
    /// Arguments object.
    #[serde(default)]
    pub args: Value,
    /// When true, produce a plan instead of applying.
    #[serde(default)]
    pub dry_run: bool,
}

/// The result of an invocation.
#[derive(Debug, Clone, Serialize)]
pub struct InvokeResult {
    /// Capability that ran.
    pub capability: String,
    /// Whether this was a plan rather than an application.
    pub dry_run: bool,
    /// Present when `dry_run` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    /// Present when `dry_run` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_ordering_is_monotonic() {
        assert!(Tier::Read < Tier::Write);
        assert!(Tier::Write < Tier::Exec);
        assert!(Tier::Exec < Tier::Destructive);
        assert!(Tier::Destructive < Tier::Admin);
        assert!(!Tier::Read.requires_approval());
        assert!(!Tier::Exec.requires_approval());
        assert!(Tier::Destructive.requires_approval());
        assert!(Tier::Admin.requires_approval());
    }

    #[test]
    fn human_only_forces_agent_unsafe() {
        let c = Capability::builder("x.y", "s")
            .human_only(true)
            .handler(
                |_, _| Ok(Plan::new("x.y", "p")),
                |_, _| Ok(Outcome::ok("d")),
            )
            .build()
            .unwrap();
        assert!(!c.agent_safe);
        assert!(c.human_only);
    }

    #[test]
    fn actor_kinds() {
        assert!(Actor::agent("claude-code").is_agent());
        assert!(!Actor::agent("claude-code").is_human());
        assert!(Actor::human(Surface::Cli).is_human());
        assert_eq!(Actor::human(Surface::Cli).label(), "human:cli");
    }
}
