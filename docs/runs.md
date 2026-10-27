# Runs, evidence and recovery

A **run** is one unit of work: what a person asked for, what the agent and the
environment did about it, what was observed, and whether the result is real. The
conversation, the team scheduler, the HTTP API, MCP and the desktop window are
views of the same run, not separate execution models.

The tables are `runs`, `run_steps`, `tool_calls`, `evidence`, `artifacts`,
`approvals` and `run_events` (migration `017_runs.sql`). `chat_*` and `team_*`
rows keep their history and are linked to a run; they are not replaced or
rewritten.

## Lifecycle

```text
intake -> planning -> awaiting_approval -> executing -> verifying -> review -> completed
                                  ^            |            |          |
                                  |            v            v          v
                                  +--------- paused / blocked / failed / unverified / cancelled
```

`RunState` validates every transition in `crates/devd-core/src/runs.rs`. A
terminal state is not reopened by accident:

* `completed`, `failed`, `unverified`, `blocked` and `paused` can be continued
  only by a deliberate retry or continuation. That call is a person's; an agent
  cannot retry its own run.
* `cancelled` is final. Retrying a cancelled conversation creates a new run for
  the same thread rather than reopening the old one.
* `executing` may go straight to `completed` when evidence already satisfies the
  policy (a read-only answer, for example), and `verifying` may complete without
  a separate review state when nothing needs a human decision.

Modes are `assistant`, `team`, `scheduled`, `api` and `headless`. They share the
same lifecycle and evidence rules; only the driver differs.

## What is recorded

| Record | What it says |
| --- | --- |
| `runs` | Goal, acceptance criteria, mode, project, state, plan, fingerprint, cancellation, usage, timestamps, parent run, thread and team-run link. |
| `run_steps` | One stage of work: kind, title, state, detail, plan data, start and finish. |
| `tool_calls` | One normalized capability call: run and step, provider call id, actor, surface, capability, redacted arguments, argument hash, approval, idempotency key, outcome, error code, exit code, duration, result and timestamps. |
| `evidence` | What was observed: kind, capability, pass/fail, claim, summary, bounded detail, and the tool call or artifact it came from. |
| `artifacts` | Bounded raw output and produced files, referenced from evidence instead of being copied into every database row. |
| `approvals` | The durable decision: hashed id, run/task/tool-call links, capability, arguments, argument and plan hashes, scope, tier, risk, actor, expiry, state, decider and reason. |
| `run_events` | Sequenced durable events with a global id and a per-run sequence. They are the replayable record, not the live stream. |

`Registry::dispatch_with` writes the tool call and derived evidence at the
boundary, so a direct alias, `devd_invoke`, an MCP tool or an internal call has
the same trail. A dry run is audited but is a plan, not a side effect, and does
not produce evidence.

An outside agent that drives devd through `devd mcp` brings no run of its own:
its session belongs to another program. Each call it makes is one headless run,
settled when the call returns, or left `blocked` with the queued request named
when a person must decide first. Approving the request records the operation in
that run and settles it; declining it cancels the run, because it can never
proceed. A caller that already has a run - the built-in assistant, a team
member - keeps it, so one tool call never opens a second, competing timeline.

Arguments are redacted by key name before they are stored; an argument hash is
kept so two calls can be compared without retaining secret values.

## Evidence and completion

`evidence_for` normalizes a capability result into one of these kinds:

`change`, `read`, `verify`, `command`, `service`, `http`, `runtime`, `database`,
`probe`, `structured`.

A file edit, commit, site or stack change is `change`. A project task named
`test`, `check`, `lint` or `build` and a process that exits 0 is `verify`. A
command that is not a check is `command`. Service readiness, HTTP/TLS probes,
runtime usability and database connectivity have their own kinds. Anything else
is `structured` and cannot satisfy a verification requirement by itself.

A read is `read`, never `change`: `fs.read`, `fs.list`, `fs.search`, `git diff`,
`git status`, `git log`, `project.detect`, `project.tasks` and
`environment.manifest` describe state, so they can support a review but cannot
satisfy a change requirement. Reading a file is not editing it, and inspecting a
service is not starting it.

A call that returns `ok` is not automatically evidence of success. Readiness is
read from the payload: a service passes only when it reports `running` and
`listening`, a stop only when it reports not running, `site.verify` only when its
own `ok` is true, and `workflow.setup` only when `verified` is true. A capability
that returns `ok` with a negative payload is recorded as evidence that did not
pass.

`CompletionPolicy::for_goal` reads the goal and acceptance criteria:

* research, explanation and review require no change or verification;
* a failing-test goal requires a change and a passing verification;
* setup, runtime, service and site goals require the matching readiness
  evidence;
* a code change requires both an inspectable change and a passing check;
* any explicit acceptance criterion requires verification.

The model's sentence that it finished is not evidence. A run whose change was
made but not checked ends `unverified`, with the missing evidence named. A run
whose check ran and failed ends `unverified` or `failed`; it never reports
success because a process stopped consuming resources.

## Artifacts

