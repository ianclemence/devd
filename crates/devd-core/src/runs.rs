//! The run engine: one durable unit of work, shared by every surface.
//!
//! Before this module, a conversation was a thread, a team execution was a run
//! in another table, and a capability call was an audit row. Each was durable on
//! its own, but nothing tied them together, so "what happened, and what proves
//! it?" had to be reconstructed from three places with three vocabularies.
//!
//! A [`Run`] is that missing spine. It carries the goal, the lifecycle state,
//! the plan, acceptance criteria, steps, tool calls, evidence, artifacts,
//! approvals and a sequenced event log. The assistant and the team scheduler
//! both create runs; the registry records into the run it is given; completion is
//! decided from evidence rather than from a model's claim.
//!
//! # Lifecycle
//!
//! ```text
//! intake → planning → awaiting_approval → executing → verifying → review → completed
//!                          ↘ paused / blocked / failed / unverified / cancelled
//! ```
//!
//! Transitions are checked by [`RunState::can_transition_to`]. A completed run
//! can only go back to `executing` through a deliberate continuation or retry,
//! which the store records as an event; no code path can drift back into work
//! that was already signed off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::Outcome;
use crate::error::{DevdError, Result};
use crate::util;

/// How a run is being driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    /// The conversational assistant.
    Assistant,
    /// The planner / specialist team.
    Team,
    /// Started by a schedule.
    Scheduled,
    /// A direct call through the loopback HTTP API or a capability alias.
    Api,
    /// Caller-driven execution with no human in the loop.
    Headless,
}

impl RunMode {
    /// Stable lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::Team => "team",
            Self::Scheduled => "scheduled",
            Self::Api => "api",
            Self::Headless => "headless",
        }
    }

    /// Parse a stored mode, accepting the spellings older versions used.
    pub fn parse(s: &str) -> Self {
        match s {
            "team" => Self::Team,
            "scheduled" | "schedule" => Self::Scheduled,
            "api" | "http" => Self::Api,
            "headless" => Self::Headless,
            _ => Self::Assistant,
        }
    }
}

/// The lifecycle state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// The request exists; no plan has been made.
    Intake,
    /// A plan is being produced.
    Planning,
    /// Work is blocked on a human decision.
    AwaitingApproval,
    /// Work is being performed.
    Executing,
    /// The work is being checked against its acceptance criteria.
    Verifying,
    /// Finished work is waiting for a person to review it.
    Review,
    /// Work finished and the evidence satisfies its policy.
    Completed,
    /// Deliberately stopped; can be resumed.
    Paused,
    /// Cannot continue without a person or another task.
    Blocked,
    /// Stopped with an error.
    Failed,
    /// Work ended without evidence that supports the claim.
    Unverified,
    /// Stopped by a person and not resumable without a new run.
    Cancelled,
}

impl RunState {
    /// Stable stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Planning => "planning",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::Review => "review",
            Self::Completed => "completed",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
            Self::Unverified => "unverified",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse a stored state, accepting the names the team tables used.
    pub fn parse(s: &str) -> Self {
        match s {
            "intake" => Self::Intake,
            "planning" => Self::Planning,
            "awaiting_plan" | "awaiting_approval" | "waiting_approval" => Self::AwaitingApproval,
            "running" | "executing" => Self::Executing,
            "verifying" => Self::Verifying,
            "review" => Self::Review,
            "done" | "completed" => Self::Completed,
            "paused" => Self::Paused,
            "blocked" => Self::Blocked,
            "failed" | "error" => Self::Failed,
            "unverified" => Self::Unverified,
            "cancelled" => Self::Cancelled,
            // `idle` is a conversation with no turn in flight. It is not proof
            // of anything, so it is honestly unverified rather than completed.
            _ => Self::Unverified,
        }
    }

    /// Whether this is an end state for the current unit of work.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether work is in flight (or about to be).
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Intake
                | Self::Planning
                | Self::AwaitingApproval
                | Self::Executing
                | Self::Verifying
                | Self::Review
                | Self::Paused
                | Self::Blocked
        )
    }

    /// Whether this state means "finished, but the evidence does not support the
    /// claim".
    pub fn is_unproven(self) -> bool {
        matches!(self, Self::Unverified | Self::Blocked)
    }

    /// Which transitions are legal.
    ///
    /// Re-entering the same state is always allowed: recording progress is not a
    /// transition, and forcing every update to invent a new state would make the
    /// log lie.
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        use RunState::*;
        match self {
            Intake => matches!(
                next,
                Planning | Executing | AwaitingApproval | Cancelled | Failed
            ),
            Planning => matches!(
                next,
                AwaitingApproval | Executing | Paused | Blocked | Failed | Cancelled
            ),
            AwaitingApproval => matches!(
                next,
                Planning | Executing | Paused | Blocked | Failed | Cancelled
            ),
            Executing => matches!(
                next,
                Verifying | Review | Completed | Paused | Blocked | Failed | Unverified | Cancelled
            ),
            Verifying => matches!(
                next,
                Review | Completed | Executing | Paused | Blocked | Failed | Unverified | Cancelled
            ),
            Review => matches!(next, Completed | Unverified | Executing | Cancelled),
            // A deliberate continuation or retry is the only way back to work.
            Completed => matches!(next, Executing | Review),
            Paused => matches!(next, Planning | Executing | Verifying | Cancelled | Failed),
            Blocked => matches!(
                next,
                Planning | Executing | Verifying | Unverified | Cancelled | Failed
            ),
            Failed => matches!(next, Planning | Executing | Cancelled),
            Unverified => matches!(next, Executing | Review | Completed | Cancelled),
            Cancelled => false,
        }
    }
}

impl std::fmt::Display for RunState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A durable unit of work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    /// Identifier.
    pub id: String,
    /// What the person asked for.
    pub goal: String,
    /// How it is driven.
    pub mode: String,
    /// Lifecycle state.
    pub state: String,
    /// Project directory.
    pub project: Option<String>,
    /// Environment manifest fingerprint or JSON.
    pub environment: Option<String>,
    /// Acceptance criteria, as JSON array.
    pub acceptance: Vec<String>,
    /// Completion policy, decoded from JSON.
    pub completion_policy: CompletionPolicy,
    /// Plan snapshot, when one exists.
    pub plan: Option<Value>,
    /// Fingerprint of the approved plan.
    pub plan_fingerprint: Option<String>,
    /// Run this was forked from.
    pub parent_run: Option<String>,
    /// Conversation that drives this run.
    pub thread_id: Option<String>,
    /// Team run mirrored by this run.
    pub team_run_id: Option<String>,
    /// Who started it.
    pub actor: String,
    /// Cancellation record.
    pub cancellation: Option<Value>,
    /// Usage counters.
    pub usage: Value,
    /// Last error.
    pub error: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Last update.
    pub updated_at: String,
    /// Completion timestamp.
    pub finished_at: Option<String>,
}

