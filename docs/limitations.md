# Known limitations

This document lists what `devd` does not implement, what it implements
partially, and the platform-specific behaviour worth knowing before relying on
it. Each entry states the consequence and, where one exists, the change that
would close it.

This document describes the current release, v0.8.0. See
[`CHANGELOG.md`](../CHANGELOG.md) for what it contains.

## Not implemented at all

| Area | State |
| --- | --- |
| Apache, Redis, Mailpit | Not started. nginx and MariaDB are the services that exist. |
| DNS server | Not started. `*.test` names resolve through a managed block in the hosts file, one entry per site; there is no wildcard and no query server. |
| SSH control of a remote host, deploy | Not started. |
| Snapshots and receipts | A hosts write snapshots the file first. Nothing else does, and no snapshot can be restored. |
| Licensing / redistribution checks | Not started. |
| Self-update and upgrade check | Not started. Nothing in `devd` contacts GitHub. The release list the mechanism will read is public and reachable (`GET /repos/ianclemence/devd/releases?per_page=3`), and the releases carry the source archives GitHub generates, but no code reads them yet. |
| An in-app conversational agent | Not started. The MCP server is the agent surface today. |
| Prebuilt Linux binary | Not attached to this release: an artifact is published only after it has been executed on its target platform, and the release is built on Windows. `scripts/release-linux.sh` produces one on any Linux machine, or through WSL or Docker on Windows. |
| Runtime, service and site panels in the app and the dashboard | Not built. The capabilities exist and the CLI covers them; the application's capability inspector can invoke them, but there are no dedicated screens. |
| Per-site PHP version routing | A site records the PHP version it was created against, and nginx has **one** FastCGI upstream — the `php-cgi` listener built from the primary runtime. Creating a second site against another version changes the record, not the listener. |
| HTTP/2 | The TLS listener is HTTP/1.1. `http2` is not enabled in the generated vhosts. |

**Consequence.** On Windows, a user can install PHP/Node/Go, install nginx and
MariaDB, create a site, and reach it over HTTPS with a chain that verifies against
`devd`'s own CA. Anything not in that sentence is either Git (M0), the
application (M1), MCP (M2) or the dashboard (M3).

## Windows is the only platform with a service provider

The service catalogues are the vendors' own Windows builds: nginx ships a Windows
zip and no Linux binary, and MariaDB publishes `winx64` archives. On Linux,
`devd service available nginx` therefore returns an empty list and installing a
service reports that no builds are published upstream for this platform.

Runtimes are less affected — Node and Go publish Linux tarballs — but PHP's
installer reads `windows.php.net`, so PHP on Linux is not installable either. The
core, Git, the CLI, the application, MCP and the dashboard run on both platforms;
the service layer is Windows-only today.

Closing it means a Linux provider that uses the distribution's packages (or builds
from source), with the same verification contract: a package is installed only
after its origin and version are known, and `devd` still never writes a shell
string.

## Runtimes and services

