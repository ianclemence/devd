# Gaps and roadmap

Ordered by: safety → the stated product goal (Windows + Linux + macOS) → universal
cross-stack value → PHP/Laravel parity. Each milestone is independently shippable and
ends with a green `cargo test --workspace` (using `scripts/dev-env.ps1` on Windows).
Matrix IDs in brackets.

## M0 — Close the dispatcher bypass [X02] · P0 · **DONE in working tree, uncommitted**

Implemented as planned for start/stop/rename/dismiss/cancel; keystrokes, polling and
resize stay direct by the existing documented decision. `cargo test -p devd-server`
passes; `cargo check --workspace --all-targets` is clean of errors. The window now
dismisses via `run("session.dismiss")`, so the approval dialog replaces the old
`window.confirm`. Not exercised in the running window.

**Problem.** `session_input/stop/rename/resize/dismiss`, `terminal_*`, and `task_cancel`
(`crates/devd-server/src/api.rs:601-727, 782`) call `workspace.sessions()` /
`orchestrator()` directly. No tier check, no audit; `session_dismiss` maps to the
Destructive capability `session.dismiss`.

**Plan.** Route each through `registry.dispatch` with the matching capability
(`session.input`, `session.stop`, `session.rename`, `session.resize`,
`session.dismiss`, `agent.cancel`), keeping human UI flows by passing the same
`Approval` the dialog already collects. Terminal keystrokes are high-frequency: dispatch
once per message but skip the audit row for `session.input` data bodies (audit the
fact, redact the payload).

**Acceptance.** (1) `session.dismiss` over HTTP returns `approval_required` without a
token. (2) Audit rows exist for each route. (3) A test enumerates every mutating
`/api` route and fails if it does not reach `Registry::dispatch` (allow-list:
`/api/invoke`, `/api/approve`, `/api/deny`, `/api/pick-folder`, `/api/open-url`,
`/api/upload`). (4) UI contract tests pass; terminal typing latency unchanged in a
manual check.

## M1 — Platform provider layer + Linux [Z01, R01, S01, S02, V01, M04] · P0 · **code done, never run on Linux**

Implemented as *adoption* rather than download (see `../../platforms.md`): `devd-core::platform` (OS, package ownership and integrity via dpkg/rpm/pacman/Homebrew, linking), `devd-services::system` (nginx, MariaDB), `devd-runtime::system` (PHP), `setup` wired to it. Windows suite: 491 pass. Linux and macOS compile (`scripts/cross-check.ps1`). **Not executed on Linux** - no environment was available (WSL has no distribution, no Docker). Package-manager installs through `toolchain.install` were not changed.

**Design.** A `PlatformProvider` boundary in `devd-core` (catalogue, install, verify,
trust, privileged hosts write, autostart, data/config dirs) with `windows`, `linux`
and `macos` implementations. Existing `cfg!(windows)` branches in
`devd-runtime/src/catalogue.rs:104`, `devd-services/src/catalogue.rs:325,333`,
`tls.rs`, `hosts.rs` move behind it.

**Linux rules** (same verification contract as Windows: origin and version known
before install, no shell strings, install result states what could not be verified):
nginx, MariaDB and PHP installed via the distro package manager (`apt`, `dnf`,
`pacman`) using argv arrays; the exact package version is read back and recorded;
signature verification is delegated to the package manager and **reported as such**.
PHP-FPM replaces php-cgi where the distro ships it; per-version pools keep `site.set_php`
semantics. Services run as the user (high ports or `setcap`), data under `$XDG_DATA_HOME/devd`.
Privileged steps (package install) are `Admin` tier and return a plan.

**Acceptance.** On Ubuntu and Fedora containers or WSL: `devd setup --yes` installs
nginx+MariaDB+PHP, `devd site create blog` serves HTTPS locally (curl with CA),
`devd service stop/start` verifies health, `site.set_php` works with two PHP series;
`catalogue` unit tests with fake package-manager output; Windows suite unchanged.
**Blocker:** needs a Linux environment to execute (U-11) — WSL/Docker via
`scripts/release-linux.ps1`.

## M2 — macOS [Z02, S02, M04, D08] · P0 (code), verification gated · **partly done**

Done: Homebrew adoption (same code as Linux), `security add-trusted-cert`/`remove-trusted-cert`/`find-certificate` for the CA, default ports, and a bug found on the way - the Node and Go catalogues treated macOS as Linux and would have downloaded Linux builds. Not done: launchd autostart, a macOS installer, the resolver for `*.test`, and any run on a Mac.

Homebrew provider (`brew install`, `brew services` replaced by devd's own supervision),
`security add-trusted-cert` for the CA, launchd agent for autostart, `.pkg`/zip release,
`rfd` dialog and PTY checked on macOS. Code and unit tests with fake `brew` output can
land from Windows; **real verification needs a macOS runner (U-15)** and will be reported
as unverified until then.

## M3 — Upstream sites (proxy) and wildcard DNS [P11, S04, P05, C01] · P1

Generalise the site record to `upstream = fastcgi(php) | http(url) | static`. Give
Node/Python/Rails/Go dev servers `name.test` + HTTPS. On Linux/macOS add a loopback
resolver (dnsmasq/systemd-resolved/`/etc/resolver`) for `*.test`. **Acceptance:** a
Vite and a Flask dev server reachable over HTTPS by name; `site.create --proxy`; plan
shows hosts/resolver changes; no regression for PHP sites.

## M4 — Project↔service wiring [V06, V07, C01, C04] · P1

Extend `devd.yml` (do not add `herd.yml`) with `runtime`, `services`, `domain`.
New read-only `project.env` reports which service each `.env` category points at
(secrets only as "set"); `project.use_service` rewrites owned keys and remembers
originals (plan `04` §1b). `db.*` becomes driver-aware. **Acceptance:** original
values restored on unset (test); manifest validation reports one message per problem;
missing runtime/service is reported by name, not crash.

## M5 — Services and capture [V02, V03, D01, V01-tests] · P1

PostgreSQL (all OSes), Redis/Valkey (Linux/macOS via package manager; Windows only
if an official build exists — otherwise documented), MongoDB; named instances with
own port/data dir; `db.dump/restore`; more unit tests for `mariadb.rs`. Mail capture on
loopback:2525 with mailboxes, `mail.wait`, bounded retention. **Acceptance:** per
service start/health/stop/remove-keeps-data tests; SMTP integration test sends and
reads a message.

## M6 — Developer feedback and integrations [D03, I02, I04, P07, M02] · P1–P2

Log-source discovery per stack with level/stack parsing; `devd mcp install|status|
uninstall` for Claude Code, Cursor, VS Code, Codex, Gemini CLI, OpenCode, Windsurf;
default IDE + `open in editor`; update check (verify checksum, rollback). All go
through the registry (new capabilities, tiers assigned: config writes are `Write`, the
agent-config edits `Write` with a plan).

## M7 — Your TODO items

- **F1 CLI audit**: enumerate every `devd` subcommand, map to a capability, add a test
  that every capability with a typed command round-trips.
- **C1–C5 Memory**: layered memory tables in `devd-core` (migration 006), asset
  ownership/visibility, loadouts per agent, on-demand retrieval, Memory screen. Needs a
  design pass first — largest unknown scope.
- **A7, B6, E8, G2/G3** as listed in `TODO.md`.

## Deferred / rejected

Dumps/insights/Xdebug/tinker (Laravel-specific) deferred behind a PHP adapter after
M5; Expose-style tunnels deferred until access control exists; Pro licensing rejected;
hosts-file helper service deferred (prefer resolver on Unix; Windows keeps the
approved elevated sync).