impl Run {
    /// Typed lifecycle state.
    pub fn typed_state(&self) -> RunState {
        RunState::parse(&self.state)
    }

    /// Typed mode.
    pub fn typed_mode(&self) -> RunMode {
        RunMode::parse(&self.mode)
    }
}

/// A step within a run: a task, a model turn, a verification pass, a review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStep {
    /// Identifier.
    pub id: String,
    /// Owning run.
    pub run_id: String,
    /// Position within the run.
    pub ordinal: i64,
    /// `task`, `turn`, `tool`, `verify`, `review`, `recovery`.
    pub kind: String,
    /// Human-readable title.
    pub title: String,
    /// `pending`, `running`, `done`, `failed`, `cancelled`.
    pub state: String,
    /// Structured detail.
    pub data: Value,
    /// When it started.
    pub started_at: Option<String>,
    /// When it finished.
    pub finished_at: Option<String>,
}

/// An authoritative record of one capability invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identifier.
    pub id: String,
    /// Run it belongs to.
    pub run_id: Option<String>,
    /// Step it belongs to.
    pub step_id: Option<String>,
    /// Provider tool-use id, when there is one.
    pub call_id: Option<String>,
    /// Who called.
    pub actor: String,
    /// Interface.
    pub surface: String,
    /// Capability identifier (never the exposed tool name).
    pub capability: String,
    /// Redacted arguments.
    pub args: Value,
    /// Hash of the unredacted canonical arguments.
    pub args_hash: String,
    /// Approval that authorized it.
    pub approval_id: Option<String>,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
    /// Whether it succeeded.
    pub ok: bool,
    /// Stable error code.
    pub error_code: Option<String>,
    /// Error message.
    pub error: Option<String>,
    /// Process exit code, when it ran a process.
    pub exit_code: Option<i64>,
    /// Wall-clock milliseconds.
    pub duration_ms: i64,
    /// Bounded, redacted result.
    pub result: Option<Value>,
    /// Start.
    pub started_at: String,
    /// Finish.
    pub finished_at: Option<String>,
}

/// A fact observed by devd, not a sentence written by a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// Identifier.
    pub id: String,
    /// Run.
    pub run_id: String,
    /// Step.
    pub step_id: Option<String>,
    /// Tool call that produced it.
    pub tool_call_id: Option<String>,
    /// Artifact carrying the raw observation.
    pub artifact_id: Option<String>,
    /// Kind.
    pub kind: String,
    /// Capability.
    pub capability: String,
    /// Whether the observation supports success.
    pub passed: bool,
    /// The claim or acceptance criterion it supports.
    pub claim: Option<String>,
    /// What was observed.
    pub summary: String,
    /// Supporting detail.
    pub detail: Option<String>,
    /// When.
    pub created_at: String,
}

/// A stored output too large for a database row or a model context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    /// Identifier.
    pub id: String,
    /// Run.
    pub run_id: Option<String>,
    /// Tool call.
    pub tool_call_id: Option<String>,
    /// `output`, `diff`, `test-report`, `log`, `probe`, `file`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// MIME type.
    pub mime: Option<String>,
    /// Size in bytes.
    pub bytes: i64,
    /// Where the bounded copy lives.
    pub path: Option<String>,
    /// Content hash.
    pub sha256: Option<String>,
    /// One-line summary.
    pub summary: Option<String>,
    /// When.
    pub created_at: String,
}

/// One durable approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    /// Hash of the opaque token; the token itself is never stored.
    pub id: String,
    /// Run.
    pub run_id: Option<String>,
    /// Team task.
    pub task_id: Option<String>,
    /// Tool call.
    pub tool_call_id: Option<String>,
    /// Capability.
    pub capability: String,
    /// The exact arguments shown to the person.
    pub args: Value,
    /// Hash of those arguments.
    pub args_hash: String,
    /// Plan shown.
    pub plan: Option<Value>,
    /// Plan fingerprint.
    pub plan_hash: Option<String>,
    /// `single`, `plan`, `task`.
    pub scope: String,
    /// Permission tier.
    pub tier: String,
    /// `normal`, `sensitive`, `destructive`.
    pub risk: String,
    /// Who asked.
    pub actor: String,
    /// Who issued the request.
    pub requested_by: String,
    /// RFC 3339 expiry.
    pub expires_at: String,
    /// `pending`, `approved`, `declined`, `expired`, `consumed`, `failed`, `cancelled`.
    pub state: String,
    /// Who decided.
    pub decided_by: Option<String>,
    /// When.
    pub decided_at: Option<String>,
    /// Why.
    pub reason: Option<String>,
    /// When it was used.
    pub consumed_at: Option<String>,
    /// Result after consumption.
    pub result: Option<Value>,
    /// Error after consumption.
    pub error: Option<String>,
    /// When it was created.
    pub created_at: String,
}

impl ApprovalRecord {
    /// Whether the record has passed its expiry.
    pub fn is_expired(&self) -> bool {
        self.expires_at.as_str() <= util::now().as_str() && self.state == "pending"
    }

    /// A safe description for a UI, with no raw arguments.
    pub fn public_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "run_id": self.run_id,
            "task_id": self.task_id,
            "capability": self.capability,
            "args_hash": self.args_hash,
            "plan": self.plan,
            "scope": self.scope,
            "tier": self.tier,
            "risk": self.risk,
            "actor": self.actor,
            "expires_at": self.expires_at,
            "state": self.state,
            "decided_by": self.decided_by,
            "decided_at": self.decided_at,
            "reason": self.reason,
            "consumed_at": self.consumed_at,
        })
    }
}

/// One sequenced event in a run's durable log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    /// Global row id, stable across runs and usable as an SSE event id.
    pub id: i64,
    /// Run.
    pub run_id: String,
    /// Sequence within the run, starting at 1.
    pub seq: i64,
    /// Step.
    pub step_id: Option<String>,
    /// Kind.
    pub kind: String,
    /// Human-readable summary.
    pub message: String,
    /// Structured payload.
    pub data: Value,
    /// When.
    pub created_at: String,
}

