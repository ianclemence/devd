# Changelog

All notable changes to `devd` are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because `devd` is pre-1.0, each `0.x` release is a milestone, and a milestone
may change capability identifiers where the design was wrong.

---

## Unreleased

### Added

- **Requirements, asked for at the right moment.** One capability (`setup.requirements`)
  lists what devd needs on this machine - Git, the PHP runtime, the services it
  manages, a model provider and its key, and the WebView2 runtime the native
  window needs - with the state of each, why it matters, and the exact operation
  that fixes it. The Windows installer asks before installing the WebView2
  runtime, because without it the window cannot open; `devd setup` prints the
  whole list and says what this run will install; the window shows it on the home
  page and in Diagnostics and installs a missing piece through the ordinary
  approval flow; the terminal dashboard lists it and installs the first one with
  `i`. Nothing is installed silently, and an optional thing that is missing never
  makes devd look broken. See `docs/setup.md`.
- **Model providers, completed.** Presets for Anthropic, OpenAI, DeepSeek, Moonshot AI
  (Kimi), Zhipu AI (GLM), Google Gemini, xAI, Mistral, Groq, Together, Fireworks,
  OpenRouter, Ollama, LM Studio and any OpenAI-compatible endpoint, in that order of
  usefulness. Each preset carries where to get a key, how its models are discovered
  (the vendor's own list, Ollama's native `/api/tags`, or a maintained catalogue for
  vendors without a listing API), and whether it runs locally. `provider.verify` runs
  one real request that must call a named tool, so "tool use works" is only claimed
  when the provider actually returned a tool call. `provider.discover_local` finds a
  running Ollama or LM Studio on this machine. Errors now say what is wrong: a
  rejected key, an exhausted quota, an unpaid balance, a rate limit with its retry
  window, a context window that is too small, or a model the account cannot use.
- **Agent team** (`team.*`): describe a goal, a planner proposes tasks with owners,
  dependencies and acceptance criteria, you approve the plan, and specialists
  (architect, researcher, implementer, QA, reviewer, infra, docs) work in parallel
  where it is safe, in isolated git worktrees that are merged only after the task is
  accepted. A task is accepted only when the tool results prove it (files changed
  and a verification that passed, where the role requires it); otherwise it is
  `unverified` and the run pauses for you. Runs persist and resume after a restart.
  GUI (Assistant > Team runs), terminal dashboard (Team and Inbox views), CLI
  (`devd team`) and API share one backend. See `docs/agents.md`.
- **Skills** (`skills.*`): 55 playbooks, techniques and principles agents load on
  demand, plus your own in `<home>/skills/`. `devd skills`.
- **Built-in assistant** with provider management (Anthropic, OpenAI, Gemini, local
  runtimes, any OpenAI-compatible endpoint), official Claude model discovery from
  Anthropic's documentation, per-key model lists, model choices per kind of work
  with fallbacks, an approval queue and an inbox, `docs.fetch` and `search.query`.
- **Operational health model** (`health.status`): healthy, degraded, starting,
  setup incomplete, unknown, critical, per component, with what each means and the fix.
- **Services already on the machine** (`services.inventory`, `services.decide`):
  installed, running, required and managed are kept apart; adopt, observe, use your
  own instance, or ignore. Nothing is replaced as a side effect.
- **Production mode** (`environment.set`): every agent change needs approval.
- Global search and command palette; rebuilt Home; 12-entry sidebar with tabs;
  new logo asset that fits its container exactly.

### Added (earlier in this release)

- **A shared stack engine** (`devd-stacks`). Stacks are structured YAML definitions
  validated when they load: 26 stacks (Next.js, Nuxt, Astro, React/Vite, Vue,
  SvelteKit, Express, Node.js, Laravel, Symfony, CodeIgniter, CakePHP, Slim, Django,
  Flask, FastAPI, Python, Rails, Go, Rust, Spring Boot, ASP.NET Core, Expo, React
  Native, plus detect-only Ruby and Java), 40 tools and 14 package managers. Each
  stack carries its official documentation, version requirements, options mapped to
  the scaffolder's real flags, tasks, services, conventions and where its commands
  were verified. Add a stack by dropping a YAML file in `<devd home>/stacks/`.
- **One initialisation engine** behind `project.init` for the CLI (`devd new`, with a
  guided flow), the terminal dashboard (Create view), the window (Create view), the
  HTTP API and MCP: validation with field-level issues, prerequisite and version
  checks, exact commands shown first, progress events, cancellation, post-creation
  verification (files, git, a build or check command) and honest `ok`/`partial`/
  `failed`/`cancelled` results.
- **Existing projects**: `project.detect` weighs manifests, lockfiles, entry points and
  config to report stack, package manager, required runtimes, tasks, services,
  environment variable names (never values) and missing tools; `project.import`
  registers a checkout without touching it. Ambiguity is reported, not hidden.
- **Task runner**: `project.tasks`, `project.run`, `project.exec`, background dev
  servers with logs (`task.*`). Destructive tasks and commands need a human's
  approval and are refused to agents.
- **Toolchain management**: `toolchain.*` lists, plans, installs, removes and
  controls tools and services (Git, Node.js, PHP, Composer, Python, Ruby, Go, Java,
  .NET, Rust, MySQL, MariaDB, PostgreSQL, Redis, SQLite, Docker, GitHub CLI, adb,
  Android SDK, EAS and more) through the host's package manager. Success is verified
  by running the tool and checking service health, never by an exit code.
