# Known limitations

This document exists so that the gaps are stated rather than discovered. Nothing
here is a surprise waiting for a user; each entry says what is missing, what the
consequence is, and what would close it.

The current release is milestone **M3**, which adds the terminal dashboard. See
[`CHANGELOG.md`](../CHANGELOG.md) for what is implemented.

## Not implemented at all

These are designed and partially scaffolded, but there is no working code:

| Area | State |
| --- | --- |
| Runtime installers (PHP, Node, Go) | Not started. No `php.new`-style install yet. |
| Services (nginx, Apache, MySQL/MariaDB, Redis, Mailpit) | Not started. |
| Sites, hosts generation, virtual hosts | Not started. |
| DNS, automatic local HTTPS, certificates | Not started. |
| SSH control of a remote host, deploy | Not started. |
| Snapshots and receipts | The layout defines the directories; no snapshot is taken. |
| Licensing / redistribution checks | Not started. |
| Self-update and upgrade check | Not started. Nothing in `devd` contacts GitHub. The release list the mechanism will read is public and reachable (`GET /repos/ianclemence/devd/releases?per_page=3`), and the releases carry the source archives GitHub generates, but no code reads them yet. |
| An in-app conversational agent | Not started. The MCP server is the agent surface today. |

**Consequence.** A user can manage Git through `devd` — from the CLI, the
application window and the terminal dashboard — and nothing else. The Services,
Sites and Runtimes pages say plainly that they are not implemented rather than
offering switches that do nothing.

## The terminal dashboard

| Item | What it is | What it would be |
| --- | --- | --- |
| The interactive path | The state, the controller, the frames and the refusals are covered by 57 automated tests, but crossterm's raw mode, the event loop and the drawing to a real console are exercised only by hand | A pty-based test that drives the real event loop and asserts on what a terminal receives |
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
| Windows process-tree termination | `taskkill /PID <pid> /T /F`, with a 15-second timeout | A Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, which would also guarantee children die when the daemon is terminated abruptly. Today, every ordinary start/stop/restart and clean shutdown path is covered; `TerminateProcess` on the daemon is not. |
| Interactive rebase | Driven by a no-op sequence editor (`GIT_SEQUENCE_EDITOR=true`), so `--autosquash` works but a user cannot reorder, drop or edit commits interactively | A real sequence-editor UI in the TUI and the GUI, which is where interactive rebase belongs. |
| Hunk-level staging | `git.stage.patch` with a unified diff, used by the application's per-hunk controls | A graphical hunk selector in the desktop shell. The capability and the tests exist today. |
| Repository index labels | A repository is labelled by its directory name | A user-settable display name. |
| `devd git repo prune`-style index cleanup | There is no capability that removes index entries whose directory has disappeared | A prune for the index itself. This is separate from `git.repo.prune`, which deletes merged branches. |
| Audit retention | The audit table grows without bound; `devd audit -n` reads the tail | A retention policy and rotation. |
| Secret scanning | A warning when staged content looks like a credential is specified | Not implemented. Previewing a staged diff before a commit shows the content but does not flag it. |
| Instance discovery | `devd ui` finds a running instance by probing the remembered port and the next few for the `X-Devd-App` header | A lock file or a named pipe, which would be exact rather than a scan. |

## Platform limits

* **No macOS.** Windows and Linux are the targets. Nothing is designed in a way
  that forbids macOS, but it is untested and unsupported.
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

## Testing gaps

* **No CI.** There are no GitHub Actions workflows in this repository, by
  decision. Every result quoted in the changelog was produced by running the
  suite locally and is reproducible with `cargo test`.
* **The HTTP API is tested in-process and once on a socket.** The test suite
  drives the real router with `tower::ServiceExt::oneshot`, and the loopback
  defences and the approval flow were additionally exercised against the release
  binary with `curl`. There is no automated test that binds a port, and no
  browser-automation test of the interface itself: the UI is checked by
  construction (a test asserts the shell carries no inline script, style or
  handler) and by hand, not by a headless browser.
* **No controlled remote fixture over the network.** Remote synchronisation is
  exercised against local bare repositories and `file://` URLs, and failure is
  exercised against a loopback address that refuses the connection. A real
  authentication failure against a real host is not covered by an automated test.
* **The MCP path is tested in-process and once over a real pipe.** 21 tests drive
  `McpServer::handle` directly, and a full session — initialize, discover,
  describe, invoke a read operation, invoke a destructive one, doctor — was run
  against the built binary through stdin/stdout with the audit rows checked
  afterwards. What is not covered is a specific third-party MCP client's
  behaviour, since that is the client's to test.
* **The dashboard is tested without a terminal.** 52 unit tests drive the
  controller with real keystrokes against a fake backend, and 5 integration tests
  drive it against the real registry and a real repository, rendering frames into
  `ratatui`'s `TestBackend` and asserting on the buffer. What is not covered is
  the part that needs a real console — raw mode, the event queue, the actual
  drawing — because this project has no way to attach a pty in its test
  environment. That path was run by hand.
* **No fuzzing** of the Git output parsers.
* **No long-running soak test** of service supervision, because services are not
  implemented yet.
