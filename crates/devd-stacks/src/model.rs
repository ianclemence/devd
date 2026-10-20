//! The structured, validated description of a stack.
//!
//! Everything framework-specific lives in these types, loaded from YAML. The
//! initialisation engine, project detection, task resolution, the CLI, the TUI,
//! the web UI and the agent interface all read the same structures; none of
//! them contains a conditional on a framework name.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A link to official documentation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocLink {
    /// Link text.
    pub title: String,
    /// Absolute https URL on the vendor's own site.
    pub url: String,
}

/// Where a definition's commands and flags were checked.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Verification {
    /// `cli-help` when the flags were read from the tool's own help output,
    /// `docs` when they come from the official documentation only.
    pub method: String,
    /// The tool and version the check was made against.
    pub against: String,
    /// ISO date of the check.
    pub date: String,
}

/// A selectable framework or template version.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VersionDef {
    /// Stable identifier, e.g. `12`.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Substituted for `{version}` in commands, e.g. `^12.0`.
    #[serde(default)]
    pub value: String,
    /// Whether this is the default selection.
    #[serde(default)]
    pub default: bool,
    /// Minimum tool versions this framework version needs.
    #[serde(default)]
    pub requires: Vec<ToolReq>,
    /// Free-form note, e.g. support status.
    #[serde(default)]
    pub note: String,
}

/// A requirement on an installed tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolReq {
    /// Tool identifier from the tool catalogue.
    pub tool: String,
    /// Minimum version, inclusive.
    #[serde(default)]
    pub min: Option<String>,
    /// Maximum version, inclusive of the given components (e.g. `8.4` allows 8.4.x).
    #[serde(default)]
    pub max: Option<String>,
    /// Why the stack needs it.
    #[serde(default)]
    pub why: String,
}

/// Kind of an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    /// Exactly one of the choices.
    Select,
    /// Any subset of the choices.
    Multi,
    /// On or off.
    Bool,
    /// Free text, validated by `pattern`.
    Text,
}

/// One value of a select or multi option.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Choice {
    /// Value passed through.
    pub value: String,
    /// Display label.
    pub label: String,
    /// Explanation.
    #[serde(default)]
    pub help: String,
}

/// A user-visible initialisation option.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OptionDef {
    /// Identifier used in requests and conditions.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Kind of input.
    pub kind: OptionKind,
    /// Default value (string, bool or array of strings).
    #[serde(default)]
    pub default: Value,
    /// What the option does and why you might change it.
    #[serde(default)]
    pub help: String,
    /// Hidden behind the advanced path by default.
    #[serde(default)]
    pub advanced: bool,
    /// Choices for select and multi.
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// Regular expression a text value must match.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Framework version ids this option applies to; empty means all.
    #[serde(default)]
    pub versions: Vec<String>,
    /// The CLI flag this option maps to, shown for transparency.
    #[serde(default)]
    pub flag: Option<String>,
}

/// A condition over option values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Cond {
    /// Option id; `version`, `package_manager`, `git` and `install` are always available.
    pub option: String,
    /// Equal to this value.
    #[serde(default)]
    pub equals: Option<Value>,
    /// One of these values.
    #[serde(default, rename = "in")]
    pub in_: Option<Vec<Value>>,
    /// A multi option containing this value.
    #[serde(default)]
    pub contains: Option<String>,
    /// Non-empty text or non-empty list.
    #[serde(default)]
    pub nonempty: Option<bool>,
    /// Invert the result.
    #[serde(default)]
    pub not: bool,
}

/// One condition or a list that must all hold.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum When {
    /// A single condition.
    One(Cond),
    /// Every condition must hold.
    All(Vec<Cond>),
}

impl When {
    /// The conditions as a slice-like vector.
    pub fn conds(&self) -> Vec<&Cond> {
        match self {
            When::One(c) => vec![c],
            When::All(v) => v.iter().collect(),
        }
    }
}

/// One element of a command line: a literal or a conditional group.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ArgItem {
    /// A literal, possibly containing `{placeholders}`.
    Lit(String),
    /// Elements included when a condition holds.
    If {
        /// The condition (or conditions that must all hold).
        when: When,
        /// Elements when true.
        args: Vec<ArgItem>,
        /// Elements when false.
        #[serde(default, rename = "else")]
        otherwise: Vec<ArgItem>,
    },
}

