# 12 — Setup, update, platforms (M01–M06, Z01–Z04)

**Evidence.** `devd setup` (`crates/devd-cli/src/setup.rs`) installs PHP, nginx and MariaDB on Windows and explains what is missing on Linux. There is no updater (`docs/limitations.md`). No tray or autostart was found by grep (U-13). Installers: NSIS (Windows), `scripts/install-linux.sh` (never executed on Linux), none for macOS. macOS appears only in `crates/devd-stacks/src/tools.rs:366-406` and as brew recipes in `tools.yaml`.

**Build environment (this session).** The default `cargo` on this machine is the MSVC toolchain, but the project requires `x86_64-pc-windows-gnu` with `rust-lld` (`.cargo/config.toml`). Mixing them makes Git's `link.exe` get picked up, and stale artifacts cause E0461. `scripts/dev-env.ps1` fixes PATH and target dir; a `cargo check --workspace` through it succeeded.

**Doc drift fixed this session:** README installer filename, `limitations.md` header version, `releasing.md` examples, `tauri.conf.json` version. Still stale: `../02-feature-matrix.md` row R03.

**Tests to add.** `install-linux.sh` in a container (install, uninstall, purge); updater verifies the checksum and rolls back on a failed start.
