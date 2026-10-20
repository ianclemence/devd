//! Embed the devd icon in the command line executable.
//!
//! The window already carries the artwork, because Tauri puts it in the
//! executable. The command line program is a plain console binary, and without
//! this it shows the default Windows application icon in Explorer and on the
//! taskbar - the one place a person sees the product without opening it.
//!
//! The resource is only embedded when a resource compiler is available. The
//! build must never depend on one, so a machine without it still builds; it
//! just keeps the default icon.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../devd-desktop/icons/icon.ico");
    println!("cargo:rerun-if-changed=devd.rc");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && !cfg!(target_env = "msvc")
    {
        return;
    }
    if !has_resource_compiler() {
        println!(
            "cargo:warning=no resource compiler (llvm-rc, windres or rc.exe) was found; \
             devd.exe keeps the default icon. Install llvm, MinGW or the Windows SDK to embed it."
        );
        return;
    }
    let rc = Path::new("devd.rc");
    if let Err(e) = embed_resource::compile(rc, embed_resource::NONE).manifest_optional() {
        // A missing icon is not worth failing a build over.
        println!("cargo:warning=the devd icon could not be embedded: {e}");
    }
}

/// Whether a resource compiler is on the machine.
fn has_resource_compiler() -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in ["llvm-rc.exe", "windres.exe", "rc.exe"] {
                if dir.join(name).is_file() {
                    return true;
                }
            }
        }
    }
    // The Windows SDK keeps its resource compiler out of PATH.
    sdk_resource_compiler().is_some()
}

fn sdk_resource_compiler() -> Option<PathBuf> {
    let root = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .filter(|p| p.join("Windows Kits").join("10").join("bin").is_dir())?;
    let bin = root.join("Windows Kits").join("10").join("bin");
    let versions: Vec<PathBuf> = std::fs::read_dir(&bin)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let mut versions = versions;
    versions.sort();
    versions.reverse();
    versions.into_iter().find_map(|version| {
        for arch in ["x64", "arm64", "x86"] {
            let candidate = version.join(arch).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}
