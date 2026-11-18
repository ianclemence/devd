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
MCP can drive devd through the same capabilities and permission model. devd also
speaks the other direction: `devd acp` exposes it as an Agent Client Protocol
builder with the same approvals and audit chain. See [`acp.md`](acp.md).

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
Moonshot AI (Kimi), Google Gemini, xAI, Mistral, Groq, Together,
Fireworks, OpenRouter, Ollama, LM Studio, and any OpenAI-compatible URL, in that
order of usefulness. Keys live in owner-only files or are read from an
environment variable; they are never in the database, logs, API responses or
model context.

How models are found depends on the vendor, and devd asks each one in its own
way: Anthropic's `GET /v1/models`, the OpenAI-compatible `GET /models`, Gemini's
`GET /models` with `x-goog-api-key`, and Ollama's native `GET /api/tags` (which
works even when the compatibility layer is missing). `provider.discover_local`
finds a running Ollama or LM Studio on this machine without sending anything
outside it.

`provider.test` checks the key and the endpoint. `provider.verify` runs one
real request that must call a named tool, so devd only says a model can use
tools when the provider actually returned a tool call. Failures say what is
wrong: a rejected key, an exhausted quota, an unpaid balance, a rate limit and
its retry window, a context window that is too small, or a model this account
cannot use.

Two different questions, two different sources:

* **What exists** — the official list. For Claude, `models.refresh_official` reads
  Anthropic's models documentation (id, price, context window, output limit,
  retirement date). No key needed.
* **What this key may use** — `provider.models` asks the provider's own model list
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

* `local_only` — only providers on this machine (loopback) are used;
* `allow_providers` / `block_providers` — by provider id, block winning;
* `max_cost_usd` — the most one run may spend. Cost is tokens times the catalogue
  price, so it is enforced only for models whose price is known; a model with no
  price is not counted, and the run's usage says so by having no `cost_usd`.

A provider the policy excludes never sees the project: it is skipped, with the
rule that excluded it named, before any request is built.

### Thinking

Reasoning is a property of a conversation, not of the window. Each conversation
carries its own effort — `none` (shown as Off), `low`, `medium`, `high`, `extra`
or `max` — and the chip under the message box shows **that conversation's**
level, read from the record when the conversation opens. A conversation that
never chose one shows the ordinary default, Medium.

The level does two things: it is handed to the provider in its own words (a
token budget for Anthropic and Gemini, an effort level for DeepSeek and the
OpenAI family, a toggle for the hosts that think by default), and it sets how
much room the reply has after thinking, from 8k tokens at Off to 64k at Max.
Off is a real request, not a hint: it is sent as the provider's own way of
disconnecting reasoning.

The level is read at **every model call**, so changing it while a reply is being
written applies from the next call; the call already running finishes at the
level it started with. A team run takes its level from the conversation that
started it and gives it to every specialist it creates: changing it in your own
conversation afterwards does not reach a run that is already going.

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
  | Design | designer, motion designer |
  | Engineering | architect, implementer, frontend, backend, mobile, data, ML, hardware |
  | Quality and security | QA, reviewer, security engineer |
  | Operations | infrastructure, reliability (SRE), release manager |
  | Knowledge | researcher, docs, content writer |

  The roster is laid out the way a small technology company is: someone decides
  what to build (leadership), someone makes it feel right (design, including
  motion), engineers of each kind build it, quality and security check it,
  operations keep it running and get it out, and knowledge people explain it.
  Hardware sits with engineering because firmware is code that has to meet a
  real board; the ML engineer is there because a model change is an experiment
  with a baseline, not an edit.

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
  and normalised into `evidence` at the registry boundary, whether the model used
  a direct tool name or `devd_invoke`. After a specialist reports, devd reads
  those rows, not the conversation text. An implementer must have changed files
  *and* passed a verification (a `project.run` of test, build, lint or check that
  exited 0); a reviewer must have read the change; QA must have run checks; infra
  must have run something that succeeded. `project.exec` and a command that is not
  a check are `command` evidence, not verification; a file read or a `git diff` is
  `read` evidence and cannot satisfy a change requirement. Readiness is read from
  the payload (a service reports `running` and `listening`), not from the call
  returning. Otherwise the report is rejected twice with the reason, then the task
  is `unverified`, its dependants do not start, and the run pauses with what is
  needed (retry with a note, or accept by hand — recorded as such).
