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
                    ┌───────────────▼──────────────────────────┐
                    │  skill crates: devd-git, devd-runtime,   │
                    │  devd-services, devd-sites               │
                    └───────────────┬──────────────────────────┘
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
   invoked without an approval. An agent cannot supply one. The built-in harness
   is the single scoped exception: it may pre-authorise `fs.delete` inside the
   conversation's project, because the project is the boundary the person chose.
   The root, `.git`, secrets and every path outside it still take the approval
   route, and no other capability gets the exception.
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

## Runs and evidence

A **run** is the unit of work the whole product shares. It has a goal,
acceptance criteria, a mode (assistant, team, scheduled, API, headless), a
project, a lifecycle state, a plan, steps, tool calls, evidence, artifacts,
approvals and a sequenced event log. Conversations and team runs keep their own
tables and link to a run; they do not get a second execution model.

`Registry::dispatch_with` is the normalized entry point. Before a handler runs
it validates the arguments against the capability's JSON Schema, checks the
run's cancellation flag and deadline, and returns a previously successful result
for an idempotent call with the same key. After it runs, it writes the audit
row, the `tool_calls` row and the evidence derived from the real outcome, carrying
the provider's call id and the approval that authorized it. A direct alias, an
MCP call and an internal call therefore produce the same record. A dry run is a
plan and produces no evidence.

Completion is evidence-aware. `CompletionPolicy::for_goal` decides whether the
goal requires a change, a verification, both, or neither; `evaluate` compares
that to typed evidence. A model's statement that it finished is not evidence, so
a code change with no passing check ends `unverified` with the missing evidence
named. The full lifecycle, policy table, context budgets, replay cursor and
recovery rules are in [`runs.md`](runs.md).

`run_events` is the durable record. The live stream (`event: live`) is only an
overlay; a disconnected client reconnects with `Last-Event-ID` and replays from
the log.

## Permission tiers

| Tier | Meaning | Approval |
| --- | --- | --- |
| `Read` | Inspection. Changes nothing. | No |
| `Write` | A reversible, project-scoped change. | No |
| `Exec` | Starts or stops something devd already manages. | No |
| `Destructive` | Can lose data — history, uncommitted work, a file. | **Yes** |
| `Remote` | Affects a machine `devd` does not own. | **Yes** |
| `Admin` | Changes the machine outside devd's folder: the hosts file, the trust store, the user `PATH`. | **Yes** |

Approval is required by the *tier*, not by the handler, so a new capability of
the same risk inherits the gate. A handler cannot forget to ask.

