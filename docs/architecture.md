# Architecture

`devd` is one core and several thin shells. The core owns every decision that
matters — what may be done, by whom, with what arguments, and what is recorded
afterwards. The shells own presentation only.

```text
                    ┌──────────────────────────────────────┐
   devd (CLI) ──────┤                                      │
   TUI         ──────┤           devd-core                  │
   GUI         ──────┤   capability registry + dispatcher   │
   HTTP API    ──────┤   permission tiers + approvals        │
   MCP server  ──────┤   audit trail + state store           │
                    └───────────────┬──────────────────────┘
                                    │  the only path to a side effect
                    ┌───────────────▼──────────────────────┐
                    │  skill crates: devd-git, (devd-sites, │
                    │  devd-runtime, devd-dns, devd-tls, …) │
                    └───────────────┬──────────────────────┘
                                    │  argv arrays, validated input
                    ┌───────────────▼──────────────────────┐
                    │  the system: git, nginx, php, mysql…  │
                    └──────────────────────────────────────┘
```

The rule that keeps this honest is that **`Registry::dispatch` is the only path
to a side effect**. Not "the recommended path" — the only one. A skill crate
registers capabilities whose handlers are the only code that touches the
outside world, and those handlers are reachable only through the dispatcher.

## The capability

A capability is a struct, not a function pointer and not a document. The fields
that matter:

| Field | Purpose |
| --- | --- |
| `id` | Dotted, unique, stable. `git.reset`, not `Reset`. |
| `group` | Discovery namespace, e.g. `git.changes`. Agents load a group, not a catalogue. |
| `summary`, `description` | The text a listing, a `--help`, a GUI tooltip and an agent tool description all read. |
| `tier` | The permission requirement. `Read`, `Write`, `Exec`, `Destructive`, `Remote`. |
| `effect` | `Read` or `Write` — what kind of side effect, orthogonal to risk. |
| `args_schema` | JSON Schema generated from the Rust argument struct. |
| `supports_dry_run` | Whether a plan can be produced without touching anything. |
| `agent_safe` | Whether an agent may invoke it at all. |
| `human_only` | Whether it is reserved for a human, approval or not. |
| `idempotent` | Whether repeating the call is harmless. |
| `undo` | The capability that reverses this one, when one exists. |
| `handler` | The implementation. The only code with side effects. |

Because the argument schema is generated from the same Rust struct the handler
deserialises, the schema cannot drift from the implementation. That is the
property the GUI, the HTTP API and the MCP tool list all depend on.

## The dispatcher

Every invocation goes through `Registry::dispatch`, which runs these checks in
order and refuses before the handler is reached:

1. **The capability exists.** Otherwise `NotFound`.
2. **The operator declined.** `Approval::Denied` stops immediately. The attempt
   is still audited, because a refusal is a fact worth keeping.
3. **Human-only reservation.** If the capability is `human_only` and the actor
   is an agent, refuse — regardless of any approval presented. An approval is
   evidence that a human agreed, not a token an agent may carry.
4. **Agent-safety reservation.** If the capability is not `agent_safe` and the
   actor is an agent, refuse.
5. **The tier's approval requirement.** `Destructive` and `Remote` cannot be
   invoked without an approval. An agent cannot supply one.
6. **Dry-run support.** A dry run is refused for a capability that cannot
   describe itself without acting.
7. **Run the handler**, then record the outcome.

The ordering is the design. Human-only is checked *before* approval so that
"approve it as an agent" is not a thing that exists. Decline is checked first of
all so that a decline is final.

## Actors and surfaces

An `Actor` is who is asking: a human at a surface, an agent naming its client, or
the daemon acting for itself. A `Surface` is where they asked from: CLI, TUI,
GUI, HTTP, MCP or daemon.

Both are recorded on every audit row. The agent actor implies the MCP surface,
which is why a test can assert that an agent-driven call is recorded as such.
The separation matters because the product's central promise is that a refusal
is not local to a surface: `devd` denies at the registry, so the same request
from a different shell is the same request.

## Permission tiers

| Tier | Meaning | Approval |
| --- | --- | --- |
| `Read` | Inspection. Changes nothing. | No |
| `Write` | A reversible, project-scoped change. | No |
| `Exec` | Runs a process to accomplish the task. | No |
| `Destructive` | Can lose data — history, uncommitted work, a file. | **Yes** |
| `Remote` | Affects a machine `devd` does not own. | **Yes** |

Approval is required by the *tier*, not by the handler, so a new capability of
the same risk inherits the gate. A handler cannot forget to ask.

A plan must be able to describe a destructive capability before it runs, so
"requires approval" implies "supports a dry run". A test asserts that for every
capability in the registry. A plan that declares itself destructive also
declares that it requires approval, so the two cannot contradict.

## The plan

A plan is what a user or an agent sees before approving: a summary, a list of
steps, a list of effects, a destructive flag, an approval flag, and structured
data — the repository, the branch, the target, the affected files.

Producing a plan is a dry run: it calls the capability's planner, never its
handler. `devd --plan` prints one and exits. The GUI and an agent use the same
plans to explain a consequence before asking for a decision.

## State

The store is SQLite, opened at `DEVD_HOME/state.db`, with a versioned schema
migrated on open. It holds the audit trail, the repository index, and — as the
service layer arrives — services, versions, sites, certificates and receipts.

`Store::open_in_memory()` gives tests the same schema with no file, which is why
the policy tests can drive the real dispatcher and still assert on audit rows.