* **Final verification.** When every task is done, devd adds a QA task that runs
  the project's own checks independently. Its report carries a verdict
  (`approve`, `changes_needed` or `blocked`) and the criteria it judged, and the
  run only becomes `done` through `completion::check`: an approving gatekeeper, a
  reviewed commit equal to HEAD, terminal tasks, met criteria and no pending
  approval. A “changes needed” returns the work for another pass; a failure that
  the person accepts by hand is recorded as `accepted_by_hand`, and the summary
  says so. The run's summary is built by devd from the recorded evidence, not from
  what agents wrote about themselves. The team run mirrors its state into the
  unified run, so the Evidence sheet on the team run reads the same steps,
  evidence, verdict and approvals the scheduler used.
* **Isolation and scorecard.** Any writing role may run in its own worktree; the
  scheduler never starts two non-isolated writers, and a per-project write lease
  (with a fencing value and expiry) admits one. A review records the HEAD commit
  it started with, so its verdict applies to that revision. The run page shows a
  scorecard: verdict and reviewed commit, verified tasks, accepted by hand,
  retries, stalls and baseline failures.
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
| **North star** | `team.charter.set` file for the project | human only |
| **Human owner and working agreement** | the same charter | human only |
| **Invited proactive roles** | the charter; a record of which roles may suggest the next work | human only |
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

`team.roles` remains the roster. The team room does not add another copy of it:
the roster, the charter and the context are three views of the same project.

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
it; human only) and `judge.status` reports whether one is set. It is optional.

### Templates

A run template is a goal written once, with blanks, and the definition of done every
run of it carries. devd ships five — `spec-to-pr`, `fix-bug`, `add-feature`,
`security-review`, `release-prep` — and your own in `<devd home>/templates/` join
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

### How the team hands work on

Every role says who usually receives its work next (`hands_to` in its file). The
planner reads those hand-offs on the roster, so the dependencies it writes follow
the flow: an author's task is followed by the QA and review tasks that depend on
it, a film by its review, a release by its notes. `team.flow` returns the same
graph (each member, who it hands to, who hands to it) for agents and for the
Team screen. A hand-off may go back as well as forward (a reviewer hands a
finding back to the author), so the graph is not required to be acyclic; it is
required to name real roles.

| Flow | Hands to |
|---|---|
| researcher, product | planner, designer, architect |
| architect | the engineers (implementer, frontend, backend, mobile, data, ML, hardware) |
| designer | frontend, motion designer, reviewer |
| engineers | QA, reviewer (mobile hands to QA and release) |
| motion designer | reviewer, content writer |
| QA | reviewer, release |
| reviewer, security | back to the author, on to release |
| infrastructure, reliability | each other, release |
| release | docs, content writer |

### Your own roles

A role of your own is a Markdown file in `<devd home>/roles/`, in the same
format as the built-in roles, and it joins the team. Three ways to make one, all
checked the same way:

* **In the window.** Team, then "Add a member": a name, a one-line summary, what
  it may do (read only, change files, or change files and run commands), the
  skills it carries, who it hands work to, and its instructions. What it must
  prove to count as done follows from what it may do.
* **By capability.** `roles.save` and `roles.delete` (human only, because a role
  decides what an agent may do). `team.flow` shows where the new member sits.
