# Interface redesign plan

Source: the design review of the running app (0.19.2 build, 785 / 1440 / 375 px). It builds on
[`design.md`](design.md) ("the workbench"), which got the direction right: projects first, sites
first-class, truth before comfort. The review found the *execution* has drifted from it: the same
objects under different names and counts, tabs that do not apply, and states that lie.

The UI is `crates/devd-server/ui/app.js` (about 13k lines) and `style.css` (about 5.7k), served by
`devd-server` and checked by `crates/devd-server/tests/ui_contract.rs`. Every phase below is
shippable alone. Nothing here changes capabilities or the approval model.

## Principles

1. One noun, one name, one number. Every count is defined once and reused.
2. A tab, button or section appears only if it applies to this object.
3. Never show an empty state before the data has arrived.
4. Action weight follows consequence: one primary per page, danger apart.
5. Wide screens get tables; narrow screens get a single-line summary that expands.

## Phase 0: defects (no design decisions, do first)

| # | Defect | Fix |
| - | --- | --- |
| 0.1 | Project tabs (History, Branches) show "No history loaded" / "No branches" while fetching | Per-tab `loading` / `loaded` / `error` state; skeleton rows; empty copy only after `loaded` |
| 0.2 | Bare spinner on a blank page (Projects, Create, Certificates, Tools, Remote) | Shared skeleton component, page header always rendered |
| 0.3 | `/sites` and any unknown route fall back to Home silently | Real not-found page, with a link to the nearest match |
| 0.4 | Assistant "Conversations" strip overlaps the heading at about 785 px; Agents strips bleed past the gutter | One page-layout wrapper owning gutters and the rail-to-strip breakpoint |
| 0.5 | Capability detail renders below a 50-row list with no scroll or focus | Scroll into view on select; side panel above 1000 px |
| 0.6 | State-database message printed twice; banner says 1 problem, list says 3 | One problem count; owned ports (nginx on 80/443) are "Expected", not "Degraded" |
| 0.7 | Mid-word wraps (`api.deepseek.co/m`, `nairobiunwin/d\public`) | `overflow-wrap: anywhere` only on paths; keep hostnames whole |
| 0.8 | Preview iframe is a white box | Dark placeholder until the frame loads; error text when it is blocked |
| 0.9 | Activity fills with the app's own reads | Hide `read` entries from UI-originated polling by default, or stop auditing them |

## Phase 1: one vocabulary and one set of numbers

- Write a small glossary (Project, Site, Service, Runtime, Run) and use it for menu labels, page
  titles, routes and tab names. Fix the swaps: Agents → `/team` while its tab "Team" → `/agents`;
  "Notes" → page "Shared"; Environment → `/server`.
- A single selector layer computes: services running, needed, problems, sites that serve. Home,
  Environment, Services, Diagnostics and the status bar all read it. Add a contract test that the
  same fixture gives the same numbers everywhere.
- Status colour: red means broken, amber needs a look, grey means optional or absent. Missing
  optional tools (pnpm, Maven) become grey.
- A site always resolves to its project, and the Projects list resolves the site back. Remove
  "not registered in Projects yet" contradictions by registering on first sight, or by offering
  one clear "Add as project" action.

## Phase 2: information architecture

Top level, four destinations (the 1440 px top bar already fits this):

| Destination | Contains |
| --- | --- |
| Home | Assistant composer, attention items, project list |
| Projects | Project list; New project; Stacks and tasks becomes a column and a filter |
| Agents | Runs, Team, Schedules, Skills, MCP, Shared notes, Activity |
| Machine | Services, Runtimes, Databases, Tools, Remote, Models, Sites table, Hosts, Certificates, Diagnostics |

- Sites stay reachable from the top level as a cross-project table, but detail pages live inside
  the project: `/projects/<id>/site`.
- Hosts and Certificates collapse into columns and a detail drawer on the Sites table, with the
  existing pages kept under Machine for machine-level actions (Sync hosts, Trust authority).
- API and tools moves under Machine → Advanced.
- Cap depth at two levels. Remote loses its inner Servers / SSH keys tabs and uses sections.
- Keep the palette; add the new destinations and a `g` shortcut set.

## Phase 3: project and site pages

Project tabs: **Overview, Files, Git, Site, Agents**.

- Git absorbs Changes, History, Branches, Tags, Remotes, Stashes, Worktrees and Recover as a
  left rail inside one view. Stage / Unstage / Stash show only when there is something to act on.
- Agents absorbs Reviews and Agent memory.
- Site (shown only if the project has one) holds Overview, Environment, Logs, Redirects and Web
  server. Laravel, Cron, Queue and Deploy appear only for stacks that use them, and the Laravel tab
  never renders "This isn't a Laravel project".
- Overview states each fact once. "Serves at: No address yet" gets a "Give it a site" action. The
  four-fact summary stays.
- Remember the last tab per project type, not globally.
- Truncated branch names and remote URLs get a tooltip and a copy button.

## Phase 4: action hierarchy and safety

- Project header: Terminal and Open site as primary/secondary; Refresh quiet; Remove and Delete in
  an overflow menu ("Danger zone" section on Overview), Delete asking for the project name.
