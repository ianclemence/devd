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
| Tools are the interface; keep the surface small and progressive | Five MCP tools (`devd_groups`, `devd_capabilities`, `devd_describe`, `devd_invoke`, `devd_doctor`) reach every capability. Agents load detail only when needed. |
| Keep the session log outside the model; the harness is replaceable | Conversations are append-only rows (`chat_events`). A restart loses nothing; runs resume from the log. |
| Orchestrator and workers for work that splits | `team.*`: a planner, a deterministic scheduler, and specialist conversations. |
| Verify with evidence, not claims | A task is accepted only when the tool results in its conversation show the work (below). |
| Human approval for consequential actions | One approval queue. Agents queue; only a person decides. |
| Keep the context lean | Tool results are shortened and old ones cleared; skills are loaded on demand. |

Claude Code itself is integrated as a *session* (`devd-workspace`): devd starts it
in the project directory with devd's MCP server attached, so it can use every
capability with the same permission model. devd does not proxy or re-implement
its tool loop; its own agent runs through the providers you configure.

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
* **Specialists** are conversations with a role prompt, a tier ceiling
  (`devd-agent::roles`), a working directory they cannot leave (paths outside it
  are refused), and a model chosen by role. Planner, architect and researcher
  change nothing; the reviewer reads and runs checks but writes nothing; only the
  implementer, QA and docs roles write files.
* **Evidence.** After a specialist reports, devd reads the tool results in its
  conversation. An implementer must have changed files *and* had a verification
  (a `project.run` of test, build, lint, check...) exit 0; a reviewer must have
  read the change; QA must have run checks; infra must have run something that
  succeeded. Otherwise the report is rejected twice with the reason, then the
  task is `unverified`, its dependants do not start, and the run pauses with
  what is needed (retry with a note, or accept by hand - recorded as such).
* **Final verification.** When every task is done devd adds a QA task that runs the
  project's own checks independently. The run's summary is built by devd from the
  recorded evidence, not from what agents wrote about themselves.
* **Persistence.** `team_runs`, `team_tasks`, `team_events`. On restart, running
  tasks go back to pending and their conversations continue; paused and
  waiting runs stay as they were.

`devd team start "..." --project .` follows the run in the terminal;
the window has Assistant > Team runs; the terminal dashboard has a Team view and an
Inbox view; all read `team.get`.

## Skills

Skills are short Markdown instructions agents load with `skills.get` when the work
calls for them. 55 ship with devd:

* **Playbooks** - `work` (choose one), `investigate`, `fix-bug`, `build-feature`,
  `refactor`, `prototype`, `improve-performance`, `ship`, `handoff`.
* **Techniques** - `how`, `why`, `blast-radius`, `architect`, `arena`, `swarm`,
  `interrogate`, `tdd`, `sharpen`, `restate`, `catch-up`, `recall`,
  `figure-it-out`, `decision-log`, `reflect`, `correct`, `unslop`, `no-comments`,
  `plain-speech`, `technical-writing`, `benchmark-checklist`,
  `create-verification-skill`.
* **Principles** - 24, one rule each (`principle-prove-it-works`,
  `principle-fix-root-causes`, `principle-subtract-before-you-add`, ...).

An agent's prompt carries eight core rules and an index of names with one line
each; the body is read on demand. Add your own in `<devd home>/skills/*.md`
(`devd skills add`); they can add to the built-in set but never replace a
built-in skill, and only a person can save or delete them.

## Tools agents have beyond the core set

`search.query` (projects, files by path, literal text in a project, conversations,
models, tasks, activity - each response says what was searched and whether it
was cut short), `docs.fetch` (public https pages only; private, loopback and
metadata addresses refused and re-checked across redirects), `skills.*`,
`team.*`, `services.inventory`, `environment.get`, and the file, git, project,
toolchain and stack capabilities.

## Local and production

`environment.set` marks the machine `production` (a person's decision, with
approval). In production an agent needs a person's approval for every operation
above read - writing a file included - plan approval cannot be waived, and the
status bar says so. Destructive, remote and admin operations need approval in
both modes. devd has no deploy capability, and agents cannot open ports or edit a
production database through it.

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
