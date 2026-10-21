# Feature acceptance matrix

Every Laravel Herd capability that was observed, and what DEVD does about it.

**Legend.**

* **Verified** — implemented, and exercised by a test or a run on this machine.
* **Partial** — implemented, with a named gap.
* **Not implemented** — nothing, and the gap is stated.
* **Different by design** — DEVD does something else, and the reason is given.

The Herd column cites `docs/research/01-herd-investigation.md`, which records
how each behaviour was observed.

---

## 1. Choosing and managing a project

| Herd capability | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| `link` — serve a directory as `name.test` | `site.create` (`devd-sites/src/caps.rs:87-120`) creates a site: directory, hostname, certificate, hosts entry | Verified | `crates/devd-sites/src/lib.rs:559-588` (12 unit tests) |
| `unlink` | `site.remove` | Verified | `crates/devd-sites/src/lib.rs:682-714` |
| `park` / `forget` | Not implemented. DEVD's projects are directories the person already has, not a directory of generated sites | Not implemented | — |
| `sites` / `links` / `parked` | `site.list` and `project.list` | Verified | `crates/devd-sites/src/lib.rs:521-557` |
| `secure` / `unsecure` / `secured` | `tls.issue`, `tls.untrust`, `tls.list`; a site is created with TLS by default | Verified | `crates/devd-services/src/tls.rs:554-609` |
| `proxy` / `unproxy` / `proxies` | Not implemented | Not implemented | — |
| `isolate` / `unisolate` / `isolated` — pin a site to one PHP version | A site records the PHP version it was created against, but nginx has **one** FastCGI upstream | Partial | `docs/limitations.md` records the gap; `crates/devd-services/src/nginx.rs:140-144` |
| `tld` — change the top-level domain | `settings.toml` holds the TLD; the hosts block is regenerated from it | Partial | `crates/devd-sites/src/hosts.rs:262-308` |
| `init` / `init:fresh` — a `herd.yml` manifest | `devd.yml` governs project **processes**; `project.import` detects and registers a project. A project-*identity* manifest is not implemented | Different by design | `docs/devd-yml.md`; `crates/devd-stacks/src/caps.rs:451-476` |
| `which` — which driver serves a directory | `project.detect` returns the detected stack with its evidence and confidence | Verified | `crates/devd-stacks/tests/engine.rs:149` |
| `site-information` | `project.detect` | Verified | same |
| `directory-listing` | Not implemented | Not implemented | — |
| `db` — open the site's database in a client | `db.list`, `db.create`, `db.drop` — MariaDB only | Partial | `crates/devd-services/src/caps.rs:860-940` |
| **Choose a project folder by browsing** | **The folder picker**: a native OS dialog in the desktop shell (`POST /api/pick-folder`, `crates/devd-server/src/picker.rs`) and an integrated folder browser in the web application (`fs.browse`, `crates/devd-core/src/browse.rs`) | **Verified** | `crates/devd-core/src/browse.rs` unit tests; `crates/devd-server/tests/ui_contract.rs::the_folder_picker_is_the_normal_way_to_choose_a_project`; driven by hand in the running application |

## 2. Runtimes

| Herd capability | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| `php:install` / `php:list` / `php:update` | `runtime.install`, `runtime.list`, `runtime.remove`; a digest-verified download from windows.php.net, and the interpreter is **run** to prove it starts | Verified (Windows) | `crates/devd-runtime/tests/install.rs:41` (network-dependent) |
| `node:install` / `node:list` — nvm | `runtime.install` for Node; DEVD does not bundle an nvm, it installs a Node build | Different by design | `crates/devd-runtime/src/catalogue.rs:204-266` |
| `use` — the global PHP version | `runtime.use`; the first installed PHP becomes primary automatically | Verified | `crates/devd-runtime/src/lib.rs:462-472` |
| `which-php` | `runtime.list` reports the path of every runtime | Verified | `crates/devd-server/ui/app.js` Runtimes screen |
| `ini` | Not implemented as a command; the runtime's `php.ini` path is reported | Partial | — |
| PHP on Linux | **Not installable.** `catalogue::php` returns an empty list off Windows, and `devd setup` now says so and points at the system package manager | Partial (honest) | `crates/devd-runtime/src/catalogue.rs:104-107`; `crates/devd-cli/src/setup.rs:54-80` |
| Node and Go on Linux | Node and Go publish Linux tarballs and are installed | Verified | `crates/devd-runtime/src/catalogue.rs:259-264` |

## 3. Web server, domains, DNS and HTTPS

