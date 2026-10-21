# Unknowns and follow-ups

| ID | Unknown | Why | Smallest next step |
|---|---|---|---|
| U-01 | Herd edition/licence | `validLicense` not read; avoided touching licensing | Read the settings window's "Herd Pro" state through the earlier DevTools method, or ask the owner |
| U-02 | Herd main-process behaviour | V8 bytecode (`.jsc`), no native engine available | Treat as black box; verify by CLI + config diff before/after a user-run action |
| U-03 | Herd Pro services, mail, insights runtime behaviour | not exercised (no licence; no state changes allowed) | Owner enables a trial, then observe `services:create` on a scratch port |
| U-04 | Herd screens re-captured | only the earlier capture exists | Re-run DevTools capture if a screen-level comparison is needed |
| U-05 | DEVD per-screen loading/empty/error/permission states | not re-audited; relies on `../03-screen-inventory.md` | Run the app, walk each ROUTE, record states |
| U-06 | Whether `site.list` shows HTTPS flag/expiry and `tls.list` expiry | read only partially | `devd site list --json`, `devd tls list` against a scratch `DEVD_HOME` |
| U-07 | Real per-stack create→run→domain flow for non-PHP stacks | needs network + toolchains | Matrix of `devd new` for one stack per language in a scratch home |
| U-08 | R05: whether project commands run with the pinned PHP/Node first on PATH | `project.exec` env construction unread | Read `devd-stacks/src/exec.rs` env block; add a test |
| U-09 | V05: port-conflict behaviour | `ports.rs` unread in depth | Unit-level read + a bind-conflict test |
| U-10 | I05: cwd-based site resolution in the CLI | `main.rs` (1624 lines) not read end to end | Read `devd-cli/src/main.rs` site commands; F1 CLI audit covers this |
| U-11 | Linux installer script behaviour | never executed on Linux (per prior doc); no Linux machine here | Run in WSL/Docker (`scripts/release-linux.ps1` documents it) |
| U-12 | Other direct-call HTTP routes beyond lines 601–727, 782 | only the handler list was skimmed | Systematic read of `api.rs` + `ui` fetch calls; add a test that enumerates routes vs registry |
| U-13 | Whether a tray/autostart exists in `devd-desktop` | grep only | Read `devd-desktop/src` |
| U-14 | Stale-doc inventory | sampled only | Run a doc-vs-registry check (capability IDs mentioned in docs that don't exist) |
| U-15 | macOS behaviour of PTY, folder dialog (`rfd`), trust store | no macOS machine | Needs a macOS runner (CI or device) — blocks verification of Z02 |