- Environment: Start needed as primary; Stop all moves behind a confirmation.
- Row actions on lists are visible on focus and touch, not hover only.
- Certificates: separate "Trust" from "Remove trust" with spacing and a quiet style for the latter.
- Inbox items show what, which agent and which path ("Declined: delete `D:\…`"), not `fs.delete`.

## Phase 5: lists and density

- Tables above about 900 px: Sites (existing), Projects (name, stack, branch, state, address),
  Services, Tools, Runtimes. Sortable headers replace the Sort pill group.
- Below that, one summary line per row with an expander; identical explanations (the reachability
  sentence) become a single page note, shown once.
- Projects KPI tiles reduced to what a person acts on: total, needs attention, without an address.
  Drop "Registered".
- New project becomes a catalogue: icon tile, name, one-line description, grouped, with a visible
  selected state; Import is the first action.

## Phase 6: polish

- Models: configured providers on top; "Add a provider" collapsed; remove endpoint jargon from
  card text; surface "Which model does which work" instead of collapsing it.
- Activity: human titles for every capability (`models.inventory` becomes "Read the model list");
  clear filter labels; group by day (exists), collapse repeated entries.
- Home: remove the redundant "All quiet" line when the rail already shows state; the "Good
  afternoon" line yields to attention items.
- Status bar: never clip text; show a short label with a tooltip for the rest.
- Skills: group by stage as the copy promises; MCP form gets an example row and an import button.

## Order, size and risk

| Phase | Size | Risk | Notes |
| --- | --- | --- | --- |
| 0 | Small | Low | Independent fixes; ship as one patch release |
| 1 | Medium | Medium | Needs the selector layer; add contract tests first |
| 2 | Medium | Medium | Route and label changes; keep redirects from old routes |
| 3 | Large | Medium | Main structural change; do project, then site |
| 4 | Small | Low | Mostly layout and confirmation |
| 5 | Medium | Low | Table component reused across pages |
| 6 | Small | Low | Copy and detail |

Risks: `app.js` is one large file, so extract page layout, skeleton and table components first and
change pages after; keep old routes redirecting so saved links and the desktop shell still work;
update `ui_contract.rs` and `docs/screens/*.png` as each phase lands.

## Verification

- Run the UI against a **copy** of the home (`DEVD_HOME` pointing at a copy), never the live one:
  even `doctor` migrates `state.db`.
- Screenshot every route at 375, 785 and 1440 px before and after each phase.
- Contract tests: identical counts across screens, no empty state before load, no tab shown for a
  stack that does not use it.
- Check the case from this review: an older build opening a newer `state.db` should show one clear
  read-only banner on every page, not only on Diagnostics.

## Status (first implementation pass)

Done and verified on a test server against a copy of the home (screenshots taken):

- 0.1 tabs show a loading state before data, not an empty one; 0.2 skeletons replace bare spinners
  (Projects, project tabs, Files, Diagnostics, page loading); 0.3 real 404 page with a nearest-match
  suggestion; 0.4 Assistant empty state no longer cut off under the strip, and the strip shares the
  page gutter; 0.5 capability detail scrolls into view and is sticky on wide screens; 0.6 one
  state-database message, and a port held by devd's own running web server is "Expected" (unit-tested).
- 1: Home and Environment count services the same way; Notes page is called Notes; the site page
  agrees with Projects about whether a project is known.
- 3: project tabs are Files / Git / Agents with a sub-row; Git buttons appear only when they apply;
  the Laravel tab is hidden for sites that do not serve PHP.
- 4: Remove and Delete moved into a "..." menu on the project header; Environment's Stop all is quiet,
  Start needed is the primary.
- 5/6: Projects tiles say what needs attention; provenance noise removed; the reachability sentence is
  stated once; Models folds the provider list; the "no model" state is an invitation, not an alarm;
  the status line truncates instead of cutting mid-word.
- Terminal: added to the main navigation; takes focus when shown; opening it no longer waits for a
  full refresh; echo is read on every keystroke (about 110 ms). Terminal input is no longer written to
  the audit trail (it stored typed characters, including anything typed at a password prompt).

Second pass (also verified with screenshots): navigation is Home, Projects, Terminal | Assistant,
Agents | Machine (Sites, Environment, Models, Activity, Diagnostics, API and tools); the Sites list
is two lines per row on narrow windows; the Preview tab explains a site with nothing to serve instead
of framing a white page; Diagnostics shows "Checking..." and no longer drops a slow check at 15 s;
Inbox says what was declined and who asked; Activity words the terminal and read capabilities.

Third pass (verified with screenshots): the swapped routes are renamed (`/agents/runs`, `/agents/team`)
and the old `/team` and `/agents` still open the right page and rewrite themselves; Skills fold by stage
(they were already grouped, eight stages) with the first open; the generic Diagnostics fallback sentences
are replaced, and a port that is still flagged says another program holds it. A port held by devd's own
running web server is marked expected (unit-tested); the stranger case was shown live with a stand-in
`nginx.exe`. Showing the owned case live needs a real nginx installed in an isolated home, which means a
download that has not been approved.

Known flaky test: `registry::tests::stopping_a_run_kills_the_program_a_handler_is_waiting_on` has a 25 s
timing bound and fails under heavy machine load; it passes alone and with `--test-threads=2`.