## Paths and settings

`Paths::discover()` resolves a root from `DEVD_HOME`, or a platform default;
`Paths::ensure()` creates the layout. `Paths::rooted(dir)` is what tests use.

Settings are layered — defaults, then a file, then the environment — so a
machine can be configured without editing a checked-in default. `devd doctor`
reports the resolved values instead of assuming them.

## Surfaces: the HTTP API and the application window

The application is the first surface that is not a terminal. It is a **loopback
HTTP API plus a client of that API**, and the API is a projection of the registry
in the same sense the CLI is: `GET /api/capabilities` calls `Registry::search`,
`POST /api/invoke` calls `Registry::dispatch`, and there is no endpoint that
holds business logic of its own. There is deliberately no `/api/git/status`
route, because that would be a second implementation of `git.status` that could
disagree with the CLI.

Two things had to be decided rather than inherited.

**Who the actor is.** A browser can claim anything, so the server **overrides**
the actor to `human:http`. A caller cannot choose to be an agent, and therefore
cannot escape the agent restrictions by claiming to be a person.

**What an approval is.** The naive design is `POST /api/invoke { approve: true }`,
which makes the approval a boolean the *client* asserts. That is the exact
property the product exists to prevent, so it is not the design. Instead,
`POST /api/invoke` for a destructive capability returns a plan and a single-use
token and **changes nothing**; `POST /api/approve { token }` then replays the
server's own stored record of the invocation — the capability, the arguments, the
actor — through the dispatcher with `Approval::Granted`. What was displayed and
what executes cannot differ, because the executing arguments never came back from
the client.

`POST /api/deny { token }` is a first-class outcome: it dispatches with
`Approval::Denied`, so the refusal reaches the dispatcher, the audit trail and
the event stream. A declined operation is therefore never indistinguishable from
one that never arrived.

The loopback boundary is defended in the server, not in the page: a `Host`
allow-list, an `Origin` check, a refusal of every `OPTIONS` request, and a
256-bit bearer token. [`app.md`](app.md) records the details, why each one is
there, and the results observed against the running binary.

## Surfaces: the terminal dashboard

`devd tui` is a projection in the same sense: `Surface::Tui` and
`Actor::human(Surface::Tui)` go to the dispatcher, and every row on screen came
out of a capability. It exists because the application window needs a browser and
an SSH session does not have one.

It is deliberately split so that nothing that matters needs a terminal:

| Part | What it is |
| --- | --- |
| `state` | Plain data: the view, the cursor, and typed rows parsed from capability results. No terminal types. |
| `controller` | Keystrokes in, capability calls out, over a `Backend` trait. |
| `view` | A pure function from the state to a frame. It reads and never writes. |
| `run` | Owns raw mode, the event loop and the terminal, and nothing else. |

The reason for the split is that the interesting behaviour is "a destructive key
must not run until it is approved, and declining must be recorded", and that
should be provable in a test rather than watched for by hand. A fake backend that
*counts what ran* makes it a measurement.

The approval flow is arrived at independently from the HTTP API's token and is
the same shape, which is the point: the dashboard calls `dispatch` with no
approval, reacts to `approval_required`, asks the registry for the plan, and
replays the exact request it already built when the answer is yes. The refusal
path dispatches with `Approval::Denied` rather than closing the prompt, so a
decline reaches the audit trail.

`view` being a pure function of the state is also what makes the frames
testable: the tests draw into `ratatui`'s `TestBackend` and assert on the
resulting buffer.

## Process execution

A child process is started with an `argv` array; the runner never builds a
command line. It is given a timeout, its output is captured with a size cap, and
stopping it kills the whole process tree — `taskkill /T /F` on Windows, a
process group on Linux. A background service is recorded by pid so a later run
can find it.

## Skill crates

A skill crate registers capabilities and nothing else. `devd-git` is the
reference: `caps::register(&mut registry, service)` declares 64 capabilities
whose handlers call a typed `GitService`. The service is where Git knowledge
lives — how to parse `for-each-ref`, that `git stash list` needs `%x1f` while
`for-each-ref` needs `%1f`, which reset mode means what.

The remaining skills follow the same shape. Their capabilities will appear in
the same list, with the same tiers and the same audit, without any new safety
decision being made.

## Why Git shells out

`devd` drives the user's own `git` executable rather than reimplementing Git.
Rebase, bisect, worktrees, submodules, LFS, credential helpers, hooks and the
index format are only faithful when Git performs them; a partial
reimplementation would be a subtle source of data loss.

The cost is that Git's output must be parsed, so parsing is treated as a place
bugs live. Format strings pin the shape of the output, `LC_ALL=C` pins the
language, the pager is disabled, and every format-string quirk that was found by
test has a comment saying why. Three real bugs were found and fixed this way
during M0 alone: `git branch -b` does not exist, `switch_safety` compared the
merge base instead of the two commits, and `%1f` is literal in `git stash list`.

## Not built yet

The registry, the tier model and the audit trail were built first so that the
remaining work is additive: a new skill registers capabilities, and a new
surface renders them. The runtime, service, site, DNS and TLS skills are still to
come — and when they land, their capabilities will appear in the CLI, the
dashboard, the application window and the agent surface without a new safety
decision being made, because all four read the same declarations. The application
window is a browser engine in application mode rather than a native shell; Tauri
is the intended eventual wrapper, and the interface does not change when that
happens because it speaks only to the local API. See
[`../CHANGELOG.md`](../CHANGELOG.md).