The tier says how much a capability can damage what devd manages. It does not say whose code runs. A
capability whose caller *writes* the code devd will run (a cron command, a deploy script, an artisan command)
is marked `agent_needs_approval`: an agent's call waits for a person to read it, whatever the tier, and a
person calling it is not asked. A test walks the shipped registry for any that were missed. See
[`domain.md`](domain.md#authority).

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

## One graphical surface

The graphical interface is one application window, not a website and not a
second native implementation. `devd ui` serves the interface from a loopback
server and opens it in a window; the server binds `127.0.0.1`, requires a bearer
token minted per process, and cannot be reached from another machine.

A native toolkit beside the HTML interface would be a second implementation of
every list, table, diff and consent screen, and the two would drift the first
time a capability changed. The released desktop shell is Tauri 2 wrapping the
same assets: it starts the same server in-process, points a webview at the same
loopback origin, registers no privileged commands, and adds no second UI. See
[`desktop.md`](desktop.md).

The terminal dashboard is the deliberate second surface: a different medium for
a different situation (SSH, a console without a browser), not a copy of the
same screens.

**The window contains a browser, not a website.** Preview embeds the running
site in an iframe, so the page you are building is one click from the page you
are editing. The iframe is sandboxed and cannot navigate the window.

## Choosing a folder

A person finds a project by looking at their device, not by typing a path. That
needs two things, and they are deliberately different.

**The listing** is a capability: `fs.browse` lists the directories inside a
location, the parent, the path from the root, the filesystem roots and the
places work is kept. It lives in `devd-core` because it is a fact about the
filesystem, and it is platform-correct: Windows drives, UNC paths, the verbatim
`\\?\` prefix, long paths, non-ASCII names, and a directory that cannot be read
— which is reported rather than hidden.

**The dialog** is not a capability. A browser page cannot open the operating
system's own folder picker, and a native dialog is a user-interface act rather
than a domain operation, so an agent should not be able to trigger one. The
desktop shell supplies it to the server as a narrow interface that can only open
a dialog and return a path; the web application, which has no shell around it,
draws its own browser over the same `fs.browse` listing. Both paths end in the
same place: a string that is inspected by the capability that acts on it.

**The brand marks** follow the same rule as the rest of the interface. They are
the owners' own artwork, compiled into the binary and served from
`/brands/{file}`, so the interface never depends on a third party being
reachable and never shows a mark that has been tampered with. A mark is only
shown for something devd actually supports, and it is never recoloured.

## Project processes

`devd.yml` is parsed by `devd-workspace::manifest`, which treats it as executable
configuration: argv arrays only, working directories contained in the project,
loopback-only health checks, an explicit environment, dependency-cycle detection,
and unknown-field rejection. A semantic hash is approved by a human
(`project.approve`, human-only); start and restart refuse without a matching
hash. Processes run under the core supervisor with log capture, health polling,
restart policy and whole-tree termination. See [`devd-yml.md`](devd-yml.md).

## Shared project state

Scratchpads and todos are SQLite rows scoped to the canonical project path, with
authorship, timestamps and optimistic revision checks. They are capabilities,
not files, so an agent uses the same typed API as the window, and an edit
conflict is reported instead of merged silently. See
[`shared-state.md`](shared-state.md).

## Process execution

A child process is started with an `argv` array; the runner never builds a
command line. It is given a timeout, its output is captured with a size cap, and
stopping it kills the whole process tree — `taskkill /T /F` on Windows, a
process group on Linux. A background service is recorded by pid so a later run
can find it.

Services add three rules on top:

* **They outlive the command that started them.** Each is spawned detached, so
  `devd service start nginx` can finish and nginx keeps running. Ownership is
  recorded in `<install dir>/devd.pid` and the pid is checked against the
  process image before it is believed, because Windows recycles pids and a
  recycled pid would make `devd` kill a stranger.
* **A start is verified, not assumed.** The port is polled until it opens or the
  service's own startup window expires. If it never opens, the process is
  killed, its run row is closed as failed, and the error carries the tail of the
  service's own log — a command that says "started" and leaves a dead service
  behind is worse than one that fails.
* **They do not inherit the caller's handles.** See
  `process::isolate_standard_handles` — without this, a piped `devd stack up`
  waited for nginx to exit.

## Services, sites and generated configuration

The dependency direction is one-way: `devd-sites` depends on `devd-services`,
never the reverse.

```text
devd-core  ←  devd-runtime  ←  devd-services  ←  devd-sites
             (PHP, Node, Go)   (nginx, MariaDB,   (sites, vhosts,
                                php-cgi, TLS)      hosts block)
```

* `devd-services` owns the service model, the catalogues, `nginx.conf` rendering,
  the internal certificate authority and MariaDB. TLS lives here rather than in a
  `devd-tls` crate because the site pipeline is its only consumer; a crate is
  split out when a second consumer appears, not in anticipation of one.
* `devd-sites` owns sites, the managed hosts block, and the calls into services
  that make a site real. There is no `devd-dns`: with the hosts file as the only
  resolver, the block is a few hundred lines and belongs where its only caller
  is.
* **`nginx.conf` is generated output.** The store is the source of truth: the
  renderer reads the site table and writes a complete file, so a hand edit is
  overwritten rather than merged. Config *content* uses forward slashes (nginx
  treats `\` as an escape); `argv` uses native paths, because arguments are
  passed as an array and never through a shell.
* **`php-cgi` is not downloaded.** It is the `php-cgi.exe` of each installed PHP
  runtime, turned into a pool of service rows on demand — by `service start php-cgi`,
  by installing nginx, or by `stack up`. A site is sent to the pool of the version it
  runs, and **only a site that serves PHP has PHP in its configuration**: what a site
  serves is decided once, in `devd-core::serving`, and both renderers follow it.
* **A site is not a project.** A project is a codebase and a site is how part of it is
  exposed over HTTP, so a project has zero, one or several sites. Finding projects,
  registering sites, configuring them and activating names are separate steps. The
  definitions, the precedence rules and who owns each piece of state are in
  [`domain.md`](domain.md).

## Skill crates

A skill crate registers capabilities and nothing else. `devd-git` is the
reference: `caps::register(&mut registry, service)` declares 64 capabilities
whose handlers call a typed `GitService`. The service is where Git knowledge
lives — how to parse `for-each-ref`, that `git stash list` needs `%x1f` while
`for-each-ref` needs `%1f`, which reset mode means what.

The other three follow the same shape and the same pattern of a typed service
under a thin capability layer: `devd-runtime` (catalogues, digests, PHP
configuration), `devd-services` (service lifecycle, nginx rendering, TLS, MariaDB)
and `devd-sites` (site lifecycle, hosts file). They register 34 more
capabilities, and they appear in the CLI, the dashboard, the application window
and the agent surface with no new safety decision being made, because all four
read the same declarations.

## Downloads and verification

Nothing is bundled. A runtime or service is downloaded from its vendor when it is
first installed, in-process with `ureq` + `rustls`, and the digest the vendor
publishes is checked before anything is unpacked. Archives are extracted to a
temporary directory and moved into place, so a failed install never leaves a
half-populated install directory that later looks installed.

Where a vendor publishes no digest — nginx is the only one — the install says so
in its own output instead of implying a verification that did not happen. Sizes
are a sanity check only: vendors round them, so a size within 5% is accepted and
the digest is what decides.

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

## Still to come

The registry, the tier model and the audit trail were built first so that the
remaining work is additive: a new skill registers capabilities, and a new surface
renders them. Apache, Redis, Mailpit, a DNS server and SSH/deploy are still to
come - and when they land, their capabilities will appear in the CLI, the
dashboard, the application window and the agent surface without a new safety
decision being made, because all four read the same declarations.

The desktop application is now a native Tauri 2 window over the same server and
the same assets, with no privileged IPC and no second interface. `devd ui` still
serves the browser view for development and for systems where WebView2 is not
available. See [`desktop.md`](desktop.md) and
[`../CHANGELOG.md`](../CHANGELOG.md).
