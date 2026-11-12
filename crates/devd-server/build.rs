//! Compiles the brand marks into the binary.
//!
//! The marks were read from the source tree at run time, by the absolute path of
//! the machine that built devd. On that machine they appeared; on every other
//! one the folder did not exist, and every provider and tool was drawn without
//! its mark. This writes a table of `(file name, bytes)` that the server
//! includes, so a mark is part of the program wherever the program runs.

use std::fmt::Write as _;
use std::{env, fs, path::PathBuf};

fn main() {
    let dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("ui").join("brands");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|entries| entries.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    names.retain(|n| n.ends_with(".svg") || n.ends_with(".png"));
    names.sort();
    let mut table = String::from("/// Every brand mark, by file name, compiled in.\npub static BRANDS: &[(&str, &[u8])] = &[\n");
    for name in &names {
        let path = dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let _ = writeln!(table, "    ({name:?}, include_bytes!({:?})),", path.display().to_string());
    }
    table.push_str("];\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("brands.rs");
    fs::write(out, table).expect("the brand table can be written");
}