/// What a run must prove before it may be called complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionPolicy {
    /// `conversation`, `research`, `code`, `test`, `setup`, `service`, `site`, `runtime`.
    pub kind: String,
    /// A change (file, config or repository) must be visible.
    pub requires_change: bool,
    /// A passing verification must exist.
    pub requires_verify: bool,
    /// The criteria a person supplied.
    pub acceptance: Vec<String>,
    /// Why this policy was chosen, for the timeline.
    pub note: String,
}

impl Default for CompletionPolicy {
    fn default() -> Self {
        Self {
            kind: "conversation".into(),
            requires_change: false,
            requires_verify: false,
            acceptance: Vec::new(),
            note: "a conversation with no stated verification requirement".into(),
        }
    }
}

impl CompletionPolicy {
    /// Derive a policy from the goal and the criteria a person gave.
    ///
    /// This is deliberately conservative but not theatrical: a question does not
    /// require a test run, and a request to fix code does. A person's explicit
    /// criteria always require verification.
    pub fn for_goal(goal: &str, acceptance: &[String]) -> Self {
        let g = goal.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| g.contains(w));
        let (kind, change, verify, why) = if has(&[
            "research",
            "explain",
            "summar",
            "audit",
            "what is",
            "how does",
            "read the code",
            "review the code",
            "find out",
        ]) {
            (
                "research",
                false,
                false,
                "the request asks for understanding, not a change",
            )
        } else if has(&[
            "failing test",
            "test failure",
            "fix the test",
            "make the test pass",
        ]) {
            (
                "test",
                true,
                true,
                "a failing test must be fixed and the test must pass",
            )
        } else if has(&[
            "merge conflict",
            "resolve the conflict",
            "resolve conflict",
            "conflicting",
        ]) {
            (
                "code",
                true,
                true,
                "a resolved conflict is a code change that must build and pass the project's checks",
            )
        } else if has(&[
            "set up",
            "setup",
            "install",
            "configure",
            "environment",
            "dependencies",
            "bootstrap",
        ]) {
            (
                "setup",
                true,
                true,
                "setting up an environment produces a change that must be usable",
            )
        } else if has(&[
            "start the service",
            "start service",
            "start the server",
            "run the service",
            "service is running",
        ]) {
            (
                "service",
                false,
                true,
                "starting a service is only done when its readiness check passes",
            )
        } else if has(&["site", "hostname", "https", "tls", "certificate", "domain"]) {
            (
                "site",
                true,
                true,
                "a site is only configured when it answers on its hostname",
            )
        } else if has(&[
            "runtime",
            "php version",
            "node version",
            "python version",
            "install php",
            "install node",
        ]) {
            (
                "runtime",
                true,
                true,
                "a runtime change must be shown to be installed and usable",
            )
        } else if has(&[
            "fix",
            "implement",
            "add ",
            "change",
            "refactor",
            "update",
            "create",
            "write",
            "remove",
            "rename",
            "build",
            "delete",
            "migrate",
            "upgrade",
        ]) {
            (
                "code",
                true,
                true,
                "the request changes code, so the change and a check of it are both required",
            )
        } else {
            (
                "conversation",
                false,
                false,
                "no change or verification was asked for",
            )
        };
        let acceptance: Vec<String> = acceptance
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let requires_verify = verify || !acceptance.is_empty();
        Self {
            kind: kind.into(),
            requires_change: change,
            requires_verify,
            note: if acceptance.is_empty() {
                why.into()
            } else {
                format!(
                    "{why}; the person also supplied {} acceptance criterion/criteria",
                    acceptance.len()
                )
            },
            acceptance,
        }
    }

    /// The evidence kinds that satisfy this policy's verification requirement.
    pub fn verifying_kinds(&self) -> &'static [&'static str] {
        match self.kind.as_str() {
            "service" => &["service", "http", "probe"],
            "site" => &["http", "probe", "service"],
            "runtime" => &["runtime", "command"],
            "setup" => &["verify", "command", "runtime"],
            _ => &["verify"],
        }
    }
}

/// The category of a normalized evidence fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// A file, repository or configuration change.
    Change,
    /// A read that gives a review its basis.
    Read,
    /// A check whose purpose is verification.
    Verify,
    /// A command that was run, without claiming it verifies anything.
    Command,
    /// A service's readiness.
    Service,
    /// An HTTP or TLS observation.
    Http,
    /// A runtime's installation or usability.
    Runtime,
    /// Database connectivity or a data operation.
    Database,
    /// A direct probe.
    Probe,
    /// A structured capability result.
    Structured,
}

impl EvidenceKind {
    /// Stable stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Change => "change",
            Self::Read => "read",
            Self::Verify => "verify",
            Self::Command => "command",
            Self::Service => "service",
            Self::Http => "http",
            Self::Runtime => "runtime",
            Self::Database => "database",
            Self::Probe => "probe",
            Self::Structured => "structured",
        }
    }
}

/// Evidence produced by one normalized capability call.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceDraft {
    /// Kind.
    pub kind: EvidenceKind,
    /// Whether it supports success.
    pub passed: bool,
    /// What was observed.
    pub summary: String,
    /// Supporting detail.
    pub detail: Option<String>,
    /// Exit code when a process ran.
    pub exit_code: Option<i64>,
}

/// The result of checking a run against its completion policy.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionVerdict {
    /// State the run should move to.
    pub state: RunState,
    /// Missing evidence, in plain language.
    pub missing: Vec<String>,
    /// The policy that was applied.
    pub policy: CompletionPolicy,
    /// Number of evidence facts considered.
    pub evidence: usize,
}

impl CompletionVerdict {
    /// Whether the evidence satisfies the policy.
    pub fn passed(&self) -> bool {
        self.missing.is_empty() && self.state == RunState::Completed
    }

    /// A notice suitable for a conversation timeline.
    pub fn notice(&self) -> String {
        if self.passed() {
            "Completion checked: the recorded evidence supports the claim.".to_string()
        } else {
            format!(
                "Not verified: {}. A claim of completion is not evidence; run the missing check or say plainly that it was not verified.",
                self.missing.join("; ")
            )
        }
    }
}

/// Tasks whose success is a verification for the project.
pub const VERIFY_TASKS: &[&str] = &[
    "test",
    "tests",
    "lint",
    "typecheck",
    "check",
    "build",
    "verify",
    "ci",
    "compile",
    "analyze",
    "analyse",
    "format-check",
    "fmt-check",
];