* **As a file**, in the folder above.
* **From a pack.** `packs.install` (human only) adds the roles, skills and tool
  servers a pack names, all or nothing. The bundled `recreate` pack adds the
  Recreator, its four skills and the craft-anything tool server (disabled);
  `packs.remove` takes exactly those back.

A role of your own may use up to the exec tier, must not name extra capabilities,
cannot take a built-in's name, may carry only skills that exist (built in or in
your own library, including one a pack installed), and may hand work only to
roles that exist. One that does not pass is refused with the reason and
nothing is written (or, for a file, reported by `team.roles` while the rest of
the team is unaffected). It joins the team **at once**, with no restart: the
roster is read again after every save or removal. A role cannot be removed while
another still hands work to it.

This keeps the end-to-end flow intact because a role of your own is not a second
kind of agent. It runs through the same loop, the same permission tiers, the same
worktree boundary and the same evidence check as devd's own: its task is accepted
when devd's record shows what its `proves` says, not when it says it is done.

### The motion designer

Makes films, animations and product motion. It carries `motion-design` (how a film
is planned: one idea, three documents, real material, restraint, looking at the
frames), `hyperframes` and `remotion-film`.

Two ways to render, and what each needs:

| | Built into devd | Remotion |
|---|---|---|
| Tools | `motion.new`, `motion.render` | `npx remotion ...` through `project.exec` |
| Install | none | Node, `create-video`, and a browser it downloads |
| Good for | markup and CSS films, UI motion, chart sequences | React components, 3D, maps, captions |
| Output | PNG frames always; an MP4 when ffmpeg is on the machine | MP4 through its own bundled encoder |

A built-in composition is a page that is a pure function of time. devd serves the
folder on a loopback port, opens it in its own browser at the composition's size,
asks for each frame's time in turn and photographs the page, so two renders are
byte-identical. The one thing devd does not carry is a video encoder: it uses an
`ffmpeg` it finds (or the path in `DEVD_FFMPEG`), and when there is none it says
so and still delivers the frames. `motion.render` with `at` makes stills of chosen
moments first, so a film is looked at before it is rendered.

A film about the team is drawn from real data: `team.trace` returns a run's tool calls in order, by specialist, with timings (arguments and results left out), which is how the launch film's 238-call run was shown rather than imitated.

Remotion's official skills (`remotion-dev/skills`) are not bundled: they are
versioned with each Remotion release and published without a license file, so
devd fetches them from their repository when a person asks (the command menu has
"Install the Remotion skills"; it is `skills.install` with that source). A skill
installed from outside keeps its reference pages: `skills.get` lists them as
`files` and reads one with `file`, so a router skill's pointers lead somewhere.

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
`approval`, `plan`, `criteria`, `verdict` or `budget`. `missing` names the
evidence devd actually looked for and did not find, in the same words the
scheduler rejected the report for. The first action is the one that unblocks the
work; accepting without proof is offered after it, and is marked dangerous
because the run's report will say a person accepted it by hand.

A run that stopped at its budget asks for the budget, not for a check: the pause
reason is read before any task is considered, so a task that never ran is never
reported as a failed review. Its one action opens the run, where the ceiling is
raised; raising it carries the run on by itself, because resuming into the same
ceiling would pause again at the next step.

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
task is about to start and the machine has since become production, when the
rules it was accepted under have changed, when its role no longer exists, or when
its scope names a path outside the project, the run pauses with
`mandate_revoked` and the reason instead of continuing. A person resuming the run
grants it again under the current rules, and resets the stall watchdog for its
tasks.

The mandate is a record, never a permission. Approvals, evidence and the role
tier still decide what may run; a mandate explains why devd believed it could
run.

### A worker that stops moving

Every model turn and every tool call moves the task's `last_step_at`. If a
running task's heartbeat does not move for **twice the provider's own request
timeout** (never less than three minutes, so a slow model is never mistaken for
a stuck one), the scheduler:

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
words (or the caller says so outright: the composer's mode, the `kind` on
`chat.send`, `devd chat --as team`):

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

