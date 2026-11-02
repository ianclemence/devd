# Agents: the assistant, the team, skills and models

devd has one agent runtime behind every interface. The window, the terminal
dashboard, the CLI, the HTTP API and MCP all call the same capabilities
(`chat.*`, `team.*`, `skills.*`, `models.*`, `approval.*`, `inbox.*`) and read the
same persisted state. Nothing about an agent run lives in a browser tab.

## Design, and where it comes from

The structure follows Anthropic's published guidance on building agents
(<https://www.anthropic.com/engineering>, <https://code.claude.com/docs/>):

| Principle | In devd |
|---|---|
| Start with a simple loop; add structure only where it pays | One loop (`devd-agent::chat`): ask the model, run the tools it asked for, append results. The team is that loop plus a scheduler. |
| Tools are the interface; keep the surface small and progressive | Five MCP tools (`devd_groups`, `devd_capabilities`, `devd_describe`, `devd_invoke`, `devd_doctor`) reach every capability; the built-in assistant also has a small set of direct names for the operations it uses constantly. Agents load detail only when needed. |
| Keep the session log outside the model; the harness is replaceable | Conversations are append-only rows (`chat_events`). A restart loses nothing; runs resume from the log. |
| Orchestrator and workers for work that splits | `team.*`: a planner, a deterministic scheduler, and specialist conversations. |
| Verify with evidence, not claims | Every capability call is recorded against a run, and completion is decided from typed evidence rows, never from the model's sentence that it finished (see [`runs.md`](runs.md)). |
| Human approval for consequential actions | One durable approval service with scope, expiry and single-use decisions. Agents queue; only a person decides. |
| Keep the context lean | A model-aware budget keeps the goal, plan, approvals, errors and recent work; older content is compacted and raw output stays as a bounded artifact. |
| Take as little of a person's attention as possible | A run that cannot continue asks one structured question, not a report; work devd would do is proposed through `team.suggest`, not done; the assistant is told to confirm once and work quietly. |
| The kind of work is decided in code | `RunKind` (answer, plan, assign, schedule) is chosen from the words or the composer before any model is asked, and picks the model chain. |
| Authority is a record | Every team task carries a **mandate**: its basis, who asked, its scope and a hash of the rules it was accepted under. |
| Proactivity is a ramp, not a switch | An intake rule starts in `observe` mode, which records what it would have done; `auto` starts a run that still waits for plan approval, and never in production. |
| What leaves the process is a row | Notifications, background reviews and scheduled starts are effects in a durable queue, retried with backoff and reported on the health screen after three failures. |
| A quiet system is the healthy one | A worker that stops moving is found by its heartbeat and stopped, rather than left holding a slot. |

devd does not launch other coding agents. Its own agents, the assistant and the
team, run through the providers you configure, and an outside agent that speaks
MCP can still drive devd through the same capabilities and permission model.

## One run model

The assistant, the team scheduler, the HTTP API and MCP share one durable run
(`runs`, `run_steps`, `tool_calls`, `evidence`, `artifacts`, `approvals`,
`run_events`). A conversation has one current run, reopened only by a deliberate
new message; a team run mirrors its state into the same spine. Direct tool
aliases, `devd_invoke` and internal calls all go through `Registry::dispatch_with`,
so schema validation, permissions, approval, the tool-call record and derived
evidence cannot differ by entry point. Completion is decided by a policy over
typed evidence: a change needs an inspectable diff and a passing check, a service
needs a readiness probe, and a model's claim that it finished is not evidence.
See [`runs.md`](runs.md) for the lifecycle, policies, context budget, replay
cursor and recovery rules.

## Providers and models

`provider.*` stores providers. The presets are Anthropic, OpenAI, DeepSeek,
Moonshot AI (Kimi), Zhipu AI (GLM), Google Gemini, xAI, Mistral, Groq, Together,
Fireworks, OpenRouter, Ollama, LM Studio, and any OpenAI-compatible URL, in that
order of usefulness. Keys live in owner-only files or are read from an
environment variable; they are never in the database, logs, API responses or
model context.

How models are found depends on the vendor, and devd asks each one in its own
way: Anthropic's `GET /v1/models`, the OpenAI-compatible `GET /models`, Ollama's
native `GET /api/tags` (which works even when the compatibility layer is
missing), and a maintained catalogue for vendors whose chat API has no listing
endpoint. `provider.discover_local` finds a running Ollama or LM Studio on this
machine without sending anything outside it.

`provider.test` checks the key and the endpoint. `provider.verify` runs one
real request that must call a named tool, so devd only says a model can use
tools when the provider actually returned a tool call. Failures say what is
wrong: a rejected key, an exhausted quota, an unpaid balance, a rate limit and
its retry window, a context window that is too small, or a model this account
cannot use.

Two different questions, two different sources:

* **What exists** - the official list. For Claude, `models.refresh_official` reads
  Anthropic's models documentation (id, price, context window, output limit,
  retirement date). No key needed.
