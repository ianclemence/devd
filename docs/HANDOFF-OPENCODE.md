# Handoff prompt for the OpenCode agent

**Open the OpenCode agent in this folder: `D:\devd\www\devd`** (the repository root; `git rev-parse --show-toplevel` prints it). Do not open it in `D:\devd`: that folder is the installed program, devd's data home and the `www` folder of projects, not the source.

Paste everything below the line into the agent as its first message.

---

You are taking over an unfinished software project from another AI agent whose usage limit ran out. Read this whole message, then read the files it names, then continue the work. The person you work for is a web developer; they have asked that you act as lead product developer and lead software engineer: **make the decisions yourself, do not ask them to choose between options, and tell them plainly what you did and what you could not verify.** They write in capitals when they are in a hurry; that is urgency, not anger.

## 1. What devd is

`devd` is an agent-native local development server manager for Windows (also Linux and macOS), written in Rust (a workspace of crates under `crates/`) with a vanilla-JavaScript web UI (`crates/devd-server/ui/app.js`, `style.css`, `index.html`, about 8 000 lines of JS, no framework, no build step, embedded in the binary with `include_str!`). It does what Laravel Herd, Laragon and Ploi do between them, and it is built so that an AI agent and a person use the same capabilities:

* It installs and supervises PHP (several versions, a pool of `php-cgi` workers per version), nginx or Apache, MariaDB, Redis, memcached, Mailpit; serves every project folder as `<name>.test` over HTTPS with certificates from its own local authority; manages the hosts file with a consent prompt.
* **Everything is a capability** (`reg.define::<Args>("group.name", "summary").description().group().tier().effect().agent_safe().handlers(plan_fn, run_fn)`). A person in the window, the CLI (`devd call <id> --args '{...}'`), and an agent over MCP all go through the same registry, the same approval rules (tiers: read < write < exec < destructive < remote < admin; an agent can never approve its own action) and the same audit trail. Read `crates/devd-core/src/capability.rs`, `registry.rs`, `docs/architecture.md`, `docs/agents.md`, `docs/security.md`.
* Since the Ploi work it also does what a server panel does for each site, locally and on remote servers: `.env`, cron, queue workers, redirects, logs, a Laravel tab, deploys with history, rollback, variables, scheduling and staging, health checks, database users and backups, per-site nginx directives, SSH keys, and **deploying to any server reachable by SSH** (DigitalOcean, Hetzner, Vultr, Ploi, Forge, a Raspberry Pi). The person's real production server is on Ploi (nginx 1.30, PHP 8.4-FPM, MySQL 8, Redis, memcached, supervisor, ufw; five Laravel sites) and devd is meant to match it.

The whole point of the project is **this person's workflow**: develop locally in devd, check it against what production allows, and deploy to any of their servers, with agents able to do every step under approval.

## 2. Where everything is on this machine

| What | Where |
| --- | --- |
| **Source repository (open the agent here)** | `D:\devd\www\devd` |
| Installed program (`devd.exe`, `devd-desktop.exe`) | `D:\devd` |
| The person's projects (each is a devd site `<name>.test`) | `D:\devd\www\<name>` (chitchatter, ghost, ghost-app, icmosha, kindlebox, luna, nairobiunwind, nairobiunwind-app, schoolmate, scout, staffroom) |
| devd's data (state.db, runtimes, services, certs, logs, cache, backups) | `D:\devd` - the same folder as the program (`DEVD_HOME` is set to it as a user environment variable) |
| Rust toolchain | `C:\tools\cargo\bin`, rustup home `C:\Users\User\.rustup`, toolchain `stable-x86_64-pc-windows-gnu`; also needs `C:\tools\mingw64\bin` on PATH; NSIS for the installer at `C:\tools\nsis\nsis-3.13\makensis.exe` |
| Installer output | `D:\devd\www\devd\dist\devd-setup-0.11.1.exe` (silent install: `devd-setup-0.11.1.exe /S /D=D:\devd`) |
| Old Laragon | removed 2026-10-24; backup in `D:\devd\backups\laragon-20261024`; the `ploi_openssh` key was recovered from the Recycle Bin to `C:\Users\User\.ssh` |

Build and check from the repository root (PowerShell or Git Bash):

```
$env:Path = 'C:\tools\cargo\bin;C:\tools\mingw64\bin;' + $env:Path
cargo clippy --workspace --exclude devd-desktop --all-targets -- -D warnings
$env:GIT_AUTHOR_NAME='devd test'; $env:GIT_AUTHOR_EMAIL='t@example.invalid'; $env:GIT_COMMITTER_NAME='devd test'; $env:GIT_COMMITTER_EMAIL='t@example.invalid'
cargo test --workspace --exclude devd-desktop --no-fail-fast
powershell -File scripts\release-installer.ps1 -Nsis C:\tools\nsis\nsis-3.13\makensis.exe
```

