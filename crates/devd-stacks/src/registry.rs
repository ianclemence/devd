//! Loading and validating stack definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use devd_core::{DevdError, Result};

use crate::env::{self, EnvInput, RESERVED};
use crate::model::*;
use crate::template;
use crate::tools::{Catalogue, ToolDef, ToolStatus};

macro_rules! builtin_stacks {
    ($($f:literal),* $(,)?) => {
        &[$( ($f, include_str!(concat!("../defs/stacks/", $f))) ),*]
    };
}

const BUILTIN: &[(&str, &str)] = builtin_stacks![
    "nextjs.yaml",
    "nuxt.yaml",
    "astro.yaml",
    "react-vite.yaml",
    "vue.yaml",
    "sveltekit.yaml",
    "express.yaml",
    "node.yaml",
    "laravel.yaml",
    "symfony.yaml",
    "codeigniter.yaml",
    "cakephp.yaml",
    "slim.yaml",
    "django.yaml",
    "flask.yaml",
    "fastapi.yaml",
    "python.yaml",
    "rails.yaml",
    "go.yaml",
    "rust.yaml",
    "spring-boot.yaml",
    "dotnet.yaml",
    "expo.yaml",
    "react-native.yaml",
    "generic.yaml",
    "java.yaml",
];

const TOOLS_YAML: &str = include_str!("../defs/tools.yaml");

const TASK_KINDS: &[&str] = &[
    "dev", "build", "test", "lint", "format", "typecheck", "install", "migrate", "other",
];

/// The loaded, validated set of stacks and tools.
#[derive(Debug, Clone)]
pub struct StackRegistry {
    /// Stacks, in definition order.
    pub stacks: Vec<StackDef>,
    /// Tools and package managers.
    pub catalogue: Catalogue,
    /// Extension files that were rejected, with the reason.
    pub rejected: Vec<String>,
}

static BUILTIN_REGISTRY: OnceLock<StackRegistry> = OnceLock::new();