## The scorecard

`team.stats` measures the work devd actually recorded: tasks accepted on proof,
tasks rejected or abandoned, extra attempts, review changes requested, checks that
passed, and median time. `models.usage` adds tokens, estimated spend and
conversation errors. The Intelligence screen puts them together.

A member is ranked from **55% accepted on proof, 25% checks passed, 20% low
rework, minus up to 10% for review changes**. A member needs three settled tasks
to be ranked; below that the record is shown and says "too new to rank". Cost and
cost per accepted task are reported beside the rank, never folded into it, so a
cheap model is not called good and an expensive one is not called bad.

A model with no recorded model on older tasks is one row, "Model not recorded",
not a guess. A model that has done work but no finished team task is shown with
its conversations, tokens and errors, unranked. The bars are the comparison; the
window does not invent a time series it does not have.

## Skills

Skills are short Markdown instructions agents load with `skills.get` when the work
calls for them. 92 ship with devd, and each belongs to one stage of the work, so
an agent finds the right one without reading all of them. The table below maps the
stages rather than listing every skill; the prompt index and `skills.list` are the
authoritative list.

| Stage | What it covers | Skills |
|---|---|---|
| **Discover** | understanding the request and the system before acting | `work` (start here), `how`, `why`, `investigate`, `check-the-request`, `sharpen`, `restate`, `choose-by-the-outcome`, `recall`, `catch-up`, `handoff`, `uncover-the-real-structure` |
| **Design** | deciding a shape | `architect`, `design-from-use`, `weigh-several-shapes`, `re-examine-the-design`, `model-the-domain`, `put-rules-in-the-compiler`, `validate-at-the-edge`, `arena`, `api-design`, `figure-it-out`, `threat-model`, `product-requirements`, `user-stories` |
| **Build** | writing the change | `interface-contract`, `build-feature`, `fix-bug`, `refactor`, `prototype`, `improve-performance`, `tdd`, `land-one-unit-at-a-time`, `make-the-narrowest-change`, `build-the-tool-if-it-repeats`, `finish-the-migration`, `plant-the-fix-in-the-structure`, `no-comments`, `benchmark-checklist`, `create-verification-skill`, `database-migrations`, `swarm`, `hyperframes`, `stageshow`, `work-on-a-branch` |
| **Check** | proving a result and keeping it honest | `prove-it-works`, `test-what-callers-see`, `fix-the-root-cause`, `report-numbers-honestly`, `keep-it-readable`, `agent-browser`, `agent-desktop`, `check-the-mail` |
| **Review** | reading someone else's change | `interrogate`, `blast-radius`, `secure-coding-review`, `ui-design-review`, `accessibility-check`, `correct`, `reflect` |
| **Ship** | landing it | `ship`, `decision-log`, `mobile-release-checklist` |
| **Words** | prose, reports, documentation | `plain-speech`, `unslop`, `technical-writing` |
| **House rules** | how work runs here: autonomy, ownership, staying restartable | `act-then-ask`, `decide-who-owns-the-state`, `make-it-safe-to-rerun`, `keep-your-context-lean` |

The order of the table is the order of the work, and the prompt index shows the
stages in that order. A skill's file names its own stage in `kind:`, so a person
who adds a skill picks the stage it belongs to; a stage of their own is shown
under its own name rather than dropped.

An agent's prompt carries nine core rules, each naming the skill that holds the
full text, and the index with one line per skill. Only names and descriptions go
into the prompt; the body is read on demand. Add your own in
`<devd home>/skills/*.md` (`devd skills add`); they can add to the built-in set
but never replace a built-in skill, and only a person can save or delete them.
`/learn <source>` (`skills.learn`, or `devd skills learn`) turns a public page, a
local file or folder, or an `owner/name` README into a skill with its material
kept under `references/`. `/learn` is itself a built-in skill, so it is in the
composer's `/` menu.

