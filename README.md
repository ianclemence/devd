# devd

A local development environment manager, written in Rust, whose every action is
a **capability** — a typed, permissioned, auditable operation reachable from the
CLI, a terminal UI, a desktop app, an HTTP API and an MCP server, all of which
are thin projections of one core.

`devd` is a ground-up replacement for the Windows-only Laragon stack: nginx and
Apache, several PHP versions side by side, MySQL/MariaDB, Redis, Mailpit, Node
and Go, automatic local HTTPS, hosts and virtual hosts, and per-project
isolation — on Windows and Linux.

> **Project status.** This repository is at **M0**: the foundation and the
> complete Git integration are implemented and tested. The service-management,
> site, DNS, TLS, TUI, GUI and MCP layers are designed but **not yet built**.
> See [Status](#status) for exactly what works today, and
> [`CHANGELOG.md`](CHANGELOG.md) for what landed in each milestone.

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
  devd-core/   the capability registry, the state store, paths, settings, events
  devd-git/    64 Git capabilities and the typed Git service beneath them
  devd-cli/    the `devd` command
```

* **64 Git capabilities** across 12 discoverable groups (`git.repo`,
  `git.changes`, `git.commits`, `git.history`, `git.branches`, `git.tags`,
  `git.remotes`, `git.merge`, `git.stashes`, `git.worktrees`, `git.config`,
  `git.recovery`).
* **A typed Git service** that drives the user's own `git` executable through
  `argv` arrays — because rebase, bisect, worktrees, submodules, LFS, credential
  helpers and hooks are only faithful when Git itself performs them.
* **Registry-level policy tests**: 98 tests, including the ones that prove a
  declined operation did not execute and that uncommitted work survives a
  decline, an abort and a branch switch.
* **`devd doctor`**, which checks the layout, the store, the Git discovery and
  the environment before anything else is attempted.

## Status

| Area | State |
| --- | --- |
| Capability registry, tiers, approvals, audit, store | **Done** (M0) |
| Git service, capabilities, CLI, tests | **Done** (M0) |
| CLI | **Done** for capabilities, Git, audit and doctor |
| TUI, desktop GUI, HTTP API, MCP server | Designed, **not built** |
| Runtime installers (PHP, Node, Go), services (nginx, Apache, MySQL, Redis, Mailpit), sites, DNS, TLS, SSH/deploy | Designed, **not built** |

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
so the repository and its license stay free of third-party binaries.

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
