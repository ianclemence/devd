# Handoff: finish devd 0.25.0, release it, then the launch video

Read all of this before touching anything. Then work through "What to do next" in order and
report plainly: what you finished, what you verified (with screenshots you looked at), what is left.
When everything is done the owner wants this file **deleted**, then the work committed, pushed,
released and installed.

## What devd is

A local development environment built for working with AI agents (Rust workspace, Windows first).
Every action is a capability in one registry (`crates/devd-core/src/registry.rs`, `crates/*/src/caps.rs`,
about 460 of them), planned, approved where needed and audited, from the window, the CLI (`devd`) or
an agent over MCP (`devd mcp`).

- Interface: `crates/devd-server/ui/{index.html,app.js,style.css}`, vanilla JS, embedded with
  `include_str!` (rebuild to see changes in a binary, or serve from disk: see "Tooling").
- Agents: `crates/devd-agent` - assistant `chat.rs`, team `team.rs`, skills `skills/*.md` (registered
  in `src/skills_data.rs`), team stats `team_stats.rs`, automations `caps_autonomy.rs`.
- Store and runs: `crates/devd-core` (SQLite, migrations in `sql/`, `memo.rs` probe cache, `runs.rs`
  completion policies, `store_automation.rs`).
- Tool catalogue: `crates/devd-stacks/defs/tools.yaml`; `docs/stacks.md` is generated - after editing
  tools.yaml run `DEVD_UPDATE_DOCS=1 cargo test -p devd-stacks --test engine`.
- Design system docs: `docs/design/{README.md,components.md,agent-ui.md,manifest.json}`; the
  `interface-contract` skill carries the agent rules; `design.manifest` serves the manifest.

## Owner's rules (follow exactly)

- **Keep this file updated as you go** (owner: "ALWAYS UPDATE THE HANDOFF"), not only at the end.
- **No component showcase page.** Components are applied in the screens; agents learn them from
  the prompt, the `interface-contract` skill and `design.manifest`.
- Show **design directions on real screens before rewriting a layout**; the owner picks.

- Design bar: the Claude Code desktop app - match or beat it, never copy. Dark only, graphite + one
  blue, colour where it means something (charts, diagrams, code), no slop, no generic dashboards, no
  unnecessary cards, every width (1600/1280/1024/800/390). **Always visualise**: take screenshots and
  look at them before claiming anything is done.
- **Never mention the reference component library by name anywhere** (code, docs, commits).
- Do not change the page background.
- Never run a dev build against the live home `D:\devd` (it migrates the database). Use a copy.
- Never touch `D:\devd\www\nairobiunwind` (real). The demo copy `nairobiunwind-demo` is ours to use;
  its database has copied real user data: invent people, never run its queue worker.
- Never print secrets. Releases are unsigned (no key on this machine): say so in the notes.
- Commit attribution: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Python edits on Windows: read and write files in **binary mode** (text mode writes CRLF and breaks
  `ui_contract`; the repo is LF). Check `git diff --stat` for whole-file churn.
- Authorised by the owner: commit, push, publish the release, update both installs.

## Build, test, tooling

```powershell
cd D:\devd\www\devd; . .\scripts\dev-env.ps1     # target C:\dt
cargo build -p devd-cli                           # C:\dt\debug\devd.exe
cargo test --workspace --no-fail-fast            # about 1,140 tests
cargo test -p devd-server --test ui_contract     # 74 interface rules
```

