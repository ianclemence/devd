//! Evidence-based project detection.
//!
//! A project's stack is decided by weighing several independent signals -
//! declared dependencies, entry points, configuration and lockfiles - against
//! the stack definitions. Nothing here depends on how the project was created.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use devd_core::capability::Ctx;
use devd_core::Result;

use crate::model::*;
use crate::registry::StackRegistry;
use crate::tasks::{self, ProjectScripts, TaskInfo};
use crate::toolchain;
use crate::tools::{cmp_version, ToolStatus};

/// A stack that matched, with the reasons.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StackMatch {
    /// Stack id.
    pub id: String,
    /// Name.
    pub name: String,
    /// Points scored.
    pub score: u32,
    /// `high`, `medium` or `low`.
    pub confidence: String,
    /// What was found.
    pub evidence: Vec<String>,
}

/// A runtime the project asks for.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeNeed {
    /// Tool id.
    pub tool: String,
    /// The constraint as written.
    pub constraint: String,
    /// The file it came from.
    pub source: String,
    /// What is installed.
    pub installed: Option<String>,
    /// Whether the installed version satisfies it (`None` when unknown).
    pub satisfied: Option<bool>,
}

/// A missing or unsuitable tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Missing {
    /// Tool id.
    pub tool: String,
    /// Why it is needed.
    pub why: String,
    /// What to do.
    pub fix: String,
}

/// A service the project appears to need.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServiceHint {
    /// Tool id.
    pub tool: String,
    /// The evidence.
    pub why: String,
    /// Installed.
    pub installed: bool,
    /// Accepting connections.
    pub running: bool,
}

/// Environment-variable requirements. Names only; values are never read out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct EnvInfo {
    /// Example files found.
    pub example_files: Vec<String>,
    /// The live env file exists.
    pub env_file_present: bool,
    /// Names the example defines.
    pub keys: Vec<String>,
    /// Names that look secret.
    pub secret_keys: Vec<String>,
    /// Names the example leaves empty and the live file does not set.
    pub unset_keys: Vec<String>,
}

/// Everything known about a project directory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProjectContext {
    /// Absolute directory.
    pub path: String,
    /// Directory name.
    pub name: String,
    /// Matching stacks, best first.
    pub stacks: Vec<StackMatch>,
    /// The chosen stack id.
    pub primary: Option<String>,
    /// Detection is uncertain; the user should confirm.
    pub ambiguous: bool,
    /// Explanation of the uncertainty.
    pub ambiguity: Option<String>,
    /// Package manager and why.
    pub package_manager: Option<String>,
    /// Evidence for the package manager.
    pub package_manager_evidence: Option<String>,
    /// Runtime versions the project asks for.
    pub runtimes: Vec<RuntimeNeed>,
    /// Manifests found.
    pub manifests: Vec<String>,
    /// Lockfiles found.
    pub lockfiles: Vec<String>,
    /// Entry points found.
    pub entry_points: Vec<String>,
    /// Top-level entries.
    pub layout: Vec<String>,
    /// Workflow commands.
    pub tasks: Vec<TaskInfo>,
    /// Quality tooling files found, by purpose.
    pub quality: BTreeMap<String, Vec<String>>,
    /// Services the project appears to need.
    pub services: Vec<ServiceHint>,
    /// Environment requirements.
    pub env: EnvInfo,
    /// Documentation files for people and agents.
    pub docs: Vec<String>,
    /// Git facts.
    pub git: Value,
    /// Conventions from the stack definition.
    pub conventions: Vec<String>,
    /// Official documentation for the stack.
    pub official_docs: Vec<DocLink>,
    /// Tools that are needed and absent or unsuitable.
    pub missing: Vec<Missing>,
    /// Anything else worth saying.
    pub warnings: Vec<String>,
}

