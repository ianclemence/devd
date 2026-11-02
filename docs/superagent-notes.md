# What devd takes from the super-agent design

DeepSeek's `dsh-superagent-prototype` (and the "Super-agent" explainer drawn from
it) is a blueprint for an always-on orchestrator: small fast router, specialised
workers, filters in front of the model, durable records, a watchdog. devd already
has the harder half of that: one registry, one run spine, plan-bound approvals,
typed evidence, durable events, a resident daemon.

This note records what was worth taking, judged against what devd already did,
what it deliberately refuses, and why. It began as a study and is now the record
of what was built: the roadmap at the end names the code, and [`agents.md`](agents.md)
describes how each piece behaves. [`autonomy.md`](autonomy.md) is the checklist a
future change must keep.

Sources: [`deepseek-ai/dsh-superagent-prototype`](https://github.com/deepseek-ai/dsh-superagent-prototype)
(`docs/000`–`007`), the Super-agent illustration, and devd's own
[`agents.md`](agents.md), [`runs.md`](runs.md), [`daemon.md`](daemon.md),
[`security.md`](security.md).

## How the prototype works, briefly

```text
Source (GitHub, Feishu) -> Trigger -> Thread -> Eval (gate = code, assessment = model)
                                              -> Decision (accept with a Mandate / stop / ignore / unavailable)
                                              -> Ticket -> Assignment -> Executor Session + directory
                                              -> Report (progress | waiting | ready | blocked | failed | dropped)
                                              -> Channel (comment, PR, chat reply)
```

Five ideas carry it:

1. **The philosophy is the spec.** "Take as little human attention as possible."
   An agent acts only when asked or when a written rule allows it, and speaks
   only when a person must act or can use the result.
2. **Authorization is a data record, not a prompt.** A *Mandate* records who
   asked, under which policy version, with which matched rules, at what strength
   (`explicit` / `inferred`), and what scope. It is revalidated before work
   resumes, and only a person's stop withdraws it.
3. **Proactive work ramps through `observe`.** Inferred candidates (a bot's
   message, an unaddressed report) are assessed; a positive assessment is
   recorded as *"would accept"* and nothing runs. Only after the operator has
   reviewed those records does the mode move to `auto`.
4. **Everything that reaches outside is a stored effect.** Relays, deliveries,
   replies and launches are rows with backoff, ordering per ticket and
   conversation, and a health entry that clears on the next success.
5. **The router never waits.** Shell commands and job waits in the router
   session are capped at seconds, so a long build cannot hold up new candidates.

## What devd already has (do not rebuild)

| Prototype idea | devd today |
| --- | --- |
| Router / workers | `team.*` planner + code scheduler + specialists with role tiers and own worktrees (`docs/agents.md`) |
| Never runs code in the router | Planner and the coordination roles cannot write; roles are Markdown with a permission ceiling |
| Gate: rules in code, then a model | `CompletionPolicy::for_goal`, role `needs`, `human_only`, tier checks in `Registry::dispatch` |
| Thread with at most one Eval at a time | `team_runs.parallel`, the scheduler tick |
| Durable record of what was observed | `runs`, `run_steps`, `tool_calls`, `evidence`, `artifacts`, `run_events` (`docs/runs.md`) |
| Stored work with retry | Partly: approvals are durable and single-use; the queue is not (`chat_queue` is the only one) |
| Watchdog | `agent-host.pid` + `Team::recover` + `Hub::reconcile` on the next host (pull, not while running) |
| Keep the prompt small | `ContextBudget`, compaction, artifacts (`crates/devd-core/src/context.rs`) |
| Background jobs do not block the chat | `chat_queue`, streamed replies, run cancellation |
| Prompt versions you can review | Not for roles/skills: they are embedded `include_str!` files |
| Panel with "what needs me" first | Inbox, team run stages, health |

## What was adopted

### 1. Attention is the metric

"Take as little human attention as possible" is a sharper design rule than
devd's "it asks before it does anything consequential". It decides not only
*whether* to ask, but how much to say, how often, and in how many places.

What devd took from it, concretely:

- **One tab per question.** The prototype's panel puts "what needs me" first and
  gives a healthy state no space at all. The Inbox now leads with the runs that
  cannot continue, then the proposals waiting for a yes or a no, then approvals
  and questions; what simply happened comes last.
- **One concrete question at a time.** A paused run currently explains itself
  with a list (`task-x (unverified): ...; task-y (blocked): ...`). The prototype
  makes the *question* a structured field (`human_action`) so it reaches the
  person instead of being folded into a report. devd's equivalent is a paused run
  with a single `needs` object: one line, the task, the missing evidence, and the
  one action that resolves it (retry with a note / accept by hand / cancel).
- **Confirmation is about the first email, not the tenth.** The prototype says
  the first time it writes somewhere new, a person gets one message confirming the
  channel and identity. devd's first use of a project by an agent could do the
  same.
- **Say it once, and be quiet while working.** devd already streams one turn;
  keeping "stays quiet while it works" explicit would stop notices being added to
  the middle of a run.

### 2. Authorization is a record, not a sentence

The strongest idea in the prototype is the *Mandate*: an immutable record of what
an accept granted — policy version, scope, strength, matched rules, the sender's
identity on that source, and how the request arrived. It is revalidated before
work resumes, and only a person's stop withdraws it.

devd is close in spirit: approvals are durable, hashed and single-use. Its *team*
authority, though, was looser before this work: once a person approved a plan,
tasks ran without a per-task record of why each one may run and how far. A
`mandate` JSON column on `team_tasks` gives devd that record without a new safety
model:

- **Origin** - the person's message, the schedule, the template, or a proactive
  suggestion, and who it came from.
- **Basis** - `explicit` (a person asked), `schedule` (a standing instruction),
  `inferred` (devd or an agent proposed it and a person accepted), or `system`
  (the harness added the step).
- **Policy version** - a hash of the rules the task was accepted under:
  environment mode, definition of done, parallel limit, working agreement.
- **Scope** - the paths it may touch, already present in `team_tasks.scope`.

The point is not ceremony. It is that **resume and revalidation become possible**:
when the project flips to production mode, when a role disappears, or when the
rules change, devd can ask "does this task's mandate still hold?" instead of
either blindly continuing or stopping everything.

### 3. Proactivity is a ramp, not a switch

The prototype's accept mode is the best-proven part of the blueprint:

| Mode | Effect |
| --- | --- |
| `off` | Nothing inferred becomes a candidate |
| `observe` (default) | Inferred candidates are assessed; a positive assessment is recorded as "would accept"; nothing runs |
| `auto` | A positive assessment accepts it |

devd had the two halves - `inbox.post` for a suggestion and schedules for
standing work — but nothing in between. It now has the ramp:

- `team.suggest` records a proposal: title, why, what would change, the check
  that would prove it. Nothing runs.
- The proposal appears in the Inbox and the team room, and a person starts a run
  from it (`team.accept_suggestion`) or dismisses it. An accepted proposal is an
  ordinary run, so plan approval still applies.
- An intake rule (`intake.set`) starts in `observe` for the same reason: it
  records what it would have done. `auto` starts the run but still waits for plan
  approval, and never starts one in production.

This is the cheapest way to feel like the super-agent picture without taking the
autonomy risk the prototype itself refused to ship.

### 4. Two layers: deterministic gate, then a model

The prototype's Eval is `gate` (code: who may start work, with what strength) then
`assessment` (a model). devd did this inside the team and in the registry, but not
at **intake**. A person typing into devd can produce:

- an answer (assistant),
- a plan (team),
- a task without a plan (`team.assign`),

and devd used to decide with prose in the system prompt. It now uses a typed
value - `RunKind { answer | plan | assign | schedule }`, chosen by
`devd_core::classify` before any model is asked — which reproduces the "router is a
small fast model, filters are code" split and makes the model choice per kind
visible in `models.roles`.

### 5. Prompt text is versioned, reviewable content

The prototype versions the whole prompt configuration; a save names its base
revision; overrides record the default they were based on; upgrades mark parts
*"Default updated"* and show the difference.

devd's roles and 63 skills are compiled into the binary. That is good for trust
(nobody can edit a permission), but it used to mean a person could not see what
shaped the last run. Each turn now records a **prompt manifest**: which role,
loadout and skills it ran under, by name and hash. The text is still compiled in
and still cannot be edited; the manifest is what makes "the run that worked and
the one that did not" comparable.

### 6. Work that reaches outside is a stored effect

The prototype stores every relay, delivery and reply as an effect row: one row per
kind and key, retried with backoff, ordered per ticket and conversation, with a
health entry per kind that clears on its own next success.

devd had one durable queue (`chat_queue`) and a bus that drops; the daemon ran
cron, backups, health checks and reconciliation every 15 seconds, and a failure was
a log line. It now has the same shape as the prototype:

```sql
CREATE TABLE effects (
  kind TEXT, key TEXT, payload TEXT, not_before TEXT,
  attempts INTEGER, last_error TEXT, state TEXT,   -- pending | done | dropped
  PRIMARY KEY (kind, key)
);
```

Outbound notifications, background reviews and scheduled starts are rows in it,
retried with backoff and reported on the health screen. See [`agents.md`](agents.md).
An approval is still not one: it was never a retried message.

### 7. Filtering belongs in front of the model

In the illustration that accompanies the design, "who's lunch?" is dropped by a
rule and "new UI looks clean" is dropped by the small model — before any expensive
worker sees it. For devd the equivalent intake is not chat but **events the project
emits**: a failing check, a health check that goes down, a schedule that was
missed. devd used to react only on a clock.

A project can now write that rule down (`intake.set`, human only): "a failing
nightly check is considered only after it has failed three times in a row, at most
once an hour". The rule is the record of why devd looked, and it starts in
`observe` mode so a person reads what it would have done before it does it.

### 8. The watchdog runs while devd runs

The illustration's watchdog is a separate agent that watches for stuck or drifting
workers. devd's recovery is still pull-based for restarts (`Team::recover` fixes a
run on the next host start), but a worker that is alive and not progressing is now
found while devd runs:

- **A heartbeat.** Every model turn and every tool call stamps
  `team_tasks.last_step_at`. Older than twice the provider's own request timeout
  is *stuck*, not *slow*.
- **One nudge, then a stop.** The first stall asks the specialist where it is and
  to report `blocked`. If the heartbeat still has not moved, the conversation is
  stopped and the task is `unverified` with the reason — the same terminal state a
  rejected report gets.
- **Not built yet: drift detection.** The idea — an implementer that has made
  twelve tool calls with no `change` evidence is drifting, so pause it rather than
  wait for the report — is sound and still open. The evidence check that would
  drive it already exists; nothing watches it early.
- **Not built yet: a per-command rule.** The prototype moves a foreground shell
  command to the background after 5 s. devd has a per-run 30-minute deadline
  (`chat.rs`, `run_ctx.deadline`) but not a per-command one, which is the rest of
  "long jobs don't block the chat".

### 9. Test the trust story, keep test debt and memory debt separate

The prototype has a test that fails when a stored record from an older schema
appears in a shape code no longer expects, and refuses older schemas outright.
devd has migrations but still a weaker guarantee for row shapes; the prototype's
"stored work" suite (`effects`, `outbox`) remains the model for the queue.

And the prototype's honesty extends to what it does *not* do: nothing deletes old
records, and the docs say so. devd has retention for logs, audit, cache and
snapshots, and the new tables follow it: finished queue rows are removed after 14
days, decided proposals after 90 (`effects.retention_days`,
`suggestions.retention_days`, recorded in [`daemon.md`](daemon.md)).