| Item | What it is | What it would be |
| --- | --- | --- |
| nginx's download | nginx.org publishes a PGP signature and no checksum file. `devd` checks the length and says, in the install result and here, that it could not check a digest. | Verify the detached PGP signature against nginx's key, which requires a PGP implementation in-process. This is the one honest gap in the digest story; PHP, Node, Go and MariaDB are all verified against a digest the vendor publishes. |
| Service versions | One version per service name. Installing another replaces the directory after the running service is stopped, and `--force` is required to replace an existing install. | Side-by-side service versions, selected per site — which is what the PHP record on a site implies but does not yet deliver. |
| Startup detection | The port opening is the health check, with a per-service window (nginx 10s, MariaDB 60s, php-cgi 10s). A service that opens its port and then fails is not detected. | A real health endpoint per service, and restart-on-failure driven by the supervisor rather than only by the next command. |
| Supervision | Each service is spawned detached and tracked by a pid file, verified against the process image so a recycled pid is not mistaken for the service. There is no daemon holding a job object. | A Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and a small background supervisor, so children die with `devd` even when it is terminated abruptly. Today every ordinary start/stop/restart path is covered; `TerminateProcess` is not. |
| Orphan recovery | If the pid file is lost, `devd` no longer knows the pid of a running service and `service stop` cannot find it. The port check still reports the service as running. | Rediscovery by listening port and image name. |
| Log rotation | nginx access/error logs, MariaDB's error log and the php-cgi log are appended forever. | Size- or age-based rotation, with the service restarted after its log is moved. |
| Downloads | Verified in-process, cached by digest under `cache/`, never resumed and never evicted. A failed download starts again. | Resume with `Range`, and an eviction policy for the cache. |
| MariaDB root | `root@localhost` has no password by default and the server binds to `127.0.0.1`. The install prints a warning; `DEVD_MYSQL_ROOT_PASSWORD` sets one at first initialisation. | A generated password stored in the operating system's credential store, which is a milestone of its own. |
| MariaDB data | `devd service remove mariadb` deletes the binaries and keeps `etc/mariadb` (the data directory), and says where it is. There is no backup or dump capability. | `db.dump`/`db.restore`, and a remove that offers to take a dump first. |
| MariaDB `test` schema | MariaDB's bootstrap creates a `test` database that every local user can write to. `devd` removes it when it creates a data directory; an existing data directory is never modified. | Nothing further; this is recorded because it is a deliberate deletion. |
| MariaDB on a non-default port | `--port` is honoured in `my.ini` and in the service row. Clients that read only the default port will not find it. | A generated `~/.my.cnf` fragment, or a client wrapper. |

## Sites, TLS and hosts

| Item | What it is | What it would be |
| --- | --- | --- |
| Certificate authority | An internal CA created on first use: ECDSA P-256, leaf valid 397 days, CA 3650 days, leaf backdated an hour. Every leaf is issued by `devd`'s own CA; there is no ACME and no public certificate. | ACME for a real domain, and a per-site choice between the internal CA and a public one. |
| Trust | `devd tls trust` runs `certutil -addstore` (Windows) or `trust anchor` (Linux). Without p11-kit on Linux it prints the manual `update-ca-certificates` recipe instead. | p11-kit integration, and macOS `security add-trusted-cert` if macOS is ever supported. |
| Revocation | No OCSP, no CRL, no certificate transparency. A leaf is trusted because the CA is trusted. | Revocation is not meaningful for a local CA of this size; a short leaf lifetime is the current mitigation. |
| Hosts file | Writing it needs elevation on Windows. The operation is a separate, explicitly approved capability (`hosts.sync`, `Tier::Admin`), snapshots the file first and preserves every line outside its own block. `DEVD_HOSTS_FILE` points it at another file, which is what makes it testable without elevation. | A DNS server on loopback, which removes the elevation requirement and gives wildcard names. |
| Site removal | `site.remove` deletes the site row, its vhost and its certificate, and keeps the project directory unless `--delete-files` is passed. It refuses to delete a directory outside the `www` root. | A trash/undo window before the directory is removed. |
| Hosts and DNS as one step | Adding a site and publishing its name are two capabilities with two tiers, on purpose. A script that forgets the second leaves the site unreachable by name. | Nothing — the split is the design. `site list` reports which names are not published. |

## The terminal dashboard

| Item | What it is | What it would be |
| --- | --- | --- |
| The interactive path | The state, the controller, the frames and the refusals are covered by automated tests, but crossterm's raw mode, the event loop and the drawing to a real console are exercised only by hand | A pty-based test that drives the real event loop and asserts on what a terminal receives |
| Mouse | Keyboard only, deliberately | Mouse selection would add a code path with no safety value |
| Long rows | A long path is truncated with an ellipsis; the full value is in the detail overlay | Horizontal scrolling, or a wrapped row |
| Detail overlays | Vertically scrollable, horizontally clipped | Wrapped JSON, which is easier to read in the application window's inspector |
| Interactive rebase | The same no-op sequence editor as everywhere else, so `--autosquash` works but commits cannot be reordered or dropped | A sequence-editor screen inside the dashboard |

## The application

| Item | What it is | What it would be |
| --- | --- | --- |
| Window shell | A browser engine in `--app` mode, opened by `devd ui`, falling back to the default browser | A Tauri v2 window. The interface does not change, because it speaks only to the local API. |
| Capability arguments | Edited as JSON in the inspector, seeded from the argument schema | A generated form per schema. |
| Native window chrome | None in application mode; the window is chrome-less | A native title bar and menu, which Tauri would provide. |
| Browser availability | `devd ui` needs a Chromium-family browser or a default browser to open a window; `devd serve` needs neither | A bundled webview. |

