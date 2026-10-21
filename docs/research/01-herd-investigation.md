# Laravel Herd 2.0.2 — investigation report

**Machine:** Windows, `x86_64-pc-windows-gnu`, Laravel Herd **2.0.2** installed at
`C:\Program Files\Herd`.
**Date of investigation:** 20–21 October 2026.
**Method:** reverse engineering with [REA](https://rea.tools/) (the
`rea-agents` package, v6.7.0, installed and registered as an MCP server), plus
direct inspection of the installed files, the running process and the operating
system.

Every statement below is labelled:

* **Observed** — read directly from the installed application, its files, its
  logs, or its running process.
* **Executed** — a command was run and its output recorded.
* **Hypothesis** — a reading that the evidence supports but does not prove.

---

## 1. The reverse-engineering environment

### 1.1 What was set up

| Item | State |
|---|---|
| Node.js | v22.19.0, npm 11.17.0 (already present) |
| REA | `rea-agents@6.7.0` installed globally (`npm install -g rea-agents`) |
| REA MCP server | Registered with OpenCode at `C:\Users\User\.config\opencode\opencode.json` via `rea setup --client opencode --yes`; the bundled skill was installed to `C:\Users\User\.agents\skills\reverse-engineer-anything` |
| `rea doctor` after setup | `healthy: true` |
| Analysis engines (Hopper, Ghidra, IDA, JADX, Kuna) | **Not available on this machine** — `doctor` reports `missing_analysis_engine` for each |

The last row matters: REA's *JavaScript/Electron* analysis engine is built in
and needs no external engine, so the whole of the JavaScript surface was
analysable. The native-binary engines were not, and nothing in this report
depends on them — Herd's logic is in JavaScript and PHP, not in compiled
native code.

### 1.2 What REA was actually used for, and what it could not do

| REA capability | Used | Outcome |
|---|---|---|
| `analyze_javascript_application` (ASAR) | Yes | The full-application graph was attempted twice. The first attempt was refused with `artifact_integrity_mismatch` on `resources/bin/herd-devtools-helper-arm64.zip` — a packaging artefact where the declared hash contradicts the shipped file, which the tool correctly reports rather than hides. Re-run with `integrity_policy=record-and-continue`, the analysis then exhausted the parent result budget on `node_modules/validator/…` (a 1.8 GB budget against a 155 MB archive that embeds a full npm tree). |
| `analyze_javascript_application` (extracted tree, source-selected) | Yes | The meaningful route: the ASAR was extracted and the analysis restricted to the application's own sources. The preload bridge was read in full and is the single most informative artefact in the installation. |
| `analyze_javascript_application` (main process) | Partly | Herd's main process is compiled to V8 bytecode (`out/main/index.jsc`), not JavaScript, so it is not analysable as a source tree. The same applies to `SiteOrdering-3E2GNui8.jsc` and `ApplicationsScanWorker-Bssk7NGr.jsc`. This is a **limitation of the target, not of the tool**. |
| `inspect_artifact` / `extract_artifact` | Yes | The ASAR was expanded and its member inventory read. |
| Native binary analysis (Ghidra / IDA / Hopper) | No | Not available on this machine and not required: no part of Herd's behaviour lives in a compiled native binary. `HerdHelper.exe` (11.8 MB) is a Windows Service for hosts-file management and is analysed in §5. |
| `capture-electron_scenario` / `observe_javascript_runtime` | Yes | Herd was relaunched with `--remote-debugging-port=9222` and driven over the Chrome DevTools Protocol: screens were captured, the DOM and computed styles read, and buttons clicked. |
| `inspect_evidence_bundle` / `export_evidence_bundle` | No | The evidence was read inline; the graph was larger than the parts that answered a question. |

**The most useful finding did not come from REA at all.** Herd ships a PHP
command-line tool, `resources/bin/herd.phar` (3.3 MB). Running
`herd.phar list` produced the complete, authoritative list of Herd's features —
every command, with its own one-line description. That is the ground truth for
what Herd can do, and everything in §4 is corroborated against it.

---

## 2. What Herd 2.0.2 is

**Observed.** `resources/app.asar/package.json`:

```json
{ "name": "Herd", "version": "2.0.2", "main": "./out/main/index.js",
  "dependencies": { "@sentry/electron", "electron-store", "electron-updater",
    "express", "floating-vue", "menubar", "portscanner", "sequelize",
    "smtp-server", "sqlite3", "vue-router", "ws", "yaml", … } }
```