/// What a step does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    /// Run a program.
    #[default]
    Exec,
    /// Write a file.
    Write,
    /// Download and unpack a zip archive.
    Fetch,
    /// Create or update KEY=VALUE lines in a dotenv file (`set`).
    Env,
    /// Set values in a JSON file by dotted path (`set`).
    Json,
    /// Create a directory.
    Mkdir,
}

/// Where a step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum StepCwd {
    /// The directory that will contain the project.
    #[default]
    Parent,
    /// The project directory itself.
    Project,
}

/// One step of project initialisation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    /// Identifier.
    pub id: String,
    /// What the step does, in a sentence.
    pub title: String,
    /// Step kind.
    #[serde(default)]
    pub kind: StepKind,
    /// Working directory.
    #[serde(default)]
    pub cwd: StepCwd,
    /// Program followed by arguments (exec).
    #[serde(default)]
    pub run: Vec<ArgItem>,
    /// Relative path (write) or URL template (fetch).
    #[serde(default)]
    pub path: Option<String>,
    /// File content template (write), or query string template (fetch).
    #[serde(default)]
    pub content: Option<String>,
    /// Run only when this holds.
    #[serde(default)]
    pub when: Option<When>,
    /// Tools that must be installed for the step.
    #[serde(default)]
    pub needs: Vec<String>,
    /// Whether the step downloads from the network.
    #[serde(default)]
    pub network: bool,
    /// Failure is reported as a warning rather than aborting.
    #[serde(default)]
    pub optional: bool,
    /// The main scaffolding step: user-supplied extra arguments are appended here.
    #[serde(default)]
    pub extensible: bool,
    /// Fixed, non-secret environment for the step.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Values for `env` and `json` steps.
    #[serde(default)]
    pub set: BTreeMap<String, String>,
}

/// Initialisation recipe.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct InitSpec {
    /// Whether a project directory is created by the scaffolder (`parent`) or
    /// by devd before the first step (`project`).
    #[serde(default)]
    pub creates_dir: bool,
    /// Ordered steps.
    pub steps: Vec<Step>,
    /// Whether the dependency-install toggle means anything for this stack.
    #[serde(default)]
    pub install_toggle: bool,
}

/// Post-initialisation verification.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct VerifySpec {
    /// Files that must exist, relative to the project.
    #[serde(default)]
    pub files: Vec<String>,
    /// Cheap commands that prove the project works, run from the project.
    #[serde(default)]
    pub commands: Vec<VerifyCommand>,
}

/// A verification command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VerifyCommand {
    /// Title.
    pub title: String,
    /// Program and arguments.
    pub run: Vec<ArgItem>,
    /// Only after dependencies were installed.
    #[serde(default)]
    pub needs_install: bool,
    /// Tools required.
    #[serde(default)]
    pub needs: Vec<String>,
}

/// Side-effect class of a task, deciding which gate it passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Reads or builds; safe to retry.
    #[default]
    Safe,
    /// Changes the project or its data; not necessarily safe to repeat.
    Mutating,
    /// Can destroy data; needs a human approval.
    Destructive,
}

/// A named workflow command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskDef {
    /// Identifier, e.g. `dev`, `test`, `migrate`.
    pub id: String,
    /// Title.
    pub title: String,
    /// `dev`, `build`, `test`, `lint`, `format`, `typecheck`, `install`, `migrate`, `other`.
    #[serde(default = "default_task_kind")]
    pub kind: String,
    /// Side-effect class.
    #[serde(default)]
    pub risk: Risk,
    /// Runs until stopped (a development server).
    #[serde(default)]
    pub long_running: bool,
    /// Program and arguments.
    pub run: Vec<ArgItem>,
    /// Available only when package.json defines this script.
    #[serde(default)]
    pub requires_script: Option<String>,
    /// Available only when this file exists in the project.
    #[serde(default)]
    pub requires_file: Option<String>,
    /// Tools required.
    #[serde(default)]
    pub needs: Vec<String>,
    /// Default port a long-running task listens on.
    #[serde(default)]
    pub port: Option<u16>,
    /// Longer explanation.
    #[serde(default)]
    pub help: String,
}