Test home: **`C:\dthome-026`** - a consistent read-only backup of live (Python `sqlite3` backup with
`mode=ro`), services emptied, schedules off, web ports 18080/18443, and the owner's DeepSeek key
copied to `secrets\providers\` so agents really run. Serve it on **port 18788**:

```powershell
$env:DEVD_HOME = "C:\dthome-026"; Start-Process C:\dt\debug\devd.exe -ArgumentList "serve","--port","18788" -WindowStyle Hidden
```

Stop it before rebuilding (it locks devd.exe):
`Get-CimInstance Win32_Process -Filter "Name='devd.exe'" | ? { $_.CommandLine -like '*--port 18788*' } | % { Stop-Process -Id $_.ProcessId -Force }`.
An older test server (`C:\dt\release\devd.exe serve --port 18787`, home `C:\dthome-025`) was started
by an earlier agent and may still run; stopping it was refused to the current agent, so ask the
owner. `C:\dthome-025\state.db` is inconsistent: do not use it.

Screenshot and end-to-end tooling in `D:\devd-promo\capture` (Node, playwright-core, Edge):
`audit.mjs` (every route at 5 widths; `UI_DIR=<ui dir>` serves the UI files from disk),
`check2.mjs`, `e2e-rich.mjs` (LaTeX, Mermaid, chart and code with the real model),
`e2e-features.mjs` (Home badge, PR link and settle, offline outbox, drafts, automations with a real
GitHub-shaped hook call; `ONLY=hooks` runs only the hook part), `livecheck.mjs`, `timing.mjs`,
`sheet.mjs` (contact sheets). Shots land in `D:\devd-promo\public\audit\...`.

## Done and verified end to end (committed in cfaa5f8 and earlier)

- Rail + context panel navigation, live sync everywhere, DOM patching (scroll position holds), role
  faces, concrete step lines, "Worked for" fold with tiles, project card with code/tests/docs diff
  badges, conversation search, the team gate (reviewer "changes needed" leads to fix and re-review),
  team stats, usage, semantic `--color-*` tokens on all components, design docs and manifest,
  plain-language capability names, callouts, motion.
- nginx: `server_names_hash_bucket_size 128`; error log is `logs/nginx.log` (what the Logs sheet
  reads); a refused `nginx -t` is appended there with a timestamp; mariadb and nginx logs fall back
  to the files they actually write. The owner's LIVE nginx.conf still lacks this until the new build
  is installed and `devd web render` runs.
- HyperFrames, Stageshow, agent-browser and agent-desktop are built-in skills; their tool entries and
  the "Recommended" section are gone.
- Approval dialog: "Approval needed", plain title, tier in words. Toasts use plain names.
- Inbox badge on Home: one count definition (`refreshHomeCounts` calls `loadInbox`), loaded at start.
- Conversation header menu: rename, link or unlink a pull request (chip shows owner/repo#N), settle
  or reopen, delete. Settled threads read quieter with a check; the PR number shows in the list.
- Automations (Team > Automations): presets "Review every pull request", "Check every push", "Your
  own event"; the address is shown once; "New address" rotates it; turning on or off keeps it. Hooks
  read GitHub's `X-GitHub-Event` header; the branch filter matches PR base/head or a push ref.
  Verified: a GitHub-shaped PR event started a team run with its goal filled in; `closed` was ignored.
- Offline outbox: sent offline shows "Waiting for devd", delivered exactly once on reconnect.
  Drafts are shared between Home and a new Assistant conversation and cleared when sent.
- Rich replies: KaTeX and Mermaid (vendored, offline), chart blocks (`bars` or `items`), built-in
  syntax highlighting (`highlightCode`), data palette `--viz-1..6`, coloured Mermaid theme, livelier
  Markdown.
- No false "not verified": goal words inside code no longer count, words match on word starts, and a
  turn with no tool call settles as an answer (a declined attempt still gets the honest notice).
- DeepSeek cache tokens are read and priced at 0.1x. Usage-limit resume shows local time; tested.
- Performance: `chat.get {brief:true}` trims tool output to head and tail (48% smaller over 12
  conversations); `chat.result` returns one step's full output when a step is opened. Idle server:
  about 1% of one core, 32 MB (debug build).

- Conversation list (done, checked at 1440/1280/1024/390 with real data): two lines per request
  (full-width title; then specialists' faces and count, Working / Waiting for you / Stopped /
  Settled, PR number, then the time); specialists fold under their request (open while the team
  works or one of them is open; the chevron toggles, kept in `AI.kidsOpen`); runs started by an
  automation or the CLI get their own "Team run" line; settled conversations sink in their group.
  Phone tab bar lost a stray "panel" tick (`rail-icon::after`).
- Reply visuals (owner: "like T3 Code's chart", "not just the charts"): `chart`, `metrics`, `steps`,
  `timeline` and `details` blocks are drawn on a near-black artifact surface (`.viz`, container
  queries for narrow widths) in a named palette (`--viz-red..pink`): chart = title, label + note,
  thick bars that grow in, the value in the colour of its bar with a small unit, a coloured
  takeaway and a source line; metrics = one strip of large coloured numbers with delta pills;
  steps = numbered nodes, green checks, the current one glowing; timeline = time, glowing node,
  event; details = coloured-edge fold. Code blocks: language name + colour dot, icon copy button,
  diffs in green/red rows; numeric table columns align right. The assistant prompt (`chat.rs`,
  "Showing data"), the `interface-contract` skill and `docs/design/manifest.json` teach the
  formats; verified with the real model (it used all of them unasked). **No component page**: the
  owner does not want one ("components are to be applied, not displayed").
- Header "Continue" no longer shows on a run that completed.
- `ui_contract`: the custom-property test now reads every definition on a line; the send test
  expects `{ quiet: true, rethrow: true }`. 74 pass; devd-agent lib 165 pass.

## What to do next, in order

1. **Home: the owner rejected the restructured Home ("THAT HOME SUCKS", "I don't want something
   generic, I want a perfect Home").** Uncommitted in app.js `renderHome` (updates shown as rows,
   a "Pick up where you left off" list, compact project rows, plain-word feed) and style.css
   (block "Home, in the order a person asks"). That is an interim state, NOT approved. Next step
   agreed with the rule "directions before CSS": three Home directions mocked as real screens in
   `D:\devd-promo\mock\home-*.html` (devd's own style.css + real test data) -> owner picks one ->
   implement only that, check 1600/1280/1024/800/390, then the other screens.
   **Owner picked B "Workbench" (2026-11-07; first said A, then "NO GO WITH B-WORKBENCH")**: work
   on the left (greeting, ask box, needs-you cards, your work with result numbers), a "This
   machine" column on the right (services card + project tiles with address, status and stack).
   Mock: `D:\devd-promo\public\audit\home-mocks\B-workbench-1440.png` (script `capture/home-mocks.mjs`).
   Code state (uncommitted, in progress): `renderHome` in app.js already has the needs-you cards
   (`homeNeed`), the work list (`homeWorkRow`, `runWords`, runs linked to their conversation by
   `thread_id`), `projectColor`, and a `home-pick` action from the A build; style.css ends with a
   "Home: a launchpad" block. Converting that to B: drop the picks and the status line, add the
   right-hand machine column (services card + project tiles + "All N projects"), result numbers
   on finished runs (only real numbers: tasks done/total, specialists, PR), then shoot every width.
   Directions were: A "Launchpad" (centred greeting + composer, projects as one-click "start here"
   picks, needs-you inline, recent work, machine as one quiet status line); B "Workbench" (work on
   the left, a live machine column of project tiles with their addresses and service lights on the
   right); C "Briefing" (a short written brief of what agents did since you were last here,
   grouped by project with result tiles, composer docked at the bottom like a chat with devd).
2. **Layout revamp of every other screen** (owner asked: rethink hierarchy, density and
   placement; no generic dashboards, no unnecessary cards, a distinct product identity). Screenshot
   each screen at 5 widths first, decide per screen, implement, re-shoot, compare.
3. **Use the component system in the screens** (owner: "use our own components to build the
   pages"). Do NOT add a showcase page.
4. **Language and copy review** of the whole app and its docs: clear, consistent, natural.
5. Full verification: `audit.mjs` at 5 widths (look at every shot), the e2e scripts, the workspace
   tests, and the browser pane.
6. Delete this file; commit; push; tag `v0.25.0` (already 0.25.0 in Cargo.toml and tauri.conf.json;
   extend the CHANGELOG entry); `scripts/release-windows.ps1 -SkipTests` after the suite passed;
   `gh release create v0.25.0 --notes-file <notes> <the new dist files> dist/SHA256SUMS.txt`
   (unsigned); update `D:\devd` and `%LOCALAPPDATA%\Programs\devd` (rename old exes to
   `*.before-0.25.0`, copy, compare hashes; only `--version` against the live home). Tell the owner;
   the launch video comes after (project in `D:\devd-promo`, Remotion; re-capture in the new design).

## Still open

- Interactive agent visualisations beyond the validated blocks (metrics, timeline, steps, details,
  chart, Mermaid). A richer sandboxed component format is a design task of its own.
- Spend is only as accurate as the model catalogue's prices; cache tiers are read for Anthropic,
  OpenAI, Gemini and DeepSeek.
- Every window read is audited (by design); polling is reduced, not removed.