/// Parse and test a version constraint such as `>=20.9.0`, `^22.13`, `~3.2`,
/// `8.3.*`, `>=8.2 <8.5` or `^20 || >=22`. `None` means it could not be read.
pub fn constraint_ok(installed: &str, constraint: &str) -> Option<bool> {
    let mut any_parsed = false;
    for alt in constraint.split("||") {
        let mut all = true;
        let mut parsed = false;
        for tok in alt.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()) {
            let (op, ver) = if let Some(v) = tok.strip_prefix(">=") {
                (">=", v)
            } else if let Some(v) = tok.strip_prefix("<=") {
                ("<=", v)
            } else if let Some(v) = tok.strip_prefix('>') {
                (">", v)
            } else if let Some(v) = tok.strip_prefix('<') {
                ("<", v)
            } else if let Some(v) = tok.strip_prefix('^') {
                ("^", v)
            } else if let Some(v) = tok.strip_prefix('~') {
                ("~", v)
            } else if let Some(v) = tok.strip_prefix("==").or_else(|| tok.strip_prefix('=')) {
                ("=", v)
            } else {
                ("=", tok)
            };
            let ver = ver.trim_start_matches('v');
            let wild = ver.trim_end_matches(".*").trim_end_matches(".x").trim_end_matches('*');
            if crate::tools::parse_version(wild).is_empty() {
                continue;
            }
            parsed = true;
            let parts = crate::tools::parse_version(wild);
            let ge = cmp_version(installed, wild) != std::cmp::Ordering::Less;
            let ok = match op {
                ">=" => ge,
                ">" => cmp_version(installed, wild) == std::cmp::Ordering::Greater,
                "<=" => crate::tools::satisfies(installed, None, Some(wild)),
                "<" => cmp_version(installed, wild) == std::cmp::Ordering::Less,
                "^" => {
                    let inst = crate::tools::parse_version(installed);
                    ge && inst.first() == parts.first()
                }
                "~" => {
                    let inst = crate::tools::parse_version(installed);
                    ge && inst.first() == parts.first() && (parts.len() < 2 || inst.get(1) == parts.get(1))
                }
                _ => crate::tools::satisfies(installed, Some(wild), Some(wild)),
            };
            all &= ok;
        }
        if parsed {
            any_parsed = true;
            if all {
                return Some(true);
            }
        }
    }
    if any_parsed {
        Some(false)
    } else {
        None
    }
}

fn read(dir: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(rel)).ok().filter(|t| t.len() < 4_000_000)
}

fn glob_exists(dir: &Path, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return dir.join(pattern).exists();
    }
    let (pre, post) = pattern.split_once('*').unwrap_or((pattern, ""));
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with(pre) && n.ends_with(post)
            })
        })
        .unwrap_or(false)
}

fn glob_read(dir: &Path, pattern: &str) -> Option<String> {
    if !pattern.contains('*') {
        return read(dir, pattern);
    }
    let (pre, post) = pattern.split_once('*')?;
    let mut hits: Vec<String> = Vec::new();
    for e in std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()) {
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with(pre) && n.ends_with(post) {
            if let Some(t) = read(dir, &n) {
                hits.push(t);
            }
        }
    }
    if hits.is_empty() {
        None
    } else {
        Some(hits.join("\n"))
    }
}

/// Dependencies declared anywhere in the project's manifests, lowercased.
fn declared_dependencies(dir: &Path) -> BTreeSet<String> {
    let mut deps = BTreeSet::new();
    if let Some(t) = read(dir, "package.json") {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            for sect in ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] {
                if let Some(o) = v.get(sect).and_then(|x| x.as_object()) {
                    deps.extend(o.keys().map(|k| k.to_ascii_lowercase()));
                }
            }
        }
    }
    if let Some(t) = read(dir, "composer.json") {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            for sect in ["require", "require-dev"] {
                if let Some(o) = v.get(sect).and_then(|x| x.as_object()) {
                    deps.extend(o.keys().map(|k| k.to_ascii_lowercase()));
                }
            }
        }
    }
    if let Some(t) = read(dir, "Gemfile") {
        let re = Regex::new(r#"(?m)^\s*gem\s+["']([^"']+)["']"#).unwrap();
        deps.extend(re.captures_iter(&t).map(|c| c[1].to_ascii_lowercase()));
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("requirements") && n.ends_with(".txt") {
                if let Some(t) = read(dir, &n) {
                    let re = Regex::new(r"(?m)^\s*([A-Za-z0-9][A-Za-z0-9_.\-]*)").unwrap();
                    deps.extend(re.captures_iter(&t).map(|c| c[1].to_ascii_lowercase().replace('_', "-")));
                }
            }
        }
    }
    if let Some(t) = read(dir, "pyproject.toml") {
        let quoted = Regex::new(r#"["']([A-Za-z0-9][A-Za-z0-9_.\-]*)\s*(?:\[[^\]]*\])?\s*(?:[<>=!~;][^"']*)?["']"#).unwrap();
        deps.extend(quoted.captures_iter(&t).map(|c| c[1].to_ascii_lowercase().replace('_', "-")));
        let kv = Regex::new(r"(?m)^([A-Za-z0-9][A-Za-z0-9_.\-]*)\s*=").unwrap();
        deps.extend(kv.captures_iter(&t).map(|c| c[1].to_ascii_lowercase().replace('_', "-")));
    }
    if let Some(t) = read(dir, "go.mod") {
        let re = Regex::new(r"(?m)^\s*(?:require\s+)?([a-z0-9.\-]+\.[a-z]+/[^\s]+)\s+v").unwrap();
        deps.extend(re.captures_iter(&t).map(|c| c[1].to_ascii_lowercase()));
    }
    if let Some(t) = read(dir, "Cargo.toml") {
        let kv = Regex::new(r"(?m)^([A-Za-z0-9_\-]+)\s*=").unwrap();
        deps.extend(kv.captures_iter(&t).map(|c| c[1].to_ascii_lowercase()));
    }
    deps
}