- `docs/stacks.md` (generated compatibility matrix, tested for staleness) and
  `docs/stacks-engine.md`.

### Changed

- **One design system across the window, the browser and the terminal.** The web and
  desktop interface is rebuilt on tokens: a single spacing scale, a type scale with
  a defined role for every size, semantic status colours that are always paired with
  a word, and one set of control styles for buttons, inputs, pills, tabs, tables and
  dialogs. The shell has an icon rail that collapses to icons on narrow windows and
  becomes a scrollable strip on a phone; every page has one heading, one action row
  and a content width that stops stretching on a wide display. The agent conversation
  is the workspace it should be: messages, tool executions, results and errors each
  have their own treatment, tool output is collapsed by default and can be copied,
  and the composer keeps the project selector on small screens. Tables scroll
  themselves instead of the page. Focus is visible, dialogs keep Tab inside them,
  the conversation announces new replies once instead of re-reading itself, and
  reduced-motion and forced-colour modes are honoured.
- **The terminal dashboard** now has a header that shows the repository state beside
  the identity, a footer that is cut to the terminal width rather than wrapping, empty
  panels that say which key to press, rounded frames, and `NO_COLOR` support.
- New visual identity: midnight-blue surfaces, azure/cyan/violet accents and a
  halftone dot-matrix logo, in the window, the terminal dashboard and the desktop
  icons.
- The HTTP API runs capabilities off the async workers so long operations never block
  the requests that poll their progress.
- Agent sessions start with `DEVD_PROJECT_DIR` and a hint to call `project.detect`.

### Fixed

- `devd-core` did not compile on Linux (a Windows-only function was called
  unconditionally).
- Port checks now work on hosts without `ss` by reading `/proc/net/tcp`.
- Processes started by devd get their own process group on Unix so stopping them
  stops their children too.


## [0.8.0] - 2026-10-20 - persistent agents, project processes, shared state and a native shell

The v0.7 roadmap is complete: sessions survive the window, agents can delegate
real work to other agents, projects declare their processes, people and agents
share durable notes and todos, and the application is a native desktop app.

### Added

**Persistent agent sessions**

* Every session has a stable id, harness, working directory, owner process,
  dimensions, lifecycle state, exit code, timestamps and a bounded transcript on
  disk. Metadata lives in SQLite (schema version 2).
* The backend owns the PTY. Switching views, opening Preview, switching tabs or
  reloading the page does not stop or restart the harness. Reopening devd
  restores the session list and reattaches to a running session.
* A session list, tabs, rename, stop, dismiss and transcript replay for completed
  and failed sessions. Output is replayed from a 2 MiB live buffer or the last
  256 KiB of the transcript, with a byte cursor and a reset signal.
* Honest reconciliation: a row whose owning process is gone is `stale`; a row
  owned by another live devd process is `unavailable`; neither is reported as
  running and attachable.

**Multi-agent orchestration**

* `agent.spawn`, `agent.wait`, `agent.status`, `agent.list`, `agent.result` and
  `agent.cancel`. A task is a real child PTY running its own harness, not a
  metadata record.
* Parent/child task relationships, depth limit 3, 8 unfinished tasks, wait
  timeout 900 s, task timeout 3600 s, structured results and non-zero exit
  reporting. A wait timeout does not kill the child.
* Claude Code tasks use `claude -p`; OpenCode tasks use `opencode run`; custom
  tasks take an argv array. The child's own permission prompts remain the
  authority.

**Project processes**

* Versioned `devd.yml` with strict validation: argv commands, project-contained
  working directories, loopback-only health checks, explicit environment,
  dependencies, restart policy, startup ordering and unknown-field rejection.
* `project.plan`, `project.approve` (human-only), `project.status`,
  `project.start`, `project.stop`, `project.restart` and `project.logs`.
* A semantic manifest hash must be approved before anything runs; editing the
  manifest invalidates the approval and blocks automatic restarts.
* Supervised processes with captured stdout/stderr, pids, exit codes, restart
  counts, health status, searchable log tails, secret redaction and whole-tree
  termination.

**Shared state**

* Project-scoped scratchpads with authorship, timestamps, optimistic revisions
  and `scratch.append` for agent findings.
* Structured todos with status, priority, assignee, task association,
  dependencies, transition history, completion, reopening and notes.
* `scratch.*` and `todo.*` capabilities are available to humans and agents;
  deletes are destructive and require human approval.

**Native desktop**

* `devd-desktop`, a Tauri 2 window over the existing server and UI. The backend
  starts in-process on an ephemeral loopback port; the window loads the same HTML
  and JavaScript.
* No Tauri commands, no shell/filesystem plugins, no remote IPC access. Top-level
  navigation is restricted to the devd origin; external links go to the system
  browser.
* Graceful shutdown stops PTYs and project processes.

**UI**

* **Agents** with session list, tabs, lifecycle pills, rename/stop/dismiss and a
  delegated-task panel with Wait, Cancel and Result.
* **Processes** with manifest review and approval, per-process start/stop/restart,
  health, pid, restart count, failure detail and filtered logs.
* **Shared** with todos and scratchpads, revision-aware saving and conflict
  reporting.
* Responsive layouts for narrow windows; the terminal remains usable when the
  session list stacks above it.

### Changed

