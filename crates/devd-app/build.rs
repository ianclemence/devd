//! Supplies the one Windows import library Rust's bundled MinGW does not ship.
//!
//! gpui links `ktmw32` (the kernel transaction manager, pulled in by its file
//! watching). On `x86_64-pc-windows-gnu` with `rust-lld`, the linker looks only
//! in Rust's self-contained MinGW directory, which has no `libktmw32.a`, so the
//! final link fails with "unable to find library -lktmw32".
//!
//! Adding the whole of an installed MinGW's library directory to the search path
//! fixes that and breaks something else: build scripts then mix two C runtimes
//! and overflow their stack. So this copies the single file into `OUT_DIR` and
//! adds only that directory. On MSVC, Linux and macOS it does nothing.

use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=DEVD_KTMW32_LIB");
    let windows_gnu = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    if !windows_gnu {
        return;
    }
    let from_env = env::var_os("DEVD_KTMW32_LIB").map(PathBuf::from);
    let from_gcc = || {
        let out = Command::new("gcc").arg("-print-file-name=libktmw32.a").output().ok()?;
        Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
    };
    let Some(lib) = from_env.or_else(from_gcc).filter(|p| p.is_file()) else {
        println!(
            "cargo:warning=libktmw32.a was not found; install MinGW-w64 or set DEVD_KTMW32_LIB to its path, or the link will fail"
        );
        return;
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    if fs::copy(&lib, out.join("libktmw32.a")).is_ok() {
        println!("cargo:rustc-link-search=native={}", out.display());
    }
}
