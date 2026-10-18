# Changelog

All notable changes to `devd` are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because `devd` is pre-1.0, each `0.x` release is a milestone, and a milestone
may change capability identifiers where the design was wrong.

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

[0.1.0]: https://github.com/ianclemence/devd/releases/tag/v0.1.0
