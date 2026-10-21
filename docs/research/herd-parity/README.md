# Herd → DEVD parity audit — executive findings

**Status:** investigation complete for the areas listed in
[unknowns-and-follow-ups.md](unknowns-and-follow-ups.md); no application code was
changed. Implementation starts from [gaps-and-roadmap.md](gaps-and-roadmap.md).

## What was investigated

| | |
|---|---|
| DEVD | `main` @ `6c7bd78`, workspace version **0.9.0**, 14 crates, 234 registered capabilities |
| Herd | **2.0.2**, Windows x64, `C:\Program Files\Herd` (Electron). Licence/edition: **not determined** — no licence state was read; Pro-gated commands are labelled `(Pro)` by Herd's own CLI |
| REA | Connected and used this session (`open_binary`, `inspect_electron_fuses`, `inspect_artifact`). The deep ASAR/preload analysis in [`../01-herd-investigation.md`](../01-herd-investigation.md) was produced earlier with REA 6.7.0 and is **re-used, not repeated** |
| Method | Herd: its own CLI (`herd list`, read-only), config files, REA on the installed binary, prior REA trace. DEVD: capability registry dump (`devd capabilities`), source reads, existing tests, a green `cargo test --workspace` on Windows (~465 passed, 0 failed, 5 ignored) |

## Headline findings

1. **Parity is strong where DEVD chose to compete and weak elsewhere.** Project
   selection (native folder picker), site creation, local CA/HTTPS, nginx, MariaDB,
   per-site PHP pinning, runtime install, Git, agents, MCP and an approval-gated
   capability registry are implemented and tested. See
   [devd-feature-map.md](devd-feature-map.md).
2. **The biggest gap against DEVD's own goal is platform, not features.** Every
   service/PHP provider is Windows-only (`cfg!(windows)` gates in
   `devd-runtime/src/catalogue.rs:104`, `devd-services/src/catalogue.rs:325,333`).
   Linux runs the core only; **macOS has no support at all**.
3. **Developer-feedback tooling is entirely absent:** mail capture, dump
   interception, app-log viewer, query insights, Xdebug capture, tinker. Of these,
   mail capture and a generic log viewer are cross-stack; the rest are PHP/Laravel
   specific.
4. **Project ↔ service wiring is missing.** Herd reads a site's `.env`
   (`db:connection`, `db:use`, manifest `services:`); DEVD has `db.*` for MariaDB
   only and nothing that connects a project to a service. This is the highest-value
   *universal* gap.
5. **A real safety defect, independent of Herd (fixed as M0 in the working tree).**
   The README says every interface goes through `Registry::dispatch`. Five window
   routes (session start/stop/rename/dismiss, task cancel) called subsystems
   directly — notably `session_dismiss` ran a **Destructive-tier** operation without
   approval or audit. Polling/keystrokes/resize are documented exceptions. Details
   and test status in [notes/13-approval-enforcement.md](notes/13-approval-enforcement.md).
6. **Some DEVD documents are stale in the unsafe direction** (claiming a gap that is
   closed): `../02-feature-matrix.md` still lists per-site PHP routing as *Partial*;
   it is implemented (`site.set_php`, `crates/devd-sites/src/caps.rs:127`).
   The reverse (claims not backed by code) was not found in the areas sampled.

## Cross-stack implication

DEVD supports 23 stack definitions (JS ×8, PHP ×2, Python ×4, Ruby ×2, Java ×2, Go,
Rust, .NET, React Native ×2). Only 2 are PHP. Most Herd parity items are
PHP-specific or Laravel-specific and **should not** be built as such. The reusable
core is: service provisioning, project↔service wiring, log/mail/HTTP capture,
per-site domains/proxies, and agent client registration. See
[cross-stack-analysis.md](cross-stack-analysis.md).

## Deliverables

| File | Contents |
|---|---|
| [herd-inventory.md](herd-inventory.md) | Herd 2.0.2 reference catalogue |
| [devd-feature-map.md](devd-feature-map.md) | What DEVD ships, with code locations |
| [feature-parity-matrix.md](feature-parity-matrix.md) | 69 rows, each traced to a note |
| [cross-stack-analysis.md](cross-stack-analysis.md) | Stack-by-stack applicability |
| [gaps-and-roadmap.md](gaps-and-roadmap.md) | Prioritised backlog with acceptance criteria |
| [unknowns-and-follow-ups.md](unknowns-and-follow-ups.md) | What could not be verified |
| [notes/](notes/) | 13 per-feature investigation notes |

## What was actually run

- `herd list` (Herd's CLI, read-only) — full command inventory captured.
- Read of `~/.config/herd/config/{config,php}.json` — settings and installed PHP.
- REA: opened `app.asar` (sha256 `e5562180…c0ed`); `inspect_electron_fuses` on
  `Herd.exe` (RunAsNode enabled, ASAR integrity **not enforced**); `inspect_artifact`
  again refuses with `artifact_integrity_mismatch` on
  `resources/bin/herd-devtools-helper-arm64.zip` — reproducing the earlier finding.
- `devd capabilities` (234), source greps for each Herd feature, `cargo test --workspace`
  on the GNU toolchain.
- **Not done:** no Herd service was started/stopped, no project created, no hosts or
  certificate change, no Pro feature exercised.
