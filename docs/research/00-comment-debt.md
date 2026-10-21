# Comment-debt audit — findings and proposed remediation

**Status: awaiting approval.** Nothing in this document has been changed in the
code except where the change was a functional fix that had to be made anyway
and is listed under *Already applied*.

**Scope.** Every source file in the workspace (`crates/**`, `scripts/**`,
`installer/**`, `.cargo/config.toml`). Prose in `docs/**`, `README.md` and
`CHANGELOG.md` is documentation, not a comment, and was not audited here.

**Method.** 113 source files were scanned. The Rust code carries 7,967 `///` and
`/** … */` documentation comments and 1,905 `//` comments, of which the vast
majority are section separators (`// ---- arguments ----`). The JavaScript
carries block comments that explain *why*. There is no flood of redundant
narration. The findings below are therefore not "too many comments" but a
small number of places where a comment, a doc comment or a piece of prose
describes something the code does not do, or where a comment marks a
workaround that should have been fixed at the source.

---

## Summary

| # | Finding | Class | Priority |
|---|---|---|---|
| 1 | `devd undo` does not exist, but the receipt API documents it as if it does | Delete | P1 |
| 2 | A 282-line duplicate terminal implementation is never compiled | Delete | P1 |
| 3 | The HTTP route comment says mutating operations go through the registry; three do not | Refactor | P0 |
| 4 | `status_for()` is dead, and its comment claims a test uses it | Delete | P1 |
| 5 | `spec_run_id()` is a stub that returns an empty string | Refactor | P2 |
| 6 | `.cargo/config.toml` documents a Zig linker workaround that no longer describes this machine | Refactor | P2 |
| 7 | `docs/limitations.md` says the app has no runtime, service or site screens; it has all three | Delete | P1 |
| 8 | The `provider_view` doc comment and the shape it returns disagree | Refactor | P2 |
| 9 | The `fs.*` capabilities are declared in the *agent* crate, with a comment that does not say so | Improve API | P2 |
| 10 | The `no-comments` skill contradicts the repository's own style rules | Enforce mechanically | P2 |

---

## 1. The receipt API documents a command that does not exist

**Files and lines**

- `crates/devd-core/src/capability.rs:368` — `/// Identifier of a receipt that
  can be passed to `devd undo`.`
- `crates/devd-core/src/capability.rs:397-401` — `Outcome::receipt()`, a builder
  that sets it.
- `crates/devd-core/src/state.rs:870-885` — `Store::create_receipt`.
- `crates/devd-core/src/state.rs:888` — `Store::get_receipt`.
- `crates/devd-core/src/sql/001_init.sql:94-101` — the `receipts` table.

**Problem.** Nothing in the workspace calls `.undo(`, `.receipt(` or
`create_receipt` outside their own definitions. There is no `devd undo`
command; `devd git undo` exists and is a different thing (it reverts the last
commit). The doc comment therefore sends a reader — human or agent — looking for
a command that will never be there, and the whole receipts table and its API
are a promise the product does not keep.

**Proposed fix.** Either (a) delete the receipt API, the `receipts` table and
the `undo` field, or (b) build the undo command. **(a) is the recommendation**:
the capability model already expresses reversibility through `Tier::Write` plus
`idempotent`, and the one destructive operation a person is most likely to want
to undo — a `git.discard` — is already a `git.reset` with a mode.

**Risk.** Low. The `receipts` table is empty in every existing installation and
nothing reads it. Removing the API is a compile-time change across two crates.

**Classification: Delete.**

---

## 2. A never-compiled duplicate of the terminal implementation

**Files and lines**

- `crates/devd-server/src/terminal.rs` — 282 lines: its own `HARNESSES` table
  (`:34-50`), its own `Terminals`/`Terminal`/`Output` types (`:52-71`) and its
  own `harnesses()` (`:74`).
- `crates/devd-server/src/lib.rs:44-46` — declares only `pub mod api; pub mod
  approval; pub mod ui;`. `terminal` is not among them.