* An **Electron** application (Chromium + Node), written in **Vue 3** with
  `vue-router`, styled with **Tailwind** and a Windows 11 "WinUI" design
  language, using **Segoe UI Variable Text** and the **Segoe Fluent Icons**
  font.
* **Sixteen separate renderer windows** (`out/renderer/*.html`): `applications`,
  `settings`, `services`, `mails`, `dumps`, `logs`, `logViewer`, `onboarding`,
  `menubar`, `updater`, `upToDate`, `whatsNew` and others. The window that
  carries the site list is *Applications*, not "Sites".
* A **system-tray application** (`menubar`) is the primary surface on Windows;
  the Applications window is opened from it.
* State lives in **`~/.config/herd/`** (XDG-style, even on Windows):
  `config/config.json`, `config/herd.json`, `config/php.json`,
  `config/integrations.json`, `config/executables.json`,
  `config/migrations.json`, `config/valet/`, `config/nginx/`,
  `config/certificates/`, `config/studio-auth/`, `bin/`, `logs/`, `.pid/`,
  `cache/`, `devtools/`, and **`herd.sqlite`** (Sequelize; tables `Dumps`,
  `Mails`, `MailHeaders`, `MailAddresses`, `MailAttachments`, `SequelizeMeta`).
* Projects are **directories**, conventionally under `C:\Users\<user>\Herd`.
* The PHP half of Herd is **Laravel Valet**, vendored into
  `resources/valet/` (a `Valet` namespace, `box.phar`, `dump.phar`,
  `adminer/`, and the Valet CLI under `resources/valet/cli`).
* Herd's own CLI is **`herd.phar`**, and it is a *thin client*: `Valet\Herd`
  makes HTTP requests to `http://127.0.0.1:<apiPort>/…` (`HerdConfiguration::
  load()->apiPort()`), where `apiPort` is 9001 by default. The Electron app owns
  the state; the CLI is a remote control.

### 2.1 The IPC surface

**Observed** in `out/preload/index.js` (48 KB, read in full). The preload
exposes two globals to the renderer: `window.electron` and `window.api`.
`window.api` has 27 top-level namespaces:

| Namespace | What it covers |
|---|---|
| `app` | version, shortcuts, new-application wizard, site wizard, updates, `apiPort`/`apiStatus`, WebView2-style helper install/uninstall, debug mode |
| `herd` | `initialize`, DNS configuration, root certificate validity, PHP version switching |
| `sites` | list, create, remove, **open in browser**, isolate, preview snapshots, `addExistingSite`, remote sites (Forge/SSH) |
| `services` | MySQL/MariaDB/PostgreSQL/MongoDB/Redis/Valkey/Typesense |
| `valet` | the Valet proxy layer |
| `mails` | SMTP capture, mailboxes, attachments, context menus |
| `dumps` | dump interception, `dump()`/`dd()` capture |
| `logViewer` | log file browsing, paging, watch |
| `expose` | public tunnels |
| `laravelCloud`, `forge`, `vapor` | deployment |
| `ai` | AI providers, models, chat |
| `applications` | the new-application builder |
| `mcp` | registering Herd's MCP server with coding agents |
| `integrations` | Forge, Cloud |
| `licensing` | Herd Pro activation, sale banners, lifetime |
| `dictation` | local Whisper-based dictation |
| `externalApps` | IDE integration, database clients |
| `store`, `system` | preferences, OS integration |

**The single most important entry** for DEVD is `sites.addExistingSite`:
it takes no arguments and returns a path. The native folder dialog is opened
inside the main process.

---

## 3. Screens, measured

The screens below were opened in the running application and captured with the
DevTools Protocol. The figures are from computed styles, not from reading the
source.

### 3.1 Applications (the site list)

* Window 1600×898. A 36 px title strip (`h-9`), then a 53 px header
  (`px-4 py-2.5`), then content.
* Header, left to right: **"＋ New"** (32 px button, `px-2.5`, `text-[13px]`,
  `rounded` 4 px), then a centred search field, then a refresh button, a sort
  flyout, a **grid/list segmented control** (a 2 px-inset pill, `rounded-full`),
  and a row of 32×32 icon buttons.
