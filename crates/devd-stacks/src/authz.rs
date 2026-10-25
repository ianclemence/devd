//! Which directories a caller may create projects in and run commands in.
//!
//! Agents are confined to the projects root, the extra roots in the settings,
//! and projects a person already imported or created. A person at a keyboard
//! may use any directory that is not part of the operating system.

use std::path::{Path, PathBuf};

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};

/// Directories agents may work in without a project being registered first.
pub fn roots(ctx: &Ctx) -> Vec<PathBuf> {
    let mut out = vec![ctx.settings.effective_www_root(&ctx.paths.www())];
    out.extend(ctx.settings.project_roots.iter().cloned());
    // A file a person attached to a message is stored under the devd home. The
    // agent has to be able to read it, so the uploads folder is a workspace root
    // even though it is not a project.
    out.push(ctx.paths.root().join("uploads"));
    out.into_iter().map(|p| normalise(&p)).collect()
}

/// Normalise a path: resolve `.` and `..` lexically, then resolve symlinks in
/// the deepest ancestor that exists so a link cannot hide a location outside
/// the roots.
pub fn normalise(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut clean = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    if let Ok(c) = std::fs::canonicalize(&clean) {
        return devd_core::util::strip_verbatim(&c);
    }
    let mut missing = Vec::new();
    let mut cur = clean.clone();
    while !cur.exists() {
        match (cur.file_name().map(|f| f.to_os_string()), cur.parent().map(|x| x.to_path_buf())) {
            (Some(f), Some(parent)) => {
                missing.push(f);
                cur = parent;
            }
            _ => break,
        }
    }
    let mut base = std::fs::canonicalize(&cur).map(|c| devd_core::util::strip_verbatim(&c)).unwrap_or(cur);
    for m in missing.into_iter().rev() {
        base.push(m);
    }
    base
}

fn is_system_dir(p: &Path) -> bool {
    let s = p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        return true;
    }
    const EXACT: &[&str] = &["/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/boot", "/dev", "/proc", "/sys", "/var", "/opt", "/root", "/home", "/tmp", "/run", "/srv", "/snap"];
    if EXACT.contains(&s) {
        return true;
    }
    for pre in ["/etc/", "/usr/", "/bin/", "/sbin/", "/lib/", "/boot/", "/dev/", "/proc/", "/sys/", "/run/"] {
        if s.starts_with(pre) {
            return true;
        }
    }
    // Drive roots and Windows system folders.
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        let rest = &s[2..];
        return rest.is_empty()
            || ["/windows", "/program files", "/program files (x86)", "/programdata"]
                .iter()
                .any(|d| rest == *d || rest.starts_with(&format!("{d}/")));
    }
    if let Some(h) = std::env::var_os("HOME") {
        let home = normalise(Path::new(&h)).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if s == home.trim_end_matches('/') {
            return true;
        }
    }
    false
}

/// Check that the caller may use `path` and return its normalised form.
pub fn authorize_dir(ctx: &Ctx, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(DevdError::invalid(format!("`{}` is not an absolute path", path.display())));
    }
    let norm = normalise(path);
    if is_system_dir(&norm) {
        return Err(DevdError::denied(format!(
            "`{}` is a system or home directory; choose a project directory inside it",
            norm.display()
        )));
    }
    if ctx.actor.is_agent() {
        let in_roots = roots(ctx).iter().any(|r| norm.starts_with(r));
        let registered = ctx
            .store
            .list_stack_projects()
            .map(|ps| ps.iter().any(|p| norm.starts_with(normalise(Path::new(&p.path)))))
            .unwrap_or(false);
        if !in_roots && !registered {
            return Err(DevdError::denied(format!(
                "agents may only work under the projects root ({}) or in projects a person imported; `{}` is neither. Ask the user to import it with project.import",
                roots(ctx).first().map(|r| r.display().to_string()).unwrap_or_default(),
                norm.display()
            )));
        }
    }
    Ok(norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_directories_are_refused() {
        for p in ["/", "/etc", "/etc/nginx", "/usr/local", "/var", "/proc/1", "C:/", "C:/Windows/System32"] {
            assert!(is_system_dir(Path::new(p)), "{p}");
        }
        assert!(!is_system_dir(Path::new("/home/user/projects/app")));
        assert!(!is_system_dir(Path::new("D:/work/app")));
    }

    #[test]
    fn dotdot_cannot_climb_out() {
        let n = normalise(Path::new("/definitely/not/there/../../x"));
        assert!(n.ends_with("x") && !n.to_string_lossy().contains(".."));
    }
}
