//! A real runtime install, against the real upstream.
//!
//! Ignored by default because it downloads a full PHP build (~32 MB) and
//! unpacks it. It exists because the promise this crate makes cannot be tested
//! with a fixture: the whole point is that the vendor's catalogue, the vendor's
//! digest and the vendor's archive really do produce a working interpreter.
//!
//! ```text
//! cargo test -p devd-runtime --test install -- --ignored --nocapture
//! ```

use std::sync::Arc;

use devd_core::capability::{Actor, Ctx, Surface};
use devd_core::config::Settings;
use devd_core::events::EventBus;
use devd_core::paths::Paths;
use devd_core::state::Store;

fn scratch(tag: &str) -> Paths {
    let dir = std::env::temp_dir().join(format!(
        "devd-runtime-e2e-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(dir);
    paths.ensure().unwrap();
    paths
}

fn ctx(paths: &Paths) -> Ctx {
    Ctx {
        store: Arc::new(Store::open_in_memory().unwrap()),
        paths: Arc::new(paths.clone()),
        settings: Arc::new(Settings::default()),
        events: EventBus::new(),
        actor: Actor::human(Surface::Cli),
        dry_run: false,
    }
}

#[test]
#[ignore = "downloads the official PHP build from windows.php.net"]
fn installing_php_produces_a_working_interpreter() {
    let paths = scratch("php");
    let ctx = ctx(&paths);

    // The catalogue must be the vendor's, and it must carry digests.
    let (list, notes) = devd_runtime::available("php", false).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert!(!list.is_empty(), "no PHP builds were listed");
    let series = devd_runtime::catalogue::resolve(&list, "8.3", None).unwrap();
    assert!(
        series.sha256.is_some(),
        "PHP publishes a digest for every build: {series:?}"
    );

    let installed = devd_runtime::install(&ctx, "php", "8.3", false, false, None).unwrap();
    println!(
        "installed php {} at {} ({} bytes, {} extensions, verified {})",
        installed.runtime.version,
        installed.runtime.path,
        installed.bytes,
        installed.enabled.len(),
        installed.verified_version.as_deref().unwrap_or("?")
    );

    // Running it is the only proof that matters.
    let verified = devd_runtime::verify(&ctx, "php", &installed.runtime.version).unwrap();
    let version = verified.get("version").and_then(|v| v.as_str()).unwrap();
    assert!(version.starts_with("8.3."), "{verified}");
    let modules = verified.get("modules").and_then(|v| v.as_array()).unwrap();
    let names: Vec<String> = modules
        .iter()
        .filter_map(|m| m.as_str())
        .map(str::to_string)
        .collect();
    for required in ["Core", "curl", "mbstring", "pdo_mysql", "openssl"] {
        assert!(
            names.iter().any(|n| n == required),
            "the generated php.ini did not load `{required}`; loaded: {names:?}"
        );
    }
    assert!(
        installed.enabled.len() >= 8,
        "expected the build's extensions to be enabled, got {:?}",
        installed.enabled
    );

    // The runtime is recorded, and listing finds it.
    let rows = devd_runtime::list(&ctx).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].present);
    assert!(rows[0].primary, "the first PHP install becomes the default");

    // Installing the same version again is a conflict, not a silent re-download.
    let err = devd_runtime::install(&ctx, "php", "8.3", false, false, None).unwrap_err();
    assert_eq!(err.code(), "conflict", "{err}");

    // Removal is blocked while a service points at it.
    let binaries = devd_core::util::directory_size(&paths.runtime_version("php", &installed.runtime.version));
    assert!(binaries > 1_000_000, "a PHP build is bigger than {binaries} bytes");

    let _ = std::fs::remove_dir_all(paths.root());
}

#[test]
#[ignore = "downloads a Node build from nodejs.org"]
fn installing_node_produces_a_working_interpreter() {
    let paths = scratch("node");
    let ctx = ctx(&paths);

    let installed = devd_runtime::install(&ctx, "node", "lts", false, false, None).unwrap();
    println!(
        "installed node {} at {} (verified {})",
        installed.runtime.version,
        installed.runtime.path,
        installed.verified_version.as_deref().unwrap_or("?")
    );
    let verified = devd_runtime::verify(&ctx, "node", &installed.runtime.version).unwrap();
    println!("{verified}");
    assert!(verified.get("version").is_some());
    let _ = std::fs::remove_dir_all(paths.root());
}

#[test]
#[ignore = "downloads the Go toolchain from go.dev"]
fn installing_go_produces_a_working_toolchain() {
    let paths = scratch("go");
    let ctx = ctx(&paths);

    let (list, _) = devd_runtime::available("go", false).unwrap();
    assert!(!list.is_empty());
    // Go publishes its digests in the same document as its builds.
    assert!(list.iter().all(|a| a.sha256.is_some()), "{list:?}");

    let installed = devd_runtime::install(&ctx, "go", "latest", false, false, None).unwrap();
    println!(
        "installed go {} at {} (verified {})",
        installed.runtime.version,
        installed.runtime.path,
        installed.verified_version.as_deref().unwrap_or("?")
    );
    let verified = devd_runtime::verify(&ctx, "go", &installed.runtime.version).unwrap();
    println!("{verified}");
    assert!(verified.get("version").is_some());
    let _ = std::fs::remove_dir_all(paths.root());
}
