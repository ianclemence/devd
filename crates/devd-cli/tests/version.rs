//! Release metadata that no compiler checks by itself.

#[test]
fn the_desktop_configuration_carries_the_workspace_version() {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../devd-desktop/tauri.conf.json"))
        .expect("the desktop configuration is readable");
    let expected = format!("\"version\": \"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(
        raw.contains(&expected),
        "tauri.conf.json must carry the workspace version ({}); the window's file version comes from it",
        env!("CARGO_PKG_VERSION")
    );
}
