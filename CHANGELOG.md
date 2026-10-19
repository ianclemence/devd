# Changelog

All notable changes to `devd` are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because `devd` is pre-1.0, each `0.x` release is a milestone, and a milestone
may change capability identifiers where the design was wrong.

---

## [0.3.0] - 2026-10-19 — M2: the agent surface

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