* **What this key may use** - `provider.models` asks the provider's own model list
  with your key. A model in the official list but absent from your account's list
  shows as `missing` in the role table; one never checked shows `unverified`.

`models.assign` chooses a first choice and fallbacks for each kind of work
(`default`, `planner`, `architect`, `researcher`, `implementer`, `qa`, `reviewer`,
`infra`, `docs`), globally or per project. Each assistant reply records the model
that answered and why. Fallbacks are tried only when the first choice fails, only
among providers you listed, and never for a conversation where you picked the
model yourself.

### Google Gemini, natively

The Gemini preset speaks the native `generateContent` API, not the
OpenAI-compatibility layer: the model list returns the real input and output token
limits, the thought signature a newer model issues with a function call is kept
and sent back with it (the call is refused otherwise), and function schemas are
reduced to the subset Gemini accepts (`$schema`, `additionalProperties`, `oneOf`
and the like are removed; `["string","null"]` becomes a nullable string). A
provider saved against the old compatibility URL keeps working unchanged.

### What a model can do, what it may see, what a run may spend

`model.capabilities` says what is known about a model — whether it takes tools or
images, its window and output limit, its price per million tokens — and where each
fact came from: the provider's own listing, a short table of well-known model
families, or *unknown*, which is reported as unknown and never assumed capable. A
model known not to take tools is skipped for agent work, with the reason.

`policy.get` and `policy.set` (human only; an agent cannot loosen the rules it
runs under) hold three rules, checked before every model call:

* `local_only` - only providers on this machine (loopback) are used;
* `allow_providers` / `block_providers` - by provider id, block winning;
* `max_cost_usd` - the most one run may spend. Cost is tokens times the catalogue
  price, so it is enforced only for models whose price is known; a model with no
  price is not counted, and the run's usage says so by having no `cost_usd`.

A provider the policy excludes never sees the project: it is skipped, with the
rule that excluded it named, before any request is built.

## The team

```
goal -> planner -> [you approve the plan] -> scheduler -> specialists -> final verification -> report
```

* **Planner** (a model) submits tasks with `team.plan_submit`: key, role,
  instructions with acceptance criteria, `depends_on`, `scope`. devd validates it
  (known roles, unique keys, real and acyclic dependencies, at most 14 tasks).
* **You approve the plan.** Nothing else runs before that. Starting a run with
  `auto_approve` is possible for a person, never for an agent, and never in
  production mode. New instructions (`team.say`) revise a plan that is waiting,
  and reach running tasks when they next stop.
* **Scheduler** (code) starts tasks whose dependencies are done, up to the
  run's parallel limit. Implementers that could run side by side each get their
  own `git worktree` on a `devd/<run>/<task>` branch under `<project>/.devd/`
  (excluded locally from git). Without git, or without a commit, writers run one
  at a time. Accepted branches are committed and merged with `--no-ff`; a dirty
  project or a conflict blocks the task and changes nothing.