* The empty state: a 4-square glyph, **"No Applications Yet"**, and
  *"Describe your first app above and Herd builds it for you — or link an
  existing project."*
* The "new application" panel is a single card, max-width ≈ 920 px, with:
  a headline (*"Let's build something amazing"*), a name field at
  `text-lg font-medium`, a description field, a model picker, a framework
  picker, an "Options" flyout, a microphone, and a primary **"Create App"**
  button with its shortcut shown in the same button.
* Top right of the panel: **"Link Existing App"** in the accent colour.

### 3.2 Settings

A **fixed 245 px sidebar** with the product mark and name at the top, then
navigation items: General, PHP, JavaScript, AI, Services (with a **"Pro"** badge
in the accent colour), Herd Pro, Sharing, Shortcuts, About. The selected item is
a rounded rectangle with a lighter surface; icons are 16 px.

The content column is centred with a maximum width of about 720 px:

* A **pill-shaped search field** at the top (fully rounded, ~288 px wide).
* An `h1` at ~24 px bold, a one-line subtitle in the muted colour, and a
  hairline rule.
* Sections: a 16 px semibold title, a 14 px muted description, then a card
  with a 1 px border and 8 px radius.
* Settings rows: 44 px minimum height, a 40 px rounded icon tile, a 14 px
  semibold title, a 13 px muted description, and a trailing toggle, chevron or
  lock.
* The "General" page has sections: Herd paths, Startup, Updates, Integrations,
  Default IDE, Internal API, Windows Defender, HerdHelper.
* The "PHP" page has a plain table (`VERSION | INSTALLED`) with an
  **"Install"** button per row.
* The "AI" page lists: Herd Studio, MCP, runtime context (Pro), dictation
  language, notifications, tool updates.
* The "Sharing" page is Expose: account, learn more.
* The "Services" page is Pro-gated with an upgrade card.

### 3.3 Design system, as observed

* **Type:** `Segoe UI Variable Text` (with `Segoe UI`, `system-ui` fallbacks).
  Base 14 px; headings 24 px; section titles 16 px; labels 12–13 px.