* The CSP allows inline *style* so xterm.js can position its screen; scripts
  remain same-origin only, with inline scripts still forbidden.
* Manifest working directories strip Windows' `\\?\` prefix before they reach a
  child process, so `cmd.exe` and harnesses start in the project rather than
  falling back to `C:\Windows`.
* The registry composition moved into `devd-bootstrap`, shared by the CLI and the
  desktop shell.

### Security

* Sessions record their owning process; only the owner may write, resize or stop.
* Terminal input/resize/rename/dismiss are human-only capabilities.
* Project manifests are executable configuration and require a human approval of
  their semantic hash.
* Project environments are explicit; the daemon's own credentials are not
  inherited.
* The new capabilities are subject to the existing tier, approval, agent-safety
  and audit rules.

### Fixed

* A ConPTY session could remain "running" for ever when its child exited but the
  reader did not reach EOF; a separate child watcher now records the true exit.
* `cmd.exe` treated canonical `\\?\` paths as UNC and started in the Windows
  directory.

### Known limitations

* A PTY cannot survive the devd process that owns it. On restart the row becomes
  `stale` and only its transcript remains.
* MCP runs in its own process, so an agent-spawned session is owned by that
  process, not the window. It is genuine, and it is not attachable from the
  window.
* Project processes are owned by the process that started them; a second devd
  reports them as supervised elsewhere rather than adopting them.
* The Windows installer is not code-signed and the Tauri window requires WebView2.

---

## [0.7.0] - 2026-10-20 — agents in the window, and a built-in browser

The application window now runs the coding agent you already use and shows the
site you are building, without leaving the app.

### Added

**Agents**

* An **Agents** view that starts Claude Code, OpenCode or a plain shell in the
  selected project directory.
* A real PTY per terminal: full-screen harness interfaces, colours, keyboard
  shortcuts and resizing work. Output is polled over the loopback API; keystrokes
  go back the same way.
* The harness runs with its own models, credentials and permission prompts.
  devd does not proxy the model, store the conversation or mediate file edits.
* Harness detection is honest: a card says "not installed" when the CLI is not
  on `PATH`, and nothing pretends to be the tool.

**Preview**

* A **Preview** view that embeds the running site in the window, with a site
  picker, reload and open-externally.
* The iframe is sandboxed, so the previewed page cannot navigate the window.

**Design**

* A new palette: warm graphite surfaces and a single terracotta accent instead
  of the blue-grey default, with larger page titles and a calmer rail.
* The terminal theme matches the window.

### Security

* Every terminal endpoint sits behind the same bearer token and loopback
  defences as the rest of the API. The working directory must exist.
* The preview iframe is sandboxed (`allow-scripts`, `allow-same-origin`,
  `allow-forms`, `allow-popups`, `allow-downloads`, `allow-modals`) and cannot
  navigate the top window.
* The CSP now allows framing HTTP(S) for the preview and still denies
  everything else by default.

### Known limitations

* One agent terminal at a time; reloading the window loses the handle while the
  PTY keeps running.
* Terminal output is polled, not archived.
* Preview refuses sites that send `X-Frame-Options` or `frame-ancestors`.

---

## [0.6.0] - 2026-10-20 — the server dashboard

The application window now shows the machine, not just repositories: the
services, the sites, the runtimes, the certificates and the hosts file, with
controls for each and a live overview.

### Added

**The server views**

* **Overview**: a status banner (running, ports, start/stop all), four counters
  (services, sites, PHP, certificates), and panels for sites, services, hosts
  and the certificate authority.
* **Sites**: the table (URL, PHP, TLS, directory), a create form with hostname,
  HTTPS and database options, open and remove. Removal is destructive and goes
  through the approval dialog.
* **Services**: version, ports, state and PID per service, with start, stop,
  restart, logs in a drawer, and remove. Missing services offer install.
* **Runtimes**: installed PHP, Node.js and Go with their primary marker,
  "install latest", "use for new sites" and remove.
* **Certificates**: the local authority, its trust state, trust for the user or
  the machine, remove trust, and the issued certificates.
* **Hosts**: the managed file, pending additions and removals, what is
  published, and sync behind the approval dialog.

**`tls.ca.status`** reports whether the authority is in the user and machine
stores, by asking `certutil` rather than remembering it in the browser. The
overview's trust notice is now a fact rather than a guess.

**Design pass**, following Emil Kowalski's design engineering principles: press
feedback on buttons (`scale(0.97)`), a visible focus ring on every control,
custom easing curves instead of the built-in ones, no entrance animation on the
command palette (it is opened from the keyboard many times a day), a 200ms
entrance for the modal, tabular numerals for ports and counts, and a responsive
three-to-two-to-one column grid.

### Fixed

* **The environment check took 40 seconds.** Every liveness check and every port
  lookup started a `tasklist` process; a machine with a few dozen listeners
  meant a few dozen of them. Process state and image names are now asked of the
  kernel (`OpenProcess`, `GetExitCodeProcess`,
  `QueryFullProcessImageNameW`), and the listener table is read once per check.
  `devd doctor` went from ~45s to ~3s, and the service list from ~5s to under
  100ms.
* The hosts block no longer proposes entries for `localhost` and its
  subdomains, which resolve without the file. `hosts.show`, `hosts.sync` and the
  overview therefore agree with `site.create`.

### Changed

* The graphical surface is described as one application window, not a website.
  The loopback API remains as the window's transport and for scripts; a native
  webview wrapper is the planned shell, with no second UI implementation.

---

## [0.5.1] - 2026-10-19 — a Windows installer

`devd` now installs like an application: download one executable, click through
the installer, and the app opens with a site already served.

### Added

**The installer (`devd-setup-<version>.exe`, `installer/devd.nsi`)**

* Per-user install under `%LOCALAPPDATA%\Programs\devd`; no administrator prompt.
* Adds `devd` to the user's `PATH`, creates Start Menu entries (the application,
  a terminal, the uninstaller) and an optional desktop shortcut, and registers
  itself in *Apps & features*.
* The last page runs `devd setup` in a console with visible progress, then opens
  the application without a console window.
* Uninstall stops the services, removes files, shortcuts, `PATH` entry and
  registry key, and keeps the data directory.

**`devd setup`**

* Idempotent first-run provisioning: install PHP, nginx and MariaDB when
  missing, start the stack, create the first site, trust the CA for the current
  user with `--yes`.
* If ports 80/443 are held by another program it moves the listeners to
  8080/8443 and persists the choice.
* The first site is `localhost`, so it needs no hosts entry and no elevation.

**`scripts/release-installer.ps1`** builds the installer with NSIS; the release
scripts are local by decision, with no CI pipeline.

### Fixed

* `www_root` is honoured when creating sites and by the `--delete-files` guard;
  it previously only affected the doctor check.
* `dns_mode` and `auto_start_services` were serialised into `settings.toml` and
  read by nothing; removed.
* A site whose host is `localhost` no longer warns that its name does not
  resolve.

### Changed

* The README leads with the installer; the standalone binary and the source
  build are the alternatives.

---

## [0.5.0] - 2026-10-19 — M4: a real site over HTTPS

The fifth milestone is the first one where `devd` does what the product is for:
it installs a runtime, installs a web server and a database, creates a site, and
serves it over TLS with a chain that verifies. Everything is downloaded from the
vendor, checked against a digest, and started and confirmed by the same
capability dispatcher that has been underneath every surface since M0.

### Added

**Runtimes (`devd-runtime`)**

* `runtime.available`, `runtime.install`, `runtime.remove`, `runtime.verify`,
  `runtime.list` and `runtime.set_primary`, plus `devd runtime …` on the CLI.
* PHP from `windows.php.net`'s `releases.json`, Node from `nodejs.org/dist` with
  its `SHASUMS256.txt`, Go from `go.dev/dl/?mode=json`. Downloads are in-process
  (`ureq` + `rustls`), digest-verified, extracted in-process (`zip`, `tar`,
  `flate2`), and moved into place only after the digest matches. Nothing is
  bundled and no external `curl` is used.
* PHP is configured after it is unpacked: the curated extension list is
  intersected with the DLLs the build actually ships, `opcache` is enabled as a
  `zend_extension`, the interpreter is run, anything that reports "Unable to
  load dynamic library" is commented out, and it is run again. The install
  reports what was enabled and what was switched back off.

**Services (`devd-services`)**

* Nine service capabilities — install, start, stop, restart, status, list, logs,
  remove and available — for nginx and MariaDB, plus `web.render`, `web.test`,
  `web.reload`, `stack.up`, `stack.down`, six `tls.*` and three `db.*`.
* nginx from nginx.org; MariaDB from `archive.mariadb.org` with the per-file
  SHA-256 from MariaDB's release API. MariaDB is initialised by its own
  `mysql_install_db.exe`, without registering a Windows service, with its data
  directory outside the install directory.
* `php-cgi` as a service built from the installed PHP runtime, and `stack up`
  creates it if it is missing.
* Detached supervision: a pid file plus a process-image check, and a start is
  confirmed by the port opening before the command reports success. A failure
  kills the process and returns the tail of its own log.
* `nginx.conf` rendered from the site table: a PHP upstream, a default server
  that returns an explanatory 404, per-site HTTP→HTTPS redirect and TLS server
  blocks, and per-host access logs. Config content uses forward slashes; `argv`
  uses native paths.

**Sites and TLS (`devd-sites`, `devd-services::tls`)**

* `site.list`, `site.create`, `site.remove`, `hosts.show` and `hosts.sync`.
* An internal ECDSA P-256 certificate authority created on first use, leaves
  valid 397 days and a full chain for nginx, all issued in-process with `rcgen`.
  The CA key never enters a response or a log.
* `tls.issue`, `tls.list`, `tls.ca.show`, `tls.trust`, `tls.untrust` and
  `tls.leaf.remove`; trusting the CA is `Tier::Admin` and uses `certutil` on
  Windows or `trust anchor` on Linux.
* A managed hosts block that snapshots the file, preserves every line it did not
  write byte for byte (including CRLF endings), and is idempotent.

**CLI**

* `devd runtime`, `devd service`, `devd site`, `devd hosts`, `devd web`,
  `devd stack`, `devd tls` and `devd db`, each mapping one-to-one onto
  capabilities. 98 capabilities in 20 groups, up from 64 in 12.

### Safety

* Hostnames are validated before they can reach a config file or a path;
  `site.remove` refuses to delete a directory outside the `www` root.
* Publishing a name and trusting a CA are separate, explicitly approved
  capabilities — never side effects of creating a site.
* The database password is passed to the one-time bootstrap on its command line
  and to clients through `MYSQL_PWD`, and is never written to a file `devd` owns.
  `root` is passwordless and bound to loopback by default, which the install
  warns about.
* A fresh MariaDB data directory has its world-writable `test` schema removed.
* nginx publishes no digest; its install says so rather than implying a
  verification that did not happen.

### Fixed

* `Path::with_extension` collapsed `certs/blog.test.crt` and
  `certs/blog.test.key` onto `certs/blog.test`, so nginx was told to load a
  certificate that `devd` had never written and refused to start.
* A detached service inherited `devd`'s stdout, so `devd stack up | Tee-Object`
  printed its result and then hung until it was killed. `devd` now clears the
  inheritance flag on its own standard handles before spawning a service.
* The debug binary overflowed the 1 MB Windows main-thread stack while parsing
  the grown command line, so even `devd --help` died before running. The stack
  reserve for this target is now 8 MB, matching Linux.
* `mysql_install_db.exe` was given `--basedir`, which it does not accept; it
  exited 7 and created nothing.
* `mysql --batch` printed a header row, which made `schema_name` appear as a
  database in `db list`.
* The MariaDB catalogue resolved `latest` to a preview series with no Windows
  package; it now reads MariaDB's release API, offers only Stable series that
  have a `winx64.zip`, and knows each file's digest before downloading it.
* The MariaDB provisioning report — data directory, whether it was initialised,
  the password warning — was collected at install time and then dropped.
* `php-cgi` was reported as "unknown version" by `service list` and `doctor`.

### Tests

See the line below; the numbers are from the run recorded in
[`docs/releases/v0.5.0.md`](docs/releases/v0.5.0.md). The suite covers the
generated vhost and its certificate paths, the hosts block's preservation and
idempotence, hostname validation before a write, the refusal to delete outside
the `www` root, the chain verifying against the CA, and re-issuance replacing the
leaf without replacing the CA. The live downloads are `#[ignore]` tests.