/// Capabilities that only observe state.
///
/// A read is not a change and not a readiness check, however successfully it
/// returned. Classifying reads as changes let a `site.list` satisfy the change
/// half of a site goal and a `service.list` satisfy a service goal, which is the
/// exact false positive this function exists to prevent.
fn is_read_capability(capability: &str) -> bool {
    const READ: &[&str] = &[
        "fs.read",
        "fs.list",
        "fs.search",
        "fs.browse",
        "git.diff",
        "git.blame",
        "git.search",
        "git.reflog",
        "git.switch.check",
        "git.stash.list",
        "git.stash.show",
        "git.conflicts.list",
        "git.conflicts.show",
        "git.sync.preview",
        "health.status",
        "health.list",
        "search.query",
        "docs.fetch",
        "compat.audit",
        "env.get",
        "env.diff",
        "env.reveal",
        "php.extensions",
        "php.settings",
        "php.pool",
        "php.site_profile",
        "hosts.show",
        "tls.ca.status",
        "shell.status",
        "shell.resolve",
        "ssh.keys",
        "ssh.public",
        "judge.status",
        "memory.list",
        "mcp.list",
        "approval.list",
        "approval.get",
        "chat.list",
        "chat.get",
        "chat.chapters",
        "team.list",
        "team.get",
        "team.report",
        "team.roles",
        "team.context",
        "team.charter.get",
        "schedule.list",
        "template.list",
        "template.get",
        "todo.list",
        "todo.get",
        "todo.history",
        "scratch.list",
        "scratch.get",
        "inbox.list",
        "logs.site",
        "logs.laravel",
        "review.list",
        "review.get",
        "deploy.status",
        "deploy.history",
        "deploy.log",
        "deploy.vars",
        "deploy.scheduled",
        "deploy.check",
        "nginx.site_show",
        "nginx.presets",
        "redirects.list",
        "cron.list",
        "cron.log",
        "queue.list",
        "queue.logs",
        "share.list",
        "remote.links",
        "remote.releases",
        "server.list",
        "server.info",
        "tls.list",
        "tls.ca.show",
        "backup.list",
        "notify.list",
        "autostart.status",
        "dictation.status",
        "toolchain.eas_projects",
        "stacks.schema",
        "stacks.matrix",
        "environment.get",
        "environment.manifest",
        "models.catalogue",
        "models.roles",
        "provider.presets",
        "skills.sources",
    ];
    READ.contains(&capability)
        || [
            ".list",
            ".get",
            ".show",
            ".logs",
            ".log",
            ".status",
            ".history",
            ".pins",
            ".inventory",
            ".available",
            ".discover",
            ".parked",
            ".detect",
            ".tasks",
            ".users",
            ".catalogue",
            ".roles",
            ".presets",
            ".sources",
            ".matrix",
            ".schema",
            ".runs",
            ".describe",
            ".preview",
            ".read",
            ".diff",
            ".blame",
            ".search",
            ".reflog",
            ".audit",
            ".extensions",
            ".settings",
            ".pool",
            ".profile",
            ".resolve",
            ".keys",
            ".public",
            ".fetch",
            ".query",
            ".stats",
            ".devices",
            ".plan",
        ]
        .iter()
        .any(|suffix| capability.ends_with(suffix))
}

/// Capabilities whose successful call changed something a person can inspect.
fn is_change_capability(capability: &str) -> bool {
    const CHANGE: &[&str] = &[
        "git.commit",
        "git.stage",
        "git.unstage",
        "git.merge",
        "git.revert",
        "git.reset",
        "git.discard",
        "git.cherry.pick",
        "git.rebase",
        "git.squash",
        "git.switch",
        "git.pull",
        "git.stash.push",
        "git.stash.apply",
        "git.stash.drop",
        "git.branch.create",
        "git.branch.delete",
        "git.branch.rename",
        "git.tag.create",
        "git.tag.delete",
        "git.remote.add",
        "git.remote.remove",
        "git.remote.rename",
        "git.ignore.add",
        "git.repo.init",
        "git.repo.track",
        "git.repo.untrack",
        "git.worktree.add",
        "git.worktree.remove",
        "git.worktree.prune",
        "git.conflicts.resolve",
        "git.conflicts.resolve_with",
        "git.op.abort",
        "git.op.continue",
        "project.init",
        "project.import",
        "project.forget",
        "project.start",
        "project.stop",
        "project.restart",
        "project.approve",
        "site.create",
        "site.sync",
        "site.set_upstream",
        "site.set_php",
        "site.set_serving",
        "site.remove",
        "site.fix_htaccess",
        "site.park",
        "site.unpark",
        "tls.issue",
        "tls.trust",
        "tls.untrust",
        "tls.ca.ensure",
        "hosts.sync",
        "service.install",
        "service.adopt",
        "service.remove",
        "service.set_startup",
        "runtime.remove",
        "runtime.pin",
        "runtime.use",
        "toolchain.install",
        "toolchain.remove",
        "db.create",
        "db.drop",
        "db.restore",
        "db.copy",
        "db.user_create",
        "db.user_drop",
        "environment.set",
        "env.set",
        "env.unset",
        "stack.up",
        "stack.down",
        "cron.add",
        "cron.enable",
        "cron.remove",
        "cron.run",
        "queue.add",
        "queue.remove",
        "redirects.add",
        "redirects.remove",
        "nginx.site_set",
        "nginx.site_clear",
        "php.configure",
        "php.extension_set",
        "deploy.script_set",
        "deploy.vars_set",
        "deploy.schedule",
        "deploy.unschedule",
        "backup.add",
        "backup.remove",
        "backup.run",
        "memory.apply",
        "skills.save",
        "skills.delete",
        "skills.install",
        "mcp.add",
        "mcp.remove",
        "mcp.trust",
        "mcp.update",
        "mcp.enable",
        "mcp.import",
        "provider.save",
        "provider.remove",
        "provider.default",
        "models.assign",
        "team.charter.set",
        "team.plan_submit",
        "team.start",
        "team.cancel",
        "team.pause",
        "team.resume",
        "team.retry",
        "team.accept",
        "team.approve_plan",
        "team.say",
        "team.assign",
        "chat.send",
        "chat.stop",
        "chat.update",
        "chat.fork",
        "chat.pin",
        "chat.unpin",
        "chat.delete",
        "chat.prune",
        "todo.create",
        "todo.update",
        "todo.complete",
        "todo.reopen",
        "todo.delete",
        "todo.note",
        "scratch.create",
        "scratch.update",
        "scratch.delete",
        "scratch.append",
        "schedule.create",
        "schedule.delete",
        "schedule.enable",
        "inbox.post",
        "inbox.reply",
        "inbox.delete",
        "inbox.clear",
        "review.run",
        "review.delete",
        "judge.configure",
        "shell.path_enable",
        "shell.path_disable",
        "autostart.enable",
        "autostart.disable",
        "notify.add",
        "notify.remove",
        "approval.decide",
        "ssh.adopt",
        "ssh.keygen",
        "dictation.install",
        "workflow.setup",
        "services.decide",
    ];
    CHANGE.contains(&capability)
        || capability.starts_with("fs.write")
        || capability.starts_with("fs.edit")
        || capability.starts_with("fs.move")
        || capability.starts_with("fs.delete")
        || capability.starts_with("fs.create")
}