| Herd capability | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| nginx | `service.install nginx`; the configuration is rendered from the sites table and checked with `nginx -t` before it is used | Verified | `crates/devd-services/src/nginx.rs:301-428` (5 tests) |
| Local domains | `site.create` assigns a hostname and publishes it | Verified | `crates/devd-sites/src/lib.rs:559-588` |
| DNS | The hosts file, managed in a marked block that is rewritten idempotently and never touches anything outside it. **No DNS server** | Partial | `crates/devd-sites/src/hosts.rs:340-453` (7 tests) |
| HTTPS | A local certificate authority: `tls.ca.ensure`, `tls.issue`, `tls.trust`, `tls.untrust`, `tls.ca.status` | Verified | `crates/devd-services/src/tls.rs:554-609` |
| Per-site server snippets | `config/server_snippets/*.conf` is included by the generated vhosts | Verified | `crates/devd-services/src/nginx.rs` |
| HTTP/2 | Not enabled | Not implemented | `docs/limitations.md` |

## 4. Services

| Herd capability | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| `services:available` | `service.available` and `toolchain.list` | Verified | `crates/devd-services/src/caps.rs` |
| `services:create` — an instance on its own port | `service.install` with a port; a service instance has its own install directory, data directory, port and pid file | Verified (nginx, MariaDB) | `crates/devd-services/src/lib.rs:353-540` |
| `services:list` | `service.list` — type, port, version and running state, with a real pid check | Verified | `crates/devd-services/src/lib.rs:1241-1263` |
| `services:start` / `services:stop` | `service.start` / `service.stop` / `service.restart`; a start that does not open its port is a failure with the service's own log tail | Verified | `crates/devd-services/src/lib.rs:757-794` |
| `services:clone` | Not implemented | Not implemented | — |
| `services:delete` — including the data | `service.remove` — removes the program and **keeps the data directory**, and says where it is | Different by design (safer) | `crates/devd-services/src/lib.rs:977-1008` |
| MySQL, PostgreSQL, Redis, MongoDB | **Not devd-managed.** They are detected on the machine and can be installed and controlled through the system package manager (`toolchain.*`, `crates/devd-stacks/defs/tools.yaml`) | Partial | `crates/devd-stacks/tests/engine.rs:360` (observe mode blocks devd from controlling a service) |
| MariaDB | devd-managed: a data directory is provisioned, the system tables are initialised and a root password may be set | Verified | `crates/devd-services/src/mariadb.rs` (483 lines, no unit tests — a gap) |
| Coexistence with an existing installation | `services.inventory` separates **installed**, **running**, **required** and **managed**; `services.decide` records `adopt`, `observe`, `external` or `ignore`. devd never replaces an installation as a side effect | Verified | `crates/devd-stacks/tests/engine.rs:321`, `:360` |
| Real health checks | A service is healthy only when the port answers **and** the process image matches the recorded pid | Verified | `crates/devd-services/src/lib.rs:197-219` |
| Mail capture | Not implemented | Not implemented | — |
| Xdebug | The extension is not bundled; `toolchain.*` can install the PHP extension | Partial | `crates/devd-stacks/defs/tools.yaml` |
| Local mail, dumps, profiling, insights | Not implemented | Not implemented | — |

## 5. Agents and developer tooling

| Herd capability | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| `mcp:install` — register Herd's MCP server with coding agents | `devd mcp` — five discovery tools over the registry, and the MCP server can be installed for agents | Verified | `crates/devd-mcp/src/lib.rs` |
| `studio` — a browser devtools with an agent | The Sessions screen: a real PTY per harness, transcripts, and the MCP server attached, so the agent has the same permission model | Verified | `crates/devd-workspace/src/session.rs`; `crates/devd-server/tests/http.rs` (a session over a real socket) |
| Agent runtime context (queries, logs, jobs, dumps) | `team.*` records what a specialist actually did, and a task is accepted only on evidence | Verified | `crates/devd-agent/tests/agent.rs:532`, `:566`, `:667` |
| `processes` — the dev stack for a site | `devd.yml` project processes: `project.plan`, `project.approve`, `project.start`, `project.status`, `project.logs`, `project.stop` | Verified | `crates/devd-workspace/src/project.rs`; `crates/devd-server/tests/http.rs:963` |
| A project browser with the recent projects | The Projects screen: search, three sort orders, pinning, and the real location of every project | Verified | `crates/devd-server/ui/app.js` (Projects) |
| Agent permissions and approvals | One approval queue. Agents queue; only a person decides. In production mode every write needs a person | Verified | `crates/devd-agent/tests/agent.rs:238`, `:278`, `:360`, `:714` |
| Git status, staging, commit, push | The complete Git lifecycle: 75 capabilities | Verified | `crates/devd-git/tests/lifecycle.rs` (27 tests), `policy.rs` (15 tests) |
| File-level change lists and diffs | The Changes tab: staged, unstaged, untracked and conflicted, with per-file diffs | Verified | `crates/devd-git/tests/lifecycle.rs` |