## What devd should refuse from the blueprint

| Prototype choice | Why not in devd |
| --- | --- |
| `danger-full-access` workers as the operator, wrappers only as a default | devd's whole claim is the registry gate; `security.md` already says an allowlist and a Job Object are not a filesystem sandbox, and this note should not weaken that |
| Thousands of tokens per run (23.2 M vs 6.1–10.8 M) | devd runs on the person's own key; `policy.set max_cost_usd` and per-run usage already exist and should stay the ceiling |
| Proactive acceptance in `auto` | devd's schedule rule (never in production, plan approval unless waived by the person) is the safer form of the same thing |
| A chat-only interface to configuration | devd's charter, roles and skills are files a person can read; keep it |
| Records that grow without bound | The prototype admits this; devd's retention model should not be copied here |

## Roadmap

All eight items below were built. This table is the record of what was taken,
where it lives and what governs it now; the behaviour itself is documented in
[`agents.md`](agents.md) and the invariants in [`autonomy.md`](autonomy.md).

| # | Change | Where it lives | Governed by |
| --- | --- | --- | --- |
| 1 | **One "needs you" answer.** A paused run produces one `needs` object (task, missing evidence, ordered actions); the Inbox and the run view read the same one. | `crates/devd-agent/src/needs.rs`; `team.get`; the Inbox and run cards in `app.js` | The words come from one function, so no two screens can disagree. |
| 2 | **Stall watchdog.** `last_step_at` moves on every turn and tool call; one nudge, then `unverified` with a reason. | `team.rs` (`watch`, `stalled`), `chat.rs`, migration `019` | The window is twice the provider's own timeout, so a slow model is never called stuck. |
| 3 | **Proposals in `observe` mode.** `team.suggest` records what devd would do and starts nothing; an accepted proposal becomes an ordinary run. | `caps_autonomy.rs`, `store_autonomy.rs` (`suggestions`), the Inbox | Only a person accepts; the accepted run still waits for plan approval. |
| 4 | **A mandate on every task.** Basis, origin, scope, role and a policy version on each task; revalidated before a task starts. | `team.rs` (`mandate`, `mandate_holds`), `store_autonomy.rs` | It is a record of authority, never a permission. |
| 5 | **A stored effects queue.** Notifications, reviews and scheduled starts are retried rows with a health entry. | `devd-ops/src/effects.rs`, `devd-agent/src/{guardian,effects_bridge}.rs`, `runner.rs` | Bounded backoff; an error only after three failures in a row. |
| 6 | **The kind of work is decided in code.** `RunKind` chooses the runner and the model chain; `models.roles` lists it. | `devd-core/src/runs.rs` (`classify`), `chat.rs`, `models.roles` | A caller can say the kind; the default is never a model's guess. |
| 7 | **Prompt provenance.** Every turn records a manifest of role, loadout and skill text by name and hash. | `prompts.rs`, `run_steps.prompts` | The content still comes from the binary and cannot be edited; the manifest is a fingerprint. |
| 8 | **Intake rules.** A project writes what event should make devd *think about* starting a run. | `intake.rs`, `guardian.rs`, `intake.*` capabilities, `intake_rules` table | New rules start in `observe`; `auto` still waits for plan approval and never runs in production. |

Two things from the study were deliberately **not** built, and the reason is
part of the design: a mode in which devd starts work nobody approved, and
workers that run with full access to the operator's machine. The prototype's own
numbers — 2 to 4 times the tokens for a run — are the other reason: devd runs on
the person's own key, so `policy.max_cost_usd` stays the ceiling.

## One paragraph

Steal the *discipline*, not the *autonomy*. The prototype's transferable content is
that authority is written down (a Mandate), that proactivity has a mode that records
what it *would* have done, that a healthy system is quiet and a stuck one is
named, and that everything that leaves the process is a stored, retried effect.
devd already enforced more than the prototype does at the point of action; this
work added what the prototype documents best: the ramp, the heartbeat and the
single question.