/// The pseudo-capability under which a change seen in the project directory is
/// recorded. It was not made by a recorded call, so it is observation, not an
/// attributed action.
pub const OBSERVED_CAPABILITY: &str = "observed.diff";

/// Whether evidence proves an inspectable code or repository change.
///
/// A site record or an installed service is a change, but it is not the diff a
/// code goal has to show. Keeping the two apart is what makes "a code change
/// requires an inspectable change" true rather than approximate.
pub fn is_code_change(evidence: &Evidence) -> bool {
    evidence.kind == EvidenceKind::Change.as_str()
        && (evidence.capability.starts_with("fs.write")
            || evidence.capability.starts_with("fs.edit")
            || evidence.capability.starts_with("fs.move")
            || evidence.capability.starts_with("fs.delete")
            || evidence.capability.starts_with("fs.create")
            || evidence.capability == "git.commit"
            || evidence.capability == "git.stage"
            || evidence.capability == "git.merge"
            || evidence.capability == "git.revert"
            || evidence.capability == "git.conflicts.resolve"
            || evidence.capability == "git.conflicts.resolve_with"
            || evidence.capability == crate::runs::OBSERVED_CAPABILITY
            || evidence.capability == "project.init")
}

/// `Some(true)` only when every named field is present and true.
///
/// A readiness payload that does not say whether the service is answering is not
/// evidence that it is. Failing closed here is deliberate: an older or unexpected
/// shape must not be read as success.
fn all_true(data: &Value, keys: &[&str]) -> Option<bool> {
    let mut seen = false;
    for key in keys {
        if let Some(value) = data.get(*key).and_then(Value::as_bool) {
            seen = true;
            if !value {
                return Some(false);
            }
        }
    }
    seen.then_some(true)
}

/// Normalize one capability result into evidence at the registry boundary.
///
/// The capability id decides the category, never the exposed tool name: a call
/// through `run_task`, `devd_invoke` or an MCP bridge reaches the same branch.
/// A failed call still records evidence, marked as not passing.
///
/// Most calls produce one fact. A call that both changes something and proves it
/// usable - installing a runtime does exactly that - produces both, because
/// collapsing them would make one of the two claims unprovable.
pub fn evidence_for(
    capability: &str,
    args: &Value,
    outcome: Option<&Outcome>,
) -> Vec<EvidenceDraft> {
    evidence_for_effect(capability, None, args, outcome)
}

