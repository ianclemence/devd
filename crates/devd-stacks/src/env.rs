//! Building the template environment for a stack, version, package manager and
//! set of option values.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

use devd_core::util;

use crate::model::{StackDef, VersionDef};
use crate::template::Env;
use crate::tools::{cmp_version, Catalogue, ToolStatus};

/// Names the engine reserves for itself; options may not use them.
pub const RESERVED: &[&str] = &["name", "version", "package_manager", "git", "install"];

/// Derived spellings of a project name.
pub fn name_forms(name: &str) -> BTreeMap<String, String> {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let mut snake = lower.join("_");
    if snake.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        snake.insert(0, '_');
    }
    let pascal: String = words
        .iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    let mut m = BTreeMap::new();
    m.insert("name_snake".into(), snake);
    m.insert("name_kebab".into(), lower.join("-"));
    m.insert("name_flat".into(), lower.join(""));
    m.insert("name_lower".into(), lower.join(""));
    m.insert("name_pascal".into(), pascal);
    m
}

fn python_exe() -> String {
    for c in ["python3", "python"] {
        if util::which(c).is_some() {
            return c.to_string();
        }
    }
    "python3".to_string()
}

fn venv_python(dir: &Path) -> String {
    if cfg!(windows) {
        dir.join(".venv").join("Scripts").join("python.exe")
    } else {
        dir.join(".venv").join("bin").join("python")
    }
    .display()
    .to_string()
}

/// Everything needed to build an [`Env`].
pub struct EnvInput<'a> {
    /// The stack.
    pub stack: &'a StackDef,
    /// The catalogue (for package manager tables).
    pub catalogue: &'a Catalogue,
    /// Selected framework version.
    pub version: &'a VersionDef,
    /// Selected package manager id (may be empty for stacks without one).
    pub pm: &'a str,
    /// Project name.
    pub name: &'a str,
    /// Project directory.
    pub dir: &'a Path,
    /// Directory that contains the project.
    pub parent: &'a Path,
    /// Normalised option values.
    pub options: &'a BTreeMap<String, Value>,
    /// Initialise git.
    pub git: bool,
    /// Install dependencies.
    pub install: bool,
    /// Planning initialisation: a virtual environment will exist by the time
    /// later steps run.
    pub init_phase: bool,
    /// Probed tools, used to adapt to the installed versions.
    pub tools: &'a BTreeMap<String, ToolStatus>,
}

/// Build the environment.
pub fn build(i: &EnvInput) -> Env {
    let mut env = Env::default();
    for (k, v) in i.options {
        env.values.insert(k.clone(), v.clone());
        if let Value::Array(a) = v {
            env.multi.insert(
                format!("opt.{k}.list"),
                a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
            );
        }
    }
    env.values.insert("name".into(), json!(i.name));
    env.values.insert("version".into(), json!(i.version.id));
    env.values.insert("package_manager".into(), json!(i.pm));
    env.values.insert("git".into(), json!(i.git));
    env.values.insert("install".into(), json!(i.install));

    env.vars.insert("name".into(), i.name.to_string());
    env.vars.insert("dir".into(), i.dir.display().to_string());
    env.vars.insert("parent".into(), i.parent.display().to_string());
    env.vars.insert("version".into(), i.version.value.clone());
    env.vars.insert("version_id".into(), i.version.id.clone());
    env.vars.insert("pm".into(), i.pm.to_string());
    for (k, v) in name_forms(i.name) {
        env.vars.insert(k, v);
    }
    env.vars.insert("python".into(), python_exe());
    let vpy = venv_python(i.dir);
    env.vars.insert("venv_python".into(), vpy.clone());

    let py: Vec<String> = match i.pm {
        "uv" => vec!["uv".into(), "run".into(), "python".into()],
        "poetry" => vec!["poetry".into(), "run".into(), "python".into()],
        _ => {
            if i.init_phase || Path::new(&vpy).exists() {
                vec![vpy]
            } else {
                vec![python_exe()]
            }
        }
    };
    env.multi.insert("py".into(), py.clone());

    if let Some(pm) = i.catalogue.package_managers.iter().find(|p| p.id == i.pm) {
        let expand = |v: &Vec<String>| -> Vec<String> {
            v.iter()
                .flat_map(|s| if s == "{py}" { py.clone() } else { vec![s.clone()] })
                .collect()
        };
        let mut dlx = expand(&pm.dlx);
        if pm.id == "yarn" {
            // `yarn dlx` exists only in Yarn 2+; Classic users run packages through npx.
            let classic = i
                .tools
                .get("yarn")
                .and_then(|t| t.version.as_deref())
                .map(|v| cmp_version(v, "2") == std::cmp::Ordering::Less)
                .unwrap_or(false);
            if classic {
                dlx = vec!["npx".into(), "--yes".into()];
            }
        }
        env.multi.insert("pm.run".into(), expand(&pm.run));
        env.multi.insert("pm.install".into(), expand(&pm.install));
        env.multi.insert("pm.add".into(), expand(&pm.add));
        env.multi.insert("pm.exec".into(), expand(&pm.exec));
        env.multi.insert("pm.dlx".into(), dlx);
    }
    env
}