## 6. Sharing and tunnels

| Herd capability | DEVD implementation | State |
|---|---|---|
| A public tunnel (Expose) | Not implemented. DEVD has no tunnel, and does not pretend to | Not implemented |
| Webhook testing | Not implemented | Not implemented |
| Public-access indicators | Not applicable without tunnels | — |

## 7. Cross-cutting

| Area | DEVD implementation | State | Test / evidence |
|---|---|---|---|
| One backend for every surface | A capability registry; the CLI, TUI, web, desktop and MCP are projections of it, and `Registry::dispatch` is the only path to a side effect | Verified | `crates/devd-core/src/registry.rs:156-263`; `crates/devd-stacks/tests/engine.rs:257` ("every surface gets the same answer") |
| Permission tiers and approval | `read < write < exec < destructive < remote < admin`; approval is required for the top three and cannot be self-approved by an agent | Verified | `crates/devd-core/src/registry.rs:344-556`; `crates/devd-git/tests/policy.rs` |
| Audit trail | Every attempt, successful or not, with secrets redacted | Verified | `crates/devd-git/tests/policy.rs:498` |
| Agent tool discovery | `devd_groups`, `devd_capabilities`, `devd_describe`, `devd_invoke`, `devd_doctor` | Verified | `crates/devd-mcp/src/lib.rs` |
| Model providers | 15 presets: Anthropic, OpenAI, DeepSeek, Moonshot (Kimi), Zhipu (GLM), Gemini, xAI, Mistral, Groq, Together, Fireworks, OpenRouter, Ollama, LM Studio and any OpenAI-compatible URL | Verified | `crates/devd-agent/tests/agent.rs:415` (the live Anthropic models page still parses) |
| Official brand marks | 51 marks served from the binary: the 15 providers, the databases, caches, search engines, runtimes and tools, and the editors | Verified | `crates/devd-server/ui/brands/`; each asset is the owner's own artwork |
| Install, update, repair, uninstall (Windows) | A per-user NSIS installer; a repairable install; an uninstaller that keeps your data unless you ask it to remove it | Partial | `installer/devd.nsi`; verified by building the installer |
| Install, update, repair, uninstall (Linux) | A per-user install script with an uninstall that keeps your data, and a `--purge` that does not | Partial | `scripts/install-linux.sh` (syntax checked; not executed — this machine has no Linux) |
| Self-update | Not implemented. Nothing in devd contacts GitHub | Not implemented | `docs/limitations.md` |

## 8. Defects found and fixed during this work

| Defect | Where | How it was found |
|---|---|---|
| A malformed CSS rule left a brace open, so the browser ignored the bottom half of the stylesheet — **including every responsive breakpoint** | `crates/devd-server/ui/style.css:1815` | The folder picker rendered with no layout at all. A contract test now checks that the stylesheet is balanced |
| `git.reflog` returned lines of text where the interface expected objects, so the Recover tab showed blank rows and offered "Reset to this" with an empty revision | `crates/devd-git/src/lib.rs:1446` | The audit of the web interface |
| A provider whose id was not itself a preset id could not be edited: the interface sent `preset: <its own id>` and the server rejected it | `crates/devd-agent/src/caps.rs:716` | The audit of the web interface |
| The Diagnostics screen claimed Services, Sites, Runtimes, DNS and certificates had no capabilities, on the same screen that renders all five | `crates/devd-server/ui/app.js:3687` | The audit of the web interface |
| "Dry run" on a capability that cannot plan threw a `TypeError` | `crates/devd-server/ui/app.js` (capabilities) | The audit of the web interface |
| Untracked and conflicted files shared a green badge, and the same file was red in one tab and green in another | `crates/devd-server/ui/app.js` (statusTag) | The audit of the web interface |
| A failed activity-log request rendered as "Nothing has happened yet" | `crates/devd-server/ui/app.js` | The audit of the web interface |
| The number of stacks was written into the interface ("24 stacks") | `crates/devd-server/ui/app.js` | The audit of the web interface |
| The `hidden` attribute did not hide anything that had a `display` rule | `crates/devd-server/ui/style.css` | The native folder-picker button was visible in a browser that has no native picker |
| A missing element id took the whole screen down instead of one control | `crates/devd-server/ui/app.js` | The folder picker's list never rendered. A contract test now checks every id the script looks up |
| The runtime installer dropped its "upstream publishes no digest" warning because a later `let` shadowed it | `crates/devd-runtime/src/lib.rs:205`/`:218` | The audit of the Rust backend |
| The HTTP API executed `session.dismiss` (a destructive capability) without the approval the capability requires | `crates/devd-server/src/api.rs:710` | The audit of the Rust backend. **Fixing this is behind the comment-debt approval gate** |