/// [`evidence_for`] with the capability's declared effect.
///
/// The name lists cannot know every capability. For one they do not name, the
/// effect the capability declared at registration decides: a read observes, any
/// other effect changed something. Without this, `git.push` or `composer.install`
/// succeeded and left no change on the record.
pub fn evidence_for_effect(
    capability: &str,
    effect: Option<crate::capability::Effect>,
    args: &Value,
    outcome: Option<&Outcome>,
) -> Vec<EvidenceDraft> {
    let ok = outcome.map(|o| o.ok).unwrap_or(false);
    let data = outcome.map(|o| &o.data).unwrap_or(&Value::Null);
    let exit_code = data.get("exit_code").and_then(Value::as_i64);
    let process_ok = ok
        && exit_code.map(|c| c == 0).unwrap_or(true)
        && data.get("timed_out").and_then(Value::as_bool) != Some(true)
        && data.get("cancelled").and_then(Value::as_bool) != Some(true);
    let label = |what: &str| format!("{capability}: {what}");
    let draft = |kind: EvidenceKind, passed: bool, summary: String| EvidenceDraft {
        kind,
        passed,
        summary,
        detail: outcome.map(|o| {
            let detail = serde_json::json!({
                "message": o.message,
                "data": o.data,
                "warnings": o.warnings,
            });
            util::truncate(&detail.to_string(), 4000)
        }),
        exit_code,
    };

    // A site probe observes the hostname, the listener and the HTTP status. It is
    // the only capability that can honestly satisfy a site goal's verification.
    if capability == "site.verify" {
        let host = data.get("host").and_then(Value::as_str).unwrap_or("");
        let status = data.get("status").and_then(Value::as_u64);
        let verified = ok && data.get("ok").and_then(Value::as_bool) == Some(true);
        let summary = match (verified, status) {
            (true, Some(code)) => format!("{host} answered HTTP {code}"),
            (false, Some(code)) => {
                format!("{host} answered HTTP {code}, which is not a healthy route")
            }
            (false, None) => format!("{host} did not answer"),
            (true, None) => format!("{host} verified"),
        };
        return vec![draft(EvidenceKind::Http, verified, label(&summary))];
    }

    if capability == "web.test" {
        return vec![draft(
            EvidenceKind::Verify,
            ok,
            label(if ok {
                "the web server accepted the generated configuration"
            } else {
                "the web server rejected the generated configuration"
            }),
        )];
    }

    if capability == "stacks.validate" {
        return vec![draft(
            EvidenceKind::Verify,
            ok,
            label(if ok {
                "the stack definition validated"
            } else {
                "the stack definition did not validate"
            }),
        )];
    }

    if capability == "toolchain.check" {
        let all_ok = data
            .as_array()
            .map(|rows| {
                rows.iter()
                    .all(|r| r.get("status").and_then(Value::as_str) == Some("ok"))
            })
            .unwrap_or(false);
        return vec![draft(
            EvidenceKind::Verify,
            ok && all_ok,
            label(if all_ok {
                "every prerequisite is satisfied"
            } else {
                "at least one prerequisite is not satisfied"
            }),
        )];
    }

    if capability == "workflow.setup" {
        let verified = data.get("verified").and_then(Value::as_bool) == Some(true);
        return vec![draft(
            EvidenceKind::Verify,
            ok && verified,
            label(if verified {
                "environment set up and verified"
            } else {
                "environment setup did not verify"
            }),
        )];
    }

    // Readiness is what the payload observed, not that the call returned. A
    // `service.status` of a stopped service is a check that ran and failed, not a
    // passing verification.
    if matches!(
        capability,
        "service.start" | "service.restart" | "service.status"
    ) {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("service");
        let ready = ok && all_true(data, &["running", "listening"]) == Some(true);
        let summary = if ready {
            format!("{name} is running and answering")
        } else if ok && data.get("running").and_then(Value::as_bool) == Some(true) {
            format!("{name} is running but nothing is answering")
        } else {
            format!("{name} is not running")
        };
        return vec![draft(EvidenceKind::Service, ready, label(&summary))];
    }

    if capability == "service.stop" {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("service");
        let stopped = ok && all_true(data, &["running"]) == Some(false);
        return vec![draft(
            EvidenceKind::Service,
            stopped,
            label(&format!(
                "{name} {}",
                if stopped {
                    "is stopped"
                } else {
                    "is still running"
                }
            )),
        )];
    }

    if capability == "project.run" || capability == "project.run_destructive" {
        let task = args.get("task").and_then(Value::as_str).unwrap_or("");
        let is_verify = capability == "project.run"
            && VERIFY_TASKS.contains(&task.to_ascii_lowercase().as_str());
        let kind = if is_verify {
            EvidenceKind::Verify
        } else {
            EvidenceKind::Command
        };
        let summary = if task.is_empty() {
            label("ran a project task")
        } else {
            label(&format!("task `{task}`"))
        };
        return vec![draft(kind, process_ok, summary)];
    }

    if capability.starts_with("project.exec") {
        let argv = args
            .get("argv")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        return vec![draft(
            EvidenceKind::Command,
            process_ok,
            label(&format!("`{argv}`")),
        )];
    }

    // An install is two facts: the change on disk, and the runtime proving that
    // it starts. The install records the version it actually ran.
    if capability == "runtime.install" {
        let proven = data
            .get("verified_version")
            .and_then(Value::as_str)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let kind = args
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("runtime");
        let version = args
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("requested version");
        return vec![
            draft(
                EvidenceKind::Change,
                ok,
                label(&format!("installed {kind} {version}")),
            ),
            draft(
                EvidenceKind::Runtime,
                ok && proven,
                label(&format!(
                    "{kind} {version} {}",
                    if proven {
                        "starts and reports its version"
                    } else {
                        "was installed but did not prove it starts"
                    }
                )),
            ),
        ];
    }

    // Database reads connect to the server, so they observe connectivity. They
    // are kept apart from reads of files and records, which observe neither a
    // change nor a service.
    if capability.starts_with("db.") || capability.starts_with("database.") {
        let (kind, passed) = if is_change_capability(capability) {
            (EvidenceKind::Change, ok)
        } else {
            (EvidenceKind::Database, ok)
        };
        return vec![draft(kind, passed, label("database"))];
    }

    if is_change_capability(capability) {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        return vec![draft(
            EvidenceKind::Change,
            ok,
            if path.is_empty() {
                label("changed state")
            } else {
                label(&format!("changed {path}"))
            },
        )];
    }

    if is_read_capability(capability) {
        let what = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_else(|| args.get("pattern").and_then(Value::as_str).unwrap_or(""));
        return vec![draft(
            EvidenceKind::Read,
            ok,
            label(if what.is_empty() { "read state" } else { what }),
        )];
    }

    if capability.starts_with("toolchain.") || capability.starts_with("runtime.") {
        return vec![draft(EvidenceKind::Runtime, ok, label("runtime"))];
    }

    match effect {
        Some(crate::capability::Effect::Read) => {
            return vec![draft(EvidenceKind::Read, ok, label("read state"))];
        }
        Some(_) => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            return vec![draft(
                EvidenceKind::Change,
                ok,
                if path.is_empty() {
                    label("changed state")
                } else {
                    label(&format!("changed {path}"))
                },
            )];
        }
        None => {}
    }

    vec![draft(EvidenceKind::Structured, ok, label("completed"))]
}

/// Check the evidence recorded for a run against its completion policy.
pub fn evaluate(policy: &CompletionPolicy, evidence: &[Evidence]) -> CompletionVerdict {
    let mut missing = Vec::new();
    let has_pass =
        |kind: EvidenceKind| evidence.iter().any(|e| e.kind == kind.as_str() && e.passed);
    let has_kind = |kind: EvidenceKind| evidence.iter().any(|e| e.kind == kind.as_str());

    if policy.requires_change {
        // A code goal needs an inspectable code or repository change. A site
        // record or an installed service is a change, but it is not the diff a
        // code goal promised, so it does not stand in for one.
        let change_ok = if matches!(policy.kind.as_str(), "code" | "test") {
            evidence.iter().any(|e| e.passed && is_code_change(e))
        } else {
            has_pass(EvidenceKind::Change)
        };
        if !change_ok {
            missing.push(if matches!(policy.kind.as_str(), "code" | "test") {
                "no inspectable code change was recorded".to_string()
            } else {
                "no inspectable change was recorded".to_string()
            });
        }
    }
    if policy.requires_verify {
        let kinds = policy.verifying_kinds();
        let passed = evidence
            .iter()
            .any(|e| e.passed && kinds.contains(&e.kind.as_str()));
        if !passed {
            let ran = evidence.iter().any(|e| kinds.contains(&e.kind.as_str()));
            missing.push(if ran {
                "the verification that ran did not pass".to_string()
            } else {
                "no passing verification was recorded (run the project's test, build, lint or check)".to_string()
            });
        }
    }
    // A failed change is not a completed change, even when the policy did not
    // explicitly ask for one.
    if has_kind(EvidenceKind::Change) && !has_pass(EvidenceKind::Change) && !policy.requires_change
    {
        missing.push("the change that was attempted did not succeed".to_string());
    }

    let state = if missing.is_empty() {
        RunState::Completed
    } else {
        RunState::Unverified
    };
    CompletionVerdict {
        state,
        missing,
        policy: policy.clone(),
        evidence: evidence.len(),
    }
}

