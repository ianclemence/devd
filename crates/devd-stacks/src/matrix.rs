//! The compatibility matrix, generated from the registry.

use serde_json::{json, Value};

use crate::registry::StackRegistry;

/// The matrix as JSON.
pub fn json(reg: &StackRegistry) -> Value {
    let stacks: Vec<Value> = reg
        .stacks
        .iter()
        .map(|s| {
            json!({
                "id": s.id, "name": s.name, "language": s.language, "category": s.category,
                "creatable": !s.init.steps.is_empty(),
                "package_managers": s.package_managers,
                "verified": s.verified,
                "versions": s.versions.iter().map(|v| json!({
                    "id": v.id, "label": v.label, "default": v.default,
                    "requires": v.requires.iter().map(|r| json!({"tool": r.tool, "min": r.min, "max": r.max})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "services": s.services.iter().map(|x| x.tool.clone()).collect::<Vec<_>>(),
                "docs": s.docs.iter().map(|d| d.url.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let tools: Vec<Value> = reg
        .catalogue
        .tools
        .iter()
        .map(|t| {
            json!({
                "id": t.id, "name": t.name, "category": t.category,
                "supported_min": t.supported_min,
                "install_with": t.install.iter().map(|r| r.manager.clone()).collect::<Vec<_>>(),
                "service_port": t.service.as_ref().map(|s| s.port),
            })
        })
        .collect();
    json!({ "stacks": stacks, "tools": tools })
}

/// The matrix as a Markdown document (docs/stacks.md).
pub fn markdown(reg: &StackRegistry) -> String {
    let mut o = String::new();
    o.push_str("# Supported stacks and compatibility matrix\n\n");
    o.push_str("<!-- Generated from crates/devd-stacks/defs by `devd stacks matrix --markdown`. A test fails if this file is stale. Do not edit by hand. -->\n\n");
    o.push_str("Every stack's commands and flags come from its official documentation or CLI help. **Verified** says which: `cli-help` means the flags were read from the tool's own `--help` output at the version shown; `docs` means they follow the official documentation (the tool was not available to run).\n\n");
    o.push_str("## Stacks\n\n| Stack | Language | Versions (tool requirements) | Package managers | Verified | Official docs |\n|---|---|---|---|---|---|\n");
    for s in &reg.stacks {
        let versions = s
            .versions
            .iter()
            .map(|v| {
                let req = v
                    .requires
                    .iter()
                    .map(|r| match (&r.min, &r.max) {
                        (Some(a), Some(b)) if a == b => format!("{} {a}.x", r.tool),
                        (Some(a), Some(b)) => format!("{} {a}-{b}", r.tool),
                        (Some(a), None) => format!("{} ≥ {a}", r.tool),
                        (None, Some(b)) => format!("{} ≤ {b}", r.tool),
                        _ => r.tool.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}{}{}", v.label, if v.default { " (default)" } else { "" }, if req.is_empty() { String::new() } else { format!(" — {req}") })
            })
            .collect::<Vec<_>>()
            .join("<br>");
        o.push_str(&format!(
            "| {}{} | {} | {} | {} | {}: {} ({}) | {} |\n",
            s.name,
            if s.init.steps.is_empty() { " *(detect and run only)*" } else { "" },
            s.language,
            versions,
            if s.package_managers.is_empty() { "—".to_string() } else { s.package_managers.join(", ") },
            s.verified.method,
            s.verified.against,
            s.verified.date,
            s.docs.first().map(|d| format!("[{}]({})", d.title, d.url)).unwrap_or_default()
        ));
    }
    o.push_str("\n## Tools and services\n\n| Tool | Category | Supported from | Install with | Service port |\n|---|---|---|---|---|\n");
    for t in &reg.catalogue.tools {
        o.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            t.name,
            t.category,
            t.supported_min.clone().unwrap_or_else(|| "—".into()),
            if t.per_project { "per project".to_string() } else { t.install.iter().map(|r| r.manager.clone()).collect::<Vec<_>>().join(", ") },
            t.service.as_ref().map(|s| s.port.to_string()).unwrap_or_else(|| "—".into())
        ));
    }
    o.push_str("\n## Operating systems\n\nThe engine runs on Windows 10/11 x64 and Linux x64. Install recipes exist per package manager (`apt`, `dnf`, `pacman`, `brew`, `winget`, `npm`, `gem`) and devd uses the ones it finds; where none applies the plan says so and gives the manual steps instead of pretending.\n");
    o
}