A test checks the wiring rather than trusting it: every file in `skills/` is
embedded in the binary, is named after itself, belongs to a known stage, names
only real roles, has a unique name and description without a type prefix, and
refers only to skills and capability groups that exist.

## Tools agents have beyond the core set

`search.query` (projects, files by path, literal text in a project, conversations,
full conversation text as `sessions`, models, tasks, activity — each response says
what was searched and whether it was cut short), `search.sessions` on its own
(best match first, paged with `offset`/`next_offset`), `docs.fetch` (public https pages only; private, loopback and
metadata addresses refused and re-checked across redirects), `web.open` (the
same fetch, returning the page's title, headings, links and form fields as well
as its text, for a page an agent has to act on), `skills.*`,
`team.*`, `services.inventory`, `environment.get`, `devd.self`, and the file, git,
project, toolchain and stack capabilities.

`devd.self` is the agent's own record: version and build, platform, home and the
layout under it, the state database and its schema, the environment mode, the
model assignment, the window's screens, the surfaces that reach devd, the current
actor, run, project and conversation, and the limits it works under. The
assistant's prompt carries a short live brief from the same source at the top of
every turn, so it does not have to infer its own version, paths or capability
index. The capability registry is still the authority for what devd can do;
`devd_groups` lists it.

Tools from **other programs' MCP servers** join the same list when a person
configures one in *Team > MCP*. They are named `mcp__<server>__<tool>`, they
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
approval). In production, an agent needs a person's approval for every operation
above read — writing a file included — and plan approval cannot be waived; the
status bar says so. Destructive, remote and admin operations need approval in
both modes, with one scoped exception: removing files inside the conversation's
project is ordinary work (the project root, `.git` and secrets files are still
refused), while any path outside that project, and every other destructive
operation, needs a person. A local site deploy is an exec action; a deploy that
reaches another machine (`remote.*`) and a push are remote actions and wait for
approval. Agents cannot open ports or edit a production database through devd.

## Services that already exist

`services.inventory` looks at databases and services on the machine and keeps four
facts apart: **installed**, **running** (the port answers and the health check
passes), **required** (a project you imported says so) and **managed** (what you
decided). `services.decide` records `adopt`, `observe`, `external` (your own
instance on a port) or `ignore` — a record only; nothing is installed, stopped or
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

## A run as one conversation

A team run is one piece of work, but its messages have lived in one conversation per specialist: to
see what the team was doing you opened each one, and the conversation you started the run from only
heard single-line notices. `chat.team_log` reads the whole run as one list instead: the person's
messages, the planner's, and every specialist's, tagged with the role that said them and interleaved
in the order they happened.

```json
{ "run": "…", "origin": "…", "roles": { "<thread>": "implementer" },
  "threads": ["…"], "events": [{ "thread_id": "…", "role": "implementer",
                                 "origin": false, "kind": "assistant", "data": { … } }] }
```

It is a read, not a copy: the per-specialist logs stay where they are, and nothing is duplicated into
the conversation that started the run, so the assistant's own context never has to be told to ignore
its team's chatter. Asking from any conversation of the run returns the same list, and a conversation
that is not part of a run comes back as itself.

The window reads it: a conversation that belongs to a run draws the whole run as one transcript —
each teammate's turn is a bubble with its face and name, the assistant's own turns stay flat as the
room's voice, and a message someone addresses to a teammate is a bubble of its own with `@role` and
what it is about. The per-specialist conversations remain reachable as the detail behind a bubble.

**Talking in the room.** `chat.talk` posts a message into the run's conversation: `to` addresses a
role (`@reviewer`), `about` names the task it is about, and both are kept with the message so the
room can show what it answers. The role that speaks is the conversation the call came from — a
specialist's own thread says `from: implementer`, the conversation that started the run says
`from: devd` — and an address to a role the run does not have is refused with the roles it does
have. A message never starts a turn by itself: it is speech, and the scheduler still decides when an
agent works. An **addressed** message is also delivered: it is queued into the conversation of the
role it names, so that agent reads it at its next step and can answer in the room. The run's budget
is what bounds a conversation between agents, not the harness refusing to deliver.