fn score(stack: &StackDef, dir: &Path, deps: &BTreeSet<String>) -> (u32, Vec<String>) {
    let mut total = 0;
    let mut ev = Vec::new();
    for r in &stack.detect.rules {
        let hit = if let Some(f) = &r.file {
            glob_exists(dir, f)
        } else if let Some(d) = &r.dependency {
            deps.contains(&d.to_ascii_lowercase().replace('_', "-")) || deps.contains(&d.to_ascii_lowercase())
        } else if let (Some(text), Some(file)) = (&r.text, &r.in_file) {
            glob_read(dir, file).map(|t| t.to_ascii_lowercase().contains(&text.to_ascii_lowercase())).unwrap_or(false)
        } else {
            false
        };
        if hit {
            total += r.weight;
            ev.push(r.evidence.clone());
        }
    }
    (total, ev)
}

fn confidence(score: u32) -> &'static str {
    if score >= 7 {
        "high"
    } else if score >= 4 {
        "medium"
    } else {
        "low"
    }
}

/// Which package manager a project uses, and the evidence.
fn detect_pm(reg: &StackRegistry, dir: &Path, candidates: &[String], default: Option<&str>) -> (Option<String>, Option<String>, Vec<String>) {
    let mut warnings = Vec::new();
    if let Some(t) = read(dir, "package.json") {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            if let Some(pmf) = v.get("packageManager").and_then(|x| x.as_str()) {
                let id = pmf.split('@').next().unwrap_or("").to_string();
                if reg.catalogue.package_managers.iter().any(|p| p.id == id) && (candidates.is_empty() || candidates.contains(&id)) {
                    return (Some(id), Some(format!("package.json packageManager = {pmf}")), warnings);
                }
            }
        }
    }
    let mut found: Vec<(String, String, std::time::SystemTime)> = Vec::new();
    for p in &reg.catalogue.package_managers {
        if !candidates.is_empty() && !candidates.contains(&p.id) {
            continue;
        }
        for f in &p.lockfiles {
            if let Ok(m) = std::fs::metadata(dir.join(f)) {
                found.push((p.id.clone(), format!("lockfile {f}"), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    found.sort_by(|a, b| b.2.cmp(&a.2));
    if let Some((id, ev, _)) = found.first() {
        let ids: BTreeSet<&String> = found.iter().map(|f| &f.0).collect();
        if ids.len() > 1 {
            warnings.push(format!(
                "more than one package manager's lockfile exists ({}); using {id}, the most recently modified. Remove the stale lockfile to settle this.",
                ids.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        return (Some(id.clone()), Some(ev.clone()), warnings);
    }
    for p in &reg.catalogue.package_managers {
        if !candidates.is_empty() && !candidates.contains(&p.id) {
            continue;
        }
        for m in &p.markers {
            if dir.join(m).exists() && !(p.id == "pip" && m == ".venv" && candidates.contains(&"uv".to_string()) && dir.join("uv.lock").exists()) {
                return (Some(p.id.clone()), Some(format!("{m} present")), warnings);
            }
        }
    }
    (default.map(|d| d.to_string()), default.map(|_| "no lockfile; using the stack's default".to_string()), warnings)
}

fn runtime_needs(dir: &Path) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = Vec::new();
    let mut push = |tool: &str, c: String, src: &str| {
        let c = c.trim().to_string();
        if !c.is_empty() {
            v.push((tool.into(), c, src.into()));
        }
    };
    if let Some(t) = read(dir, "package.json") {
        if let Ok(j) = serde_json::from_str::<Value>(&t) {
            if let Some(n) = j.pointer("/engines/node").and_then(|x| x.as_str()) {
                push("node", n.into(), "package.json engines.node");
            }
        }
    }
    for f in [".nvmrc", ".node-version"] {
        if let Some(t) = read(dir, f) {
            let c = t.trim().trim_start_matches('v').to_string();
            if c.chars().next().map(|ch| ch.is_ascii_digit()).unwrap_or(false) {
                push("node", format!("={c}"), f);
            }
        }
    }
    if let Some(t) = read(dir, "composer.json") {
        if let Ok(j) = serde_json::from_str::<Value>(&t) {
            if let Some(n) = j.pointer("/require/php").and_then(|x| x.as_str()) {
                push("php", n.into(), "composer.json require.php");
            }
        }
    }
    if let Some(t) = read(dir, "go.mod") {
        if let Some(c) = Regex::new(r"(?m)^go\s+(\d+\.\d+(?:\.\d+)?)").unwrap().captures(&t) {
            push("go", format!(">={}", &c[1]), "go.mod go directive");
        }
    }
    if let Some(t) = read(dir, "pyproject.toml") {
        if let Some(c) = Regex::new(r#"requires-python\s*=\s*["']([^"']+)["']"#).unwrap().captures(&t) {
            push("python", c[1].into(), "pyproject.toml requires-python");
        }
    }
    if let Some(t) = read(dir, ".python-version") {
        push("python", format!("={}", t.trim()), ".python-version");
    }
    if let Some(t) = read(dir, ".ruby-version") {
        push("ruby", format!("={}", t.trim().trim_start_matches("ruby-")), ".ruby-version");
    }
    if let Some(t) = read(dir, "Gemfile") {
        if let Some(c) = Regex::new(r#"(?m)^\s*ruby\s+["']([^"']+)["']"#).unwrap().captures(&t) {
            push("ruby", format!("={}", &c[1]), "Gemfile ruby");
        }
    }
    if let Some(t) = read(dir, "global.json") {
        if let Ok(j) = serde_json::from_str::<Value>(&t) {
            if let Some(n) = j.pointer("/sdk/version").and_then(|x| x.as_str()) {
                push("dotnet", format!(">={n}"), "global.json sdk.version");
            }
        }
    }
    if let Some(t) = read(dir, "Cargo.toml") {
        if let Some(c) = Regex::new(r#"rust-version\s*=\s*["']([^"']+)["']"#).unwrap().captures(&t) {
            push("rust", format!(">={}", &c[1]), "Cargo.toml rust-version");
        }
    }
    if let Some(t) = read(dir, "pom.xml") {
        if let Some(c) = Regex::new(r"<(?:java\.version|maven\.compiler\.release|maven\.compiler\.source)>(\d+)<").unwrap().captures(&t) {
            push("java", format!(">={}", &c[1]), "pom.xml java version");
        }
    }
    v
}

const QUALITY: &[(&str, &[&str])] = &[
    ("lint", &[".eslintrc", ".eslintrc.json", ".eslintrc.js", "eslint.config.js", "eslint.config.mjs", "eslint.config.ts", "biome.json", ".rubocop.yml", ".golangci.yml", "phpstan.neon", "phpcs.xml", "ruff.toml", ".flake8", "clippy.toml", ".pylintrc"]),
    ("format", &[".prettierrc", ".prettierrc.json", "prettier.config.js", "pint.json", ".editorconfig", "rustfmt.toml", ".php-cs-fixer.php"]),
    ("typecheck", &["tsconfig.json", "mypy.ini", "pyrightconfig.json", "jsconfig.json"]),
    ("test", &["phpunit.xml", "phpunit.xml.dist", "pytest.ini", "tox.ini", "jest.config.js", "jest.config.ts", "vitest.config.ts", "vitest.config.js", ".rspec", "playwright.config.ts", "cypress.config.ts", "conftest.py"]),
    ("ci", &[".github/workflows", ".gitlab-ci.yml", ".circleci"]),
    ("container", &["Dockerfile", "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml", ".devcontainer"]),
];

const ENTRY_POINTS: &[&str] = &[
    "artisan", "manage.py", "bin/rails", "bin/console", "spark", "bin/cake.php", "main.go", "cmd", "src/main.rs", "src/lib.rs", "app.py", "main.py", "wsgi.py", "index.js", "server.js", "app.js", "src/index.ts", "src/main.ts", "app/page.tsx", "src/app/page.tsx", "pages/index.tsx", "app/app.vue", "src/App.vue", "src/App.tsx", "src/pages", "Program.cs", "mvnw", "gradlew", "public/index.php",
];

/// Inspect a directory.
pub fn detect(ctx: &Ctx, reg: &StackRegistry, dir: &Path, stack_override: Option<&str>) -> Result<ProjectContext> {
    if !dir.is_dir() {
        return Err(devd_core::DevdError::not_found(format!("{} is not a directory", dir.display())));
    }
    let deps = declared_dependencies(dir);
    let mut matches: Vec<(StackMatch, bool)> = Vec::new();
    for s in &reg.stacks {
        let (sc, ev) = score(s, dir, &deps);
        if sc >= s.detect.min_score && !ev.is_empty() {
            matches.push((
                StackMatch { id: s.id.clone(), name: s.name.clone(), score: sc, confidence: confidence(sc).into(), evidence: ev },
                s.category == "language",
            ));
        }
    }
    // Frameworks outrank generic language stacks; within a class, higher score first.
    matches.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.score.cmp(&a.0.score)));
    let mut warnings = Vec::new();
    let mut stacks: Vec<StackMatch> = matches.iter().map(|m| m.0.clone()).collect();

    let mut primary = stacks.first().map(|s| s.id.clone());
    let mut ambiguous = false;
    let mut ambiguity = None;
    if let Some(o) = stack_override {
        match reg.stack(o) {
            Some(s) => {
                if !stacks.iter().any(|m| m.id == o) {
                    stacks.insert(0, StackMatch { id: s.id.clone(), name: s.name.clone(), score: 0, confidence: "user".into(), evidence: vec!["chosen explicitly".into()] });
                }
                primary = Some(o.to_string());
            }
            None => warnings.push(format!("unknown stack `{o}` ignored")),
        }
    } else if stacks.len() >= 2 {
        let frameworks: Vec<&(StackMatch, bool)> = matches.iter().filter(|m| !m.1).collect();
        if frameworks.len() >= 2 && frameworks[1].0.score * 10 >= frameworks[0].0.score * 7 {
            ambiguous = true;
            ambiguity = Some(format!(
                "{} and {} both match strongly ({} vs {} points). This may be a monorepo or a full-stack project; confirm which one to work on.",
                frameworks[0].0.name, frameworks[1].0.name, frameworks[0].0.score, frameworks[1].0.score
            ));
        }
    }
    if stacks.is_empty() {
        ambiguous = true;
        ambiguity = Some("no known stack matched; choose one explicitly or work with the project's own scripts and Makefile".into());
    } else if stacks.first().map(|s| s.confidence == "low").unwrap_or(false) && stack_override.is_none() {
        ambiguous = true;
        ambiguity = Some(format!("only weak evidence for {}", stacks[0].name));
    }
    let stack = primary.as_deref().and_then(|id| reg.stack(id));

    // Package manager.
    let ecos: Vec<String> = stack.map(|s| s.package_managers.clone()).unwrap_or_default();
    let (pm, pm_ev, pm_warn) = detect_pm(reg, dir, &ecos, ecos.first().map(|s| s.as_str()));
    warnings.extend(pm_warn);
    let pm_id = pm.clone().unwrap_or_default();

    // Layout, manifests, locks.
    let mut layout: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if e.path().is_dir() { format!("{n}/") } else { n }
                })
                .filter(|n| !matches!(n.as_str(), ".git/" | "node_modules/" | "vendor/" | "target/" | ".venv/"))
                .collect()
        })
        .unwrap_or_default();
    layout.sort();
    layout.truncate(60);
    let manifest_names = ["package.json", "composer.json", "Gemfile", "pyproject.toml", "requirements.txt", "go.mod", "Cargo.toml", "pom.xml", "build.gradle", "build.gradle.kts", "global.json", "Makefile"];
    let manifests: Vec<String> = manifest_names.iter().filter(|f| dir.join(f).exists()).map(|s| s.to_string()).collect();
    let mut lockfiles = Vec::new();
    for p in &reg.catalogue.package_managers {
        for f in &p.lockfiles {
            if dir.join(f).exists() && !lockfiles.contains(f) {
                lockfiles.push(f.clone());
            }
        }
    }
    let entry_points: Vec<String> = ENTRY_POINTS.iter().filter(|f| dir.join(f).exists()).map(|s| s.to_string()).collect();

    // Quality tooling.
    let mut quality: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (purpose, files) in QUALITY {
        let hits: Vec<String> = files.iter().filter(|f| dir.join(f).exists()).map(|s| s.to_string()).collect();
        if !hits.is_empty() {
            quality.insert(purpose.to_string(), hits);
        }
    }

    // Tools.
    let scripts: ProjectScripts = tasks::read_scripts(dir);
    let dirs = toolchain::managed_dirs(ctx);
    let mut tool_ids: Vec<String> = Vec::new();
    if let Some(s) = stack {
        if let Some(r) = &s.runtime {
            tool_ids.push(r.clone());
        }
        for t in &s.tasks {
            tool_ids.extend(t.needs.iter().cloned());
        }
    }
    if let Some(p) = reg.catalogue.package_managers.iter().find(|p| p.id == pm_id) {
        tool_ids.push(p.tool.clone());
    }
    let needs_raw = runtime_needs(dir);
    tool_ids.extend(needs_raw.iter().map(|n| n.0.clone()));
    tool_ids.push("git".into());
    tool_ids.push("yarn".into());
    let tools: BTreeMap<String, ToolStatus> = toolchain::probe(reg, &tool_ids, &dirs);

    let runtimes: Vec<RuntimeNeed> = needs_raw
        .into_iter()
        .map(|(tool, constraint, source)| {
            let installed = tools.get(&tool).filter(|t| t.installed).and_then(|t| t.version.clone());
            let satisfied = installed.as_ref().and_then(|v| constraint_ok(v, &constraint));
            RuntimeNeed { tool, constraint, source, installed, satisfied }
        })
        .collect();

    // Tasks.
    let task_list = tasks::resolve(reg, stack, dir, &pm_id, &scripts, &tools, &dirs);

    // Environment.
    let mut env = EnvInfo::default();
    let key_re = Regex::new(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=(.*)$").unwrap();
    for f in [".env.example", ".env.sample", ".env.dist", ".env.template", "env.example", "env"] {
        if let Some(t) = read(dir, f) {
            env.example_files.push(f.to_string());
            for line in t.lines() {
                if let Some(c) = key_re.captures(line) {
                    let k = c[1].to_string();
                    let empty = c[2].trim().trim_matches(['"', '\'']).is_empty();
                    if !env.keys.contains(&k) {
                        env.keys.push(k.clone());
                    }
                    if is_secret_name(&k) && !env.secret_keys.contains(&k) {
                        env.secret_keys.push(k.clone());
                    }
                    if empty && !env.unset_keys.contains(&k) {
                        env.unset_keys.push(k);
                    }
                }
            }
            break;
        }
    }
    let live = read(dir, ".env");
    env.env_file_present = live.is_some();
    let mut live_set: BTreeSet<String> = BTreeSet::new();
    let mut live_db: Option<String> = None;
    let mut live_url_schemes: Vec<String> = Vec::new();
    if let Some(t) = &live {
        for line in t.lines() {
            if let Some(c) = key_re.captures(line) {
                let val = c[2].trim().trim_matches(['"', '\'']);
                if !val.is_empty() {
                    live_set.insert(c[1].to_string());
                }
                if &c[1] == "DB_CONNECTION" {
                    live_db = Some(val.to_string());
                }
                if let Some((scheme, _)) = val.split_once("://") {
                    if ["postgres", "postgresql", "mysql", "mariadb", "redis", "mongodb"].contains(&scheme) {
                        live_url_schemes.push(scheme.to_string());
                    }
                }
            }
        }
    }
    env.unset_keys.retain(|k| !live_set.contains(k));
    if env.env_file_present {
        // Keys only needed by the example but absent from the live file.
    }

    // Services.
    let mut svc: Vec<(String, String)> = Vec::new();
    let mut add = |tool: &str, why: String| {
        if !svc.iter().any(|(t, _)| t == tool) {
            svc.push((tool.into(), why));
        }
    };
    if let Some(s) = stack {
        for sv in &s.services {
            // Only unconditional requirements apply to an existing project.
            if sv.when.is_none() {
                add(&sv.tool, sv.why.clone());
            }
        }
    }
    match live_db.as_deref() {
        Some("mysql") => add("mysql", "DB_CONNECTION=mysql in .env".into()),
        Some("mariadb") => add("mariadb", "DB_CONNECTION=mariadb in .env".into()),
        Some("pgsql") | Some("postgres") | Some("postgresql") => add("postgresql", "DB_CONNECTION=pgsql in .env".into()),
        _ => {}
    }
    for scheme in live_url_schemes {
        match scheme.as_str() {
            "postgres" | "postgresql" => add("postgresql", "a postgres:// URL is configured in .env".into()),
            "mysql" => add("mysql", "a mysql:// URL is configured in .env".into()),
            "mariadb" => add("mariadb", "a mariadb:// URL is configured in .env".into()),
            "redis" => add("redis", "a redis:// URL is configured in .env".into()),
            _ => {}
        }
    }
    for f in ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"] {
        if let Some(t) = read(dir, f) {
            let l = t.to_ascii_lowercase();
            if l.contains("image: postgres") || l.contains("image: \"postgres") {
                add("postgresql", format!("{f} defines a postgres service"));
            }
            if l.contains("image: mysql") {
                add("mysql", format!("{f} defines a mysql service"));
            }
            if l.contains("image: mariadb") {
                add("mariadb", format!("{f} defines a mariadb service"));
            }
            if l.contains("image: redis") {
                add("redis", format!("{f} defines a redis service"));
            }
            add("docker", format!("{f} is present"));
            break;
        }
    }
    let svc_ids: Vec<String> = svc.iter().map(|(t, _)| t.clone()).collect();
    let svc_tools = toolchain::probe(reg, &svc_ids, &dirs);
    let services: Vec<ServiceHint> = svc
        .into_iter()
        .map(|(tool, why)| {
            let running = reg
                .tool(&tool)
                .and_then(|t| t.service.as_ref())
                .map(|s| std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], s.port).into(), std::time::Duration::from_millis(300)).is_ok())
                .unwrap_or(false);
            ServiceHint { installed: svc_tools.get(&tool).map(|t| t.installed).unwrap_or(false), running, tool, why }
        })
        .collect();

    // Docs.
    let mut docs = Vec::new();
    for f in ["README.md", "README", "README.rst", "AGENTS.md", "CLAUDE.md", "CONTRIBUTING.md", ".github/copilot-instructions.md", ".cursorrules", "docs", "DEVELOPMENT.md", "HACKING.md"] {
        if dir.join(f).exists() {
            docs.push(f.to_string());
        }
    }

    // Git.
    let git = if dir.join(".git").exists() {
        let head = read(dir, ".git/HEAD").unwrap_or_default();
        let branch = head.trim().strip_prefix("ref: refs/heads/").map(|s| s.to_string());
        let cfg = read(dir, ".git/config").unwrap_or_default();
        serde_json::json!({ "is_repo": true, "branch": branch, "has_remote": cfg.contains("[remote "), "detached": branch.is_none() })
    } else {
        serde_json::json!({ "is_repo": false })
    };

    // Missing tools.
    let mut missing: Vec<Missing> = Vec::new();
    fn want(reg: &StackRegistry, tools: &BTreeMap<String, ToolStatus>, missing: &mut Vec<Missing>, tool: &str, why: String, min: Option<&str>) {
        let st = tools.get(tool);
        let name = reg.tool(tool).map(|t| t.name.clone()).unwrap_or_else(|| tool.into());
        match st {
            Some(s) if s.installed => {
                if let (Some(m), Some(v)) = (min, &s.version) {
                    if cmp_version(v, m) == std::cmp::Ordering::Less && !missing.iter().any(|x| x.tool == tool) {
                        missing.push(Missing { tool: tool.into(), why: why.clone(), fix: format!("{name} {v} is older than {m}; upgrade it (toolchain.install {tool})") });
                    }
                }
            }
            _ => {
                if !missing.iter().any(|x| x.tool == tool) {
                    missing.push(Missing { tool: tool.into(), why, fix: format!("toolchain.install {{ tool: \"{tool}\" }}") });
                }
            }
        }
    }
    if let Some(s) = stack {
        if let Some(r) = &s.runtime {
            want(reg, &tools, &mut missing, r, format!("{} projects run on it", s.name), s.default_version().requires.iter().find(|q| &q.tool == r).and_then(|q| q.min.as_deref()));
        }
    }
    if let Some(p) = reg.catalogue.package_managers.iter().find(|p| p.id == pm_id) {
        want(reg, &tools, &mut missing, &p.tool, format!("the project uses {}", p.name), None);
    }
    for r in &runtimes {
        if r.satisfied == Some(false) {
            missing.push(Missing {
                tool: r.tool.clone(),
                why: format!("{} asks for {} (installed: {})", r.source, r.constraint, r.installed.clone().unwrap_or_default()),
                fix: format!("install a version satisfying {} (toolchain.install {{ tool: \"{}\" }})", r.constraint, r.tool),
            });
        } else if r.installed.is_none() {
            want(reg, &tools, &mut missing, &r.tool, format!("{} asks for {}", r.source, r.constraint), None);
        }
    }
    for sv in &services {
        if !sv.installed {
            missing.push(Missing { tool: sv.tool.clone(), why: sv.why.clone(), fix: format!("toolchain.install {{ tool: \"{}\" }}", sv.tool) });
        }
    }
    if let Some(s) = stack {
        if !task_list.iter().any(|t| t.kind == "test") && s.init.steps.is_empty() {
            warnings.push("no test command was found".into());
        }
    }

    Ok(ProjectContext {
        path: dir.display().to_string(),
        name: dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        stacks,
        primary,
        ambiguous,
        ambiguity,
        package_manager: pm,
        package_manager_evidence: pm_ev,
        runtimes,
        manifests,
        lockfiles,
        entry_points,
        layout,
        tasks: task_list,
        quality,
        services,
        env,
        docs,
        git,
        conventions: stack.map(|s| s.conventions.clone()).unwrap_or_default(),
        official_docs: stack.map(|s| s.docs.clone()).unwrap_or_default(),
        missing,
        warnings,
    })
}

