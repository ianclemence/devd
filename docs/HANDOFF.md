# Handoff: finish the devd revamp, release it, make the launch video

You are continuing work another agent started with the owner of devd. Read this whole file before
touching anything. It says what devd is, what was done, what is half done, what is still to do, how
to build and test safely on this machine, and the rules the owner has set. Work through
"What to do next" in order. Report back plainly: what you finished, what you verified, what is left.

## What devd is

A local development environment built for working with AI agents (Rust workspace, Windows-first).
Every action is a capability in a registry (456 of them: `crates/devd-core/src/registry.rs`,
`crates/*/src/caps.rs`), planned, approved where needed and audited, whether it comes from the
desktop window, the CLI (`devd`), or an agent over MCP (`devd mcp`).

- `crates/devd-server/ui/{index.html,app.js,style.css}`: the whole interface, a vanilla-JS single
  page embedded into the binary with `include_str!` (rebuild to see changes in the binary).
- `crates/devd-agent`: the built-in assistant (`chat.rs`), the agent team (`team.rs`: planner,
  implementer, reviewer, QA... with plan approval, evidence checks), skills (`skills/*.md`,
  registered in `src/skills_data.rs`), team stats (`src/team_stats.rs`).
- `crates/devd-core`: store (SQLite), registry, runs, `memo.rs` (probe cache).
- `crates/devd-stacks/defs/tools.yaml`: the tool catalogue. `docs/stacks.md` is generated from it:
  after editing tools.yaml run `DEVD_UPDATE_DOCS=1 cargo test -p devd-stacks --test engine`.
- Live updates: `GET /api/events` (SSE). The window opens it once at start (`openLive` in app.js);
  `onLive` / `onDurable` / `onOtherWork` route events to the screens.

## Rules from the owner (follow exactly)

- Design bar: the Claude Code desktop app - match or beat its polish, never copy it. "No slop."
  Palette is **graphite + one blue** (chosen by the owner). One type family (Inter Tight), one
  type scale, sentence-case labels, no decoration, every width must work (1600, 1280, 1024, 800, 390).
- Never run a dev build against the live home `D:\devd` (it migrates the database). Test on a copy:
  `C:\dthome-025` (already prepared: services table emptied, schedules off, ports 18080/18443).
  Always set `DEVD_HOME` explicitly; sessions inherit a stale `D:\devd-data`.
- Never touch the real project `D:\devd\www\nairobiunwind`; the demo copy `nairobiunwind-demo` only.
  Its database holds copied real user data: use invented people, never run its queue worker.
- Never print `.env` secrets. Releases are unsigned (no key on this machine) - say so in notes.
- Commit attribution line: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Python edits on this Windows machine: read/write in **binary mode**. Text mode writes CRLF and
  breaks `crates/devd-server/tests/ui_contract.rs` (the repo is LF). Check `git diff --stat` for
  whole-file churn after scripted edits.
- The owner has authorised for this task: stage, commit, push, publish the GitHub release, update
  the installed version, then build the launch video.

## Build and test

```powershell
cd D:\devd\www\devd
. .\scripts\dev-env.ps1            # target dir C:\dt (a long path crashes gcc on ring)
cargo build -p devd-cli            # C:\dt\debug\devd.exe
cargo test --workspace             # ~1,060 tests; all passed at commit 3dedaee
cargo test -p devd-server --test ui_contract   # the interface rules (72 tests)
```

Test server on the copy (stop it before rebuilding - it locks devd.exe):

```powershell
$env:DEVD_HOME = "C:\dthome-025"; Start-Process C:\dt\debug\devd.exe -ArgumentList "serve","--port","18787" -WindowStyle Hidden
```

Screenshot tooling lives in `D:\devd-promo\capture` (Node + playwright-core driving Edge):
`audit.mjs` (every route at five widths; `UI_DIR=<ui folder>` serves app.js/style.css/shell from
disk without a rebuild), `local-ui.mjs`, `conv.mjs`, `check2.mjs`, `livecheck.mjs` (proves a new
conversation appears live), `timing.mjs` (API calls per screen with durations), `sheet.mjs`
(contact sheets). Before/after shots: `D:\devd-promo\public\audit\{before,after1,conv-before,nav2,final,check2}`.

## Done and committed (commits 3dedaee and 0e37240, both pushed to origin/main)

See `docs/review-2026-11.md` for root causes and verification. In short:
- Navigation: 72px rail + 264px context panel + page (no top bar, no tab bar). Panel folds to an
  overlay below 1024px; rail becomes a bottom bar with a More sheet below 640px.