### Known limitations (at 0.5.0)

* Apache, Redis and Mailpit are not implemented; there is no DNS server, SSH or
  deploy.
* Services are Windows-only: the vendor archives used here are Windows builds.
* nginx has no published digest.
* One `php-cgi` listener serves every site, so per-site PHP versions are recorded
  but not routed.
* The application and the dashboard have no Runtimes, Services or Sites panels.

---

## [0.4.0] - 2026-10-19 — M3: the terminal dashboard

The fourth milestone adds the surface that works where the others cannot: a
console with no browser to open, and an SSH session with nothing forwarded. It is
the third projection of the same registry, and it changes nothing about how the
product decides what may happen.

### Added

**The terminal dashboard (`devd-tui`, `devd tui`)**

* Nine views — Dashboard, Projects, Changes, History, Branches, Capabilities,
  Activity, Doctor, Help — reachable by digit (`1`–`9`) or by `Tab`.
* Git actions from the keyboard without a second implementation: `s` stage, `u`
  unstage, `a` stage all, `c` commit, `Enter` toggle a file in or out of the
  index, `n` new branch, `t` track a repository by path, `/` search the registry,
  `x` run any capability with arguments typed as JSON.
* A frame is a pure function of the state, which is what lets the tests render
  one and assert on the buffer.
