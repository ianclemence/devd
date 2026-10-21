# Feature-parity matrix

**Herd context for every row** (not repeated): Herd 2.0.2, Windows x64, edition
unknown. Herd evidence is `[CLI]` (`herd list`), `[CFG]` (config files), `[REA]`,
`[PRIOR]` (`../01-herd-investigation.md`) — see [herd-inventory.md](herd-inventory.md).
DEVD paths are relative to `D:\devd`; "cap" = capability ID.

**Status codes** (from the brief): **IV** implemented & verified · **PI** partial ·
**BO** backend-only · **UI** UI-only/disconnected · **DO** documentation-only ·
**NI** not implemented · **PL** platform/stack limitation · **ID** intentional
divergence · **UN** unknown. **Rec:** adopt / adapt / defer / reject / investigate.
**Pri:** P0 (safety/blocks goal) … P3. **Conf:** H/M/L.
**Applic.:** U universal · L language · F framework · S service · LV Laravel · P platform.

| ID | Area | Herd behaviour | Herd ev. | DEVD | DEVD implementation | Applic. | Platform | Gap | Verification | Conf | Rec | Pri | Note |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| P01 | Project | Native folder dialog to add existing app | PRIOR §6 | **IV** | `devd-server/src/picker.rs`; `fs.browse` (`devd-core/src/browse.rs`); `ui_contract.rs` | U | Win/Linux desktop; web fallback | macOS dialog untested | contract test exists; add macOS run | H | keep | — | 01 |
| P02 | Project | `which`/`site-information` detect driver+framework | CLI | **IV** | `project.detect`; `devd-stacks/src/detect.rs` (828 lines); `tests/engine.rs` | U | all | none found | tests | H | keep | — | 01 |
| P03 | Sites | `link` dir as `name.test`, `--secure`, `--isolate` | CLI | **IV** | `site.create` (`devd-sites/src/caps.rs`) | U | Win; Linux needs nginx (PL) | Linux/macOS unusable end to end | 12 unit tests | M | adapt | P0 | 02 |
| P04 | Sites | `unlink` | CLI | **IV** | `site.remove` (Destructive, approval) | U | Win | — | tests lib.rs | M | keep | — | 02 |
| P05 | Sites | `park`/`forget`: serve every subdir | CLI | **NI** | — | U | — | no "watched directory" concept | none | H | adapt | P2 | 02 |
| P06 | Sites | list sites with URL, path, HTTPS, PHP, Node ver | CLI | **PI** | `site.list`, `project.list` | U | — | Node version per site absent; HTTPS flag unverified | UN | M | adapt | P2 | 02 |
| P07 | Sites | `open` in browser; `edit` in configured IDE | CLI, CFG `defaultIde` | **PI** | `POST /api/open-url` (`api.rs:949`); no IDE launcher found by grep | U | — | no default-IDE setting or open-in-editor | grep | M | adopt | P2 | 11 |
| P08 | Sites | `tld` change | CLI | **PI** | `tld` in `settings.toml` (`devd-core/src/config.rs`) | U | — | no capability to change + re-issue certs/hosts atomically | read config only | M | adapt | P3 | 02 |
| P09 | Sites | `loopback` address | CLI | **NI** | — | U | — | none | grep | H | defer | P3 | 02 |
| P10 | Sites | `directory-listing` toggle | CLI | **NI** | — | U | — | none | grep `autoindex` | H | defer | P3 | 02 |
| P11 | Sites | `proxy`/`unproxy` to upstream (Reverb, Docker) | CLI | **NI** | no `proxy_pass` in repo | U | — | non-PHP stacks (Node/Rails/Django) need this to get `.test` + HTTPS | grep | H | adopt | **P1** | 02 |
| P12 | HTTPS | `secure`/`unsecure`/`secured` per site with expiry | CLI | **PI** | `tls.issue`, `tls.list`; TLS on by default at `site.create` | U | — | no per-site unsecure; expiry shown? UN | partial | M | adapt | P2 | 04 |
| P13 | Workspaces | isolated worktree served on own domain+cert; merge back | CLI | **PI/BO** | 4 `git.worktree.*` caps; team worktrees `devd-workspace/src/orchestrate.rs` | U | — | no domain/cert per worktree, no merge-back flow tied to a site | read | M | adapt | P2 | 08 |
| P14 | Projects UI | favourites/groups, sort, grid/list | CFG, PRIOR | **IV** (pin, 3 sorts) | Projects screen (`app.js`) | U | — | grid/list toggle not confirmed | prior matrix | M | keep | P3 | 01 |
| R01 | Runtime | `php:install/list/update` | CLI, CFG | **PL** | `runtime.*`; `catalogue.rs:104` empty off-Windows | L(PHP) | **Win only** | Linux/macOS install; no patch-update flow | tests network-dependent | H | adopt | **P0** | 03 |
| R02 | Runtime | `use` global PHP | CLI | **IV** | `runtime.use` | L | Win | — | lib.rs | H | keep | — | 03 |
| R03 | Runtime | `isolate`/`unisolate` per-site PHP | CLI | **IV** | `site.set_php` (`caps.rs:127`), per-version FastCGI listeners | L(PHP) | Win | `02-feature-matrix.md` row is stale | read code, tests not re-run for this | M | keep; fix doc | P3 | 03 |
| R04 | Runtime | Node via nvm, per-site Node | CLI | **ID/PI** | `runtime.install node@lts` | L(JS) | Win/Linux | no multiple-Node-per-project selection | read | M | adapt | P1 | 03 |
| R05 | Runtime | `php`/`composer` shims honour site's PHP | CLI | **UN** | `project.exec` runs framework commands; env resolution unread | L(PHP) | — | whether the project's pinned runtime is on PATH is unverified | none | L | investigate | P2 | 03 |
| R06 | Runtime | `ini`, debug.ini | CLI | **NI** | — | L(PHP) | — | — | grep | H | defer | P3 | 03 |
| R07 | Runtime | bundled PHP extensions (redis, mongodb, xdebug) | PRIOR §4.3 | **PI** | `toolchain.*` can install | L(PHP) | — | not bundled; no per-version ext manager | read | M | defer | P3 | 03 |
| R08 | Runtime | (Herd: PHP+Node only) Go, Python, Ruby, Java, .NET, Rust | — | **DEVD-originated** | `tools.yaml` (36 tools), 23 stacks | U | per-OS recipes | detect/run only for several | `engine.rs` | H | keep | — | 03 |
| S01 | Server | vendored nginx, generated config | PRIOR | **IV** | `devd-services/src/nginx.rs`; `web.*` | U | Win only | Linux/macOS provider | 5 tests | H | adopt | **P0** | 05 |
| S02 | HTTPS | local CA + trust store | PRIOR | **IV** | `devd-services/src/tls.rs`; `tls.trust` Admin | U | Win certutil; Linux `trust` | macOS `security add-trusted-cert` missing | 7 cfg gates | H | adopt | **P0** | 04 |
| S03 | DNS | hosts managed by privileged service (no UAC each time) | PRIOR | **ID** | `hosts.sync` Admin, approval, snapshot | U | Win elevation per sync | friction; no helper | tests hosts.rs | H | adapt | P2 | 02 |
| S04 | DNS | dnsmasq wildcard (Unix) | PRIOR | **NI** | — | U | Linux/macOS | no wildcard, no resolver | limitations.md | H | adopt | P1 | 02 |
| S05 | HTTPS | HTTP/2 | — | **NI** | limitations.md | U | — | not enabled | doc | H | defer | P3 | 02 |
| S06 | Lifecycle | start/stop/restart all or one | CLI | **IV** | `stack.up/down`, `service.*` | U | Win | — | tests | H | keep | — | 05 |
| S07 | Health | `status` one-screen | CLI | **IV** | `health.status`, Home, Diagnostics | U | — | — | tests | M | keep | — | 05 |
| V01 | Services | MySQL/MariaDB instances | CLI(Pro) | **IV (MariaDB)** | `devd-services/src/mariadb.rs` (483 lines, 6 in-file unit tests; the older `02-feature-matrix.md` saying "none" is stale) | S | Win | MySQL not managed; coverage of init/root-password paths unchecked | 6 `#[test]` (this session) | H | keep; extend tests | P1 | 05 |
| V02 | Services | PostgreSQL, MongoDB, Redis, Valkey, Typesense | CLI(Pro), PRIOR | **NI (managed)** / **PI** via `toolchain.*` (system package manager) | `tools.yaml` | S | Win: no official Redis | no devd-managed instance | none | H | adopt | **P1** | 05 |
| V03 | Services | multiple instances, clone, own port | CLI(Pro) | **NI** | one version per service name | S | — | single instance per name | limitations.md | H | adapt | P2 | 05 |
| V04 | Services | delete removes data | CLI(Pro) | **ID (safer)** | `service.remove` keeps data dir | S | — | no dump/backup | doc | H | keep; add `db.dump` | P2 | 05 |
| V05 | Services | port assignment/conflicts | — | **UN** | `devd-core/src/ports.rs` | S | — | behaviour on conflict not exercised | none | L | investigate | P2 | 06 |
| V06 | Wiring | project's `.env` ↔ service (db:connection, manifest services, `${ENV}` ports) | CLI, PRIOR §4.2 | **NI** | — (planned as "1b" in `04-implementation-plan.md`) | U/S | — | nothing links a project to a service | grep | H | adopt | **P1** | 06 |
| V07 | Wiring | `db:create/drop/list/use` against the site's own server | CLI | **PI** | `db.*` MariaDB only, not `.env`-driven | S | Win | MySQL/Postgres/SQLite | read | M | adapt | P1 | 06 |
| V08 | Wiring | open DB in client (TablePlus…) | CLI | **NI** | — | S | — | — | grep | H | defer | P3 | 06 |
| V09 | Services | adopt/observe existing installs | — | **DEVD-originated IV** | `services.inventory`, `services.decide` | S | — | — | `engine.rs:321,360` | H | keep | — | 05 |
| C01 | Config | `herd.yml` project identity manifest (`init`) | CLI, PRIOR | **NI** | `devd.yml` is processes only | U | — | no declarative runtime+services+domain manifest | docs | H | adapt (extend `devd.yml`, don't add a second file) | P1 | 07 |
| C02 | Config | manifest validated, one message per problem | PRIOR | **IV** for `devd.yml` | `devd-workspace/src/manifest.rs` (744 lines) | U | — | — | tests | M | keep | — | 07 |
| C03 | Config | run project processes; approval of exact manifest | CLI(`processes`) | **IV (DEVD stronger)** | `project.plan/approve/start/stop`, hash-bound approval | U | — | not auto-detected (Herd detects dev/vite/pail/queue) | `http.rs:963` | H | keep | — | 07 |
| C04 | Config | portability/secret safety | — | **UN** | env explicit, not inherited (README) | U | — | `.env` secrets never read at all today | read | L | investigate | P2 | 07 |
| D01 | Debug | local SMTP capture, mailboxes, `mail:wait` | CLI, CFG | **NI** | — (plan `04` §2) | U | — | — | grep smtp | H | adopt | **P1** | 09 |
| D02 | Debug | dump interception | CFG | **NI** | — | LV/PHP | — | needs runtime hook | grep | H | defer (PHP adapter) | P3 | 09 |
| D03 | Debug | log viewer (parsed app logs) | CLI | **PI** | `service.logs`, `project.logs`, `stacks.log` raw tails | U | — | no per-site app-log discovery/parse/search | read | M | adopt | P1 | 10 |
| D04 | Debug | insights: N+1, slow queries | CLI(Pro) | **NI** | — | LV | — | Laravel-specific | grep | H | reject for now | P3 | 10 |
| D05 | Debug | Xdebug capture, `debug.ini` | CLI(Pro) | **NI** | extension via `toolchain.*` only | L(PHP) | — | — | grep | H | defer | P3 | 10 |
| D06 | Debug | `tinker` | CLI | **NI** | agent sessions give a shell | F(LV) | — | stack REPLs (rails c, python, node) are a cross-stack equivalent | grep | M | adapt | P3 | 10 |
| D07 | Processes | auto dev stack (artisan dev, vite, queue) | CLI | **PI** | `project.tasks`, `project.run`, long-running tasks | F | — | tasks come from stack defs; start/log/restart present | tasks.rs | M | keep | P3 | 07 |
| D08 | Terminal | open terminal / agent in project | PRIOR | **DEVD-originated IV** | `session.*`, real PTY, reattach | U | Win/Linux | macOS PTY untested | `http.rs` | H | keep | — | 11 |
| I01 | Agents | MCP server | CLI | **IV** | `devd-mcp` (1131 lines), 5 discovery tools | U | — | — | `docs/mcp.md` | H | keep | — | 11 |
| I02 | Agents | `mcp:install` to 8 agents, global/project | CLI | **NI** | no installer code found by grep; `docs/mcp.md` shows a manual JSON snippet | U | — | — | grep | M | adopt | **P1** | 11 |
| I03 | Agents | Studio (browser devtools + agent) | PRIOR | **ID** | Preview screen + sessions | U | — | element picking deliberately not done | `04` §6 | M | defer | P3 | 11 |
| I04 | Agents | IDE integration / default IDE | CFG | **NI** | — | U | — | — | grep | M | adopt | P2 | 11 |
| I05 | CLI | per-site CLI acts on cwd's site | CLI | **UN** | `devd call`, typed commands | U | — | cwd-based site resolution unverified | none | L | investigate | P2 | 11 |
| I06 | Sharing | Expose tunnel | CLI | **NI** | limitations.md | U | — | needs a service + access control | doc | H | defer | P3 | 11 |
| M01 | Setup | first-run, requirements, partial-failure reporting | PRIOR | **IV (Win)** | `devd setup`, `setup.requirements` | U | Win; Linux says what's missing | macOS none | `setup.rs` | M | keep | — | 12 |
| M02 | Setup | updater | PRIOR | **NI** | limitations.md | U | — | — | doc | H | adopt | P2 | 12 |
| M03 | Setup | tray + autostart | CFG | **NI** | no tray/autostart code found | U | — | — | grep | M | defer | P3 | 12 |
| M04 | Setup | installer/uninstaller | PRIOR | **PI** | NSIS (Win); `install-linux.sh` | U | macOS none | Linux script unexecuted; no macOS | prior doc | M | adopt | **P0** | 12 |
| M05 | Setup | Defender exclusion | CFG | **NI** | — | P(Win) | Win | slow dev servers possible | — | L | investigate | P3 | 12 |
| M06 | Edition | Pro licensing | PRIOR | **N/A** | DEVD has no tiers | — | — | — | — | H | reject | — | 12 |
| X01 | Safety | approval enforced centrally | — | **IV** for registry paths | `registry.rs:156-263`; `policy.rs` | U | — | — | tests | H | keep | — | 13 |
| X02 | Safety | every interface uses the dispatcher | — | **IV after M0 (uncommitted)** — was PI: `session_start/stop/rename/dismiss` and `task_cancel` called subsystems directly; `session_dismiss` is Destructive | `api.rs` `dispatch_route`, `start_session`; tests in `tests/http.rs` (refused DELETE keeps session, audit rows, approval completes, source guard). Keystrokes/poll/resize remain documented exceptions | U | — | none known beyond U-12 | `cargo test -p devd-server`: 25 + 17 + 10 passed | H | done; commit | P0 | 13 |
| X03 | Safety | Herd's own posture (REA) | REA | — | Herd: ASAR integrity validation off, RunAsNode on | — | — | informational, not a DEVD gap | REA | H | note | — | 13 |
| Z01 | Platform | Linux service+PHP providers | — | **PL** | `catalogue.rs` | U | Linux | all of it | greps | H | adopt | **P0** | 12 |
| Z02 | Platform | macOS support | — | **NI** | `tools.yaml` has brew recipes only | U | macOS | all of it | grep | H | adopt | **P0** | 12 |
| Z03 | Platform | build env on Windows | — | **PI** | `.cargo/config.toml` assumes GNU; MSVC default toolchain fails | U | Win | fixed by `scripts/dev-env.ps1` | run this session | H | done | — | 12 |
| Z04 | Docs | docs match code | — | **PI** | stale rows in `02-feature-matrix.md`, `limitations.md` | U | — | version strings fixed this session; matrix row R03 still stale | diff | H | fix | P3 | 12 |