- Conversations/runs grouped by project, specialists nested under the request (`chat.list` now
  returns a `team` link per thread: `store_team::thread_team_links`).
- Live sync for every screen; DOM patching (`morph` in app.js) so scroll, open cards and images
  survive updates; jump-to-latest pill; "Worked for Xm Ys" fold with result tiles; role faces;
  concrete step lines; edit diffs; project card (branch, changes); run view as a conversation.
- Team gate: reviewer/security/QA "changes needed" -> fix task + re-review, max 2 rounds
  (`verdict_needs_changes`, `request_changes` in team.rs).
- Performance: SSE starts at newest event (`latest_event_id`); git status loaded 4 at a time only
  where shown; `memo` cache for health/toolchain/requirements, invalidated by any write.
- Graphite+blue tokens, type scale, non-scaling icon strokes, callouts/modals/toasts restyled.
- Skills: built-in `agent-browser`, `agent-desktop`; tools: FFmpeg, HyperFrames, agent-browser,
  agent-desktop; Skills page "Recommended": HyperFrames, Stageshow.
- Prompts: agents narrate one sentence of intent before each batch of tool calls.

## Done in the 0.25.0 commit (version bumped, CHANGELOG written, tests passing, NOT yet released)

Version is 0.25.0 in `Cargo.toml` and `crates/devd-desktop/tauri.conf.json`; the CHANGELOG has the
0.25.0 entry. `cargo test --workspace` passed (1,074 tests) and `ui_contract` passed (72) after the
last change. The debug binary was rebuilt after the commit for the visual QA pass below.
Also in this commit:
- Fixed a regression from the navigation work: the context panel's styles were named `.panel`,
  `.panel-head`, `.panel-body` - devd's generic card classes - and broke API & Tools, Team room
  and other card screens. Every navigation rule is now scoped to `#panel` and the element's class
  is `ctx-panel`. Never style `.panel*` for navigation again.
- Semantic colour names (`--color-*`, see below) and quiet motion (rise-in for new messages,
  press feedback, page fade; all off under reduced motion).

- Long code blocks fold (`md-code is-long`, Expand/Collapse is a `<details>`).
- Search inside a conversation (search icon in the chat header, Ctrl F): CSS Custom Highlight API,
  opens folds that hold a match (`findBar`, `findRun`, `findStep` in app.js).
- Diff badges code / tests / docs counted from the real patch (`diffKinds` in app.js, shown in the
  project card). Verified on the demo project: code +33 −32, tests +107 −0.
- New capabilities `team.stats` and `models.usage` (`crates/devd-agent/src/team_stats.rs`,
  registered in caps.rs before `chat.list`); views: "How the team is doing" on the Team room page
  (`teamRecordHtml`), "Usage" on the Models page (`usageHtml`). No cost shown on purpose: prices and
  cache tiers are not recorded per call.
- Plain-language capability names everywhere people read them (`capLabel`, `loadCapSummaries`):
  Activity, Home "Recently", approvals, evidence tables, step fallbacks. Raw ids only in tooltips.
- Inbox repaints live on inbox/approval/team events (`onOtherWork`).
- Jump-to-latest pill anchored 14px above the end of the messages (measured, not fixed).
- Run goal renders as calm Markdown, folded when long.
- `chat.fork` now emits a live event.
- `docs/HANDOFF.md` (this file).

## Design system (new in this pass)

- `docs/design/README.md`: principles, the complete colour token set, shape/size/depth,
  typography, spacing, motion, layering, component layers and the process for adding a component.
- `docs/design/components.md`: the 68-reference review with a decision and reason for each, plus the
  existing DEVD component audit and the consolidations made.
- `docs/design/agent-ui.md` and `docs/design/manifest.json`: what an agent may emit, what it must
  not, and the machine-readable component list. The manifest is served by the read capability
  `design.manifest`; the `interface-contract` skill carries the same rules into every relevant
  agent prompt.
- The system is applied in the product, not in a separate gallery. Existing controls (buttons,
  fields, selects, checks, notices, tables, rows, tabs, drawers, modals, the conversation) are the
  components; they use the semantic tokens. The one new inline rendering is the agent `chart`
  block, drawn by the same stylesheet inside a conversation. Keyboard hints use the shared `kbd`
  treatment.
- Shape/size/depth tokens were added; `--radius` and `--radius-lg` now point at the field and box
  radii, so no rendered radius changed. The page background was not changed.

## Navigation: the decision