* **An approval prompt that is the operation.** `d` (discard) is
  `Tier::Destructive`, so the dashboard asks the registry for the plan, shows it
  verbatim, and keeps the `InvokeRequest` it built. `y` replays *that* request,
  never a rebuild from the screen. `n` dispatches the same request as refused, so
  the refusal is recorded rather than discarded — the same two-step principle as
  the CLI and the application window.
* **A branch switch that would lose work becomes a confirm**, with the list of
  files at risk taken from `git.switch.check`.
* `devd tui` refuses to start when stdin or stdout is not a terminal, and says
  what to use instead. Entering raw mode on a pipe killed the process with
  `0xc0000005` and no output on Windows; that is now a sentence and exit code 2.

### Safety

* **Control characters are neutralised before they reach the terminal.** A branch
  name or path containing an escape sequence is a command being sent to the
  reader's console, not text. One function replaces them, and a test renders a
  branch named with `ESC [ 2 J` and asserts nothing reached the buffer.
* **A modal owns every key**, so a keystroke cannot both answer a prompt and act
  on the view behind it. `Esc` on an approval means *no*, not *cancel the
  question*.
* **The dashboard has no path to a shell.** It calls capabilities, and
  capabilities are typed structs turned into `argv` arrays.
* **A plan is not counted as an action.** The Activity view labels a dry run
  `plan`, an application `applied` and a refusal `refused`.

### Changed

* The footer's first line is the message line and starts empty, rather than
  defaulting to a hint; each view's key hints now carry the common tail.

### Fixed

* **The release binary linked by Zig crashed at startup.** On this
  `x86_64-pc-windows-gnu` host the final link goes through a `cc` that forwards
  to Zig, and the result faulted at `0xc0000005` inside its own code before
  `main` ran — release only, with no output, while debug and the test suite were
  fine. `.cargo/config.toml` now pins `linker = "rust-lld"` for that target
  alone, with the reason recorded beside it. C sources are still compiled by
  `cc`, and MSVC builds are unaffected.

### Tests