The composer in a room speaks to the team, not only to the assistant: sending while a run is open
also records the message as a direction for the run (`team.say`), so speaking during the work steers
it — tasks that have not started take the instruction, and running tasks get it when they next stop.
The browser client's composer does the same, and a run that has already finished refusing the
direction is not shown as a failed message, because the message itself was delivered.

Every member's brief says the room exists and how to speak in it, on the first attempt and on every
restart, because a member that does not know it can be heard cannot ask a question, comment on a
change, or answer the teammate who addressed it.

The browser client reads the same room: a conversation that belongs to a run draws every member's
messages from `chat.team_log`, each turn under the name and face of the role that opened it, a change
of speaker starting a new turn, and an addressed message as its own quiet block with `@who` and what
it is about.

## Settling a conversation

A finished conversation leaves the working list without being deleted: **Settle**
in its header moves it to a **Settled** shelf at the bottom of the list, dimmed
and still one click from being opened, reopened, renamed or deleted. Settling
keeps the log, the chapters and the linked pull request; only the place in the
list changes. Speaking to a settled conversation reopens it, so work can never
carry on out of sight.

The conversation's own menu (the three dots at the end of its header) is where
everything about the conversation rather than the work lives: **Fork from here**,
**Pin to the top**, **Mark as settled**, **Rename**, **Link a pull request** and
**Delete**, the last one in the colour that means it cannot be undone. The bar
keeps only what a person does while working — stop, where the files are, the
changes, the review — so the header is no longer a row of identical icons.
Rename and the pull request link are forms: the same dialog a capability with
fields gets everywhere else in the window, not a bespoke prompt.

A conversation can carry the pull request it is about: `chat.link_pr` records it
(and the header shows it as a quiet link), and `chat.settle` is the record behind
the shelf, so an agent, a script and the window agree on what is settled.

## Putting a conversation aside

Snoozing is not settling. Settling says the work is finished; snoozing says it can
wait for a better hour. **Snooze for an hour** and **Snooze until tomorrow** in
the conversation's menu (`chat.snooze`, human only) move it out of the working
groups onto a **Snoozed** shelf, dimmed, with the time it comes back instead of
how long ago it moved — "until 21:00", "until 09:00 tomorrow". It returns to the
working list by itself when that time passes; no job runs, nothing is scheduled,
the list simply asks a stored instant which side of now it is on. **Wake it now**
clears it.

The instant is stored, not a duration, and it is compared as a time rather than as
text: a snooze set in another time zone is still a moment, and a client that opens
the list an hour later has to agree with one that was already open. A time that has
already passed is refused with the reason, and speaking to a snoozed conversation
wakes it the way a message wakes a settled one — a message is the person asking
for it now.

## A conversation's own worktree

Two pieces of work on one checkout tread on each other: a stash, a branch switch, a half-finished
edit. A conversation can own a **worktree** of its project instead — a real branch
(`devd/<conversation>`) in a real folder (`<project>/.devd/worktrees/<conversation>-chat`), in the
same repository — so its changes cannot collide with anything else, and the project's folder stays
as it was until someone merges the branch.

- `chat.worktree` does it: `create` makes the worktree (or finds the one this conversation already
  has), `attach` points the conversation at a folder that already exists, and `detach` puts it back
  on the project. Detaching never deletes the folder or the branch, so nothing a conversation did is
  lost by going back.
- It is a person's decision. The window offers it beside the conversation's other actions; an agent
  cannot move its own workspace.
- The conversation's tools run in that folder — file reads and writes, `project.*`, and the
  `git.*` calls the agent makes — because the harness hands every call the conversation's working
  folder, not the project path. `AGENTS.md`, `devd.yml` and the project's own commands are read from
  there.