impl StackRegistry {
    /// The built-in registry, validated once.
    pub fn builtin() -> &'static StackRegistry {
        BUILTIN_REGISTRY.get_or_init(|| {
            Self::load_builtin().unwrap_or_else(|e| panic!("built-in stack definitions are invalid: {e}"))
        })
    }

    /// Parse and validate the built-in definitions.
    pub fn load_builtin() -> Result<StackRegistry> {
        let catalogue: Catalogue = serde_yaml::from_str(TOOLS_YAML)
            .map_err(|e| DevdError::invalid(format!("tools.yaml: {e}")))?;
        let mut stacks = Vec::new();
        for (file, text) in BUILTIN {
            let s: StackDef = serde_yaml::from_str(text)
                .map_err(|e| DevdError::invalid(format!("{file}: {e}")))?;
            stacks.push(s);
        }
        let reg = StackRegistry {
            stacks,
            catalogue,
            rejected: Vec::new(),
        };
        reg.validate_catalogue()?;
        for s in &reg.stacks {
            reg.validate_stack(s)
                .map_err(|e| DevdError::invalid(format!("stack `{}`: {e}", s.id)))?;
        }
        let mut seen = BTreeSet::new();
        for s in &reg.stacks {
            if !seen.insert(&s.id) {
                return Err(DevdError::invalid(format!("duplicate stack id `{}`", s.id)));
            }
        }
        Ok(reg)
    }

    /// Built-ins plus every valid definition in `dir`. Invalid files are
    /// reported in `rejected` and never become available.
    pub fn with_extensions(dir: &Path) -> StackRegistry {
        let mut reg = Self::builtin().clone();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return reg;
        };
        let mut files: Vec<_> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yaml") | Some("yml")))
            .collect();
        files.sort();
        for path in files {
            let label = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    reg.rejected.push(format!("{label}: {e}"));
                    continue;
                }
            };
            let def: StackDef = match serde_yaml::from_str(&text) {
                Ok(d) => d,
                Err(e) => {
                    reg.rejected.push(format!("{label}: {e}"));
                    continue;
                }
            };
            if reg.stacks.iter().any(|s| s.id == def.id) {
                reg.rejected.push(format!("{label}: a stack with id `{}` already exists", def.id));
                continue;
            }
            if let Err(e) = reg.validate_stack(&def) {
                reg.rejected.push(format!("{label}: {e}"));
                continue;
            }
            reg.stacks.push(def);
        }
        reg
    }

    /// Find a stack.
    pub fn stack(&self, id: &str) -> Option<&StackDef> {
        self.stacks.iter().find(|s| s.id == id)
    }

    /// Find a tool.
    pub fn tool(&self, id: &str) -> Option<&ToolDef> {
        self.catalogue.tools.iter().find(|t| t.id == id)
    }

    fn validate_catalogue(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for t in &self.catalogue.tools {
            if !ids.insert(&t.id) {
                return Err(DevdError::invalid(format!("duplicate tool `{}`", t.id)));
            }
            if !t.per_project && t.exes.is_empty() {
                return Err(DevdError::invalid(format!("tool `{}` names no executable", t.id)));
            }
            for d in &t.depends {
                if !self.catalogue.tools.iter().any(|x| &x.id == d) {
                    return Err(DevdError::invalid(format!("tool `{}` depends on unknown `{d}`", t.id)));
                }
            }
            for l in &t.docs {
                check_url(&l.url)?;
            }
            if let Some(r) = &t.version_regex {
                Regex::new(r).map_err(|e| DevdError::invalid(format!("tool `{}` version_regex: {e}", t.id)))?;
            }
            for r in t.install.iter().chain(t.remove.iter()) {
                let ok = [
                    "devd-runtime", "apt", "dnf", "pacman", "brew", "brew-cask", "winget", "npm", "gem", "manual",
                ];
                if !ok.contains(&r.manager.as_str()) {
                    return Err(DevdError::invalid(format!("tool `{}`: unknown manager `{}`", t.id, r.manager)));
                }
                if r.manager != "manual" && r.packages.is_empty() {
                    return Err(DevdError::invalid(format!("tool `{}`: {} recipe has no packages", t.id, r.manager)));
                }
            }
            if !t.per_project && t.install.is_empty() {
                return Err(DevdError::invalid(format!("tool `{}` has no install recipe", t.id)));
            }
            if t.trust.is_empty() {
                return Err(DevdError::invalid(format!("tool `{}` states no trust model", t.id)));
            }
        }
        for p in &self.catalogue.package_managers {
            if !self.catalogue.tools.iter().any(|t| t.id == p.tool) {
                return Err(DevdError::invalid(format!("package manager `{}` uses unknown tool `{}`", p.id, p.tool)));
            }
        }
        Ok(())
    }

    fn known_tool(&self, id: &str) -> Result<()> {
        if self.tool(id).is_none() {
            return Err(DevdError::invalid(format!("unknown tool `{id}`")));
        }
        Ok(())
    }

    /// Validate one stack definition against the catalogue.
    pub fn validate_stack(&self, s: &StackDef) -> Result<()> {
        let bad = |m: String| Err(DevdError::invalid(m));
        if !Regex::new(r"^[a-z][a-z0-9-]*$").unwrap().is_match(&s.id) {
            return bad(format!("id `{}` must be lowercase letters, digits and hyphens", s.id));
        }
        if s.docs.is_empty() {
            return bad("lists no official documentation".into());
        }
        for d in &s.docs {
            check_url(&d.url)?;
        }
        if s.versions.is_empty() {
            return bad("defines no versions".into());
        }
        if s.versions.iter().filter(|v| v.default).count() != 1 {
            return bad("must mark exactly one version as default".into());
        }
        let mut vids = BTreeSet::new();
        for v in &s.versions {
            if !vids.insert(&v.id) {
                return bad(format!("duplicate version `{}`", v.id));
            }
            for r in &v.requires {
                self.known_tool(&r.tool)?;
            }
        }
        if let Some(r) = &s.runtime {
            self.known_tool(r)?;
        }
        for pm in &s.package_managers {
            if !self.catalogue.package_managers.iter().any(|p| &p.id == pm) {
                return bad(format!("unknown package manager `{pm}`"));
            }
        }
        Regex::new(&s.name_pattern).map_err(|e| DevdError::invalid(format!("name_pattern: {e}")))?;

        let mut oids: BTreeMap<&str, &OptionDef> = BTreeMap::new();
        for o in &s.options {
            if RESERVED.contains(&o.id.as_str()) {
                return bad(format!("option id `{}` is reserved", o.id));
            }
            if oids.insert(&o.id, o).is_some() {
                return bad(format!("duplicate option `{}`", o.id));
            }
            if matches!(o.kind, OptionKind::Select | OptionKind::Multi) && o.choices.is_empty() {
                return bad(format!("option `{}` has no choices", o.id));
            }
            if let Some(p) = &o.pattern {
                Regex::new(p).map_err(|e| DevdError::invalid(format!("option `{}` pattern: {e}", o.id)))?;
            }
            for v in &o.versions {
                if !vids.contains(v) {
                    return bad(format!("option `{}` names unknown version `{v}`", o.id));
                }
            }
            if o.kind == OptionKind::Select {
                let d = o.default.as_str().unwrap_or_default();
                if !o.choices.iter().any(|c| c.value == d) {
                    return bad(format!("option `{}` default `{d}` is not one of its choices", o.id));
                }
            }
        }

        let cond_ok = |c: &Cond| -> Result<()> {
            if RESERVED.contains(&c.option.as_str()) {
                return Ok(());
            }
            let Some(o) = oids.get(c.option.as_str()) else {
                return Err(DevdError::invalid(format!("condition refers to unknown option `{}`", c.option)));
            };
            if matches!(o.kind, OptionKind::Select) {
                let vals: Vec<&Value> = c.equals.iter().chain(c.in_.iter().flatten()).collect();
                for v in vals {
                    let t = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string());
                    if !o.choices.iter().any(|ch| ch.value == t) {
                        return Err(DevdError::invalid(format!(
                            "condition on `{}` names value `{t}` which is not a choice",
                            c.option
                        )));
                    }
                }
            }
            Ok(())
        };
        let when_ok = |w: &Option<When>| -> Result<()> {
            if let Some(w) = w {
                for c in w.conds() {
                    cond_ok(c)?;
                }
            }
            Ok(())
        };
        fn walk(items: &[ArgItem], f: &dyn Fn(&Cond) -> Result<()>) -> Result<()> {
            for i in items {
                if let ArgItem::If { when, args, otherwise } = i {
                    for c in when.conds() {
                        f(c)?;
                    }
                    walk(args, f)?;
                    walk(otherwise, f)?;
                }
            }
            Ok(())
        }

        let mut sids = BTreeSet::new();
        for st in &s.init.steps {
            if !sids.insert(&st.id) {
                return bad(format!("duplicate step `{}`", st.id));
            }
            when_ok(&st.when)?;
            for n in &st.needs {
                self.known_tool(n)?;
            }
            match st.kind {
                StepKind::Exec => {
                    if st.run.is_empty() {
                        return bad(format!("step `{}` has no command", st.id));
                    }
                    walk(&st.run, &cond_ok)?;
                }
                StepKind::Write => {
                    if st.path.is_none() || st.content.is_none() {
                        return bad(format!("write step `{}` needs path and content", st.id));
                    }
                }
                StepKind::Fetch => {
                    let url = st.path.clone().unwrap_or_default();
                    if !url.starts_with("https://start.spring.io/") {
                        return bad(format!("fetch step `{}` may only download from an allow-listed host", st.id));
                    }
                }
                StepKind::Env | StepKind::Json => {
                    if st.path.is_none() || st.set.is_empty() {
                        return bad(format!("step `{}` needs path and set", st.id));
                    }
                }
                StepKind::Mkdir => {}
            }
            if st.extensible && st.kind != StepKind::Exec {
                return bad(format!("step `{}`: only exec steps take extra arguments", st.id));
            }
        }
        if s.init.steps.iter().filter(|x| x.extensible).count() > 1 {
            return bad("at most one step may take extra arguments".into());
        }

        let mut tids = BTreeSet::new();
        for t in &s.tasks {
            if !tids.insert(&t.id) {
                return bad(format!("duplicate task `{}`", t.id));
            }
            if !TASK_KINDS.contains(&t.kind.as_str()) {
                return bad(format!("task `{}` has unknown kind `{}`", t.id, t.kind));
            }
            if t.run.is_empty() {
                return bad(format!("task `{}` has no command", t.id));
            }
            for n in &t.needs {
                self.known_tool(n)?;
            }
            walk(&t.run, &cond_ok)?;
        }
        for sv in &s.services {
            self.known_tool(&sv.tool)?;
            when_ok(&sv.when)?;
        }
        for r in &s.detect.rules {
            if r.file.is_none() && r.dependency.is_none() && r.text.is_none() {
                return bad(format!("detect rule `{}` tests nothing", r.evidence));
            }
            if r.text.is_some() != r.in_file.is_some() {
                return bad(format!("detect rule `{}` needs both text and in_file", r.evidence));
            }
        }

        // Expand every command with default options for every package manager,
        // so an unknown placeholder is caught when definitions load rather
        // than when a user clicks Create.
        let pms: Vec<String> = if s.package_managers.is_empty() {
            vec![String::new()]
        } else {
            s.package_managers.clone()
        };
        let tools: BTreeMap<String, ToolStatus> = BTreeMap::new();
        for pm in &pms {
            for v in &s.versions {
                let name = "sample-app";
                let dir = Path::new("/tmp/sample-app");
                let mut options = BTreeMap::new();
                for o in &s.options {
                    options.insert(o.id.clone(), o.default.clone());
                }
                let input = EnvInput {
                    stack: s,
                    catalogue: &self.catalogue,
                    version: v,
                    pm,
                    name,
                    dir,
                    parent: Path::new("/tmp"),
                    options: &options,
                    git: true,
                    install: true,
                    init_phase: true,
                    tools: &tools,
                };
                let mut e = env::build(&input);
                let forms = env::name_forms(name);
                for (k, val) in forms {
                    e.vars.insert(k, val);
                }
                for st in &s.init.steps {
                    match st.kind {
                        StepKind::Exec => {
                            template::expand(&st.run, &e)?;
                        }
                        StepKind::Write | StepKind::Fetch | StepKind::Env | StepKind::Json => {
                            if let Some(p) = &st.path {
                                template::expand_text(p, &e)?;
                            }
                            if let Some(c) = &st.content {
                                template::expand_text(c, &e)?;
                            }
                            for val in st.set.values() {
                                template::expand_text(val, &e)?;
                            }
                        }
                        StepKind::Mkdir => {}
                    }
                }
                for t in &s.tasks {
                    template::expand(&t.run, &e)?;
                }
                for f in &s.verify.files {
                    template::expand_text(f, &e)?;
                }
                for c in &s.verify.commands {
                    template::expand(&c.run, &e)?;
                }
                for o in &s.options {
                    if let Some(d) = o.default.as_str() {
                        template::expand_text(d, &e)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn check_url(u: &str) -> Result<()> {
    if !u.starts_with("https://") {
        return Err(DevdError::invalid(format!("documentation link `{u}` must be https")));
    }
    Ok(())
}