fn is_secret_name(k: &str) -> bool {
    let l = k.to_ascii_lowercase();
    ["password", "passwd", "secret", "token", "api_key", "apikey", "private_key", "credential", "passphrase", "_key"].iter().any(|m| l.contains(m))
}

/// Pick a path-like argument into an absolute normalised directory.
pub fn dir_of(path: &str) -> PathBuf {
    crate::authz::normalise(Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraints_are_evaluated() {
        assert_eq!(constraint_ok("22.22.0", "^20.19.0 || >=22.12.0"), Some(true));
        assert_eq!(constraint_ok("22.22.0", "^22.22.3 || ^24.15.0 || >=26.0.0"), Some(false));
        assert_eq!(constraint_ok("8.3.6", "^8.2"), Some(true));
        assert_eq!(constraint_ok("8.1.0", "^8.2"), Some(false));
        assert_eq!(constraint_ok("3.13.1", ">=3.10,<3.14"), Some(true));
        assert_eq!(constraint_ok("3.14.0", ">=3.10,<3.14"), Some(false));
        assert_eq!(constraint_ok("20.1.0", "=20"), Some(true));
        assert_eq!(constraint_ok("21.0.0", "=20"), Some(false));
        assert_eq!(constraint_ok("3.2.1", "~3.2"), Some(true));
        assert_eq!(constraint_ok("1.2.3", "latest"), None);
    }
}