- The project stays the project: the sidebar still groups the conversation under it, the run still
  records it, and `devd.self` reports the worktree as `current.workdir` so an agent can say where it
  is instead of guessing.
- A repository with no commit has nothing to branch from, so `create` is refused with that reason
  rather than committing on the person's behalf.

The folder is kept out of the project's status through `.git/info/exclude`, the same way the team's
worktrees are, so `git status` in the project stays clean while a conversation works.

Deleting the conversation takes an **empty** worktree with it: the folder and its branch go when
there is nothing uncommitted and the branch is merged into the project's current branch. Anything
else is left exactly where it is, and `chat.delete` says where and why — "its worktree was kept
because it has changes that were never committed". Deleting a conversation is never a way to lose a
change by accident. `chat.prune`, which deletes many at once, reports how many worktrees it kept.

## Performance

`perf.measure` answers "is devd still fast?" with numbers from this process rather than an adjective:
opening and migrating the state database, writing 500 conversations, listing them, appending 500
events to one log, and reading that conversation back. Each reading carries a budget, and the
answer says whether it is inside it.

Every reading is the **best of three runs**, because it is measured from a window that is also
drawing and a server that is also serving: the minimum reports the work, an average reports the
machine's mood.

The window writes its own launch to `startup.json` beside the state on its first frame, and
`perf.measure` reports it as two readings rather than one, because they are two waits:

| Reading | Warm devd | Cold devd |
|---|---|---|
| **devd ready** — wait for a running devd, starting one if needed | ~150 ms | 4–6 s |
| **window** — from there to the first frame | ~0.9 s debug | ~0.9 s debug |

The split is why they are separate: a single number would say "devd takes six seconds", which is
true only when nothing was running, and would hide the other five and a half seconds that no longer
happen. Cold start is measured, visible on the Diagnostics screen, and was the next lever — the
window used to open the store, migrate it and build the whole capability registry *before* asking
whether a devd was already running, and then throw all of it away to attach. Now it only bootstraps
on the path where it becomes the server:

| Launch (debug build) | Before | After |
|---|---|---|
| Warm devd, window opens | 1.03 s | **0.75 s** (attach: 147 ms → 7 ms) |
| No devd, one is started | 2.36 s | **1.85 s** |

The daemon prints its own phase line when it starts — `devd started in 176 ms: paths 6 ·
settings 5 · store 44 · launchers 0 · git 81 · capabilities 24 · agents 16` — so a slow start is a
list rather than a mystery.

What is left of a warm launch is the window toolkit itself, not devd's work. The launch record
carries every phase, and on this machine it reads like this:

| Phase | Debug build |
|---|---|
| Attach to the running devd | 9 ms |
| Load the theme | 1 ms |
| Application and platform start-up, and the agent runtime | 285 ms |
| Window creation | 450 ms |
| Building the shell | 1 ms |
| First frame | 1 ms |

Two experiments in this round tried to cut it and measured nothing: moving the spell-check
dictionary to a background thread (the dictionary was never the cost — the time between the
runtime and the first frame is window creation, and the measurement now says so) and reordering
work inside the shell. Both were reverted rather than kept for looking plausible; the instrumentation
that disproved them stayed.

The budgets are **regression lines, not targets**: several times what the work costs on an ordinary
machine, so a busy afternoon is not a failure and a change that makes the harness ten times slower
is. The same readings run as a test, with a wider line so a loaded build machine does not fail the
suite; a miss there is a regression worth explaining, not a verdict on the machine.

```bash
devd call perf.measure
```

An agent can call it too: it is read-only, and it is the honest answer to "how fast are you" that
does not depend on the model's opinion of itself. The same readings are on **Settings →
Diagnostics → Speed**, each with its budget and whether it is inside it, so the question "is devd
the problem?" is answered on the screen where a person asks it rather than in a terminal.