The rail shipped as **Home · Inbox · Assistant · Runs · Team · Projects · Sites · Machine**, with
**Models · Activity · API · Diagnostics** at the foot. This is the agreed structure for 0.25.0:
Inbox and Runs are now real, separately addressable pages with their own lists, and splitting
`Agents` into `Runs` (the work) and `Team` (the people and the room) makes the two questions
distinct. The earlier sketch that kept `Agents` and put Activity/API/Diagnostics under Machine
predates those pages; it is superseded, not drifted. Do not re-merge them without a design pass.

## Visual QA (done in this pass)

1. **Built UI verified.** `cargo build -p devd-cli` from this tree; test server on
   `C:\dthome-025` (port 18787) using the macro. `node capture/check2.mjs` in
   `D:\devd-promo` captured Team record, Models Usage, Inbox, API & tools, skills, services, MCP
   and certificates at desktop, tablet and phone widths. Team record and Usage render with real
   data (5 KPI tiles + 3-row role table; per-model and per-member token tables).
2. **API & Tools at five widths.** `audit.mjs` covers `api-tools` at 1600/1280/1024/800/390.
   The registry stays one column and does not scroll sideways; the filter shrinks rather than
   squeezing the list.
3. **Full 130-shot audit.** `public/audit/after3` holds every route at five widths, with
   `report.json` / `issues.jsonl` from the overflow/clipping checker: **130 shots, 0 issues, 0
   errors**. `public/audit/final2` holds the screens changed afterwards (diagnostics, assistant,
   Projects, the folder picker at rest and after a typed path, Clone's picker, Team, Models, Inbox,
   and API & tools at five widths).
4. **Buttons.** `.btn`, `.btn-mini`, `.btn-ghost`, `.btn-primary`, `.btn-icon` are used per the
   system; icon-only buttons in the rail, chat header, thread list and sheet headers carry
   `aria-label`; destructive actions are quiet or confirmed. The 72 `ui_contract` tests cover
   handlers, ids, one primary action per view and the shared motion language.

### Fixes made after the 0.25.0 commit

- Diagnostics: the status icon sat above the "Healthy, ..." sentence because the larger
  `ui-diag` notice inherited the compact callout's `top: 6px`; it is now centred on the first
  line and the text has a 52 px gutter.
- A new conversation's "What are we working on?" panel no longer explains how to choose a
  project, does not show the provider's remote host, and shortens the approval line to
  "Asks first" / "Automatic".
- The Diagnostics "Registry" cloud of group counts is replaced by one sentence explaining what
  the registry is, a count, and a link to API and tools. The full searchable list already lived
  there; two presentations of the same counts helped nobody.
- Adding a folder: the picker now opens on the starting folder (Clone's destination picker
  previously ignored it), lists folders only, accepts a typed path on Enter, disables its confirm
  button until a folder is open, and uses one vocabulary - "Add a folder", "Browse...",
  "Import a project". Files could not be chosen, so showing them was noise.
- Form submit buttons follow their forms: Clone, Start a member, Save the rule and Create the
  schedule are disabled until their required fields are filled. A disabled button is clearer than
  a live one that answers with a warning.
- Every component colour now uses the semantic names (`--color-primary`, `--color-base-100`,
  `--color-base-200`, `--color-base-300`, `--color-base-content`, `--color-info`,
  `--color-success`, `--color-warning`, `--color-error`). The palette values and the page
  background are unchanged; the old palette variables remain only as the source of those values.
  `ui_contract` now fails if a component rule reaches for them directly.
- A conversation can be renamed from its header with `chat.update`; the agent can call the same
  capability. Team rows still show the task name, not the conversation title.
- Usage adds an estimated spend column and total from the model catalogue, with unpriced
  conversations counted separately. It is labelled as an estimate; cache tiers are still not
  recorded per call.
- The one raw `<details>` in the review studio now uses the shared `ui-fold` treatment.

## What to do next, in order

