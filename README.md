# devd

A local development environment manager, written in Rust, whose every action is
a **capability** — a typed, permissioned, auditable operation reachable from the
CLI, a terminal UI, a desktop app, an HTTP API and an MCP server, all of which
are thin projections of one core.

`devd` is a ground-up replacement for the Windows-only Laragon stack: nginx and
Apache, several PHP versions side by side, MySQL/MariaDB, Redis, Mailpit, Node
and Go, automatic local HTTPS, hosts and virtual hosts, and per-project
isolation — on Windows and Linux.

> **Project status.** This repository is at **M4**: the foundation, the complete
> Git integration, the CLI, the **local application** (a loopback HTTP API plus
> the interface it serves), the **MCP server**, the **terminal dashboard**, the
> **runtime installers**, the **service stack** (nginx, MariaDB, php-cgi) and
> **sites served over internal HTTPS** are implemented and tested. Apache, Redis,
> Mailpit, a DNS server and SSH/deploy are designed but **not yet built**, and the
> service layer is Windows-only in this milestone. See [Status](#status) for
> exactly what works today, and [`CHANGELOG.md`](CHANGELOG.md) for what landed in
> each milestone.

---

## Why capabilities

Most tools expose a command line, and then each new interface re-implements the
same logic slightly differently — which is where the safety bugs live. `devd`
has one registry. A capability declares what it does, how risky it is, whether a
dry run is possible, whether an agent may invoke it, and what its arguments look
like as JSON Schema. Every surface reads that declaration instead of inventing
its own rules.

The consequences are enforced in code, not in prompt text:

| Guarantee | How it is enforced |
| --- | --- |
| A destructive action cannot run without approval | `Tier::Destructive` / `Tier::Remote` are checked in `Registry::dispatch`, before the handler |
| An agent cannot approve its own action | `Actor::is_agent()` fails the approval check unconditionally; there is no agent-supplied approval path |
| Some operations are reserved for humans | `human_only` is checked before approval is even considered |
| A declined action does not run | `Approval::Denied` short-circuits the dispatcher, and the attempt is still audited |
| A refusal cannot be bypassed from another surface | Every surface calls the same dispatcher; the "workaround" is a different capability with its own tier |
| A force push cannot be reached by accident | It is a separate capability (`git.push.force`), not a flag, so no code path can build a forced update |
| Secrets do not reach the audit trail | Argument objects are redacted before being recorded |
| Input never becomes a shell string | Arguments are typed structs turned into `argv` arrays; branches and paths are validated against option injection |

## What works today

```text
crates/
  devd-core/     the capability registry, the state store, paths, settings, events
  devd-git/      64 Git capabilities and the typed Git service beneath them
  devd-runtime/  PHP, Node and Go installers, verified against the vendor's digest
  devd-services/ nginx, MariaDB, php-cgi, TLS and the service stack
  devd-sites/    sites, their generated vhosts and the managed hosts block
  devd-server/   the loopback HTTP API and the application interface it serves
  devd-mcp/      the Model Context Protocol server for agents
  devd-tui/      the terminal dashboard
  devd-cli/      the `devd` command
```

* **98 capabilities** across 20 discoverable groups. The 64 Git ones are listed
  below; the rest are `runtime`, `service`, `web`, `stack`, `tls`, `db`, `site`
  and `hosts`.
* **64 Git capabilities** across 12 discoverable groups (`git.repo`,
  `git.changes`, `git.commits`, `git.history`, `git.branches`, `git.tags`,
  `git.remotes`, `git.merge`, `git.stashes`, `git.worktrees`, `git.config`,
  `git.recovery`).
* **A typed Git service** that drives the user's own `git` executable through
  `argv` arrays — because rebase, bisect, worktrees, submodules, LFS, credential
  helpers and hooks are only faithful when Git itself performs them.
* **Runtime installers** (`devd runtime install php`) that read the vendor's own
  catalogue, check the digest the vendor publishes, unpack in-process, and
  configure what they installed — PHP's extensions are enabled one at a time and
  any that cannot load are switched back off and reported.
* **A service stack** (`devd service`, `devd web`, `devd stack`): nginx and
  MariaDB from their vendors, `php-cgi` from the installed PHP runtime, started
  detached, tracked by pid file and process image, and confirmed listening on
  their port before the command reports success. `nginx.conf` is rendered from
  the site table; it is output, not configuration you edit.
* **Sites over HTTPS** (`devd site`) from an internal certificate authority
  created on first use — an ECDSA P-256 CA and leaf, a full chain for nginx, and
  `devd tls trust` to put the CA in the operating system's trust store. No
  `openssl` binary, no bundled certificate files.
* **A managed hosts block** (`devd hosts`) that preserves every line it did not
  write, byte for byte, snapshots the file first, and is idempotent.
* **An application** (`devd ui`) — a window onto a loopback HTTP API that is a
  pure projection of the same registry. Projects, the working tree, history,
  branches, tags, remotes, stashes, worktrees, conflicts and recovery, plus the
  capability inspector, the audit trail and the environment check.
* **An MCP server** (`devd mcp`) — the same registry, published to agents as five
  discovery tools rather than one tool per capability, so the list does not grow
  as the product does. An agent can explore the catalogue and run read-tier
  capabilities; an operation that needs approval is refused, recorded, and
  returned with the plan and an explicit instruction not to claim it happened.
* **A terminal dashboard** (`devd tui`) for the console and for SSH. Nine views,
  the same Git actions as the CLI, the whole registry filterable, the audit
  trail and the doctor. A destructive key raises the registry's own plan and runs
  nothing until you approve it; declining records the refusal in the audit trail.
* **Two-step approvals** that survive review: a destructive operation returns a
  plan and a single-use token, changes nothing, and only runs when that token is
  approved. The client never asserts the approval.
* **A test suite** that proves a declined operation did not execute, that
  uncommitted work survives a decline, an abort and a branch switch, that `/api`
  rejects a bad token, a foreign `Host`, a foreign `Origin` and every `OPTIONS`
  preflight, that an agent is refused a destructive operation and handed its plan
  instead, that a dashboard keystroke changes the real index while a declined one
  changes nothing at all, and that a generated vhost names the files that exist —
  including the certificate paths, whose dots are not an extension.
* **`devd doctor`**, which checks the layout, the store, the Git discovery and
  the environment before anything else is attempted — and now reports the
  installed runtimes, services and sites.

## Status

| Area | State |
| --- | --- |
| Capability registry, tiers, approvals, audit, store | **Done** (M0) |
| Git service, capabilities, CLI, tests | **Done** (M0) |
| CLI | **Done** for capabilities, Git, services, sites, audit and doctor |
| Local application (`devd ui`, HTTP API, interface) | **Done** (M1) |
| MCP server (`devd mcp`) | **Done** (M2) |
| TUI (`devd tui`) | **Done** (M3) |
| Runtime installers: PHP, Node, Go | **Done** (M4) |
| Services: nginx, MariaDB, php-cgi FastCGI; stack lifecycle | **Done** (M4) |
| Sites: vhosts, internal TLS, hosts block | **Done** (M4) |
| Services still to come: Apache, Redis, Mailpit | Planned, **not built** |
| DNS server, SSH/deploy, mail capture | Planned, **not built** |
| Runtime, service and site panels in the app and the TUI | Planned, **not built** — the CLI and the capability API cover them |

Nothing above is claimed as finished when it is not. The registry, the tier
model and the audit trail were built first precisely so the remaining surfaces
can be added without re-deciding any of the safety rules.

## Install (from source)

Windows or Linux, with a Rust toolchain and `git` on `PATH`:

```bash
git clone https://github.com/ianclemence/devd.git
cd devd
cargo build --release
```

The binary is `target/release/devd` (`devd.exe` on Windows). Put it on `PATH`
and run ``devd doctor``.

There is no bundled PHP, nginx or database: those are downloaded on first run,
so the repository and its license stay free of third-party binaries. What
`devd` downloads is checked against the digest the vendor publishes, with
nginx's missing checksum called out in the install output and in
[`docs/limitations.md`](docs/limitations.md).

### Upgrading

There is no self-update yet. An upgrade is a source upgrade:

```bash
git pull
cargo build --release
devd doctor            # the layout and store are checked before anything runs
devd stack down && devd stack up   # if services were running
```

The state store carries a schema version and is migrated on open, so an older
installation's sites, services and audit rows are kept. Downloaded runtimes and
services are cached by digest under the data directory and are not re-fetched.

## Using the CLI

```bash
devd doctor                       # check the environment
devd capabilities                 # every capability, grouped
devd capabilities --group git.changes
devd capabilities commit          # search across ids and summaries
devd describe git.reset           # summary, tier, arguments, JSON Schema
devd call git.status --arg repo=.
devd --plan git discard a.txt     # show what would happen; change nothing
```

### Git

The Git commands read like Git, and keep Git's vocabulary:

```bash
devd git status                   # branch, upstream, ahead/behind, changed files
devd git stage .                  # stage everything
devd git diff --staged            # review, then
devd git commit -m "message"
devd git log -n 5

devd git branch list
devd git switch feature
devd git fetch
devd git pull
devd git push

devd git stash push -m "wip"
devd git worktree add ../wt feature
devd git undo                     # recover from a bad reset via the reflog
```

Repositories, history rewriting and recovery:

```bash
devd git repo init D:\code\new --initial-branch main
devd git repo track .
devd git repo list

devd git rebase --onto main
devd git rebase --root --autosquash
devd git squash 3 -m "one commit"
devd git bisect start HEAD v1.0.0
```

### Runtimes, services and sites

```bash
devd runtime available php          # what the vendor publishes, and where
devd runtime install php            # newest stable; digest-checked, then verified
devd runtime list

devd service available nginx
devd service install nginx
devd service install mariadb --port 3306
devd stack up                       # php-cgi, then mariadb, then nginx
devd service status nginx

devd site create blog               # directory, starter page, vhost, certificate
devd site list
devd hosts sync -y                  # one approval: write the managed block
devd tls ca show                    # the CA to trust, and where it is
devd tls trust -y                   # add it to the OS trust store

devd db create blog
devd db list
devd db drop blog -y                # destructive: plan first, then approval
```

`devd site create` prints the URL. The two pieces that need a privilege — the
hosts file and the trust store — are separate, explicitly approved capabilities,
never a side effect of creating a site.

## Using the app

```bash
devd ui            # opens the application window
devd ui --no-open  # serves it without opening a window
devd serve         # headless: the API only, no window
```

`devd ui` starts an HTTP API on `127.0.0.1` and serves the interface from inside
the binary. It is a browser engine in application mode — no installer, no
`node_modules`, no build step. If a `devd` instance is already running, the
command reopens that one instead of starting a second server.

The window is a pure client of the same registry the CLI uses: every panel calls
`/api/invoke` with real capability ids. There are no placeholder controls. The
services, sites and runtimes panels say plainly that they are not implemented
yet rather than showing switches that do nothing.

Inside it:

* **Projects** — every repository `devd` knows about, with live branch, upstream
  and dirty state.
* **Changes** — staged and unstaged files, per-file diffs, stage, unstage,
  discard, commit (with amend), and a warning if a staged file looks like it
  contains a credential.
* **History** — the log, commit inspection, and revert or cherry-pick.
* **Branches, Tags, Remotes** — create, switch (with the real overwrite check
  first), merge, rename, delete; fetch, pull, push, force-push, tags.
* **Stashes, Worktrees** — the full lifecycle.
* **Recover** — unmerged conflicts with resolution, the reflog, reset modes,
  squash, bisect, and continue/abort for an interrupted operation.
* **Capabilities** — search the whole registry, read a declaration, run it with
  edited JSON arguments at your own risk tier.
* **Activity** — the audit trail, with plan, applied and refused marked
  separately.
* **System** — the same checks as `devd doctor`.

`Ctrl+K` opens a command palette backed by the registry's own ranked search, so
the palette cannot offer an action the CLI cannot perform.

A destructive action in the window shows the server's plan and a countdown, and
the confirm button is red. Approving replays the server's stored arguments, never
the client's — see [`docs/app.md`](docs/app.md) for why that distinction matters
and for the four loopback defences that were verified against a real socket.

## Using the dashboard

```bash
devd tui
```

For the console, and for an SSH session with nothing forwarded. Nine views —
projects, changes, history, branches, the whole registry, the audit trail, the
doctor — with `1`–`9` to jump and `Tab` to cycle.

```text
 ┌ Changes - main: 1 staged, 2 unstaged, 1 untracked in C:/code/devd ──────────────┐
 │M staged     README.md                                                           │
 │M unstaged   README.md                                                           │
 │M unstaged   crates/devd-core/src/lib.rs                                         │
 │? untracked  NOTES.md                                                            │
 └─────────────────────────────────────────────────────────────────────────────────┘
  Enter stage/unstage   s stage   u unstage   d discard   a stage all   c commit   r refresh   ? help   q quit
```

`s` stages, `u` unstages, `a` stages everything, `c` commits, `Enter` toggles a
file in or out of the index, `n` creates a branch, `t` tracks a repository, `/`
searches the registry and `x` runs any capability in it.

`d` discards, so it does not run: the registry's own plan is shown — the files,
and the sentence that says the content is not recoverable from Git — and nothing
happens until `y`. Declining records the refusal in the audit trail rather than
dismissing it. A branch switch that would overwrite local work becomes the same
kind of confirm, using the registry's own overwrite check.

It refuses to start when it has no terminal, and says what to use instead. A
branch name containing an escape sequence cannot reach your console: control
characters are replaced before anything is drawn.

See [`docs/tui.md`](docs/tui.md) for the keys, the approval transcript, and how
each claim above is tested.

## Using devd from an agent

```bash
devd mcp                   # serve the Model Context Protocol on stdio
devd mcp --client claude-code
devd mcp --list            # print the tool list
```

```json
{ "mcpServers": { "devd": { "command": "devd", "args": ["mcp", "--client", "claude-code"] } } }
```

The server publishes **five discovery tools**, not one tool per capability:
`devd_groups`, `devd_capabilities`, `devd_describe`, `devd_invoke`,
`devd_doctor`. The catalogue is explored at call time, so a capability added
tomorrow is discoverable immediately and the tool list never grows. A test
asserts that two servers over registries of different sizes publish an identical
list.

An agent reads the registry and runs read-tier capabilities. An operation that
needs approval is **refused**, recorded in the audit trail, and returned to the
agent *with the plan* and an explicit instruction not to report it as having
happened — because the failure being defended against is an agent that
confidently describes work it was not allowed to do. A `human_only` operation is
refused for an agent even if an approval is presented.

See [`docs/mcp.md`](docs/mcp.md) for the tool contract, the intended call
sequence, and the transcript of a real session.

### Approvals

Operations that rewrite history, discard work, or touch a remote require
approval. Interactively you are shown a plan — the repository, the branch, the
target and the affected files — and asked. Non-interactively the command fails
rather than guessing:

```console
$ devd git discard app.txt
discard 1 path(s)
  - paths: app.txt
  DESTRUCTIVE: this can destroy data
  requires explicit approval
devd: approval required for 'git.discard': ... re-run interactively, or pass --yes to approve it deliberately
$ echo $?
3
```

`--yes` is the deliberate, explicit form of that approval. `--plan` shows the
plan and stops. Passing `--yes` to a *dry run* is meaningless, and the tool does
not do anything without one of these two.

Exit codes are stable so scripts can branch on the reason:

| Code | Meaning |
| --- | --- |
| 2 | invalid arguments, or unsupported |
| 3 | approval required, or denied |
| 4 | not found |
| 5 | conflict |
| 1 | anything else |

## Documentation

* [`docs/architecture.md`](docs/architecture.md) — the core, the registry, tiers,
  the store, and how each surface projects it.
* [`docs/app.md`](docs/app.md) — the interface decision, the loopback hardening,
  the two-step approval flow, and what was observed when it ran.
* [`docs/mcp.md`](docs/mcp.md) — the agent surface: the five discovery tools, why
  the list is small, and what an agent is handed when it is refused.
* [`docs/tui.md`](docs/tui.md) — the terminal dashboard: the views, the keys, the
  approval prompt, and the tests behind each claim.
* [`docs/git-cli.md`](docs/git-cli.md) — every Git command, mapped to the
  capability it calls.
* [`docs/security.md`](docs/security.md) — the threat model and the specific
  protections, including what Git offers and where `devd` deliberately does not
  go.
* [`docs/limitations.md`](docs/limitations.md) — what is missing, what is
  weaker than intended, and why.
* [`CHANGELOG.md`](CHANGELOG.md) — what landed in each milestone.

## License

MIT. See [`LICENSE`](LICENSE).

`devd` ships no third-party runtime binaries. nginx, Apache, PHP, MySQL,
MariaDB, Redis, Mailpit, Node and Go are downloaded at the user's request, from
their upstream distributions, under their own licenses. Nothing copyleft is
linked into a `devd` binary.

"devd" is a working name and is not derived from, and does not use, any third
party's trademark.