(The git identity variables are only because the person's `.gitconfig` was deleted; do not write a global git config.) CI (`.github/workflows/ci.yml`) runs build, clippy with `-D warnings`, and tests on Windows, Linux and macOS: keep all three green. `cargo build -p devd-cli` is the quick debug build; to look at the UI run `target\debug\devd.exe ui --no-open --port 8794` **after stopping any running devd UI server** (a running `devd.exe` locks the file and the build then fails with "Access is denied"; check the build output, do not assume it worked). Editing `app.js` or `style.css` needs a rebuild to show, because they are embedded. Always run `node --check crates/devd-server/ui/app.js` after editing the script: a lost string escape once blanked the whole window while every structural test passed (there is now a test for it).

## 3. The crates, briefly

`devd-core` (capability registry, state store with SQLite migrations in `crates/devd-core/sql/`, settings, paths, download, scheduling), `devd-runtime` (PHP/Node/Go/Python/Ruby install, `php.ini` handling in `phpconf.rs`), `devd-services` (nginx, Apache, MariaDB, Redis, Mailpit, PHP listeners and worker pools, redirects, per-site nginx directives `siteconf.rs`, `phpcaps.rs`), `devd-sites` (sites, hosts file, certificates), `devd-git`, `devd-workspace`, `devd-stacks`, `devd-agent` (the built-in assistant and team), `devd-mcp`, `devd-server` (HTTP API + embedded UI), `devd-cli`, `devd-tui`, `devd-desktop` (WebView2 shell), `devd-bootstrap` (the composition root: every capability crate registers here), and **`devd-ops`** (new: everything a server panel does; modules `env`, `cron`, `daemons`, `logs`, `deploy`, `notify`, `ploi`, `laravel`, `dbadmin`, `health`, `ssh`, `servers`, `remote`, `limits`, `runner`, capabilities in `caps.rs`, `caps_more.rs`, `caps_remote.rs`). Operations data lives in one generic table `ops_items (kind, id, site, data JSON)` (migration 013) through `Store::ops_put/get/list/delete` in `crates/devd-core/src/store_ops.rs`.

A background loop (`devd_ops::runner`) runs cron jobs, database backups, scheduled deploys and health checks while a long-lived devd process (the window, `devd ui`, `devd serve`) is open; exactly one process owns it (a pid file). Jobs due while devd was closed are skipped or marked missed, never run late.

Documentation of what exists is in `docs/operations.md` (read it fully), `docs/reference.md`, `CHANGELOG.md` (the `[Unreleased]` section lists everything added in this stretch), `docs/limitations.md`.

## 4. State of the work (be honest about this)

**Built, tested, clippy-clean and committed for 0.11.0 (2026-10-24), and the installer has been run over the existing install:**

1. PHP: a pool of workers per version behind an nginx `upstream` (`php_workers`, default 4, ports 9084/9184/9284/9384 for 8.4), a `local`/`production` limits profile (`php.configure`), a per-site profile served by a second pool with the other profile's ini 50 ports higher (`php.site_profile`), an extension manager, effective-value reporting, `php.pool` (start/stop/restart a version). The legacy single `php-cgi` listener is retired by `prune_php_services`.
2. Services: Redis (redis-windows/redis-windows) and memcached (jefyt/memcached-windows) download on Windows and are checked against the SHA-256 GitHub records; `devd setup --services/--php/--profile`; a first-run "Set up devd" sheet in the UI.
3. `devd-ops`: env, cron (own cron parser), queue workers (as supervised services with a launcher script), redirects (nginx and Apache), Laravel log viewer, deploy script/history/rollback/hook, notifications (inbox, Slack, Discord, webhook), Ploi API (`ploi.*`, token in a private file), limits comparison against the Ploi server (`ploi.limits_set`, `deploy.check`), Laravel tab (`laravel.*`), database users and scheduled backups (`db.user_*`, `backup.*`), per-site nginx directives with presets validated by `nginx -t` and rolled back on failure, deploy variables / scheduled deploys / staging worktrees, health checks and machine monitoring, SSH keys (`ssh.*`).
4. **Remote servers** (`servers.rs`, `remote.rs`, `caps_remote.rs`): register servers, pin host keys in devd's own known_hosts, `server.connect/info/run/provision`, `remote.link/deploy/releases/rollback/env_push/site_create/ssl/steps_set`. Two modes: `releases` (local build, tar over SSH, `releases/<ts>`, shared `.env` and `storage`, steps on the server, atomic `mv -Tf current.new current`, prune, instant rollback) and `git` (the server pulls and runs the deploy script as written).
5. UI: **Server** has Overview, Sites, Services, Tools, Runtimes, Databases, Remote, Certificates and Hosts. Each site has its own page (`/server/sites/<name>`) with Overview, Environment, Deploy, Cron, Queue, Logs, Laravel, Redirects, Web server, Database and Preview. Operations and Preview are reachable by their old routes for one release but are not in the server strip. Services shows one row per PHP version; every screen says its own name once (the breadcrumb only appears for a project path or a site; a page title that repeats the open tab is visually hidden).

**Verified live on this machine (with the installed build of an earlier point in this stretch):** env set/get/unset with backup and masking, cron add/run/log, redirects (301 and wildcard 302 with real HTTP), a deploy and its log, `deploy.check` warnings, a per-site profile (PHP really ran with 128M/5M/8M/30 and went back), the 16-way concurrency improvement of the worker pool, the old Operations screens in a browser. The 0.11.0 site pages and server tabs were checked in a headless browser against the installed data set.

**NOT verified, and the first things to do:**

* **Checked on 2026-10-24 (0.11.1), on the person's machine in isolated homes, not on a second computer:** the published 0.11.0 binary matches its checksum and a clean first run installs PHP, nginx, MariaDB, Redis, memcached and Mailpit; the Apache download, checksum, config, HTTPS and the `mod_proxy_balancer` pool run against a real Apache 2.4.69; the "Set up devd" sheet completes in the window; every screen was read against the machine's real state at 1280 and 375 px. Found and fixed on the way: Redis never started (MSYS path), the first-run Install PHP sent no version, the sheet reinstalled PHP that was there. **Still unverified:** a truly separate machine, the NSIS installer run end to end, a program other than devd holding port 80 during first run (the sheet reports success when something answers on the port), and `devd path enable` on a real user PATH (its registry code is tested against a throwaway key).
* **Nothing in section 4.4 has ever run against a real server.** The tests cover the generated scripts, the archive contents, the SSH arguments, host-key handling and (Linux CI only, `crates/devd-ops/tests/remote_e2e.rs`) the whole releases flow through a stand-in `ssh`. It was not run on Windows (no symlinks, no `mv -T`) and not against a real machine. When the person has a throwaway server (a DigitalOcean or Hetzner droplet), do: `ssh.keygen` (or `ssh.adopt`), put the public key in `authorized_keys`, `server.add`, `server.connect`, `server.info`, read the plan of `server.provision`, run it, `remote.link`, `remote.env_push`, `remote.site_create`, `remote.ssl`, `remote.deploy`, change something, deploy again, `remote.rollback`. Fix what breaks and add a test for each fix. Never run server-touching capabilities against the person's real Ploi server (206.81.14.76) without their explicit instruction.
* The Ploi API calls (`ploi.*`) have never run: there is no token. `ploi.connect` is human-only.

## 5. What to build next (in this order; decide the details yourself)

1. Finish the verification above and fix whatever it finds. Commit nothing until the person says so; when they do, make logical commits, not one.
2. **Remote operations parity** (the person's servers will need what the local machine has): `remote.logs` (tail nginx/PHP/Laravel logs over SSH), `remote.env_diff` (compare the local `.env` with the server's `shared/.env`, secrets masked, reusing `env::diff`), `remote.cron` and `remote.queue` (manage cron entries and supervisor programs on the server for a link), `remote.backup` (dump a remote database over SSH to the local backups folder), `remote.health` (a health check against the link's domain), and **push local config to a server** so cron/workers/redirects do not drift. Each is a `remote`-tier capability with a plan that shows the exact commands.
3. A **workflow skill for agents**: a documented, tested sequence an agent follows ("change locally, `deploy.check`, `remote.deploy` to staging, `health.check`, promote, `remote.rollback` on failure") and an end-to-end test that drives it through the MCP server (`crates/devd-mcp`). Update `docs/agents.md`.
4. `ploi.*` completeness once a token exists: sync cron/queues/redirects from Ploi to devd and back, `deploy.script` pull and push, showing Ploi deployments.
5. MySQL as a download (blocked: MySQL publishes no SHA-256 for the Windows zip; MariaDB stands in), PostgreSQL/MongoDB downloads (same blocker), firewall (`ufw`) management for a server, DNS provider integrations (Cloudflare first), Docker, status pages: lower priority, server-side, build only when the person asks.
6. UI polish: keep auditing screens. The audit script used for this lives in the previous session's scratchpad; recreate it: for every view check that the page, tab and crumb do not repeat a name, nothing is unstyled (computed `background-color: rgb(239,239,239)`), nothing scrolls sideways, no `undefined`/`null`/`NaN` text, at desktop and 375 px widths.

## 6. Rules of the road (some learned the hard way)

* **Never delete or overwrite anything of the person's without care.** A previous slip with a PowerShell variable (`$home`, a read-only automatic variable that expanded to the profile folder) deleted most of `C:\Users\User`. Use literal paths, assert them, check for links first, delete in the smallest scope, and say what you removed. Delete only what the person authorised. Regenerable caches and build outputs are fine to clear (the person asked: "always clear any caches or builds we no longer need"), and tell them what you cleared.
* The PowerShell tool refuses `Remove-Item` on folders directly under a drive root (`D:\something`); the Bash tool's `rm -r` on exact paths works. Large inline heredocs in the Bash tool sometimes fail to parse; write files with the file-writing tool and run them.
* Line endings: some files are CRLF. Edit by exact-string replacement that preserves them (the helper pattern: read with `newline=''`, replace, write with `newline=''`); a naive rewrite turns a one-line change into a whole-file diff.
* Windows facts: no symlinks without developer mode; `cargo test` needs the git identity variables above; the three git `lifecycle` tests and the `remote_e2e` test depend on the environment; tests that ran fine alone have failed under heavy disk load (agent tests have a 40 s timeout), re-run before concluding.
* Secrets: nothing writes a token, key or password to the database, settings or audit trail; argument names containing `password`, `secret`, `token`, `api_key`, `app_key` are redacted by `registry::redact`. `env.reveal`, `ploi.connect`, `ssh.adopt`, `remote.env_push` are human-only. Private keys are never returned by anything.
* Every capability has a plan (what it will do, shown before approval), a tier, and a description that tells an agent what it is for. New capabilities go in a `register` function reached from `devd_bootstrap::Bootstrap::at`, with a unit or integration test, and a line in `docs/operations.md` or `docs/reference.md` and `CHANGELOG.md`.
* UI rules: one name per screen (see above), controls use the existing classes (`btn`, `btn-mini`, `btn-primary`, `btn-ghost`, `ui-row`, `ui-head`, `chip`, `pill`, `notice`), selects that act on change use `data-change`, not `data-action` (a click on a `data-action` select runs the handler), and every `data-action` needs a handler in `ACTIONS` (a contract test enforces it).
* Say what you verified and what you did not. If a test fails, report the output; do not claim a success you did not see.

## 7. Things on the person's machine that are not code but need doing (offer, do not assume)

* **Laragon** was removed on 2026-10-24. Its `data`, `www`, `etc`, hosts file and the `umami` dump are in `D:\devd\backups\laragon-20261024`; the `ploi_openssh` RSA key (encrypted, PuTTY comment `ploi`) was recovered from the Recycle Bin into `C:\Users\User\.ssh\ploi_openssh` (and `.ppk`), owner-only. Verify the passphrase and add it with `ssh.adopt` before relying on it. Everything else of value had moved: projects to `D:\devd\www`, its MySQL held only the abandoned `umami` database. `php` and `mysql` on the PATH (they came from Laragon) are gone.
* The earlier profile deletion left broken per-user items: shortcuts and startup entries pointing into deleted `AppData` (Zoom, LINE, GitHub Desktop, Teams, Helium, FFmpeg, fnm, Python installers), Start-menu links to programs that no longer exist (PuTTY, Python 3.12, WinHTTrack, Driver Booster), and a Start search that does not work although Windows Search and the Start packages are registered. Windows Update will not fix per-user data. The remedies are `Reset-AppxPackage` for Microsoft.Windows.Search / StartMenuExperienceHost / ShellExperienceHost (per user, no admin), rebuilding the search index, `sfc /scannow` and `DISM /Online /Cleanup-Image /RestoreHealth` (the person must run those elevated), reinstalling the apps, and removing dead shortcuts. Ask before changing any of it.
* OneDrive was re-linked and Documents and Pictures restored into it. `D:\recovered` still holds PhotoRec output (6 `.docx`, 1 `.pptx`, 1 `.pdf`, 36 `.jpg` may be theirs); offer to pull those into one folder for review, then delete the rest only if they say so.
* `.gitconfig` and `.ssh` do not exist yet; `git config --global user.name/user.email` needs the person's real name and address.

## 8. First thing to do now

Read `docs/operations.md`, `docs/agents.md` and `git status`; run the verification chain in section 2; install the result; then tell the person in a few sentences what state you found, and start section 5, item 1.