**Problem.** The module is not compiled, so it is not merely unused: it cannot
be, and every future reader will assume it is live. It duplicates
`crates/devd-workspace/src/session.rs`, which *is* live and is the
implementation the HTTP layer calls. Two implementations of the same thing,
one of them invisible, is exactly the kind of drift the rest of the codebase is
carefully arranged to avoid.

**Proposed fix.** Delete `crates/devd-server/src/terminal.rs`.

**Risk.** None to behaviour: the file is not compiled, so deleting it cannot
change what the program does. It removes a trap.

**Classification: Delete.**

---

## 3. The HTTP route comment is contradicted by three routes

**Files and lines**

- `crates/devd-server/src/api.rs:423-430` — the comment above the terminal
  routes.
- `crates/devd-server/src/api.rs:686-691` — `POST /api/sessions/:id/stop` calls
  `sessions().stop(&id)` directly.
- `crates/devd-server/src/api.rs:710-718` — `DELETE /api/sessions/:id` calls
  `sessions().dismiss(&id)` directly.
- `crates/devd-server/src/api.rs:674-683` — `POST /api/sessions/:id/rename`
  calls `sessions().rename(...)` directly.
- `crates/devd-server/src/api.rs:498-516` — `POST /api/sessions` starts a
  session directly.
- Compare `crates/devd-workspace/src/caps.rs:161-174` (`session.start`),
  `:355-367` (`session.rename`), `:384-396` (`session.stop`) and `:412-425`
  (`session.dismiss`).

**Problem.** The comment says: *"Mutating operations that an agent can also
perform are declared as capabilities and dispatched through the registry."* All
four operations above are declared as capabilities **and are not dispatched
through the registry** over HTTP. `session.dismiss` is `Tier::Destructive` as a
capability, which means the CLI, the TUI and MCP all require an explicit human
approval to dismiss a session; over HTTP it requires nothing. The same operation
has two different permission outcomes depending on the surface, which is the
one thing the registry exists to prevent.

This is a code defect that a comment is currently hiding, which is why it
belongs in this report: the comment is the only place the contradiction is
visible.

**Proposed fix.** Route the four mutating session operations through
`registry.dispatch()` in the HTTP layer, exactly as `/api/invoke` does, so the
approval gate applies uniformly. `session.dismiss` then requires an approval
over HTTP too, and the approval flow (`/api/approve`, `/api/deny`) is reused
rather than duplicated.

**Risk.** Moderate. `crates/devd-server/tests/http.rs:866` exercises a session
lifecycle over HTTP and will need the approval round-trip added; the change is
contained to `api.rs` and the four handlers.

**Classification: Refactor.** *This is a behavioural change and needs its own
tests, not just a comment edit.*

---

## 4. `status_for()` is dead, and its comment claims otherwise

**Files and lines**

- `crates/devd-server/src/api.rs:390-398` — `pub fn status_for(err: &DevdError)
  -> StatusCode`, with the doc comment *"Re-exported so tests can assert on the
  status a `devd` error maps to"*.
- `crates/devd-server/src/lib.rs:471-488` — `error_response()`, the mapping
  that is actually used.

**Problem.** `status_for` has exactly one occurrence in the workspace: its own
definition. Its doc comment says a test asserts on it; no test does. It is also
*wrong*: it does not handle `DevdError::Timeout` or `DevdError::Command`, which
`error_response` maps to 504 and 502. Two mappings of the same enum, one dead
and one wrong.

**Proposed fix.** Delete `status_for`. If a test wants to assert the mapping,
it should assert through `error_response`, which is the function the server
uses.

**Risk.** None. The function is unreachable from any live path.

**Classification: Delete.**

---

## 5. `spec_run_id()` is a stub

**Files and lines**

- `crates/devd-core/src/process.rs:364-367` — `fn spec_run_id(_guard:
  &HashMap<String, ManagedProcess>, _name: &str) -> String { String::new() }`
- `crates/devd-core/src/process.rs:256` — `Ok((pid, spec_run_id(&guard, &spec.name)))`

**Problem.** The function takes two arguments it does not use and returns an
empty string. Its result is handed back to callers as the run identifier, so
every supervised process reports an empty run id. A reader reasonably assumes
the identifier is computed and has a bug; it has neither.