* **Colour:** a neutral dark ramp; the accent is a warm red
  (`#b4463b`, seen in the boot ring in `applications.html` and the "Link
  Existing App" link). Status colours are separate.
* **Shape:** 4 px controls, 8 px cards, 10 px some cards, fully-rounded
  segmented controls and search fields.
* **Motion:** 200 ms `cubic-bezier(0.1, 0.9, 0.2, 1)` flyouts; 250 ms fades;
  a 450 ms "glass materialise" that scales and blurs a card open.
* **Density:** 44 px rows, 32 px buttons, 10 px horizontal padding.

**DEVD does not use any of Herd's colours.** DEVD's own palette is graphite
and phosphor-mint (`--bg #0a0a0b`, `--accent #7cf5c0`), which predates this work
and is unrelated to Herd's warm red.

### 3.4 What Herd gets right that DEVD did not

1. **One screen, one job.** The Applications window is a site list and nothing
   else. Settings is a separate window with its own navigation.
2. **A search field in the header of every list.** DEVD puts search in a
   command palette, which is excellent for operators and wrong for a list
   screen.
3. **Empty states that say what to do next.** *"Describe your first app above
   and Herd builds it for you — or link an existing project."*
4. **A segmented grid/list control.** DEVD renders one layout per collection.
5. **Rows are 44 px and the icon tile is 40 px.** DEVD's rows are 32 px.
6. **Progress and status are attached to the thing they describe**, not shown
   as a global percentage.

---

## 4. The feature inventory (from `herd.phar list`)

**Executed.** `herd.phar list` output, verbatim in structure. This is the
authoritative list of what Herd 2.0.2 can do.

### 4.1 Sites, domains and configuration

| Command | What it does |
|---|---|
| `link` | Serve a directory as `name.test`; `--secure` for HTTPS, `--isolate=8.4` to pin a PHP version |
| `unlink` | Stop serving a linked site |
| `park` / `forget` | Serve / stop serving every subdirectory of a directory as `folder.test` |
| `parked` / `links` / `sites` | List what is served, with URL, path, PHP and Node version |
| `secure` / `unsecure` / `secured` | HTTPS with a locally trusted certificate; list them with expiry |
| `proxy` / `unproxy` / `proxies` | Serve `domain.test` as an nginx proxy to another host (Reverb, Docker) |
| `isolate` / `unisolate` / `isolated` | Serve one site with a specific PHP version |
| `tld` | Change the top-level domain (`test`, `wip`, `herd`, `local`, `localhost`) |
| `loopback` | Change the loopback address sites resolve to |
| `init` / `init:fresh` | Apply or create a `herd.yml` manifest: link, PHP version, HTTPS and services |
| `open` | Open a site in the browser |
| `which` | Which Herd driver serves a directory (Laravel, Statamic, static, …) |
| `site-information` | The driver's site information as JSON (framework and versions) |
| `directory-listing` | Whether nginx lists folder contents when there is no index file |
| `db` | Open the site's database in the configured client, using the credentials in `.env` |

### 4.2 `herd.yml` — the project manifest

**Observed** in `resources/valet/cli/Valet/Manifest.php`:

```yaml
name: my-app
php: "8.3"
secured: false
aliases:
  - www.my-app.test
services:
  mysql:
    version: latest
    port: ${DB_PORT}
integrations:
  forge: []
```

* `services.<name>.port` may be `${ENV_VAR}`; the manifest is completed from
  `.env` at read time (`complementConfigWithEnv`), and a missing variable is
  reported by name (`getMissingEnvironmentVariables`).
* The service types the manifest knows are redis, postgresql, mongodb, mysql,
  mariadb, meilisearch, minio and reverb — with a port key per type.
* `Manifest::validate` rejects an unknown service, a missing port, a PHP
  version that is not supported, and a missing name, with one message per
  problem.

**DEVD's equivalent** is `devd.yml`, which already exists and governs project
*processes* rather than project *identity*. The two are complementary: Herd's
manifest says "this directory is a Laravel app on PHP 8.3 with MySQL"; DEVD's
says "this directory has these long-running commands".

### 4.3 Runtimes

| Command | What it does |
|---|---|
| `php:install` / `php:list` / `php:update` | Install, list (with update status) and update PHP versions |
| `node:install` / `node:list` | Install and list Node majors through Herd's own nvm |
| `use` | Change the global PHP version |
| `which-php` | The path of the PHP binary that serves a site |
| `ini` | Open a PHP version's `php.ini` (or `debug.ini`) in the IDE |
| `laravel:update` | Update the bundled Laravel installer |

Supported PHP versions (from `Valet\Herd::SUPPORTED_PHP_VERSIONS`): 7.4, 8.0,
8.1, 8.2, 8.3, 8.4, 8.5, 8.6.

**Observed**: `resources/bin/nvm/` contains a complete `nvm-windows`
(`nvm.exe`, `install.cmd`, `elevate.cmd`, `setuserenv.vbs`), and
`resources/bin/phpherd/`, `resources/bin/phpmongodb/` and `resources/bin/
phpredis/` contain the Herd PHP extensions for every supported version.

### 4.4 Services (Herd Pro)

| Command | What it does |
|---|---|
| `services:available` | The service types that can be installed, with their latest version (Pro) |
| `services:versions` | The installable versions of one type (Pro) |
| `services:create` | Install a service instance, e.g. `herd services:create mysql --port=3307` (Pro) |
| `services:list` | Installed instances with type, port, version and running state (Pro) |
| `services:start` / `services:stop` | Start or stop an instance by id or name (Pro) |
| `services:clone` | Clone an instance under a new name and port (Pro) |
| `services:delete` | Delete an instance **and all of its data** (Pro) |

The service types the UI knows about (from `out/renderer/assets/services-*.js`
and `applications-*.js`): **mysql, mariadb, postgresql, mongodb, redis,
valkey, typesense**.

**Note on the paid tier.** Herd Pro's service management is, mechanically,
"install a versioned binary, give it a port, a data directory and a start
command". That is not proprietary: it is what DEVD's `service.*` capability
family already does for nginx and MariaDB. Implementing the same capability for
PostgreSQL, Redis and MongoDB is engineering work, not licensed work.

### 4.5 Mail, dumps, logs and insights (Herd Pro)

| Command | What it does |
|---|---|
| `mail:list` / `mail:latest` / `mail:show` / `mail:links` | Browse captured mail |
| `mail:mailboxes` | Mailboxes are the SMTP username; mail without one lands in "Default" |
| `mail:wait` | Block until a matching mail arrives; exit 1 on timeout |
| `mail:config` | The SMTP host and port, and the `.env` values an app needs |
| `mail:clear` / `mail:delete` | Remove captured mail |
| `insights:list` | What Herd captured for a site: dumps, queries, log lines, jobs, HTTP requests, cache events, grouped by request |
| `insights:analyze` | Find N+1 queries, duplicate queries, slow queries, errors and slow requests, with file and line and a fix hint; `--after=<cursor>` to verify a fix |
| `insights:capture` | Whether runtime context is captured for coding agents |
| `insights:clear` | Discard what was captured |
| `insights:status` / `insights:watch` | Which sites have captured entries; stream new ones |
| `debug:start` / `debug:stop` | Capture an Xdebug session and print the data as JSON |
| `logs:files` / `logs:read` | A site's log files and their entries, newest first |
| `processes:start` / `stop` / `output` | A site's `artisan dev` stack, the Vite dev server, `pail` and the queue worker |

**Observed**: the Pro features are gated by `licensing.hasValidLicense()`; the
renderer receives `?validLicense=0` in the settings URL when there is no
licence, and the Pro panels render a lock.

### 4.6 Agentic development

| Command | What it does |
|---|---|
| `mcp:install` / `mcp:uninstall` / `mcp:status` | Register Herd's MCP server with Claude Code, Cursor, VS Code, Codex, Junie, Gemini CLI, OpenCode and Windsurf, globally or per project |
| `studio` | Open Herd Studio (a browser devtools with a coding agent) for a site |
| `processes` | The dev processes Herd runs for a site |

**Observed** in the log and in `resources/bin/`:
* `herd-mcp.phar` (1.65 MB) is Herd's MCP server.
* `herd-opencode` and `herd-devtools-helper` are downloaded on first run (the
  log records `Installed herd-opencode 2.0.32 … from opencode.zip` and
  `Installed herd-devtools-helper 0.28.3 … from herd-devtools-helper.zip`).
* `herd-devtools-helper` runs a WebSocket server on `ws://127.0.0.1:4421/ws`
  with a REST API at `http://127.0.0.1:4421/api` and a health check at
  `/health`.
* The protocol is a `host.tools.register` message: the host registers a tool
  (`GetHerdInsights`) with the connected client, and the client answers
  `models.list`, `opencode.providers.list` and the rest. The log shows the
  assistant connected and received the tool.

This is the same shape as DEVD's own MCP server and agent hub, and it is the
pattern DEVD already implements.

### 4.7 Sharing

`share` opens an Expose tunnel and prints the public URL; `fetch-share-url`
prints the URL of a running tunnel. `resources/bin/expose.phar` (18 MB) and
`expose.bat` are the vendored client. The Setting page offers an Expose
account.

### 4.8 The rest

`status` is a one-screen health check: Herd version, PHP and Node defaults,
PHP-FPM socket health, mail and dump servers, and every core and extra service
with its state. `start`, `stop` and `restart` work on all services or one of
nginx, dnsmasq or php. `tinker` opens an interactive session (Tinkerwell when
installed). `studio`, `workspace`/`workspaces` (a workspace is an isolated copy
of a site on its own branch, with its own domain) and `db:create`/`db:drop`/
`db:list`/`db:use` round it out.

---

## 5. Configuration, services and the machine

**Observed.**

* **nginx** is vendored (`resources/bin/nginx/nginx.exe`) and configured by
  `resources/config/nginx/nginx.conf` plus a generated `herd.conf`. The
  generated file listens on `127.0.0.1:80`, passes PHP to a FastCGI socket,
  and includes per-site `server_snippets`. The packaged template has
  `client_max_body_size 512M` and gzip on.
* **DNS** is `dnsmasq` on Unix (`address=/.test/127.0.0.1`) and, on Windows,
  the **`HerdHelper` Windows Service** writes the hosts file. The log records
  *"Updating hosts file via HerdHelper Windows Service"* and *"Server says:
  Hosts file updated."* `HerdHelper.exe` (11.8 MB) is the only native binary
  in the installation, and its whole job is that hosts file.
* **TLS**: a local authority under `config/certificates/`, with a root
  certificate installed into the machine's trust store.
* **PHP** runs as `php-cgi` in FastCGI mode on a base port of 9000 plus the
  version offset (`basePhpPort: 9000`, `baseDebugAddition: 100`). The log
  shows `PHP 85 on port 9085` and `Starting CGI`.
* **Dumps**: a dump server that intercepts `dump()`/`dd()` calls, with a
  `dumpFontSize` and a colour per dump kind.
* **Mail**: a local SMTP server on port 2525 (`smtpPort: 2525`), implemented
  with `smtp-server` and `mailparser`.
* **Persistence**: `config.json` holds the API port (9001), the base PHP port,
  the SMTP port, autostart, the debug colour for each dump kind, the custom
  site groups ("Favorites"), the default IDE, the sites display mode
  (`grid`/`list`), the sort order and direction, the inspector widths, the log
  exclusions and the "show new dumps on top" preference.
* **Per-version configuration**: `config/php.json` records the installed patch
  version of each PHP major (`installed_8.5: 8.5.11`) and an internal counter
  (`installed_internal_8.5: 15`) used to decide when a new build is worth
  offering.

---

## 6. The native folder dialog

**Observed.** Clicking **"Link Existing App"** in the Applications window opened
a **native Windows folder picker** — the Windows common item dialog in folder
mode — with the title:

> **Select the folder of your existing PHP project.**

The dialog is the operating system's own, with the standard **"Folder:"** field
at the bottom and **"Select Folder"** / **"Cancel"** buttons. It is opened by
the main process in response to the `sites.add-existing-site` IPC message,
which takes no arguments and returns the chosen path.

This is the reference for DEVD's folder picker (Phase 4.3): **browse, do not
type**.

---

## 7. What DEVD takes from this, and what it does not

### Taken

| Herd | DEVD |
|---|---|
| A native folder picker for "add an existing project" | `POST /api/pick-folder` in the desktop shell (`rfd`), plus an integrated folder browser in the web application (`fs.browse`) |
| One list of projects | `project.list` now returns the union of imported projects and indexed repositories, so the web application, the CLI and the TUI agree |
| `herd.yml` describing a project's identity | DEVD's `devd.yml` stays about processes; a project-identity manifest is noted as future work rather than invented here |
| Versioned service instances with their own port and data directory | The `service.*` capability family already has this shape for nginx and MariaDB; it is the natural home for PostgreSQL, Redis and MongoDB |
| A `status` command that reports the truth about every service | DEVD's `service.list` and `health.status` already do this; the *health* indicator must be a real protocol check, not a TCP connect |
| MCP registration for coding agents | DEVD's `devd-mcp` already does this and is the surface the new `fs.browse` and folder-picker work plugs into |

### Not taken

| Herd | Why |
|---|---|
| The warm-red palette | Prohibited, and unnecessary. DEVD keeps graphite and phosphor-mint. |
| The Segoe UI Variable / Segoe Fluent Icons typography | DEVD is cross-platform; a Windows-only icon font would break Linux. DEVD uses its own inline SVG sprite. |
| `electron-store` / `sequelize` / SQLite for state | DEVD's architecture is a **capability registry** with a SQLite state store and a single dispatch path. Herd's architecture is an IPC bridge. DEVD's is stronger for its purpose and is not being replaced. |
| A licensed-gated feature set | The requirement is independent equivalents. Where an equivalent needs a commercial service, that is documented rather than pretended. |
| The "new application" prompt builder | That is a product decision about generating code, not a development-environment capability. DEVD's `project.init` scaffolds from a stack definition, which is the equivalent with a different, more honest shape. |

---

## 8. Uncertainties

* **Herd Studio** could not be fully characterised: it is driven by the
  vendored `herd-devtools-helper` binary and the `opencode` build, and it needs
  a licence and a signed-in agent to do anything. Its *protocol* is documented
  from the log and the preload; its screens were not observed.
* **The main process's own logic** is V8 bytecode and was not decompiled. The
  behaviour it implements was reconstructed from the preload contract, the
  log, the configuration files and the running application — all of which are
  direct evidence, but the implementation itself was not read.
* **Herd Pro's service install** was not exercised: it requires a licence. The
  *shape* of the feature is known from the CLI's own descriptions and from the
  ordinary (non-Pro) services.
* **Expose** was not run. It requires an Expose account; the vendored client
  and the settings page were inspected instead.
* The **Windows Defender exclusion** and the **HerdHelper service** are
  machine-level integrations that need elevation; they were read from the
  settings screen, not performed.