**226 tests pass** (up from 169), zero warnings, in debug and in release. The 57
new ones are in `devd-tui`: 52 unit tests driving the controller with real
keystrokes against a backend that counts what ran, and 5 integration tests
against the real registry and a real repository — including that staging from
the dashboard changes the real index, and that a declined discard leaves the file
on disk with the refusal in the audit table.

---

## [0.3.0] - 2026-10-18 — M2: the agent surface

The third milestone adds the Model Context Protocol server. It is the third
projection of the same registry, after the CLI and the application window, and it
makes the product's premise checkable: an agent driving `devd` reaches the same
functions as a person, with the same permission checks.

### Added

**The MCP server (`devd-mcp`)**

* `devd mcp` serves the Model Context Protocol on stdin/stdout;
  `devd mcp --client <name>` names the caller in the audit trail;
  `devd mcp --list` prints the tool list.
* **Five discovery tools instead of one tool per capability**:
  * `devd_groups` — the domains and their sizes;
  * `devd_capabilities` — search by text, by domain, or both, with a true total;
  * `devd_describe` — one capability, with the JSON Schema of its arguments;
  * `devd_invoke` — run one by id, optionally as a dry run;
  * `devd_doctor` — the environment check.
  A capability added later is discoverable and invocable without changing this
  crate, and a test asserts that two servers over registries of different sizes
  publish an identical tool list.
* The protocol handles `initialize`, `ping`, `tools/list`, `tools/call`, and
  answers `resources/list`, `resources/templates/list` and `prompts/list` with
  empty lists, because clients probe them. A notification is never answered. An
  unknown method is `-32601`; an unknown tool names `tools/list`; a missing
  required argument is `-32602`; malformed input is `-32700`.

### Safety

* **The actor is forced.** `McpServer::new` discards the actor in the context it
  is handed and installs an agent actor, so a caller cannot present a human
  context and inherit a human's authority. A test asserts it.
* **A refused operation hands the agent its plan.** When an approval-gated
  operation is refused, the server asks the registry for a plan — itself a dry
  run, recorded as a plan rather than an application — and returns it with the
  refusal, the error code, and an explicit instruction not to report the
  operation as having happened. That instruction exists because the failure mode
  being defended against is an agent that confidently describes work it was not
  allowed to do.
* **A capability that cannot describe itself gets no invented plan**, and the
  message says so. Both branches are covered by tests.
* `human_only` is refused for an agent before approval is considered, so an
  approval could never unlock it.
* Tool results are data, never instructions: the only text that reaches a model
  is written by the registry or by this crate, never echoed from repository
  content.
* The refusal tests assert against a counter incremented by the handler, so
  "nothing ran" is a measurement rather than an inference.

### Changed

* The Git doctor check moves from `devd-cli` to `devd-git`
  (`devd_git::doctor_check`), so the CLI, the application and the MCP server
  share one implementation instead of three that could disagree.
* `App::registry` is now an `Arc<Registry>`, because two surfaces move it into a
  long-lived task while the CLI still holds the application.

### Tests

169 tests, all passing (up from 148):

* `devd-mcp`: 21 — the protocol, the five-tool surface and the assertion that it
  does not grow with the registry, the forced actor, discovery, and the refusal
  paths (destructive, `human_only`, cannot-plan, dry run, unknown capability,
  unknown tool, missing argument) plus the doctor.