1. **Commit the QA fixes**, then tag and release (authorised): version is already 0.25.0 in
   `Cargo.toml` and `crates/devd-desktop/tauri.conf.json` and the CHANGELOG has the entry.
   `git tag -a v0.25.0`, push main and the tag. Release:
   `scripts/release-windows.ps1 -SkipTests` (workspace and contract suites passed), then
   `gh release create v0.25.0 --notes-file ... <the new dist files> dist/SHA256SUMS.txt`
   (attach only the new version's files). Notes say **unsigned**.
   0.24.1 was never published on GitHub; leave it unpublished unless the owner asks.
2. **Update the installs**: `D:\devd` and `%LOCALAPPDATA%\Programs\devd`. Rename `devd.exe` and
   `devd-desktop.exe` to `*.before-0.25.0`, copy the new ones, compare hashes, and set the
   uninstall registry entry to 0.25.0. Do not run anything that opens the live home to "check"
   it; `--version` is safe.
3. **Launch video** (~60 s, dark, desktop landscape 1600x1000, real devd UI, no mention of Claude,
   zoom in/out like the owner's reference videos, no button left highlighted in any frame).
   Project: `D:\devd-promo` (Remotion 4.0.541 already set up; HyperFrames is an alternative the
   owner suggested). Story: tell the assistant a goal -> it brings in the team -> approve the plan
   once -> the team works (handoffs) -> the reviewer asks for changes -> fixed -> assistant reports
   back -> you verify (tests, the email in Mail). Real captures from the earlier run are in
   `public/real/` but show the OLD design: re-capture in the new design against the live devd
   0.25.0 after installing (stage scripts `capture/run1..6.mjs`; lib.mjs drives Edge over CDP 9333,
   `settle()` keeps the pointer off controls). Write a timing sheet, build, render
   `out/devd-launch.mp4`, extract a contact sheet and review every cut before handing it over.

## Design polish: semantic colour names (owner's request)

The owner wants colour named by role, DaisyUI-style. The names are defined at the end of
`crates/devd-server/ui/style.css`, mapped onto the graphite + blue tokens:

| Name | Variable | Use |
|---|---|---|
| primary / primary-content | `--color-primary`, `--color-primary-content` | the main brand colour (devd blue): the one action that matters, where you are |
| secondary / secondary-content | `--color-secondary`, `--color-secondary-content` | the optional second brand colour: quieter emphasis |
| accent / accent-content | `--color-accent`, `--color-accent-content` | the optional accent: rare highlights, live state |
| neutral / neutral-content | `--color-neutral`, `--color-neutral-content` | unsaturated dark chrome (rail, chips) |
| base-100 | `--color-base-100` | the page background |
| base-200 | `--color-base-200` | a darker/raised shade for elevation (cards, panels) |
| base-300 | `--color-base-300` | the next elevation (controls, hover) |
| base-content | `--color-base-content` | text on base colours |
| info / info-content | `--color-info`, `--color-info-content` | informative messages |
| success / success-content | `--color-success`, `--color-success-content` | success and safe states |
| warning / warning-content | `--color-warning`, `--color-warning-content` | warnings and caution |
| error / error-content | `--color-error`, `--color-error-content` | errors, danger, destructive actions |

**Done (0.25.0):** the palette the owner asked for is implemented - graphite surfaces with a
single devd blue - and it is what the app renders. Every component rule now uses the semantic
names; the old palette variables remain only as the source of their values, so nothing rendered
changed and the page background is untouched. `ui_contract::components_use_the_semantic_colour_names`
fails if a component reaches for `var(--accent)`, `var(--bg-surface)`, `var(--ok)` and the like
directly. The `-content` pairings are unchanged from the palette that shipped.

Still to check when the palette is next revised: every `-content` pairing at 4.5:1 contrast, and
turning the old palette variables into aliases of the semantic ones (the reverse of today's
mapping) once nothing else reads them.

## Still open (document, do not fake)

From the owner's large request, not built yet - each needs real design, not a stub:
- Composer: the auto-growing composer, attachments, dictation, stop/send and queued messages
  already exist on the Assistant screen; Home still has no dictation button.
  NOTE: devd never had auto-compaction, a "#" indicator or "Send now"; context is compacted
  deterministically against the model window (`devd-core::context`). A user toggle would need a
  policy decision about overflow, not a cosmetic switch.
- LaTeX and Mermaid rendering (needs vendored libraries embedded in the binary; must work offline).
- Interactive in-chat visualisations (an agent-authored, sandboxed component format).
- PR linking and a thread "settled" state. Renaming now exists for the person and the agent
  (`chat.update`); automatic title regeneration does not.
- Offline message queue with pending/sending/sent/failed states and de-duplicated resend.
  (The server's `chat_queue` is different: it holds messages sent while the agent is working.)
- Automations: webhook-triggered tasks with filtering/routing, GitHub PR monitoring and automated
  review feedback. Schedules (`schedule.*`), intake rules (`intake.*`) and deploy hooks
  (`/hooks/deploy/...`) exist; they are not a general webhook router.
- Cache-tier analytics and true per-call spend need the provider layer to record cache tokens and
  the price used at call time. Usage now shows an estimate from the catalogue, labelled as such.
- Performance: server memory in long conversations, idle CPU, catch-up payload size
  (`chat.get` re-sends tool results), cold probe latency (`health.status` 5-25 s; run probes in
  parallel), 90 write capabilities without dry run, every UI read is audited.
- Team commits nothing at the end of a run; no standing team monitors a live site.
- Promo re-capture in the new design; the launch video is explicitly deferred.