**Proposed fix.** Either give it a real implementation (a monotonic counter
plus the process name, which is what "run id" implies), or remove it and let
the supervisor report the pid and the name it already has. The second is
simpler and loses nothing that is used.

**Risk.** Low. The value is not consumed anywhere that depends on it being
non-empty.

**Classification: Refactor.**

---

## 6. `.cargo/config.toml` documents a machine-specific workaround that is no longer accurate

**Files and lines**

- `.cargo/config.toml:1-30` — the whole file.

**Problem.** The comment describes a machine where `cc` is a launcher that
forwards to the Zig compiler, and where Zig-linked binaries crash at startup.
On the machine this work was done on, `cc` is a portable MinGW-w64 GCC and the
Zig launcher is gone. The settings are still correct — `rust-lld` is the right
linker for this target and the 8 MB stack reserve is still needed — but the
*reason* recorded is now wrong, and a future reader will look for Zig.

**Proposed fix.** Rewrite the comment to state the two facts that are still
true: (a) the `x86_64-pc-windows-gnu` target is used because MSVC build tools
are not required, and `rust-lld` is used as the linker so the build needs no
MSVC installation; (b) the 8 MB stack reserve is needed because the clap derive
macro's single frame exceeds the 1 MB Windows main-thread reserve in an
unoptimised build. Keep the scope to one target triple.

**Risk.** None. A comment.

**Classification: Refactor.**

---

## 7. `docs/limitations.md` says the application has screens it has

**Files and lines**

- `docs/limitations.md:23` — *"Runtime, service and site panels in the app and
  the dashboard | Not built. The capabilities exist and the CLI covers them;
  the application's capability inspector can invoke them, but there are no
  dedicated screens."*

**Problem.** The web application has a **Server** area with Overview, Sites,
Services, Runtimes, Certificates, Hosts and Preview screens
(`crates/devd-server/ui/app.js:699`, `:827`), and the command palette reaches
them. The document says the opposite. A limitation document that is wrong is
worse than one that is missing, because it is what a reader trusts when they
decide whether something is possible.

**Proposed fix.** Rewrite the row to describe what is actually missing: there is
no per-site PHP version routing, no HTTP/2, and the TUI has no runtime, service
or site screens. Those are the real gaps.

**Risk.** None. Documentation.

**Classification: Delete and replace with the true limitation.**

---

## 8. `provider_view` and its documentation disagree

**Files and lines**

- `crates/devd-agent/src/caps.rs:450-473` — `provider_view()`.
- `crates/devd-agent/src/caps.rs:716-718` — `provider.save` rejects a `preset`
  that is not a known preset.

**Problem.** `provider.save` treats `preset` as a *hint*: sending an unknown
one is an error. The web interface therefore could not edit a provider whose id
was not itself a preset id (`anthropic-2`, or any custom provider), because it
sent `preset: <the provider's own id>`, which the server rejected with
*"unknown preset"*. The function's shape did not say which fields were
optional, so nothing warned the interface.

This was found and **fixed** while implementing the brand marks (the view now
carries `preset`, and the interface sends it only when it is real). It is listed
here because the underlying pattern — a struct whose optionality is only
discoverable by reading the handler — is worth a general remedy.

**Proposed fix.** General remedy: for every capability whose arguments are
partly optional and partly inherited from an existing record, state the
inheritance in the argument's doc comment, and have the *view* carry the same
field names the *save* accepts. `provider_view` now does this for `preset`.

**Risk.** Low. Documentation and a shape already aligned.

**Classification: Improve API or abstraction.** *Partly already applied.*

---

## 9. The `fs.*` capabilities live in the agent crate

**Files and lines**

- `crates/devd-agent/src/caps.rs:984-1039` — `fs.list`, `fs.read`, `fs.search`,
  `fs.write`, `fs.edit`, `fs.move`, `fs.delete`.
- `crates/devd-agent/src/fs.rs` — the implementation.

**Problem.** Reading and writing files is not an agent concern. The
capabilities are declared in the crate that builds the agent, so a reader
looking for "what can devd do with files" has to know that the answer lives in
`devd-agent`. The new `fs.browse` capability would inherit the same
confusion.