Verified over a real pipe against the built binary as well: the transcript and
the resulting audit rows are in [`docs/mcp.md`](docs/mcp.md#observed).

### Known limitations (at 0.3.0)

* The TUI is still not built.
* Services, sites, runtimes, DNS and TLS remain designed only.
* There is no conversational agent inside the application; an agent connects from
  outside over MCP, which is the same registry either way.

---

## [0.2.0] - 2026-10-18 — M1: the application

The second milestone turns the core into something a person can actually use: a
local application served by the `devd` binary on loopback, opened as a
chrome-less window. It adds a surface, not a second implementation — the server
is generic over the registry, and the interface is a pure client of it.

### Added

**The local application (`devd-server`)**

* `devd ui` starts an HTTP API on `127.0.0.1` and serves the interface from
  inside the binary, then opens it in an application window (Chromium in
  `--app` mode where available, otherwise the default browser). `devd serve`
  runs the API headless.
* If a `devd` instance is already running, `devd ui` probes for it, through the
  `X-Devd-App` response header, and **reopens it** rather than starting a second
  server. A port held by an unrelated program is left alone and the next port is
  tried. The chosen port is remembered in the settings table.
* Twelve pages, all composed in the browser from `/api/invoke` calls: Projects,
  a project workspace with Changes, History, Branches, Tags, Remotes, Stashes,
  Worktrees and Recover, plus Capabilities, Activity and System.
* A `Ctrl+K` command palette backed by the registry's own ranked search
  (`/api/capabilities?q=`), which runs read-tier capabilities inline and opens
  the inspector for everything else.
* Per-file diff rendering, hunk-level staging and reversing, commit with amend, a
  credential-looking-staged-file warning, branch switching that consults
  `git.switch.check` first, conflict resolution, the reflog, reset modes, squash,
  bisect, and continue/abort for an interrupted operation.
* An approval modal that renders the server's plan with a countdown, a red
  confirm button for destructive and remote tiers, and the arguments shown as
  the server recorded them.
* A **Quit devd** action calling `POST /api/shutdown`, so closing the window and
  stopping the process are not the same confusing thing.

**Two-step approvals over HTTP**

* `POST /api/invoke` returns `{ approval_required, plan, token, tier,
  expires_in_ms, args_hash }` and **changes nothing**.
* `POST /api/approve { token }` replays the **server's own stored arguments**
  with `Approval::Granted`. The client cannot assert an approval.
* `POST /api/deny { token }` dispatches with `Approval::Denied`, so a refusal is
  audited rather than invisible.
* A token is single-use, lives 120 seconds, is bounded to a fixed number of
  pending decisions, and is bound to the capability, the arguments and the actor.

**Loopback hardening**

* `Host` allow-list (`127.0.0.1:P`, `localhost:P`, `[::1]:P`) that defuses DNS
  rebinding, refused with `403`.
* `OPTIONS` is refused with `405` before any other check, so no cross-origin
  caller can complete a preflight.
* A foreign `Origin` is refused with `403`.
* A 256-bit session token, generated at startup, injected into the served HTML
  and required on every `/api` request with a constant-time comparison. It is
  never written to disk and no endpoint returns it.
* The actor for every request is forced to `human:http` by the server; a caller
  cannot choose its own actor.

**The interface itself**

* Three files — `index.html`, `app.js`, `style.css` — embedded with
  `include_str!`. No build step, no framework, no `node_modules`.
* A Content-Security-Policy of `default-src 'none'` with same-origin script,
  style and connect, and a test asserting the shell contains no inline script,
  no `style=` attribute and no `on*=` handler.
* Dark, calm, keyboard-first; monospace for machine data; reduced
  `prefers-reduced-motion` respected.

**Shared environment checks**

* The doctor checks move into `devd-core::doctor`, so the CLI and the server
  report the same thing from the same code. `devd`'s contribution is the Git
  check; the server appends nothing the CLI lacks.

### Changed

* `Surface::can_prompt()` distinguishes a surface that can *answer* a prompt from
  one that is a *person*. The application can answer prompts without being
  treated as an interactive human, and an agent cannot.
* `Store::open_in_memory()` now migrates, so an in-memory store is usable
  immediately instead of failing every table read.
* A Git command that fails now reports what Git said on **stdout** when stderr is
  empty. Git writes "nothing to commit" and similar to stdout, so the previous
  behaviour turned a real, explainable cause into an empty message.

### Safety

* A destructive operation over HTTP returns a plan and changes nothing until it
  is approved; the arguments that run are the ones the server stored, never the
  ones the client sent a second time.
* A declined operation is recorded with the actor, the tier, the redacted
  arguments and the reason, and its token cannot be replayed.
* The application never receives a capability it could not reach from the CLI,
  and the CLI never gains a rule the application lacks.

### Tests

148 tests, all passing (up from 98):

* `devd-server` unit: 17, covering the approval store (single use, expiry,
  capacity, binding to capability and arguments) and the request guards.
* `devd-server` HTTP: 22, driving the real router: the four loopback defences,
  the token requirement, capability search and description, an approval-gated
  invocation that changes nothing until approved, a decline that is audited and
  applies nothing, token replay, and the error envelope.
* `devd-cli`: 4, including port selection.
* `devd-core` and `devd-git` as before, plus the new doctor and failure-reporting
  tests.

Verified against the running release binary as well, not only in tests: the
security matrix and the Git approval flow in
[`docs/app.md`](docs/app.md#what-was-actually-observed).

### Known limitations (at 0.2.0)

* The TUI and the MCP server are still not built.
* Services, sites, runtimes, DNS and TLS remain designed only, and the
  corresponding panels say so instead of offering controls that do nothing.
* The window is a browser engine in application mode, not a native shell. Tauri
  remains the intended eventual wrapper; the UI code does not change when that
  happens, because it speaks only to the local API.
* A capability's arguments are edited as JSON in the inspector rather than by a
  generated form.

---

## [0.1.0] - 2026-10-18 — M0: foundation and Git

The first milestone. It delivers the part of the product that everything else
will be built on — the capability registry and its permission model — together
with one skill area, Git, implemented end to end so the model is proven against
a real, stateful, destructive domain rather than a toy one.

### Added

**The core (`devd-core`)**

* A capability registry. A capability carries an id, a group, a summary, a
  description, a permission tier, a side-effect class, a JSON Schema for its
  arguments, and flags for dry-run support, agent safety, human-only
  reservation, idempotence and reversal.
* Five permission tiers — `Read`, `Write`, `Exec`, `Destructive`, `Remote` —
  with approval required for the last two.
* A single dispatcher (`Registry::dispatch`) that every surface calls. It
  enforces, in order: an explicit decline, the human-only reservation, the
  agent-safety reservation, the tier's approval requirement, and dry-run
  support — before the handler runs.
* An audit trail. Every attempt is recorded with the actor, the surface, the
  tier, the redacted arguments, the outcome and the message, including attempts
  that were refused.
* Argument redaction, so a secret that reaches an interface never reaches the
  audit file.
* A SQLite-backed state store with a versioned schema, integrity checking and an
  in-memory mode for tests.
* Path layout, layered settings, an event bus, port allocation, health
  probing and a process runner that kills whole process trees.

**Git (`devd-git`)**

* A typed Git service over the user's own `git` executable. Every call is an
  `argv` array; nothing is ever assembled into a shell string, and revisions,
  refs, relative paths and remote URLs are validated before use.
* 64 capabilities in 12 discoverable groups:
  * `git.repo` — init, open, track, untrack, list, describe.
  * `git.changes` — status, diff (working tree, index, path, revisions,
    diffstat), stage, unstage, hunk-level staging and reversing from a patch,
    discard, ignore.
  * `git.commits` — create, amend, allow-empty, squash.
  * `git.history` — log, show, blame, search, reflog.
  * `git.branches` — list, create, rename, delete, switch, switch safety check,
    upstream, prune merged.
  * `git.tags` — list, create (annotated or lightweight), delete, local or
    remote.
  * `git.remotes` — list, add, remove, rename, fetch, pull, push, force push,
    sync preview, outgoing, incoming.
  * `git.merge` — merge, rebase (including full-history and `--autosquash`),
    cherry-pick, revert.
  * `git.stashes` — list, push (including untracked), show, apply, pop, drop.
  * `git.worktrees` — list, add, remove, prune.
  * `git.config` — get, set, with credential-bearing keys refused.
  * `git.recovery` — conflicts list and show, resolve a side, continue, abort,
    reset (soft/mixed/hard), bisect start/mark/reset.
* Safety properties enforced by construction:
  * **Force push is a separate capability** (`git.push.force`), not a flag, so
    no code path can reach a forced update without its own approval gate.
  * **Both `git.push` and `git.push.force` are `Tier::Remote`**, so publishing to
    a machine `devd` does not own needs a human.
  * **Remote URLs that embed a password are rejected**, because such a URL is
    written to `.git/config` in plain text.
  * **Configuration writes to credential-bearing keys are refused**, including
    `*extraheader`, `user.password`, `*.token` and similar.
  * **`LC_ALL=C`** is forced and the pager is disabled, so output parsing is
    stable and nothing blocks waiting for a reader.
  * **Interactive rebase is driven by a no-op sequence editor**, so an
    `--autosquash` rebase cannot hang waiting for a human who is not there.
  * `GIT_TERMINAL_PROMPT=0`, so Git can never stop to ask for a credential.

**The CLI (`devd-cli`)**

* `devd capabilities`, `devd describe`, `devd call`, `devd audit`,
  `devd doctor`.
* A Git command set that keeps Git's vocabulary: `status`, `diff`, `stage`,
  `unstage`, `discard`, `commit`, `log`, `branch`, `switch`, `fetch`, `pull`,
  `push`, `clone`, `merge`, `stash`, `worktree`, `remote`, `tag`, `conflicts`,
  `continue`, `abort`, `reflog`, `undo`, `revert`, `reset`, `cherry-pick`,
  `rebase`, `squash`, `bisect`, `repo`.
* `--plan` to see a plan and change nothing; `--yes` as the explicit,
  deliberate approval for a non-interactive run; `--json` for machine output.
* Stable exit codes: 2 invalid, 3 approval required or denied, 4 not found,
  5 conflict, 1 otherwise.

### Safety

* An agent cannot approve its own operation. The dispatcher refuses before the
  tier check, and no surface can supply an approval on an agent's behalf.
* Human-only capabilities are refused for an agent even when an approval is
  presented.
* A declined destructive operation is refused and audited, and the working tree
  is untouched. This is asserted by test, not by review.
* An aborted merge or rebase restores the previous state and preserves
  uncommitted work in files the operation did not touch.
* A plan that declares itself destructive now also declares that it requires
  approval; the two can no longer contradict each other.

### Tests

98 tests, all passing:

* `devd-core`: 43, including the registry tests that prove an agent cannot
  perform a human-only operation, that a declined operation never executes, and
  that secrets are redacted from the audit trail.
* `devd-git` unit: 9, covering revision, ref, path and remote-URL validation,
  option-injection refusal, redaction, and the credential-key rule.
* `devd-git` lifecycle: 30, against real repositories in temporary directories —
  the full Git lifecycle, hunk staging, amend, branches, rebase, autosquash,
  squash, conflict detection, resolution, abort and recovery, reset modes, the
  stash lifecycle, the worktree lifecycle, tags, cherry-pick, revert, clone,
  fetch, pull, push, force-push-with-lease recovery, diverged pulls,
  credential rejection, an unreachable remote, ignore rules, and paths and
  repository paths containing spaces.
* `devd-git` policy: 16, driving the registry — declined operations do not run,
  approval is required then honoured, an agent cannot self-approve, a
  human-only capability is refused for an agent, force push is unreachable from
  the ordinary push schema, a dry run changes nothing, destructive plans name
  what they destroy, every attempt is audited, the actor and surface are
  recorded, secrets never reach the audit trail, and a resolved conflict can be
  continued through the registry.

### Known limitations (at 0.1.0)

* Only the Git capability group is registered. Service, site, DNS, TLS and
  runtime capabilities are designed but not implemented.
* There is no TUI, no desktop GUI, no HTTP API and no MCP server yet. The
  capability declarations and JSON Schemas they need already exist.
* No macOS support. Windows and Linux are the target platforms.
* Windows process-tree termination uses `taskkill /T /F` rather than a Job
  Object.
* Git hooks and credential helpers are the user's own; `devd` neither installs
  nor executes hooks from a repository it merely opens.

---

[0.3.0]: https://github.com/ianclemence/devd/releases/tag/v0.3.0
[0.2.0]: https://github.com/ianclemence/devd/releases/tag/v0.2.0
[0.1.0]: https://github.com/ianclemence/devd/releases/tag/v0.1.0