* **Specialists** are the members of devd's own team, a small startup in a
  folder. Each is a conversation with a role prompt, a tier ceiling, a working
  directory they cannot leave (paths outside it are refused), a model chosen by
  role, and a loadout of skills. The roster (`team.roles`):

  | Part of the company | Members |
  |---|---|
  | Leadership | planner, product manager |
  | Design | designer |
  | Engineering | architect, implementer, frontend, backend, mobile, data |
  | Quality and security | QA, reviewer, security engineer |
  | Operations | infrastructure |
  | Knowledge | researcher, docs |

  Planner, product, architect and researcher plan or write documents and change
  no code; the reviewer and the security engineer read and run checks but write
  nothing; the engineers and the designer change code in their own copy of the
  project and must prove it with a check that passed.

  Each role is one Markdown file in `crates/devd-agent/roles/`: the front matter
  states the tier, whether it writes, what it must prove and which skills it
  carries, and the body is its instructions. A test checks that every role
  parses, that every skill it names exists, and that the permissions are
  coherent (a role that must prove a change may change files, one that must run
  a check may run commands, nobody may use destructive or remote operations).
* **Evidence.** Every specialist tool call is recorded in the run's `tool_calls`
  and normalized into `evidence` at the registry boundary, whether the model used
  a direct tool name or `devd_invoke`. After a specialist reports, devd reads
  those rows, not the conversation text. An implementer must have changed files
  *and* had a verification (a `project.run` of test, build, lint, check... exit 0)
  pass; a reviewer must have read the change; QA must have run checks; infra must
  have run something that succeeded. `project.exec` and a command that is not a
  check are `command` evidence, not verification; a file read or a `git diff` is
  `read` evidence and cannot satisfy a change requirement. Readiness is read from
  the payload (a service reports `running` and `listening`), not from the call
  returning. Otherwise the report is rejected
  twice with the reason, then the task is `unverified`, its dependants do not
  start, and the run pauses with what is needed (retry with a note, or accept by
  hand — recorded as such).
* **Final verification.** When every task is done devd adds a QA task that runs the
  project's own checks independently. The run's summary is built by devd from the
  recorded evidence, not from what agents wrote about themselves. The team run
  mirrors its state into the unified run, so the Evidence sheet on the team run
  reads the same steps, evidence and approvals the scheduler used.
* **Persistence.** `team_runs`, `team_tasks`, `team_events`, mirrored into the
  unified run. On restart, running tasks go back to pending and their conversations
  continue; paused and waiting runs stay as they were.

`devd team start "..." --project .` follows the run in the terminal;
the window has Assistant > Team runs; the terminal dashboard has a Team view and an
Inbox view; all read `team.get`.

### The team room

