# DEVD feature map (v0.9.0, commit 6c7bd78)

Source of truth: `devd capabilities` (234 registered), the crates, and
`crates/devd-server/ui/app.js` `ROUTES` (line 5342). Test status: `cargo test
--workspace` on `x86_64-pc-windows-gnu` — all suites passed (~465 tests, 5 ignored),
run this session.

## Architecture

One registry (`devd-core/src/registry.rs`), tiers `read < write < exec <
destructive < remote < admin`, approval enforced in `Registry::dispatch`. CLI
(`devd-cli`), TUI (`devd-tui`), window (`devd-desktop` + `devd-server` UI) and MCP
(`devd-mcp`) are projections. **Caveat:** several window routes bypass dispatch —
see [notes/13](notes/13-approval-enforcement.md).

## Capabilities by group (234)

| Group | IDs (count) | Crate |
|---|---|---|
| Git | `git.*` (64) | `devd-git` |
| Projects & stacks | `project.*` (17), `stacks.*` (8), `toolchain.*` (10) | `devd-workspace`, `devd-stacks` |
| Runtimes | `runtime.available/install/list/remove/use/verify` | `devd-runtime` |
| Services/web | `service.*` (9), `services.decide/inventory`, `web.*` (3), `stack.up/down`, `db.*` (3) | `devd-services` |
| Sites/TLS/hosts | `site.create/list/remove/set_php`, `tls.*`, `hosts.show/sync` | `devd-sites`, `devd-services` |
| Agents | `chat.*`, `agent.*`, `team.*` (14), `session.*` (10), `approval.*`, `inbox.*`, `models.*`, `provider.*`, `skills.*` | `devd-agent`, `devd-workspace` |
| Shared state | `scratch.*`, `todo.*`, `task.*` | `devd-core` |
| Platform | `fs.*` (8), `search.query`, `docs.fetch`, `dictation.*`, `environment.*`, `health.status`, `setup.requirements` | `devd-core`, `devd-agent` |

## Screens (window) — `ROUTES`

| Route | Screen | Backed by |
|---|---|---|
| `/` | Home (health, quick actions, live items) | `health.status`, `session.list`, `service.list` |
| `/inbox` | Inbox / approvals | `inbox.*`, `approval.*` |
| `/assistant` | Built-in agent chat | `chat.*` |
| `/team` | Team runs and roster (B6 in TODO: roster screen still open) | `team.*` |
| `/projects`, `/create`, `/develop` | Project list, create from stack, detect/import + tasks | `project.*`, `stacks.*`, `fs.browse` |
| `/agents`, `/skills`, `/shared`, `/processes` | Sessions (PTY), skills, scratch/todos, `devd.yml` processes | `session.*`, `skills.*`, `scratch/todo`, `project.*` |
| `/server`, `/server/services|sites|tools|runtimes|preview|certificates|hosts` | Server overview and tabs | `service.*`, `site.*`, `toolchain.*`, `runtime.*`, `tls.*`, `hosts.*` |
| `/models`, `/activity`, `/diagnostics`, `/api-tools` | Providers, audit, doctor, capability browser | `provider.*`, audit, `health`, registry |

Also: command palette (search), folder picker (native via `POST /api/pick-folder`
in the desktop shell; web fallback via `fs.browse`), approval dialog, detail drawer.
Per-screen loading/empty/error states were not re-audited this session (see
unknowns); `../03-screen-inventory.md` is the existing record.

## Implemented and tested (spot-verified)

| Area | Evidence |
|---|---|
| Folder picker | `devd-server/src/picker.rs`, `devd-core/src/browse.rs`, `ui_contract.rs::the_folder_picker_is_the_normal_way_to_choose_a_project` |
| Per-site PHP | `devd-sites/src/caps.rs:127` `site.set_php`; limitations.md records it done |
| nginx config render+test | `devd-services/src/nginx.rs` (5 tests) |
| TLS CA | `devd-services/src/tls.rs` |
| Hosts block | `devd-sites/src/hosts.rs` (7 tests), `hosts.sync` is Admin |
| Real health | port answers **and** pid image matches (`devd-services/src/lib.rs:197-219`, per prior doc) |
| Stack detection | `devd-stacks/src/detect.rs`, `tests/engine.rs` |
| Team evidence gating | `devd-agent/tests/agent.rs` |
| Git | `devd-git/tests/lifecycle.rs`, `policy.rs` |

## Platform matrix (as coded)

| Capability | Windows | Linux | macOS |
|---|---|---|---|
| Core, Git, CLI, TUI, window, MCP | yes | yes | not targeted: the only macOS awareness is the OS string in `devd-stacks/src/tools.rs:366-406` |
| Node, Go install | yes | yes (tarballs) | no |
| PHP install | yes (windows.php.net) | **no** (empty catalogue) | no |
| nginx / MariaDB install | yes | **no** (`catalogue.rs:325,333`) | no |
| TLS trust | `certutil` | `trust anchor` / manual | **no** |
| Hosts sync | yes (elevation) | untested here | no |
| Installer | NSIS per-user | `scripts/install-linux.sh` (not executed on Linux per prior doc) | none |
| Tool recipes (`tools.yaml`) | winget | apt/dnf/pacman | `brew` recipes **exist** for most tools |