**Proposed fix.** Move the `fs.*` capabilities and their implementation into
`devd-core` (or a new `devd-fs` crate), and have `devd-agent` depend on it as
it already does on `devd-core`. This is a mechanical move: the handler bodies
do not use anything from `devd-agent`.

**Risk.** Low to moderate. A crate-boundary move touches `Cargo.toml` files, the
composition root in `crates/devd-bootstrap/src/lib.rs:59-64`, and the module
list of `devd-agent`. The capability ids and their behaviour do not change, so
no test outside the crate needs to change.

**Classification: Improve API or abstraction.**

---

## 10. The `no-comments` skill contradicts the repository's own rules

**Files and lines**

- `crates/devd-agent/skills/no-comments.md` — a shipped skill telling agents to
  avoid comments.
- `crates/devd-server/ui/style.css:1-15` — the stylesheet's own rules, which
  are written as prose precisely so the reasoning survives.

**Problem.** The repository's convention is that a comment explains *why* and
never restates the code; the Rust code is 100% `///` documentation comments and
`//` section separators, and the JavaScript explains each non-obvious decision
in a block. A skill that tells an agent to remove comments, shipped inside the
product and offered to agents working in this repository, encourages exactly the
behaviour the codebase is organised to avoid. It is the clearest example of
"an instruction that encourages agents to reproduce obsolete patterns".

**Proposed fix.** Rewrite the skill so it says what the codebase actually does:
*delete a comment that repeats the code; keep and sharpen a comment that
explains a decision, a constraint, an invariant or a security property.* Add
the "no comment" rule to the stylesheet's own list so the two agree.

**Risk.** None. A skill is prose; it is loaded on demand and changes no
behaviour.

**Classification: Enforce mechanically** — the rule belongs in the skill *and*
in the repository's own agent instructions, so that an agent reading either one
does the same thing.

---

## Already applied

These were functional fixes that had to be made to deliver other work. They are
listed so the record is complete.

| Change | Where | Why |
|---|---|---|
| A malformed CSS rule that silently disabled the bottom half of the stylesheet — including every responsive breakpoint — was removed. | `crates/devd-server/ui/style.css:1815` | The rule left a brace open, so the browser ignored everything after it. The application looked correct at desktop width and fell apart below 960 px. A contract test now checks that the stylesheet is balanced. |
| `git.reflog` now returns structured entries instead of lines of text. | `crates/devd-git/src/lib.rs:1446-1470`, `crates/devd-git/src/caps.rs:697-708` | The interface expected objects and received strings, so the Recover tab rendered blank rows and offered "Reset to this" with an empty revision. The comment-free code now matches what the doc says it does. |
| The "Not yet implemented" panel on the Diagnostics screen, which claimed Services, Sites, Runtimes, DNS and certificates had no capabilities, was removed. | `crates/devd-server/ui/app.js:3684-3689` | The same screen renders all five. The panel was stale text shipped next to working features. |
| The false "uncertain" badge next to "no known stack" was removed. | `crates/devd-server/ui/app.js:566-575` | "Uncertain" only means something when there is a stack to be unsure about. |
| The "24 stacks" figure, written into the interface, now comes from the server. | `crates/devd-server/ui/app.js:797`, `:4278` | A number written in the interface is a number that will be wrong one day. |
| The `hidden` attribute now hides what it says it hides. | `crates/devd-server/ui/style.css:17-25` | A rule that sets `display` beat the attribute, so a button that was meant to be hidden was visible. |

---

## What was deliberately *not* changed

* **The `///` documentation comments.** 7,967 of them. They are the API
  documentation of a library, they are generated into the schema the interface
  and the agents read, and they state the invariants that the types cannot.
  They are the model the rest of the repository should follow.
* **The section separators** (`// ---- arguments ----`). They cost one line
  each and make a 2,000-line file navigable.
* **The `.cargo/config.toml` settings themselves.** Only the reason recorded
  for them is out of date.
* **The prose in `docs/agents.md` and the skills.** Apart from finding 10, that
  document is accurate and is the best description of the agent system in the
  repository.
