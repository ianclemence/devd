# Handoff: finishing the native app's screens

Branch `gpui-native`. Written 2026-11-12 for the next agent. Read this top to bottom before touching code.
Owner's standing demand, in their words: *finished, nothing half-baked, no slop, check every screen, test every
action, review every status for grammar and language.* They are angry about missing screens. Do not report "done"
until every item under "Must be tested" has been verified in the real window, not just compiled.

## 1. Rules that apply to everything

- Never name third-party apps in any text. No drag-and-drop.
- Do not commit, push, tag or publish unless asked. Release `v0.28.0` is an **unpublished draft**; CI for the last tag
  (`eff7fcb`) finished green on Windows, macOS (arm64, x86_64) and Linux. **Do not publish it** until the screens below
  are finished. After that: commit, re-point/re-tag, push, wait for CI, the maintainer signs `SHA256SUMS.txt`
  (`DEVD_RELEASE_KEY_FILE` only if already set; never search for or read key files), then publish.
- Never kill processes by name except the debug `devd-app` the screenshot scripts launch (nothing else is running as
  that name at the moment, check with `Get-Process devd-app` first: the user's installed app has the same name).
- Always verify UI by driving the real window and reading screenshots.
- Git Bash rewrites arguments starting with `/` (turns `/skills` into a Windows path). Run the screenshot scripts from
  the **PowerShell tool**, not Bash.

## 2. How to build, run and look

- Build: `cargo build -p devd-app` (output: `target/debug/devd-app.exe`). Tests: `cargo test -p devd-app -p devd-app-core`
  (41 + 112 passed at last run). Clippy: `cargo clippy -p devd-app -p devd-app-core --all-targets`
  (a run was started in the background at the end of this session; its output is in
  `C:\Users\User\AppData\Local\Temp\claude\415f63c55b6fe1b8\97e7c71b-5245-413e-813f-c138786ad823\tasks\baajesl96.output`;
  re-run it, the result was not read). `devd-tui` has a pre-existing `large_enum_variant` warning that is not ours.
- Real data lives in `D:\devd` (set `DEVD_HOME=D:\devd`). It contains the owner's real sites and a database; **do not
  run destructive actions against it** (remove database, remove runtime, delete mail). Use a scratch `DEVD_HOME`
  for destructive tests (create one under the scratchpad; the window starts its own devd against it).
- Screenshot tools (scratchpad, not in repo):
  `C:\Users\User\AppData\Local\Temp\claude\415f63c55b6fe1b8\97e7c71b-5245-413e-813f-c138786ad823\scratchpad\`
  - `tour.ps1 -Route "/runtimes" -Name pg-runtimes -DevdHome "D:\devd" -Wait 6 [-ClickX x -ClickY y] [-Down n]`
    launches `target\debug\devd-app.exe <route>`, optionally clicks at window-relative pixels, screenshots to
    `<Name>.png`, then stops it. A second click can follow with `-ClickX2/-ClickY2`, a third (after `-Settle`) with
    `-ClickX3/-ClickY3`. The window is 1376x899.
  - `pagesall.ps1` shoots all 18 pages to `pg-<id>.png`.
  - Routes are paths: `/runtimes`, `/databases`, `/tools`, `/remote`, `/hosts`, `/certificates`, `/mail`,
    `/schedules`, `/automations`, `/skills`, `/mcp`, `/notes`, `/new-project`, `/stacks`, `/diagnostics`,
    `/capabilities`, `/connect`, `/flags`.
- Inspect real reply shapes before reading fields: `"%LOCALAPPDATA%\Programs\devd\devd.exe" --json call <cap> --arg k=v`
  with `DEVD_HOME=D:\devd`; `devd capabilities [prefix]` lists capabilities. **The installed devd.exe may be older than the
  source**; a capability named in `crates/devd-core/src/registry.rs` can still be absent at runtime (see `stack.list`).

## 3. What was done

Earlier in this branch (all committed, `gpui-native`, last commit `eff7fcb`): end-of-turn gate, hash-chained audit
(+ streaming verifier, tolerant of rows from older devd), reviewer checklist skill, `devd://` links, `devd run --task`,
packs, MCP lifecycle hooks, activity-feed rules, typed feature flags, provider details, audience-bound agents;
installers/CI for Windows (NSIS), macOS (.dmg), Linux (.deb/.tar.gz); logo/icon cleanup; chat "assistant is working"
indicator and cross-process conversation polling; the live Tallybook test with DeepSeek (see section 8).

**This session, uncommitted** (the working tree is dirty; `git status` shows these):

| File | What |
|---|---|
| `crates/devd-app-core/src/pages.rs` (new) | The table of 18 pages in 5 areas, `find`, `area_of`, `tabs`, `is_current`, tests. |
| `crates/devd-app-core/src/nav.rs` | `Route::Page(String)`, `on_rail()` (a page lights its area's rail icon), `parse` falls through to pages. |
| `crates/devd-app-core/src/{lib,links}.rs` | Module export; `devd://` links know pages. |
| `crates/devd-app/src/screens/modal.rs` (new) | Dialog system: `Modal::{Confirm, Form, Detail}`, `FormSpec`/`Field` (text, optional, number), Escape/Enter bindings, focus return, scrim click, tests for `build_args`. |
| `crates/devd-app/src/screens/pages.rs` (new, ~1150 lines) | The 18 pages, `Act`/`Ask`, `Shell::run_act`, `Row`, `rows_card`, `list_or`, `tab_strip`, `frame`. |
| `crates/devd-app/src/shell.rs` | CloseModal/AcceptModal actions, `modal` field, overlay, `Route::Page` dispatch, every page in the command menu (Ctrl+K), `focus_handle()`. |
| `crates/devd-app/src/backend.rs` | `Connection::get`; `api.capabilities` special case (GET `/api/capabilities`). |
| `crates/devd-app/src/screens/{lists,activity}.rs` | Tab strip added to Projects, Sites, Server, Team and Settings screens. |
| `crates/devd-app/src/theme.rs` | Stronger hover/active tints (dark `#ffffff12`/`1c`, light `#00000010`/`1a`) — the owner said hover was too faint. |

Design contract the pages follow: head (title + one-line detail + page actions on the right), the area's tab strip,
then lists drawn by one function (52px rows: state dot, title + optional mono tag, faint detail line, state in words in
the state's colour, side text, action buttons), dialogs for anything that needs asking. Removals confirm first
(`Shell::confirm`), creations use `open_form(FormSpec)`. Every button runs a capability through the normal path, so
approvals still work.

Verified in the real window this session: all 18 pages rendered (before the fixes below); runtimes, databases,
certificates, hosts, mail, schedules, diagnostics looked correct with real data; Remove-runtime confirmation dialog and
Install-a-runtime form dialog look right; the skill detail dialog loads and renders markdown; row hover is visible;
titles that were clipped (`PHP 8.5....`) are fixed for the page lists.

## 4. Known defects (owner reported "many errors", especially in the projects pages, and a missing icon)

**UPDATE, later the same day: items 1, 2 and 3 below are now fixed in the working tree** (still uncommitted) and
verified in screenshots. New project reads `stacks.list` (plural; there is no `stack.list`), shows 23 stacks with a
"Start one" form each. Stacks and tasks lists existing projects; imported ones show their stack, package manager and
tasks (`title`, `command`, run via `project.run {path, task, background?}`, `project.run_destructive` confirms first),
others offer "Set up in devd" (`project.import {path}`, argument name unverified). Icons `clock` and `wrench` were
added. Clippy is clean for `devd-app` and `devd-app-core`. **Still unverified:** actually submitting the New project
form (a real scaffold), running a task, and `project.import`. Fix `InitRequest`'s doc comment that still says
`stack.list`. Items 4 and 5 and everything in sections 5–6 remain open.

Original notes:

1. **New project is broken.** It calls `stack.list`, which the running devd does not expose
   (`not found: no capability 'stack.list'`). Find the real source of the stack catalogue (`crates/devd-stacks`,
   `crates/devd-cli/src/devcat.rs`, `crates/devd-stacks/src/caps.rs`; `devd capabilities project` shows only
   `project.init`), register `stack.list` if it is genuinely missing from the capability table (add it with a test,
   and check `registry.rs:68` which already names it), or read the catalogue another way. Then verify the form:
   `project.init` takes `{stack, name, destination?, git, install}` (check `crates/devd-stacks/src/init.rs`
   `InitRequest` for exact field names; the form currently sends `stack`, `name`, `destination`, `git`, `install`).
   `project.init` is tier `exec`: confirm it asks for approval when it should. Test a real scaffold in a scratch
   folder, including a failure (bad name) and what the toast says.
2. **Stacks and tasks is wrong.**
   - Task buttons all read "task": tasks are objects with `id`, `title`, `kind`, `risk`, `long_running`, `command`,
     `available`, `unavailable_reason` (not `name`/`task`). Show `title`, tooltip/secondary line with `command`; disable
     (with the reason) when `available` is false; mark `long_running` and `risk` != safe visibly; run with
     `project.run {path, id}` (check the exact arg names in `devd capabilities`/the schema; `project.run_destructive`
     needs approval and must say so).
   - Most projects say "no stack recognised" and "Looking for tasks…" forever. `project.detect` returned
     `{stacks:[{id,name,...}], primary, ambiguous}` and the code reads `primary` as a string; check it against real
     replies for each of the 12 projects (some are worktree folders and genuinely have no stack; say "No stack
     recognised" once, not a loading state). The per-project `read_with` queries are fired for up to 12 projects at
     once on every draw; rate-limit, or only load the project the user expands.
   - Names are still clipped (`chitchatt…`, `dev…`, `kindleb…`) because `ui::Clip::new` inside a flex row without a
     definite width collapses. Use plain nowrap/ellipsis text for headings, or give the container a width.
   - Grammar bug: "What 12 projects is built with" — rewrite ("The stack of each of your 12 projects, and the tasks it
     can run.", handle 1).
   - Redesign as a list that follows the same row pattern as other pages, with tasks revealed per project, rather than 12
     tall cards of buttons.
3. **Missing icon in an empty state.** The icon table (`crates/devd-app/src/assets.rs`) has only: home inbox assistant
   runs team folder globe machine models activity settings search plus check x chevron-right chevron-down arrow-up
   arrow-left arrow-right external terminal branch play stop refresh alert info copy file diff command lock database
   bolt more sun sidebar. `pages.rs` asks for **`clock`** (Schedules empty state, blank tile seen in the screenshot) and
   **`wrench`** (Tools empty state). Add proper SVGs for both (match the existing line style: 24px viewBox, 1.5–1.75px
   stroke, `currentColor`), then audit every `icon("…")`, `.icon("…")` and `ui::empty("…")` literal in `pages.rs`,
   `modal.rs` and everything else against that table, and add a unit test that fails if any icon name used in the
   crate is not in the table (grep the sources in the test, or centralise names in constants).
4. **Review every page for errors the owner may have seen.** After 1–3, re-shoot all 18 pages and the five area
   screens and look for: clipped text, blank states, wrong fields (`-`/`–` placeholders), stuck loading
   skeletons, raw JSON, status words that are jargon.
5. Smaller things noticed: cron job row says "next now" for a future time (the `ago` helper is for past times; add a
   "in 3m" form); the skill rows' blank description issue was resolved by reverting to `ui::Clip::new`, verify it
   rendered; the Automations page tells users to run `devd call automation.set` — either build the create form or make
   the message friendlier; the "Notes" page (memory proposals) uses guessed field names (`title`/`summary`/`text`,
   `memory.apply`), check against `memory.*` capabilities and real data; Connect agents uses `agent_client.*` field
   names (`clients[].found/added/current/config/problem`) that were guessed, verify with `devd --json call
   agent_client.list`; Flags toggles send `"on"`/`"off"`, verify `flags.set` arguments; Remote uses `share.start`
   `{name, minutes}`, verify; Databases `db.users`, `db.dump`, `backup.*` argument names, verify; Hosts publishes via
   `hosts.sync`, verify it asks for elevation on Windows and what the toast says.

## 5. What still has to be built

- **Status wording review across the whole app** (explicit owner request): grammar, tone, sentence case, no jargon,
  no developer words ("capability", "mutation"), consistent tense. Sources: `crates/devd-app-core/src/status.rs`
  (`status::confirmation` success toasts: **extend it with every new action** — runtime.use/install/remove, db.*,
  backup.*, tls.*, schedule.*, cron.*, automation.*, skills.*, mcp.*, share.*, hosts.sync, mail.*, memory.apply,
  agent_client.*, flags.set, autostart.*, update.apply, project.init/run, toolchain.*; today most fall back to a
  generic line), every `page_head` subtitle, empty states, errors (`devd could not answer` is fine; raw server
  messages like `not found: no capability` must be translated), button labels, dialog text.
- **Themes task (queued, not started):** three themes total (the current one plus two new), each with its own dither /
  shader / grain background, consistent with the app's design, themed on the biblical David and his capabilities
  (shepherd, harp, sling, crown, psalms…), applied to **all** screens (today only Home has the dither corner: see
  `crates/devd-app/src/ui/dither.rs`, `assets/devd-logo*.png`). Look at paper.design shaders for reference. Add a
  theme switcher in Settings and persist it; add themed tests (every colour token present in every theme).
- Detail dialogs exist for mail and skills only; check the old screens for anything else the owner would expect in a
  dialog (e.g. a site's settings, run details).
- Keyboard: Escape/Enter bind to the dialog; verify focus returns correctly and that typing in a form never triggers
  global shortcuts.
- Re-run the live "everything works" check of the window after all fixes, then commit.

## 6. Must be tested (checklist; tick each by driving the real window)

For each of the 18 pages: opens from its tab, from the rail area, from Ctrl+K, and from a `devd://` link; shows a
loading state, an empty state with an icon, an error state; every row action and header button.

- Server: Runtimes (use, install form incl. bad input, remove confirm), Databases (create, back up, remove confirm,
  schedule backups form, run backup), Tools (install/remove confirm; these call the system package manager), Remote
  (share form, stop sharing).
- Sites: Hosts (publish with pending changes; elevation prompt), Certificates (issue form, issue again, trust/untrust
  confirm), Mail (open message, delete, delete all confirm, empty state).
- Team: Schedules (create team work, run now, pause/resume, delete; add cron job, run, pause, remove), Automations,
  Skills (open, install from GitHub form, delete own skill), MCP servers (add form, test, turn on/off, remove), Notes.
- Projects: New project (each stack's form), Stacks and tasks.
- Settings: Diagnostics (update, autostart toggle), API and tools (grouping, tiers), Connect agents, Flags.
- Dialogs: Escape closes; Enter confirms; clicking outside closes; a failed action shows its error in the dialog or a
  toast, never silently; approval-gated actions surface the approval overlay.
- Every area screen (Projects, Sites, Team, Server, Settings) shows the tab strip and highlights the right tab; the rail
  highlights the area when on a page; Back/Forward navigates between pages.
- Narrow and wide window sizes; light theme (`Settings`); long names (use the `break-ui` skill idea: long
  hostnames, emoji subjects, 100+ rows — Skills has 85 and Hosts/Certificates ~12, scrolling must work).
- Hover: rows, tabs, buttons, palette entries, rail items all respond; no hover on non-interactive text.
- `cargo test` for the workspace, `cargo clippy` (see above), then a CI run on the tag.

## 7. Where the code is, and gotchas

- gpui 0.2.2, toolchain `stable-x86_64-pc-windows-gnu`, edition 2024. In edition 2024, `impl IntoElement` return types
  capture the `&mut Context` borrow; helpers that take `cx` and return elements return `AnyElement` or `Div`, and
  calls like `section("X", list_or(.., cx), cx)` must be split into `let`s (two borrows of `cx`).
- `ui::Clip` collapses inside a flex row with no definite width; for titles use
  `div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()`. For paths use `Clip::tail`.
- `InteractiveText` click race (mouse-up seen on the next frame): handled in `ui/links.rs`.
- Data flow: `devd(cx).sync.watch(&QueryKey)` returns a `Snapshot` (`data`, `error`, `is_first_load()`, `list(path)`);
  mutations are `Mutation::new(cap, args).invalidating(domain)`. `Shell::run_act` maps a capability's first segment to an
  invalidation domain through a hard-coded match; extend it when adding actions whose domain differs (unknown domains
  invalidate `"site"`, which is probably wrong for `stack`, `memory`, etc.: check `crates/devd-app-core/src/sync.rs` for
  `related()`).
- Approvals: `NeedsApproval` replies go to `Devd.decisions` and the overlay (`shell.rs`); human-only capabilities are
  refused for agents, not for the window.
- Icon files: `crates/devd-app/src/assets.rs` (embedded SVG strings).
- Old web UI is the reference for what screens existed (areas and tabs); the owner believes they were lost in the
  redesign. If you find any further missing screen, add it to `PAGES` in `devd-app-core/src/pages.rs` (a test fails
  if a page has no tab).

## 8. The Tallybook live test (to be reported to the owner, not yet fully reported)

The assistant (DeepSeek, capped time) was asked to build a PHP + MariaDB expense-splitting app in `D:\devd\www\welcome`,
rename the folder to `tallybook`, and verify the site. Findings: it built the app and a database exists
(`tallybook…` database and user); it was **refused the folder rename** because the project scope confines it to its
folder (consider a `project.rename` capability, human-approved); the run was marked *completed* although 3 todos were
blocked (the end-of-turn gate should treat blocked todos as not done — check `crates/devd-agent/src/gate.rs`); no
approvals were needed; the audit chain verified, with a false positive on rows written by an older devd fixed by
counting them as unchained; devd:// links resolved; the assistant wrongly claimed devd rewrote a `..` path (the model
had done it; verified in `chat_events`). Artifacts: scratchpad `tallybook-task.json`, `tally.out.txt`.

## 8b. Test findings after the project fixes (scratch home, real window)

Verified: New project form opens with the cursor in the first field, typing works, Enter submits, a real Go project was
scaffolded (go.mod, main.go, main_test.go, .git, .gitignore); submitting a name that already exists shows a red error
toast ("invalid input: the request is not valid: <path>\GOTEST already exists and is not empty; choose another name or
destination (name)"). `devd call project.init` via the CLI registers the project (`project.list` then shows it with
`source: "init"`, stack `go`).

Open problems found:

1. **No feedback while a scaffold runs.** After "Create project" the dialog closes and nothing says it is running or that
   it finished (no toast at 1 s, 7 s). Scaffolds take 10–60 s. Needs: a "Creating gotest…" running indicator (toast or a
   row in a running-work area; `project.init` emits `stack.progress` events and accepts a `run_id`, `stacks.runs` /
   `stacks.log` give history), then a success toast naming the project and a button to open it (go to
   `/stacks`), or the failure reason. Also `status::confirmation` has no wording for `project.init`.
2. **A project created through the window may not be registered** if the window is closed before the run's checks
   finish (the first run, killed at ~18 s, left a complete folder that `project.list` did not show; the CLI run that
   finished did register). Decide whether closing the window should cancel, or the run should keep going in the
   daemon, and make the New project page show in-flight and recently finished runs. **Confirmed:** a second window run (`UITEST`) left
   open for ~55 s did register (`project.list` showed `gotwo` and `UITEST`), so the loss only happens when the window is
   closed early. The last screenshot of that run (47 s) was not produced; re-take it to see what, if anything, the
   window showed when the scaffold finished.
3. Error text is raw (full path, parenthesised field name). Translate: "A project called GOTEST already exists in
   <folder>. Choose another name or folder."
4. Name handling: typing in the form produced capital letters because Caps Lock was on during automation; check that
   Go/Node names that are lower-case-only are validated with a clear message, not a server error.
5. Stacks and tasks subtitle for zero projects says "your 0 projects"; handle 0 ("You have no projects yet.").
6. The Stacks page issues one `project.tasks` query per imported project each draw; check it is cached, not refetched
   every frame (watch `devd` request log / CPU while the page is open).
7. Typing very fast (several characters in one frame, as `SendKeys` does) dropped all but the first character in the
   dialog's text field. Real typing is slower, but test paste (Ctrl+V of a long string) and fast typing in
   `ui/input.rs`; characters must never be lost.
8. Not yet exercised: running a task (Run), a destructive task confirmation, `project.import` ("Set up in devd"),
   scaffolding a stack that needs network (Express/React), a stack whose tools are missing (e.g. Python without uv),
   cancelling a scaffold.

## 8c. How to test: commands and recipes

Setup (PowerShell):

```powershell
cd D:\devd\www\devd
cargo build -p devd-app
cargo test -p devd-app -p devd-app-core
cargo clippy -p devd-app -p devd-app-core --all-targets      # must print no warnings
cargo test --workspace                                       # before any commit
$s = "C:\Users\User\AppData\Local\Temp\claude\415f63c55b6fe1b8\97e7c71b-5245-413e-813f-c138786ad823\scratchpad"
```

Scratch data home (safe for destructive tests; the window starts its own devd against it):

```powershell
$h = "$s\scratchhome"; New-Item -ItemType Directory -Force "$h\www" | Out-Null
$env:DEVD_HOME = $h
D:\devd\www\devd\target\debug\devd.exe --json call project.list
```

Drive the real window (scripts in the scratchpad; screenshots land next to them, read them with the Read tool):

```powershell
# one page, optional click at window-relative pixels
& "$s\tour.ps1" -Route "/runtimes" -Name pg-runtimes -DevdHome "D:\devd" -Wait 8 -ClickX 1125 -ClickY 388
# every page
& "$s\pagesall.ps1"
# a scripted flow: click:x:y ; type:<one char per step, SendKeys is too fast for more> ; key:{ENTER} ; wait:secs ; shot:name
$chars = "g;o;t;e;s;t" -split ";" | % { "type:$_" }
& "$s\flow.ps1" -Route "/new-project" -Name f -DevdHome $h -Wait 10 -Steps ("click:1112:658;" + ($chars -join ";") + ";key:{ENTER};wait:30;shot:done")
```

Useful coordinates in the 1376x899 window: tab strip y≈192; first page action button top right (x≈1100–1170, y≈145);
list rows start y≈280 and are 54 px apart; action buttons sit at the right edge (x≈1125). Dialog buttons: Cancel/OK at
y≈510–583 depending on the dialog. Take a screenshot first, then click.

Check what a capability really returns before reading fields:

```powershell
$env:DEVD_HOME = "D:\devd"
$d = "D:\devd\www\devd\target\debug\devd.exe"
& $d capabilities            # all; add a prefix to filter, e.g. `capabilities project`
& $d --json call stacks.list
& $d --json call project.tasks --arg path=D:\devd\www\devd
& $d --json call project.init --plan --arg stack=go --arg name=x   # --plan changes nothing
```

Per-feature recipes (all in the scratch home unless read-only):

- **New project:** Go (no network) → expect folder with go.mod/main.go/.git and the project in `project.list`; same name
  again → friendly error; empty name → form refuses with "Project name is required" (check wording); Express/React need
  network and npm: expect progress, then success or a readable failure.
- **Stacks and tasks:** after a project exists, open the page → tasks listed; Run `build` → toast + output somewhere
  (decide where task output is shown); a long-running task (`run`) → "keeps running" and a way to stop it; a
  destructive task → confirmation dialog first; project without a stack → "Set up in devd" → card gains stack and tasks.
- **Databases** (needs MariaDB in the scratch home; may not start there): create `scratch_db`, back up, list users,
  remove with confirm; schedule backups with `0 3 * * *`; invalid cron → error text readable.
- **Runtimes:** use a different PHP version; install a runtime (large download: test with a small one or cancel);
  remove managed runtime (confirm names the version; sites using it).
- **Certificates / Hosts:** issue for `scratch.test`; trust/untrust (Windows prompts; the user must approve, do not
  automate system dialogs); hosts publish shows the elevation prompt; the pending count in the subtitle updates.
- **Mail:** send a test mail to the catcher (`devd call mail.*` or PHP `mail()` from a site), open it, delete, delete all.
- **Schedules:** schedule team work (cron `*/5 * * * *`), run now, pause/resume, delete; cron job add/run/remove.
- **Skills / MCP / Connect agents / Flags:** install skills from a small public repo; add MCP server `echo` with a
  harmless command, test, enable, remove; connect/disconnect an agent client only if its config file can be restored
  (back it up first); toggle a flag and confirm it persists after restart; env override shows "set in the environment".
- **Capabilities page:** 85+ rows group correctly; filter box (not built yet: add one, the plan called for it).
- **Approvals:** run an exec/admin action as the window and confirm the overlay appears for those that need it.
- **Hover/focus/keyboard:** Tab through dialogs; Escape/Enter; Ctrl+K lists every page; Back/Forward.
- **Visual regression:** re-shoot all 18 pages plus Home, Inbox, Assistant, Team, Projects, Sites, Server, Settings,
  Activity in dark and light, at 1376x899 and a narrow width (resize the window), and compare against earlier shots.
- **Release path:** after everything is green: `git add -A`, commit, `git tag -f v0.28.0`, push, watch the Release
  workflow, verify the Windows installer installs to `%LOCALAPPDATA%\Programs\devd` and creates Start menu shortcuts.

Things to grep or assert in tests so regressions are caught:

- every `icon("…")` / `.icon("…")` / `ui::empty("…")` name exists in `assets.rs` (add the test);
- every `Page` in `PAGES` renders without panicking against empty, loading and error snapshots (write snapshot-driven
  tests for the pure helpers; the gpui parts need the screenshots above);
- every capability string used in `pages.rs` exists in the registry (add a test that loads the capability list and
  checks `Act::new("…", "cap.id"`/`FormSpec{capability}`/`QueryKey::new("cap.id"` literals: this would have caught
  `stack.list` and the guessed field names);
- every `Act`/`FormSpec` argument key exists in that capability's JSON schema (`GET /api/capabilities` has schemas).

## 9. Order of work

1. Fix section 4 (stack.list, Stacks page, icons) and re-shoot everything. 2. Extend `status::confirmation` and do the
language pass. 3. Test every action per section 6 (scratch `DEVD_HOME` for destructive ones). 4. Workspace tests +
clippy. 5. Commit (message ends with the attribution line the harness gives), re-tag, push, CI, signing, publish,
install the Windows build locally. 6. Themes task. 7. Report to the owner: what was restored, what was fixed, what the
live test found.
