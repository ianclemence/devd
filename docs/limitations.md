# Known limitations

This document exists so that the gaps are stated rather than discovered. Nothing
here is a surprise waiting for a user; each entry says what is missing, what the
consequence is, and what would close it.

The current release is milestone **M0**. See [`CHANGELOG.md`](../CHANGELOG.md) for
what is implemented.

## Not implemented at all

These are designed and partially scaffolded, but there is no working code:

| Area | State |
| --- | --- |
| TUI | Not started. |
| Desktop GUI (Tauri v2) | Not started. The capability JSON Schemas it needs exist. |
| HTTP API | Not started. |
| MCP server | Not started. No agent can yet discover or invoke a capability. |
| Runtime installers (PHP, Node, Go) | Not started. No `php.new`-style install yet. |
| Services (nginx, Apache, MySQL/MariaDB, Redis, Mailpit) | Not started. |
| Sites, hosts generation, virtual hosts | Not started. |
| DNS, automatic local HTTPS, certificates | Not started. |
| SSH control of a remote host, deploy | Not started. |
| Snapshots and receipts | The layout defines the directories; no snapshot is taken. |
| Licensing / redistribution checks | Not started. |

**Consequence.** A user can manage Git through `devd` today and nothing else. The
CLI's `capabilities` listing shows the whole surface, which currently consists of
Git.

## Partial or weaker than intended

| Item | What it is | What it would be |
| --- | --- | --- |
| Windows process-tree termination | `taskkill /PID <pid> /T /F`, with a 15-second timeout | A Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which would also guarantee children die when the daemon is terminated abruptly. Today, every ordinary start/stop/restart and clean shutdown path is covered; `TerminateProcess` on the daemon is not. |
| Interactive rebase | Driven by a no-op sequence editor (`GIT_SEQUENCE_EDITOR=true`), so `--autosquash` works but a user cannot reorder, drop or edit commits interactively | A real sequence-editor UI in the TUI and the GUI, which is where interactive rebase belongs. |
| Hunk-level staging | Available as `git.stage.patch` with a unified diff; not exposed as a CLI subcommand | A GUI hunk selector, which is the natural home for it. The capability and the tests exist today. |
| Repository index labels | A repository is labelled by its directory name | A user-settable display name. |
| `devd git repo prune`-style index cleanup | There is no capability that removes index entries whose directory has disappeared | A prune for the index itself. This is separate from `git.repo.prune`, which deletes merged branches. |
| Audit retention | The audit table grows without bound; `devd audit -n` reads the tail | A retention policy and rotation. |
| Secret scanning | A warning when staged content looks like a credential is specified | Not implemented. Previewing a staged diff before a commit currently shows the content but does not flag it. |

## Platform limits

* **No macOS.** Windows and Linux are the targets. Nothing is designed in a way
  that forbids macOS, but it is untested and unsupported.
* **No containers as the substrate.** `devd` runs services as the invoking user
  on the host. This is a deliberate decision, not a gap, but it means a service
  `devd` starts runs with that user's privileges.
* **`git` must be on `PATH`** (or named in `git_executable` in settings). `devd`
  drives the user's own Git rather than bundling one.

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

## Testing gaps

* **No CI.** There are no GitHub Actions workflows in this repository, by
  decision. Every result quoted in the changelog was produced by running the
  suite locally and is reproducible with `cargo test`.
* **No controlled remote fixture over the network.** Remote synchronisation is
  exercised against local bare repositories and `file://` URLs, and failure is
  exercised against a loopback address that refuses the connection. A real
  authentication failure against a real host is not covered by an automated test.
* **No fuzzing** of the Git output parsers.
* **No long-running soak test** of service supervision, because services are not
  implemented yet.
* **The MCP path is untested end to end** because the MCP server does not exist.
  The registry-level policy tests stand in for it: they drive
  `Registry::dispatch` with an agent actor and assert the refusals.