fn default_name_pattern() -> String {
    "^[a-z0-9][a-z0-9._-]{0,63}$".to_string()
}

fn default_task_kind() -> String {
    "other".to_string()
}

/// One detection rule.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DetectRule {
    /// A file or directory exists.
    #[serde(default)]
    pub file: Option<String>,
    /// A dependency is declared in package.json, composer.json, Gemfile, pyproject or requirements.
    #[serde(default)]
    pub dependency: Option<String>,
    /// File must contain this text (with `in_file`).
    #[serde(default)]
    pub text: Option<String>,
    /// File for `text`.
    #[serde(default)]
    pub in_file: Option<String>,
    /// Points contributed.
    pub weight: u32,
    /// Human-readable evidence shown to users and agents.
    pub evidence: String,
}

/// Detection configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DetectSpec {
    /// Score needed to report the stack at all.
    #[serde(default = "default_min_score")]
    pub min_score: u32,
    /// Rules.
    #[serde(default)]
    pub rules: Vec<DetectRule>,
}

fn default_min_score() -> u32 {
    3
}

/// A service a project depends on, conditional on its options.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceReq {
    /// Tool catalogue id of the service.
    pub tool: String,
    /// Only when this holds.
    #[serde(default)]
    pub when: Option<When>,
    /// Why.
    #[serde(default)]
    pub why: String,
}

/// An environment variable a project expects.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvVar {
    /// Name.
    pub name: String,
    /// Required to run.
    #[serde(default)]
    pub required: bool,
    /// Value is secret; never shown or logged.
    #[serde(default)]
    pub secret: bool,
    /// Explanation.
    #[serde(default)]
    pub help: String,
}

/// Which programs `project.exec` may run for the stack, and which arguments mark
/// a command as destructive.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExecPolicy {
    /// Allowed program names (the package manager's own program is always allowed).
    #[serde(default)]
    pub programs: Vec<String>,
    /// Argument values that make a command destructive.
    #[serde(default)]
    pub destructive_args: Vec<String>,
}

/// A complete stack definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StackDef {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Language.
    pub language: String,
    /// `web`, `frontend`, `backend`, `mobile`, `systems`, `language`.
    pub category: String,
    /// One line.
    pub summary: String,
    /// Longer description.
    #[serde(default)]
    pub description: String,
    /// Official documentation.
    pub docs: Vec<DocLink>,
    /// Where the commands were verified.
    pub verified: Verification,
    /// Primary tool.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Package managers offered, default first.
    #[serde(default)]
    pub package_managers: Vec<String>,
    /// Framework versions.
    pub versions: Vec<VersionDef>,
    /// Options.
    #[serde(default)]
    pub options: Vec<OptionDef>,
    /// Pattern a project name must match.
    #[serde(default = "default_name_pattern")]
    pub name_pattern: String,
    /// Explains the name rules.
    #[serde(default)]
    pub name_hint: String,
    /// Initialisation recipe; no steps means the stack is detect-only.
    pub init: InitSpec,
    /// Verification.
    #[serde(default)]
    pub verify: VerifySpec,
    /// Workflow commands.
    #[serde(default)]
    pub tasks: Vec<TaskDef>,
    /// Detection.
    #[serde(default)]
    pub detect: DetectSpec,
    /// Services.
    #[serde(default)]
    pub services: Vec<ServiceReq>,
    /// Environment variables.
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Expected layout, for documentation and agents.
    #[serde(default)]
    pub structure: Vec<String>,
    /// Conventions an agent should follow.
    #[serde(default)]
    pub conventions: Vec<String>,
    /// Command policy.
    #[serde(default)]
    pub exec: ExecPolicy,
}

impl StackDef {
    /// The default version.
    pub fn default_version(&self) -> &VersionDef {
        self.versions
            .iter()
            .find(|v| v.default)
            .unwrap_or(&self.versions[0])
    }

    /// A version by id.
    pub fn version(&self, id: &str) -> Option<&VersionDef> {
        self.versions.iter().find(|v| v.id == id)
    }
}