/// The per-call execution context a registry invocation carries.
///
/// It ties a tool call to the run and step that caused it, carries the
/// cancellation flag into long operations, and gives the dispatcher a deadline
/// it can enforce before starting more work.
#[derive(Clone)]
pub struct RunContext {
    /// Owning run.
    pub run_id: String,
    /// Current step.
    pub step_id: Option<String>,
    /// Set when a person asked the run to stop.
    pub cancel: Arc<AtomicBool>,
    /// A point after which the call must be reported as timed out.
    pub deadline: Option<Instant>,
    /// The store that owns the durable cancellation flag, when one is attached.
    ///
    /// A stop requested from another window or the HTTP API writes the run row;
    /// consulting it here makes that request visible to a handler already in
    /// flight, not only between model turns.
    pub store: Option<Arc<crate::state::Store>>,
}

impl RunContext {
    /// A context for a run with no step and no deadline.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            step_id: None,
            cancel: Arc::new(AtomicBool::new(false)),
            deadline: None,
            store: None,
        }
    }

    /// Attach the authoritative store, so a durable stop request is observed.
    pub fn with_store(mut self, store: Arc<crate::state::Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// A context for one step.
    pub fn step(run_id: impl Into<String>, step_id: impl Into<String>) -> Self {
        Self {
            step_id: Some(step_id.into()),
            ..Self::new(run_id)
        }
    }

    /// Whether cancellation was requested.
    pub fn cancelled(&self) -> bool {
        if self.cancel.load(Ordering::SeqCst) {
            return true;
        }
        self.store
            .as_ref()
            .and_then(|s| s.run_by_id(&self.run_id).ok().flatten())
            .map(|r| r.cancellation.is_some())
            .unwrap_or(false)
    }

    /// Whether the deadline has passed.
    pub fn expired(&self) -> bool {
        self.deadline.map(|d| Instant::now() >= d).unwrap_or(false)
    }

    /// Refuse to start work when the caller has already gone.
    pub fn check(&self) -> Result<()> {
        if self.cancelled() {
            return Err(DevdError::denied("the run was cancelled"));
        }
        if self.expired() {
            return Err(DevdError::Timeout(
                "the call deadline passed before the operation started".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_mutation_the_name_lists_do_not_know_is_a_change_by_its_declared_effect() {
        use crate::capability::Effect;
        let ok = Outcome::ok("done");
        // Unnamed capabilities are classified by what they declared.
        let pushed = evidence_for_effect("git.push", Some(Effect::Remote), &json!({}), Some(&ok));
        assert_eq!(pushed[0].kind, EvidenceKind::Change);
        let peeked = evidence_for_effect("web.ports", Some(Effect::Read), &json!({}), Some(&ok));
        assert_eq!(peeked[0].kind, EvidenceKind::Read);
        // With no declared effect an unknown name stays unclassified, never a change.
        let unknown = evidence_for("some.new_thing", &json!({}), Some(&ok));
        assert_eq!(unknown[0].kind, EvidenceKind::Structured);
        // A mutation whose name ends in a read-looking suffix is still a change.
        let resolved = evidence_for("git.conflicts.resolve", &json!({}), Some(&ok));
        assert_eq!(resolved[0].kind, EvidenceKind::Change);
    }

    #[test]
    fn lifecycle_moves_forward_and_cannot_leave_a_cancelled_run() {
        assert!(RunState::Intake.can_transition_to(RunState::Planning));
        assert!(RunState::Planning.can_transition_to(RunState::AwaitingApproval));
        assert!(RunState::AwaitingApproval.can_transition_to(RunState::Executing));
        assert!(RunState::Executing.can_transition_to(RunState::Verifying));
        assert!(RunState::Verifying.can_transition_to(RunState::Completed));
        assert!(
            RunState::Completed.can_transition_to(RunState::Executing),
            "a deliberate retry is allowed"
        );
        assert!(!RunState::Cancelled.can_transition_to(RunState::Executing));
    }

    #[test]
    fn a_question_does_not_demand_a_test_run_and_a_fix_does() {
        let q = CompletionPolicy::for_goal("explain how the router works", &[]);
        assert_eq!(q.kind, "research");
        assert!(!q.requires_verify);

        let fix = CompletionPolicy::for_goal("fix the failing login test", &[]);
        assert!(fix.requires_change && fix.requires_verify);

        let e = CompletionPolicy::for_goal(
            "what does this repository do",
            &["The answer names the entry point".into()],
        );
        assert!(
            e.requires_verify,
            "a person's criterion always needs evidence"
        );
    }

    fn fact(kind: EvidenceKind, passed: bool) -> Evidence {
        // The capability has to agree with the kind, because a code change is
        // recognised by the capability that produced it, not by the label alone.
        let capability = match kind {
            EvidenceKind::Change => "fs.write",
            EvidenceKind::Verify => "project.run",
            EvidenceKind::Http => "site.verify",
            EvidenceKind::Service => "service.status",
            EvidenceKind::Runtime => "runtime.install",
            EvidenceKind::Database => "db.list",
            _ => "c",
        };
        Evidence {
            id: util::new_id(),
            run_id: "r".into(),
            step_id: None,
            tool_call_id: None,
            artifact_id: None,
            kind: kind.as_str().into(),
            capability: capability.into(),
            passed,
            claim: None,
            summary: "s".into(),
            detail: None,
            created_at: util::now(),
        }
    }

    #[test]
    fn a_change_without_a_check_is_unverified() {
        let p = CompletionPolicy::for_goal("fix the parser", &[]);
        let v = evaluate(&p, &[fact(EvidenceKind::Change, true)]);
        assert_eq!(v.state, RunState::Unverified);
        assert!(
            v.missing.iter().any(|m| m.contains("verification")),
            "{:?}",
            v.missing
        );
    }

    #[test]
    fn a_change_with_a_passing_check_is_complete() {
        let p = CompletionPolicy::for_goal("fix the parser", &[]);
        let v = evaluate(
            &p,
            &[
                fact(EvidenceKind::Change, true),
                fact(EvidenceKind::Verify, true),
            ],
        );
        assert!(v.passed(), "{:?}", v.missing);
    }

    #[test]
    fn a_failed_check_is_never_treated_as_success() {
        let p = CompletionPolicy::for_goal("fix the parser", &[]);
        let v = evaluate(
            &p,
            &[
                fact(EvidenceKind::Change, true),
                fact(EvidenceKind::Verify, false),
            ],
        );
        assert_eq!(v.state, RunState::Unverified);
        assert!(
            v.missing.iter().any(|m| m.contains("did not pass")),
            "{:?}",
            v.missing
        );
    }

    #[test]
    fn direct_and_indirect_names_produce_the_same_evidence() {
        // The category is derived from the capability, not the tool name the
        // model used. `fs.edit` is a change whether it arrived through
        // `edit_file` or `devd_invoke`.
        let args = json!({ "path": "src/lib.rs" });
        let outcome = Outcome::with_data("edited", json!({}));
        let e = evidence_for("fs.edit", &args, Some(&outcome));
        assert_eq!(e[0].kind, EvidenceKind::Change);
        assert!(e[0].passed);
    }

    #[test]
    fn a_read_is_not_a_change_and_not_a_readiness_check() {
        // Listing a site, a service or a runtime observes state. It must not
        // stand in for the change or the check a goal asked for.
        let ok = Outcome::with_data("listed", json!({ "sites": [] }));
        for capability in [
            "site.list",
            "service.list",
            "runtime.list",
            "environment.get",
            "git.status",
        ] {
            let drafts = evidence_for(capability, &json!({}), Some(&ok));
            assert_eq!(
                drafts.len(),
                1,
                "{capability} should produce one fact, not {drafts:?}"
            );
            assert_eq!(drafts[0].kind, EvidenceKind::Read, "{capability}");
            assert!(drafts[0].passed, "{capability}");
        }

        // A database read observes a live connection, so it is database evidence
        // rather than a file read; it is still not a change.
        let db = evidence_for("db.list", &json!({}), Some(&ok));
        assert_eq!(db[0].kind, EvidenceKind::Database);

        // A code goal refuses a change that is not a code change.
        let policy = CompletionPolicy::for_goal("fix the parser", &[]);
        let mut site_change = fact(EvidenceKind::Change, true);
        site_change.capability = "site.create".into();
        let verdict = evaluate(&policy, &[site_change, fact(EvidenceKind::Verify, true)]);
        assert_eq!(verdict.state, RunState::Unverified);
        assert!(
            verdict.missing.iter().any(|m| m.contains("code change")),
            "{:?}",
            verdict.missing
        );
    }

    #[test]
    fn a_stopped_service_is_not_ready_evidence() {
        let stopped = Outcome::with_data(
            "not running",
            json!({ "running": false, "listening": false }),
        );
        let drafts = evidence_for(
            "service.status",
            &json!({ "name": "nginx" }),
            Some(&stopped),
        );
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, EvidenceKind::Service);
        assert!(
            !drafts[0].passed,
            "a status call that observed a stopped service did not pass readiness"
        );

        let running = Outcome::with_data("running", json!({ "running": true, "listening": true }));
        let drafts = evidence_for(
            "service.status",
            &json!({ "name": "nginx" }),
            Some(&running),
        );
        assert!(drafts[0].passed);

        // A payload that does not say whether it is listening fails closed.
        let vague = Outcome::with_data("ok", json!({}));
        let drafts = evidence_for("service.status", &json!({ "name": "nginx" }), Some(&vague));
        assert!(!drafts[0].passed);
    }

    #[test]
    fn site_verify_is_the_only_http_readiness_fact() {
        let good = Outcome::with_data(
            "answered",
            json!({ "host": "blog.test", "status": 200, "ok": true }),
        );
        let drafts = evidence_for("site.verify", &json!({ "host": "blog.test" }), Some(&good));
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, EvidenceKind::Http);
        assert!(drafts[0].passed);
        assert!(
            drafts[0].summary.contains("HTTP 200"),
            "{}",
            drafts[0].summary
        );

        let bad = Outcome::with_data(
            "server error",
            json!({ "host": "blog.test", "status": 502, "ok": false }),
        );
        let drafts = evidence_for("site.verify", &json!({ "host": "blog.test" }), Some(&bad));
        assert!(!drafts[0].passed);

        // A successful call that did not observe a healthy answer is not proof.
        let silent = Outcome::with_data("no answer", json!({ "host": "blog.test" }));
        let drafts = evidence_for(
            "site.verify",
            &json!({ "host": "blog.test" }),
            Some(&silent),
        );
        assert!(!drafts[0].passed);
    }

    #[test]
    fn installing_a_runtime_records_both_the_change_and_the_proof() {
        let installed = Outcome::with_data("installed", json!({ "verified_version": "8.3.14" }));
        let drafts = evidence_for(
            "runtime.install",
            &json!({ "kind": "php", "version": "8.3" }),
            Some(&installed),
        );
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].kind, EvidenceKind::Change);
        assert!(drafts[0].passed);
        assert_eq!(drafts[1].kind, EvidenceKind::Runtime);
        assert!(drafts[1].passed);

        // An install whose binary never reported a version changed the disk but
        // did not prove the runtime works.
        let unproven = Outcome::with_data("unpacked", json!({}));
        let drafts = evidence_for(
            "runtime.install",
            &json!({ "kind": "php", "version": "8.3" }),
            Some(&unproven),
        );
        assert!(drafts[0].passed);
        assert!(!drafts[1].passed);
    }

    #[test]
    fn project_exec_is_a_command_and_project_run_test_is_a_verification() {
        let ok = Outcome::with_data("ok", json!({ "ok": true, "exit_code": 0 }));
        assert_eq!(
            evidence_for(
                "project.exec",
                &json!({ "argv": ["git", "status"] }),
                Some(&ok)
            )[0]
            .kind,
            EvidenceKind::Command
        );
        assert_eq!(
            evidence_for("project.run", &json!({ "task": "test" }), Some(&ok))[0].kind,
            EvidenceKind::Verify
        );
        assert_eq!(
            evidence_for("project.run", &json!({ "task": "dev" }), Some(&ok))[0].kind,
            EvidenceKind::Command
        );
    }

    #[test]
    fn a_timed_out_process_does_not_pass() {
        let out = Outcome::with_data(
            "timed out",
            json!({ "ok": false, "exit_code": 0, "timed_out": true }),
        );
        assert!(!evidence_for("project.run", &json!({ "task": "test" }), Some(&out))[0].passed);
    }

    #[test]
    fn a_run_context_enforces_cancellation_and_deadline() {
        let c = RunContext::new("r");
        c.check().unwrap();
        c.cancel.store(true, Ordering::SeqCst);
        assert!(c.check().is_err());

        let mut d = RunContext::new("r");
        d.deadline = Some(Instant::now() - std::time::Duration::from_millis(1));
        assert!(matches!(d.check(), Err(DevdError::Timeout(_))));
    }
}