A result up to 24 KB is stored inline on the tool call. A larger result is
written to `<root>/artifacts/<run>/<capability>-<id>.json` (8 MiB cap) and the
row keeps a bounded preview plus the artifact id, byte count and SHA-256. The
tool call and every evidence row derived from it reference the same artifact, so
a timeline can show what was observed without loading it into the model's
context, and a replay returns the stored result rather than the preview.

## Approvals

There is one approval service (`Store::create_approval`, `consume_approval`,
`decline_approval`, `reschedule_approval`) and one table. Every surface uses it:

* the conversation queues an `agent_requests` row that is a projection of a
  durable approval (`approval_id`);
* the HTTP token flow consumes the same durable row;
* the by-id endpoints and the desktop window decide by id;
* `Hub::decide` consumes it before dispatching the approved operation.

The returned token is 256 random bits. Only its SHA-256 hash is stored as
`approvals.id`, so a database read cannot replay a decision. The row records the
capability, the exact arguments, their hash, and the plan hash when the approval
is plan-bound. Consumption is a compare-and-set on `state='pending'`:

* a second use fails rather than repeating the operation;
* a declined or expired row cannot be consumed;
* expiry is checked when the decision is made, not only by a sweeper;
* changed arguments or a changed plan do not match the stored hashes;
* restart and recovery read the same row, so a decision made in another window
  or after a restart is not lost.

Pending legacy `agent_requests` are migrated as pending approvals with
`approval_id = id`; nothing is auto-granted. An agent cannot approve its own
operation: the decider must be a human actor, and the registry's `human_only`
check runs before approval is considered.

## Context

`crates/devd-core/src/context.rs` assembles the model's working set:

* a model-aware window, from the catalogue when available and from the model
  name otherwise, with output and tool-call reserve subtracted;
* system and project instructions, discovered from repository and nested
  guidance files, marked as project data rather than policy;
* the goal, acceptance criteria, current plan, run state and pending approvals;
* notices, errors and approval decisions, which are never silently dropped;
* recent messages and tool results, with older results summarized and their raw
  output kept as an artifact;
* a `ContextManifest` recording what was included, what was summarized, why, and
  the estimated budget.

The estimate is deliberately conservative (`chars / 4`) when an exact tokenizer
is not available. Compaction is deterministic and testable. A long conversation
does not wait for a provider context-overflow error to be shortened.

## Events and replay

`run_events` is append-only. Each row has a global `id` (the SSE cursor) and a
per-run `seq`. `GET /api/events`:

* replays durable events after `Last-Event-ID` (or `after`), then tails live
  events;
* sends `stream.lag` when a subscriber falls behind instead of silently losing
  events;
* keeps `chat.delta` live for token streaming, but a disconnected client can
  reconstruct the conversation from the durable rows.

`GET /api/runs/:id/events` pages backwards with `before` and forwards with
`after`; the default is newest first. Runs have no page of their own: the Run
sheet opens from the conversation or from the team run it belongs to, and reads
the same endpoints, so a long timeline is paged rather than truncated.

## Environment and setup

`environment.manifest` (read, idempotent) describes a project's detected stack,
declared runtimes, needed services, sites, tasks, environment-variable **names**,
ports, and isolation. Its fingerprint ignores `generated_at`, so two inspections
of the same environment compare equal.

`workflow.setup` (write, approval-gated, dry-run) applies what is missing in a
fixed order: inspect, install runtimes, start services, optionally run one
project task, optionally create a site, then re-inspect and verify. Every step
records `done`, `skipped` or `failed` with the real result. `verified` is true
only when no step failed and the re-inspection finds nothing missing.

## Site verification

`site.verify` (read, idempotent, dry-run) answers the question a site goal
actually asks: the hostname is in the hosts file, it resolves to loopback, the
listener accepts a connection, TLS validates against devd's own
`certs/devd-ca.crt`, and the site answers with a status below 500. The call
returns `ok=false` with the failing checks in its payload rather than failing
outright, so the run records what was observed instead of only that a probe ran.
It reads no environment proxy and follows no redirect: a site that redirects is
judged by the status it returned, not by where it points.

## Retry and recovery

`run.retry` is human-only. It refuses a team run, an active run, or a run with a
pending approval. It resumes a failed, unverified, paused, blocked, review or
completed run; a cancelled run starts a fresh one for the same conversation.
The assistant checks the durable cancellation flag between steps and hands the
same flag to handlers through `RunContext`, so a stop is visible to in-flight
work. An idempotent capability with a key replays its first successful result
instead of repeating the side effect; a non-idempotent call requires a new
decision rather than an automatic retry.

A key is scoped to the capability that used it and compared against the stored
argument hash. The same key under a different capability, or with different
arguments, is a `conflict`, not a replay: returning another call's result would
be worse than refusing. A recorded result that cannot be read is a conflict too,
because running the operation again is exactly the double effect the key exists
to prevent.

## What is not claimed

* Evidence is a record of what devd observed. It does not prove that an external
  service behaved correctly after the observation.
* A replay reconstructs state from durable rows; it does not reverse an external
  side effect such as a deployment or a payment.
* Cancellation reaches handlers that check `RunContext` and processes that devd
  supervises. A third-party program that ignores termination is reported as
  still running rather than as stopped.
* Process isolation is described in `docs/limitations.md`: an allowlisted
  environment and, on Windows, a Job Object are not a filesystem sandbox.
