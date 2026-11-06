# Handoff: finish the devd revamp, release it, make the launch video

You are continuing work another agent started with the owner of devd. Read this whole file before
touching anything. It says what devd is, what was done, what is half done, what is still to do, how
to build and test safely on this machine, and the rules the owner has set. Work through
"What to do next" in order. Report back plainly: what you finished, what you verified, what is left.

## What devd is

A local development environment built for working with AI agents (Rust workspace, Windows-first).
Every action is a capability in a registry (454 of them: `crates/devd-core/src/registry.rs`,
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

## Done and committed (commit 3dedaee on main, not yet pushed)

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
last change. The binary at `C:\dt\debug\devd.exe` is OLDER than this commit: rebuild before use.
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

## What to do next, in order

1. **Verify the built UI**: `cargo build -p devd-cli`, start the test server, run
   `node capture/check2.mjs` (from `D:\devd-promo`) and look at every shot in
   `public/audit/check2`. The Team record ("How the team is doing") and Models "Usage" sections have
   NOT been seen rendered yet (the old binary lacked `team.stats` / `models.usage`): check them with
   real data. Then run `UI_DIR=D:/devd/www/devd/crates/devd-server/ui node capture/audit.mjs
   http://127.0.0.1:18787 public/audit/after2` and review all 130 shots (5 widths) for regressions.
2. **API & Tools screen at all sizes** (owner asked): check `api-tools` at 1600/1280/1024/800/390;
   the registry table must not squeeze or scroll sideways on phones.
3. **Buttons review** (owner asked): every button's action works and its style matches the system
   (`.btn`, `.btn-mini`, `.btn-ghost`, `.btn-primary`, `.btn-icon`): one primary per view, icon
   buttons have `aria-label`, destructive actions are quiet icons or confirmed.
4. **Commit, push, release** (authorised): bump version in `Cargo.toml` AND
   `crates/devd-desktop/tauri.conf.json` (current 0.24.1 -> 0.25.0), add a CHANGELOG entry, commit,
   `git tag -a v0.25.0`, push main and the tag. Release: `scripts/release-windows.ps1 -SkipTests`
   (after the workspace suite passed), then `gh release create v0.25.0 --notes-file ... <the new
   dist files> dist/SHA256SUMS.txt` (attach only the new version's files). Notes say "unsigned".
   0.24.1 was never published on GitHub either; mention or publish it as the owner prefers.
5. **Update the installs**: `D:\devd` and `%LOCALAPPDATA%\Programs\devd`. Rename `devd.exe` and
   `devd-desktop.exe` to `*.before-0.25.0`, copy the new ones, compare hashes. Do not run anything
   that opens the live home to "check" it; `--version` is safe.
6. **Launch video** (~60 s, dark, desktop landscape 1600x1000, real devd UI, no mention of Claude,
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

To do: move every component rule from the older tokens (`--accent`, `--bg-surface`, `--ok`,
`--danger`...) to these names, area by area, keeping the rendered colours identical (compare
screenshots before and after); then make the older tokens aliases of the new names, not the other
way round. Check every `-content` pairing for at least 4.5:1 contrast. Add a ui_contract test
that new rules use only `--color-*` names for colour.

## Still open (document, do not fake)

From the owner's large request, not built yet - each needs real design, not a stub:
- Composer: compact auto-growing redesign, voice controls polish, unified drafts, send/interrupt.
  NOTE: devd never had auto-compaction or a "#" indicator (checked git history); building
  conversation auto-compaction + "Send now" to interrupt it is a feature of its own.
- LaTeX and Mermaid rendering (needs vendored libraries embedded in the binary; must work offline).
- Interactive in-chat visualisations (an agent-authored, sandboxed component format).
- Agent-driven thread renaming/title regeneration; PR linking and thread "settled" state.
- Offline message queue with pending/sending/sent/failed states and de-duplicated resend.
- Automations: webhook-triggered tasks with filtering/routing, GitHub PR monitoring and automated
  review feedback (schedules exist: `schedule.*`; deploy hooks exist: `/hooks/deploy/...`).
- Usage cost and cache-tier analytics need per-call price/cache data recorded at the provider layer.
- Performance: server memory in long conversations, idle CPU, catch-up payload size
  (`chat.get` re-sends tool results), cold probe latency (`health.status` 5-25 s; run probes in
  parallel), 90 write capabilities without dry run, every UI read is audited.
- Team commits nothing at the end of a run; no standing team monitors a live site.
- Inbox/Projects information architecture review; promo re-capture in the new design.