A team is more than a planner and a task queue. devd gives every project a
**team room**: the written context that a person and their agents share, before
and during a run. The design follows Anthropic's
[Building effective human-agent teams](https://claude.com/resources/articles/building-effective-human-agent-teams):
a north star, clear owners, broad but bounded context, a shared bar for quality,
and room to learn from mistakes.

The room has five parts:

| Part | Where it lives | Who can change it |
|---|---|---|
| **North star** | `team.charter.set` file for the project | a person only |
| **Human owner and working agreement** | the same charter | a person only |
| **Invited proactive roles** | the charter; a record of which roles may suggest the next work | a person only |
| **Shared context** | `team.context`: sites and addresses, managed services, installed runtimes, `AGENTS.md`/`CLAUDE.md` knowledge, recent lessons, trust record | read-only, assembled from devd's records |
| **The record** | team tasks, evidence, acceptances by hand, lessons and missteps in the run summary | devd writes it; a person reads it |

`team.charter.get` and `team.context` are readable by the assistant and by every
team member. `team.charter.set` is human-only: the person writes the standard;
an agent may not quietly rewrite the north star it is judged against.

The context is injected into every assistant turn and every specialist prompt,
bounded to a short brief, so agents do not have to rediscover the project,
the site that exposes it, the services it needs, or the lessons from the last
run. The window shows the same brief under **Team > Team room**; it is not a
hidden prompt.

**Trust is earned, but it is not a permission bypass.** The trust record counts
what each role has had accepted, what was accepted by hand, what was unverified,
and how many attempts it took. It is shown to the person. Approvals, evidence
and the role tier still decide what may run; a good record never removes a
person's decision.

**Lessons are recorded, not invented.** When a run ends, devd appends the
failures, unverified tasks, accepted-by-hand work and repeated attempts to the
summary and to the context of the next run. A model's reflection is useful, but
it is not allowed to replace the events the harness observed.

`devd team.roles` remains the roster. The team room does not add another copy of
it: the roster, the charter and the context are three views of the same project.

### A definition of done

`team.start` takes `done_when`, a list of statements the finished work must
satisfy (`devd team start ... --done-when "the tests pass" --done-when "the README
documents every command"`; the window has a field for it). The planner sees them
in its brief. When every task is done, devd asks Jev (below) how likely each
statement is to hold given what the run recorded: task titles, states, reports
and check results, never source files. A statement under 60% likely is not met;
devd adds an implementer task for each one and a QA task to verify them, up to
`max_rounds` extra passes (default 2, at most 5), then checks again. What is
still open after the last pass is listed in the run's summary. Without a Jev key
nothing is machine-checked and the run says so: the statements are listed for you
to confirm.

### Jev

[Jev](https://docs.typesafe.ai) is TypeSafe's System One model: it takes a state
and typed questions and returns a calibrated probability, not prose to parse.
That is the right tool where devd's own code, not an agent, needs a verdict.
`judge.configure` stores the key (an owner-only file; `TYPESAFE_API_KEY` overrides
it; a person only) and `judge.status` reports whether one is set. It is optional.

### Templates

A run template is a goal written once, with blanks, and the definition of done every
run of it carries. devd ships five - `spec-to-pr`, `fix-bug`, `add-feature`,
`security-review`, `release-prep` - and your own in `<devd home>/templates/` join
them (same Markdown format; they cannot replace a built-in).

```bash
devd template list
devd template run fix-bug --project . --param report="checkout returns a 500"
```

The window's *Team runs* form has a template menu that fills the goal and the
definition of done for you to edit.

### Schedules

A schedule starts a team run on a clock, from a template or a goal:

```bash
devd schedule create weekly-security "mon 09:00" --project . --template security-review
devd schedule create nightly-tests "every 6h" --project . --goal "Run the checks and report what is failing" --done-when "The report names every failing test"
devd schedule list
devd schedule run weekly-security   # now, without changing the next time
```

Times are `every 30m` / `every 6h` / `every 1d` (at least five minutes), `daily 09:30`,
`weekdays 09:30`, `weekends 10:00` or a list such as `mon,wed,fri 18:00`, in the
machine's local time. Schedules run only while a process that owns the agents is
running (the application, `devd serve`, the terminal dashboard). A schedule never
starts a run while the last one it started is still going, skips an occurrence
that was missed by more than two hours and says so, and waits for a person to
approve each plan unless `--yes-plan` was given when it was made (never in
production mode). Only a person creates, changes or deletes schedules; an agent
can list them and start one now.

### Your own roles

Put a Markdown file in `<devd home>/roles/`, in the same format as the built-in
roles, and it joins the team. A role of your own may use up to the exec tier, may
not name extra capabilities, cannot take a built-in's name, and may only carry
skills that exist. A file that does not load is reported by `team.roles` with the
reason; the rest of the team is unaffected.

### What a run needs from you

A run that cannot continue says one thing, not a list of everything that went
wrong. `team.get` returns a `needs` object:

```json
{
  "scope": "task",
  "task_key": "search",
  "role": "implementer",
  "title": "Add the search box",
  "reason": "not-proved",
  "missing": ["no check (test, lint, build, check) passed"],
  "actions": [
    { "id": "retry",  "label": "Try it again",          "capability": "team.retry",  "dangerous": false },
    { "id": "accept", "label": "Accept it as it is",    "capability": "team.accept", "dangerous": true  },
    { "id": "look",   "label": "Read what it did",      "capability": null }
  ]
}
```

`reason` is one of `no-report`, `not-proved`, `blocked`, `failed`, `merge`,
`approval`, `plan` or `criteria`. `missing` names the evidence devd actually
looked for and did not find, in the same words the scheduler rejected the report
for. The first action is the one that unblocks the work; accepting without proof
exists, is offered after it, and is marked dangerous because the run's report
will say a person accepted it by hand.

The Inbox, the run view and `team.get` all read this one object, so no two
screens can phrase the same stuck run differently. A run with nothing waiting has
`needs: null`. The sentence at the top of the run (`Needs you: ...`) stays as the
record of the decision the scheduler made; `needs` is the question.

### A mandate on every task

Every team task is granted a **mandate** before it runs:

| Field | Meaning |
|---|---|
| `basis` | `explicit` (a person asked), `schedule` (a standing instruction), `inferred` (devd or an agent proposed it and a person accepted), or `system` (the harness added the step, such as final verification). |
| `origin` | Who asked, as an actor label. |
| `role`, `scope` | What it may do and what it may touch. |
| `policy_version` | A hash of the rules it was accepted under: environment mode, definition of done, parallel limit, working agreement. |
| `granted_at` | When. |

The mandate is why a resumed task can be re-checked rather than assumed. When a
task is about to start and the machine has since become production while the
rules it was accepted under have changed, when its role no longer exists, or when
its scope names a path outside the project, the run pauses with
`mandate_revoked` and the reason instead of continuing. A person resuming the run
grants it again under the current rules, and resets the stall watchdog for its
tasks.

The mandate is a record, never a permission. Approvals, evidence and the role
tier still decide what may run; a mandate explains why devd believed it could.

### A worker that stops moving

Every model turn and every tool call moves the task's `last_step_at`. If a
running task moves no heartbeat for **twice the provider's own request timeout**
(never less than three minutes, so a slow model is never mistaken for a stuck one),
the scheduler:

1. writes a `stalled` event and asks the specialist, in its own conversation,
   where it is and to report `blocked` with one sentence if it is stuck;
2. if that gets no answer and the heartbeat has still not moved, stops the
   conversation and records the task as `unverified` with the reason, which is
   the same terminal state a rejected report gets.

Nothing is invented: the reason names the minutes of silence and the fact that
it did not answer. A person's retry or resume starts the count again.

### Proposals, not work

`team.suggest` records work devd would do, in one row:

```json
{ "project": "...", "role": "implementer", "title": "Split the parser",
  "why": "the parse function is 900 lines and changed in four of the last five commits",
  "plan": ["extract the tokenizer", "keep the tests passing"],
  "accepts": ["the parser test suite passes"] }
```

Nothing runs. The proposal appears in the Inbox under **Proposed, not done** and
in the team room, and nobody is asked twice: proposing the same title while the
first is still open returns the first. A person either starts a run from it
(`team.accept_suggestion`, human only) or declines it (`team.dismiss_suggestion`).
An accepted proposal starts an ordinary run that still waits for plan approval,
and the run records who accepted it.

Agents may propose; only a person decides. The assistant is told to use it for
improvements it notices while doing something else, rather than doing them
unasked or listing them in a report.

### Rules for starting work on its own

A project can write down what event should make devd *think about* starting a
run. `intake.set` (human only) writes one rule:

```bash
devd call intake.set '{"project":"D:\\work\\shop","name":"nightly",
  "trigger":"check.failed","goal":"Make the nightly check pass again and add a regression test",
  "condition":{"min_failures":3,"cooldown_minutes":60}}'
```

The triggers are `health.down` (a watched site is down), `check.failed` (a
project's own check failed) and `schedule.missed` (a schedule was due while devd
was off). The condition is how many times in a row, and how often at most. The
mode is the ramp:

| Mode | What happens |
|---|---|
| `observe` (the default for a new rule) | The rule records a **proposal** ("devd would have started a run that does this") and stops there. No model is called. |
| `auto` | The rule starts a run, which still waits for a person to approve its plan. It never starts one in production; there it records a proposal instead. |

The rules are evaluated by the guardian in the process that owns the agents
(`crates/devd-agent/src/guardian.rs`) against facts devd recorded itself: a health
check's state, a run that ended `failed` or `unverified`, a schedule's own last
error. A model never decides whether the event happened. A rule that sees its
condition stop holding starts its count again, so "three times in a row" means
three in a row.

The team room shows each rule in words ("when a watched site is down, at most
once every 60 minutes → Restart the site and say why it stopped"), and the
project's prompt tells the agents what the rules are, so they do not propose the
same thing again. `intake.list` and `intake.delete` manage them.

### The queue for work that leaves the process

A message to be delivered, a review nobody is waiting for, a scheduled start:
each is a row in `effects`, with its payload, attempts, last error and the time
it may be tried again. The daemon drains the queue every fifteen seconds, settles
a row that worked, and backs a failing one off from two seconds, doubling to ten
minutes, dropping it after about a week with the reason. Queuing the same kind
and key again replaces the payload instead of stacking a second copy.

`devd_ops::effects::health` reports a kind as `error` only after one of its rows
has failed three times in a row, and `ok` again on the next success, so a
one-off network blip is not an unhealthy devd. A partial run of the queue reports
how many rows were attempted, not which succeeded: every row is its own decision.

Background reviews (`review.start`) are queued rather than run on a thread, so a
process that stops with a review pending tries it again and a review that keeps
failing is visible on the health screen.

### What kind of work is this

Before any model is asked, `devd_core::classify` decides the `RunKind` from the
words (or the caller says so outright: the composer's mode, `chat.send`'s `kind`,
`devd chat --as team`):

| Kind | Reached by | Model chain |
|---|---|---|
| `answer` | everything else | the `default` role |
| `plan` | "have the team build X", "team: ...", "ask the team to ..." | the `planner` role |
| `assign` | "@qa check the flow", "assign the reviewer to ..." | the `default` role until the specialist is known |
| `schedule` | a schedule or a rule | the `default` role |

The kind is stored on the conversation, so a retry or a resume reaches the same
runner. `models.roles` lists what each kind reaches, so the answer to "what runs
when I say this" is one table rather than something to infer.

### Which prompts a turn ran under

Every turn records a **manifest** on its `run_steps` row: the role, its loadout
and every skill it read, by name and the first twelve hex digits of a hash of the
text. The content is still compiled into devd and still cannot be edited — the
manifest is a fingerprint, not a switch. It is what makes "the run that worked
and the run that did not" comparable: `prompts::differs` names the parts that
were added, removed or changed between two manifests.

### Who runs the agents

Conversations and runs are driven by threads in one process and recorded in the
shared database. The application, `devd serve`, the terminal dashboard and the
commands that follow a run (`devd chat`, `devd team`) are hosts. The first one to
start claims the agents (`agent-host.pid` in the devd home) and recovers whatever a
previous process left half done; a one-line command run beside it, such as
`devd provider list`, never touches the application's work. A run started by a
command that exits immediately (`devd call team.start`) has no host driving it
until the next host starts, which resumes it. Use `devd team start` to follow a
run in the terminal.

## Skills

Skills are short Markdown instructions agents load with `skills.get` when the work
calls for them. 63 ship with devd, and each belongs to one stage of the work, so
an agent finds the right one without reading all of them:

| Stage | What it covers | Skills |
|---|---|---|
| **Discover** | understanding the request and the system before acting | `work` (start here), `how`, `why`, `investigate`, `check-the-request`, `sharpen`, `restate`, `choose-by-the-outcome`, `recall`, `catch-up`, `handoff` |
| **Design** | deciding a shape | `architect`, `design-from-use`, `weigh-several-shapes`, `re-examine-the-design`, `model-the-domain`, `put-rules-in-the-compiler`, `validate-at-the-edge`, `arena`, `api-design`, `figure-it-out`, `threat-model`, `product-requirements`, `user-stories` |
| **Build** | writing the change | `build-feature`, `fix-bug`, `refactor`, `prototype`, `improve-performance`, `tdd`, `land-one-unit-at-a-time`, `make-the-narrowest-change`, `build-the-tool-if-it-repeats`, `finish-the-migration`, `plant-the-fix-in-the-structure`, `no-comments`, `benchmark-checklist`, `create-verification-skill`, `database-migrations`, `swarm` |
| **Check** | proving a result and keeping it honest | `prove-it-works`, `test-what-callers-see`, `fix-the-root-cause`, `report-numbers-honestly`, `keep-it-readable` |
| **Review** | reading someone else's change | `interrogate`, `blast-radius`, `secure-coding-review`, `ui-design-review`, `accessibility-check`, `correct`, `reflect` |
| **Ship** | landing it | `ship`, `decision-log`, `mobile-release-checklist` |
| **Words** | prose, reports, documentation | `plain-speech`, `unslop`, `technical-writing` |
| **House rules** | how work runs here: autonomy, ownership, staying restartable | `act-then-ask`, `decide-who-owns-the-state`, `make-it-safe-to-rerun`, `keep-your-context-lean` |

The order of the table is the order of the work, and the prompt index shows the
stages in that order. A skill's file names its own stage in `kind:`, so a person
who adds a skill picks the stage it belongs to; a stage of their own is shown
under its own name rather than dropped.

An agent's prompt carries eight core rules, each naming the skill that holds the
full text, and the index with one line per skill. Only names and descriptions go
into the prompt; the body is read on demand. Add your own in
`<devd home>/skills/*.md` (`devd skills add`); they can add to the built-in set
but never replace a built-in skill, and only a person can save or delete them.

A test checks the wiring rather than trusting it: every file in `skills/` is
embedded in the binary, is named after itself, belongs to a known stage, names
only real roles, has a unique name and description without a type prefix, and
refers only to skills and capability groups that exist.

## Tools agents have beyond the core set

`search.query` (projects, files by path, literal text in a project, conversations,
models, tasks, activity — each response says what was searched and whether it
was cut short), `docs.fetch` (public https pages only; private, loopback and
metadata addresses refused and re-checked across redirects), `skills.*`,
`team.*`, `services.inventory`, `environment.get`, and the file, git, project,
toolchain and stack capabilities.

Tools from **other programs' MCP servers** join the same list when a person
configures one in *Agents → MCP*. They are named `mcp__<server>__<tool>`, they
are not devd capabilities, and their output is data rather than instruction. A
call to a server marked `ask` waits for a person's approval; `allow` means a
person decided a model may use it without asking; `deny` keeps it out of the
list. Only a person can add, remove or trust a server. See [`mcp.md`](mcp.md) for
the whole model.

The assistant also carries its open todo list for the project into every turn,
and a short list of the calls it uses most (read, edit, run, status), so common
work does not spend turns rediscovering argument shapes.

### Direct tools, and why they are still capabilities

Discovery stays the default: five tools reach every capability, so the surface
does not grow with the catalogue. But the work an agent does constantly has a
short name beside them:

| Tool | Capability |
| --- | --- |
| `read_file`, `write_file`, `edit_file`, `list_files`, `search_files` | `fs.*` |
| `project_info`, `project_tasks`, `run_task` | `project.detect`, `project.tasks`, `project.run` |
| `git_status`, `git_diff` | `git.status`, `git.diff` |
| `todo_list`, `todo_create`, `todo_complete` | `todo.*` |

Each is only a name. The schema and description come from the registered
capability at the start of the run, and the call is turned into `devd_invoke` of
that capability, so it has the same validation, tier rules, approval queue and
audit record as the long way around. A direct tool cannot mean something
different from the capability it names, and adding one does not add a code path.

### The plan is visible

`todo.create` writes project state, and the Assistant shows it: a strip above
the conversation with done/total and the next few titles, and a sheet with every
item. The strip is re-read while the conversation is open, so a plan the agent
changes in one turn is visible in the next. A conversation with no open todos
shows nothing.

### Writing while it works

A message written while the assistant is working is **queued**, not refused. It
waits in `chat_queue` until the loop reaches a step boundary, then becomes an
ordinary user turn in the log; the header shows how many are waiting. Appending
it immediately would put a user message between a tool call and its result,
which is not a valid model conversation, so the boundary is the earliest safe
place. Stopping discards the queue and says so. The queue is durable: a process
that stops with messages waiting delivers them and resumes the conversation when
the host starts again.

### One reply, written live

A person's message gets one assistant turn, however many model calls it takes to
answer. A line of intent, a tool call, the result and the final explanation are
parts of the same turn: they share one avatar and one timestamp, in the order the
agent produced them. They used to render as separate messages, which made one
question look like two assistants speaking.

The reply is streamed as the provider writes it. The server asks for the
provider's event stream, forwards each piece on `GET /api/events` as
`chat.delta`, and the window paints it into the turn being written. The live text
is only an overlay: when the reply is logged, the persisted event replaces it in
exactly the same place. Nothing is shown twice, and a reload shows the same
conversation from the log. Providers that ignore `stream` still work; their
answer arrives whole and is logged once.

## Local and production

`environment.set` marks the machine `production` (a person's decision, with
approval). In production an agent needs a person's approval for every operation
above read — writing a file included — plan approval cannot be waived, and the
status bar says so. Destructive, remote and admin operations need approval in
both modes, with one scoped exception: removing files inside the conversation's
project is ordinary work (the project root, `.git` and secrets files are still
refused), while any path outside that project, and every other destructive
operation, needs a person. devd has no deploy capability, and agents cannot open
ports or edit a production database through it.

## Services that already exist

`services.inventory` looks at databases and services on the machine and keeps four
facts apart: **installed**, **running** (the port answers and the health check
passes), **required** (a project you imported says so) and **managed** (what you
decided). `services.decide` records `adopt`, `observe`, `external` (your own
instance on a port) or `ignore` - a record only; nothing is installed, stopped or
changed. `observe` and `external` stop devd controlling the service. devd never
replaces an installation as a side effect: removal is its own approved,
destructive step and keeps the data directory.

## Health

`health.status` returns one of `healthy`, `degraded`, `starting`,
`setup_incomplete`, `unknown` or `critical`, built from component checks (core,
git, managed services, the AI provider's last real test). A component says whether
devd needs it; optional things that are not set up become setup steps and notes,
never an unhealthy platform. A service is healthy only after it answers. The
sidebar shows the state compactly, goes stale after 90 seconds, and the
Diagnostics page lists every component with what it means and the fix.

## Deleting conversations, runs and messages

Nothing is deleted on its own; a person decides, and every deletion asks for approval first (agents cannot do it).

| What | Window | Command | Capability |
|---|---|---|---|
| One conversation | the cross beside it in Assistant, or Delete in its header | `devd chats delete <id>` | `chat.delete` |
| Every finished conversation | Clear finished in the Assistant list | `devd chats prune [--days N]` | `chat.prune` |
| One finished run | the cross beside it, or Delete in its header (Agents > Runs) | `devd team delete <run>` | `team.delete` |
| Every finished run | Clear finished in the Runs list | `devd team prune [--days N]` | `team.prune` |
| One inbox message | Delete in its sheet | `devd call inbox.delete '{"id":"..."}'` | `inbox.delete` |
| Answered or dismissed messages | Clear under Earlier in the Inbox | `devd chats clear-inbox` | `inbox.clear` |

Deleting a conversation stops it first if it is working, then removes its log and the approval requests it made. Deleting a
run removes its tasks, its log and the conversations its tasks had; a run that is still working must be cancelled first, and
work already merged stays. `prune` never touches a conversation that is working or waiting for you, and a run that is not
over. Deleted data cannot be recovered.

## Under a reply: copy, fork, chapters

Every reply in the Assistant has three actions beneath it (shown when you hover it):

* **Copy** puts the reply's text on the clipboard as Markdown.
* **Fork from here** (`chat.fork`) starts a new conversation containing everything up to and including that reply; the original stays exactly as it is. Approvals that were still waiting are not copied, and the answers to any tool calls that were open at that point are.
* **Pin as chapter** (`chat.pin`, `chat.unpin`, `chat.chapters`) gives that reply a title. **Chapters** at the top of the conversation lists them and jumps to one. Chapters belong to the conversation they were pinned in; a fork starts without them.