## Partial or weaker than intended

| Item | What it is | What it would be |
| --- | --- | --- |
| Windows process-tree termination | `taskkill /PID <pid> /T /F`, with a 15-second timeout | A Windows Job Object, as above |
| Interactive rebase | Driven by a no-op sequence editor (`GIT_SEQUENCE_EDITOR=true`), so `--autosquash` works but a user cannot reorder, drop or edit commits interactively | A real sequence-editor UI in the TUI and the GUI, which is where interactive rebase belongs. |
| Hunk-level staging | `git.stage.patch` with a unified diff, used by the application's per-hunk controls | A graphical hunk selector in the desktop shell. The capability and the tests exist today. |
| Repository index labels | A repository is labelled by its directory name | A user-settable display name. |
| `devd git repo prune`-style index cleanup | There is no capability that removes index entries whose directory has disappeared | A prune for the index itself. This is separate from `git.repo.prune`, which deletes merged branches. |
| Audit retention | The audit table grows without bound; `devd audit -n` reads the tail | A retention policy and rotation. |
| Secret scanning | A warning when staged content looks like a credential is specified | Not implemented. Previewing a staged diff before a commit shows the content but does not flag it. |
| Instance discovery | `devd ui` finds a running instance by probing the remembered port and the next few for the `X-Devd-App` header | A lock file or a named pipe, which would be exact rather than a scan. |

## Agent terminals and preview

| Item | What it is | What it would be |
| --- | --- | --- |
| Agent harnesses | The terminal runs `claude`, `opencode` or a shell as installed on the machine. There is no bundled agent, no model proxy and no API key stored by devd. | Additional harnesses by command template, and per-project harness presets. |
| Sessions | One agent terminal at a time in the window. Its id lives in the page, so reloading the window loses the handle while the PTY keeps running until it exits or the server stops. | A session list with reattach, and adoption after a reload. |
| Output | Polled over the loopback API every 80ms, with 2 MB of scrollback in memory. It is not archived. | A WebSocket transport, and saved transcripts. |
| Permissions | The harness's own prompts are the authority. devd does not mediate file edits or commands the agent runs. | Nothing — proxying those would make the agent's guarantees devd's problem. |
| Preview | The site is embedded in an iframe with `sandbox` set. A site that sends `X-Frame-Options` or a `frame-ancestors` policy will refuse to render; devd's generated vhosts do not. | A per-site "block embedding" switch, and a device-size preview. |
| Preview certificates | An HTTPS site renders without a warning only after the local authority is trusted. The page says when it is not. | A one-click trust from the preview, which exists but is the same admin action. |

## Platform limits

* **No macOS.** Windows and Linux are the targets. Nothing is designed in a way
  that forbids macOS, but it is untested and unsupported.
* **The installer is Windows x64 and per-user.** There is no ARM64 build and no
  machine-wide install: it writes to `%LOCALAPPDATA%\Programs\devd`, the current
  user's `PATH` and `HKCU`. It is unsigned, so SmartScreen warns once; the
  published `SHA256SUMS.txt` covers the download. Uninstalling keeps the data
  directory.
* **No containers as the substrate.** `devd` runs services as the invoking user
  on the host. This is a deliberate decision, not a gap, but it means a service
  `devd` starts runs with that user's privileges.
* **`git` must be on `PATH`** (or named in `git_executable` in settings). `devd`
  drives the user's own Git rather than bundling one.
* **`devd ui` needs a browser engine.** It prefers a Chromium-family browser in
  `--app` mode and otherwise falls back to the default browser or the system
  opener. `devd serve` needs none of that; it is the API alone.
* **`devd tui` needs an interactive terminal.** With no terminal on stdin or
  stdout it exits with a one-line explanation and code 2 rather than starting.
  Entering raw mode on a pipe is not a condition crossterm reports on Windows,
  and the attempt killed the process outright before this guard existed.

## Environment-specific notes

These were found while building and testing on Windows with the GNU toolchain,
and are recorded so they are not rediscovered:

* **Command-line length on Windows.** A single `git` invocation is limited by
  the Windows command-line length. Very large `stage` calls, or a repository
  with tens of thousands of changed paths, could exceed it. The patch-staging
  path avoids this by writing the patch to a file inside `.git` rather than
  passing it as an argument, which is why hunk staging works at any size.
* **CRLF translation.** `devd` never changes `core.autocrlf`. If a repository
  has it enabled, files checked out will have CRLF endings, which is Git's
  behaviour and not something `devd` overrides. Output parsers trim
  line-ending artefacts so that reads behave identically on both platforms.
* **Format-string differences within Git.** `git for-each-ref` and `git stash
  list` do not share a format language: `%1f` is the separator byte for the
  former and a literal for the latter, which needs `%x1f`. Both call sites carry
  a comment and the parsing is covered by test, because this silently produced
  an empty list before it was found.
* **Git hooks.** Hooks that exist in a repository run when Git runs them during
  a commit. `devd` does not install, bypass or disable them. Read-tier
  operations execute no repository content. An explicit strict mode is a
  candidate for a later milestone.
* **The final link must not go through Zig on this host.** With no MSVC build
  tools, the GNU target's default linker here is a `cc` that forwards to Zig.
  Linking the release `devd` that way produced a binary that faulted at
  `0xc0000005` inside its own code before `main`, with no output — release only,
  while debug and the whole test suite were fine. `.cargo/config.toml` pins
  `linker = "rust-lld"` for `x86_64-pc-windows-gnu` alone, with the reason
  recorded beside it. C sources are still compiled by `cc`, and a build on MSVC
  never reads that section.
* **The debug binary needed a bigger stack before it could print `--help`.**
  Windows reserves 1 MB for the main thread. In an unoptimised build, the single
  frame that clap's derive macro generates for this command line outgrew it, so
  `devd --help` died with "has overflowed its stack" before parsing anything,
  while the release build was fine. The same config section raises the reserve to
  8 MB — what a Linux main thread and a spawned Rust thread already get. It is
  reserved address space, not committed memory.
* **A detached service must not inherit the caller's handles.** On Windows a
  service started with `DETACHED_PROCESS` still inherited `devd`'s own stdout.
  When that stdout was a pipe — `devd stack up | Tee-Object out.txt`, or an
  editor's terminal — the shell waited for the pipe to close, which meant waiting
  for nginx to exit, which is forever. The command printed its result and then
  appeared to hang. `process::isolate_standard_handles` clears the inheritance
  flag on `devd`'s own three standard handles before a service is spawned; every
  child gets its handles explicitly, so nothing is lost.
* **`Path::with_extension` replaces everything after the last dot.** Certificate
  files are named after the host (`certs/blog.test.crt`, `certs/blog.test.key`),
  and building those paths with `with_extension` produced `certs/blog.crt` — so
  nginx was told to load a file that `devd` had never written, and refused to
  start. The paths are now built from the whole file name, `atomic_write` derives
  its temporary name the same way, and a test asserts the exact leaf paths.
* **`mysql_install_db.exe` has no `--basedir`.** The Windows bootstrap takes
  `--datadir`, `--port`, `--password` and a few others; it works out its base
  directory from where it runs. Passing `--basedir` made it fail with
  "unknown variable" and exit 7. The argument is gone.
* **MariaDB's archive listing cannot answer "is there a Windows build?".**
  `archive.mariadb.org` holds directories for preview series and for stable
  releases that shipped no Windows package, so resolving `latest` from that
  listing picked 13.2.0, a preview with no zip at all. The catalogue now reads
  MariaDB's release API, which states each series' status and lists each
  release's files with a per-file SHA-256; only Stable series that actually have
  a `winx64.zip` are offered.
* **`mysql --batch` prints a header row.** Parsing `SHOW DATABASES` naively made
  `schema_name` appear as a database. The client is now invoked with
  `--skip-column-names`.

## Sessions, agents and the desktop shell (v0.8.0)