## Shipping what a conversation did

The loop from a change to a review has three steps, and each one is a real surface rather than a
copy of one:

1. **Review.** The **Changes** action in a conversation's header opens the project's own Git screen
   on the folder the conversation works in — the worktree when it has one, the project otherwise.
   Staging, per-file diffs, the commit message, pull and push are the same screen a person uses for
   any project, so the two cannot drift. Each change in the list has a **read mark** — a dot in front
   of it, hollow until it has been looked at — and a heading that says how far the review has got
   ("Changes 2 · 1 read"). A change that has been read dims and steps back, so what has not been
   read is what the eye finds first. A group's heading offers **Mark all read** — quiet, because
   reading is not a change to the repository, and the staging button beside it should be the one the
   eye lands on. The marks belong to the window's session: they are a review aid, not a record of
   anything the host should remember.
2. **Report.** `chat.changes` is the same fact as data: one entry per path with its status and
   whether it is staged, the branch and its upstream, and how many commits are ahead or behind.
   The window uses it to say what is waiting; an agent reads it before it says what it did.
3. **Review the branch.** `chat.pr` opens the pull request a pushed branch becomes, through the
   GitHub CLI, and links it to the conversation. A branch that has not been pushed is refused with
   that reason: a review is of commits the host can see. Without a title the host fills the title
   and body from the commits, so the review starts from what actually changed. It is remote, so a
   person approves it, and the header then shows the pull request as a link.

The branch is not forced on anyone: a conversation in the project's own folder can be reviewed and
committed the same way, minus the isolation.

The browser client carries the same three: the conversation's menu has **Review the changes**, the
worktree toggle and **Open a pull request**, and its header shows the branch beside the linked
review. It reads `chat.changes` when a conversation opens rather than on every poll, so the numbers
in the header and the Git screen are the same ones the window uses.

## Deleting conversations, runs and messages

Nothing is deleted on its own; a person decides, and every deletion asks for
approval first (agents cannot do it).

| What | Window | Command | Capability |
|---|---|---|---|
| One conversation | the cross beside it in the Assistant, or Delete in its header | `devd chats delete <id>` | `chat.delete` |
| Every finished conversation | Clear finished in the Assistant list | `devd chats prune [--days N]` | `chat.prune` |
| One finished run | the cross beside it, or Delete in its header (Runs) | `devd team delete <run>` | `team.delete` |
| Every finished run | Clear finished in the Runs list | `devd team prune [--days N]` | `team.prune` |
| One inbox message | Delete in its sheet | `devd call inbox.delete '{"id":"..."}'` | `inbox.delete` |
| Answered or dismissed messages | Clear under Earlier in the Inbox | `devd chats clear-inbox` | `inbox.clear` |

Deleting a conversation stops it first if it is working, then removes its log
and the approval requests it made. Deleting a run removes its tasks, its log, the
conversations its tasks had, and the Inbox messages and undelivered reports that
were about it; cancelling a run resolves those messages, so a decision that can
no longer be taken stops being offered. A run that is still working must be
cancelled first, and work already merged stays. `prune` never touches a
conversation that is working or waiting for you, and a run that is not over.
Deleted data cannot be recovered.

## Under a reply: copy, fork, chapters

Every reply in the Assistant has three actions beneath it (shown when you hover it):

* **Copy** puts the reply's text on the clipboard as Markdown.
* **Fork from here** (`chat.fork`) starts a new conversation containing
  everything up to and including that reply; the original stays exactly as it is.
  Approvals that were still waiting are not copied, and the answers to any tool
  calls that were open at that point are.
* **Pin as chapter** (`chat.pin`, `chat.unpin`, `chat.chapters`) gives that reply
  a title. **Chapters** at the top of the conversation lists them and jumps to
  one. Chapters belong to the conversation they were pinned in; a fork starts
  without them.