| Item | What it is | What it would be |
| --- | --- | --- |
| Reattachment across a backend restart | Impossible, and reported as such. A PTY handle belongs to the process that opened it. On startup a row whose owner is gone becomes `stale`; its transcript remains readable. | A detached supervisor process that owns PTYs independently of the UI process, with a reconnect protocol. This is a deliberate non-goal for the current single-process model. |
| One owner per PTY | A session is owned by one devd process. Another process can list it and read the transcript but cannot attach. MCP launched by an agent is a separate process, so an agent-spawned session belongs to the MCP process, not the window. | A privileged IPC or named-pipe bridge from MCP to the running UI server. The metadata model already records the owner; the transport is missing. |
| Quitting the desktop app | Stops PTYs and project processes, as a normal exit should. A crash or `TerminateProcess` may leave children; Windows Job Objects would close that gap. | A job object with kill-on-close on Windows, and a reaper on Linux. |
| Delegation harness modes | Claude uses `claude -p` (print mode) and OpenCode uses `opencode run`. Interactive permission prompts are not answered for the child; if the harness needs a decision it can fail, and that failure is the result. | A PTY-level handoff that lets a person answer the child's prompt without proxying its model. |
| Delegation across machines | Local only. There is no remote agent pool. | Remote executors with the same typed task contract. |
| Project process adoption | A project process belongs to the devd process that started it. A second process reports it as supervised elsewhere rather than adopting it. | A small detached supervisor with pid-file ownership discovery. |
| Windows `.cmd` shims | Started through `cmd.exe /c <shim> <args>`. Arguments remain separate, but a shim can itself interpret them. | Vendor-native `.exe` wrappers where available. |
| Manifest secrets | Declared values are redacted from log reads on a best-effort key match. A program can print a secret in another form. | Structured secret references (OS credential store) rather than literal values in `devd.yml`. |
| Tauri packaging | `cargo build` produces the portable executable; the NSIS installer script installs both binaries. The build is not code-signed and requires WebView2 (present with current Edge). | Code signing and an MSI/NSIS bundle produced by `cargo tauri build` in CI. |
| Tauri and `devd ui` at once | Each starts its own backend. Sessions and processes are not shared; each reports the other's resources as unavailable/unsupervised. | A single-instance handshake that hands the window to an existing backend (the token would then need a secure handoff). |
| Shared-state access control | Project-scoped by canonical path, not by per-user ACL. A local user who can read `state.db` can read every note. | Per-user encryption or an OS credential-store key. |

## Agents, team, search, services (what is not done)

Stated plainly, so nothing is assumed:

* **The team's isolation is git worktrees.** Without git, or before the first
  commit (devd makes one), writing tasks run one at a time. Dependencies are not
  shared across worktrees: an implementer installs what it needs in its own copy.
* **Merges are plain `git merge --no-ff`.** A conflict blocks the task for a person;
  devd does not resolve conflicts or rebase.
* **Evidence is read from `project.run` and file capabilities.** Work an agent does
  by other means (for example a long shell session in Claude Code) is not counted;
  those sessions are not governed by the team scheduler.
* **Team tasks use the providers you configured.** Gemini and local runtimes go
  through their OpenAI-compatible endpoints; there is no native Gemini adapter, and
  model capabilities beyond what a provider lists (tool support, modalities) are not
  tracked per model. The official list devd fetches itself is Anthropic's.
* **The model fallback chain is per role.** There is no cost or privacy policy
  engine; the only guard is that fallbacks use only providers you listed.
* **No deployment.** devd prepares and verifies; it has no deploy capability, no
  rollback, no production secret manager, and does not manage remote hosts.
  Production mode is a set of safeguards on this machine, not a deployment target.
* **Service adoption records decisions; it does not migrate or isolate.** `external`
  points devd at an instance you run; devd does not start a second managed copy on
  another port, and does not compare a service's version with a project's needs
  (stacks do not declare service versions). The devd-managed nginx/MariaDB/PHP
  (`service.*`) and the host-managed catalogue (`toolchain.*`) are still two
  managers, reconciled in `services.inventory` but not merged.
* **Search is live, not indexed.** File search is by path (30,000 entries per
  query), text search is literal and per project, and neither covers logs or remote
  repositories. Conversation search covers titles.
* **The terminal dashboard** shows team runs and the inbox but approves plans only;
  creating a run, steering it (`team.say`) and answering questions are CLI/window
  actions.
* **`docs.fetch` checks the address before connecting;** a hostile DNS server could
  still answer differently between that check and the connection.
