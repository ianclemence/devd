# The devd interface

This is the design reference for the application window: the direction it takes,
the tokens it is built from and the rules every screen follows. Capabilities and
their authority are described in [`domain.md`](domain.md) and
[`security.md`](security.md); this document is about how a person sees and uses
them.

## What the interface has to answer

devd has four nouns, and the window is the only place a person can see how they
relate:

| Noun | The person's question |
| --- | --- |
| Project | What code am I building? |
| Site | At what address is it reachable, and what is actually serving it? |
| Runtime | Which interpreter will run it? |
| Service | What infrastructure has to be running for it to work? |

Truth before comfort: a site with no entry point says so, PHP is shown only
where PHP is effective, a pin that does nothing is named as unused, stopped is a
state rather than a warning, and "running" is shown only after a service answers.

## The direction: the workbench

The interface is a **workbench** with the environment always in view.

* **Projects are the primary axis.** Home, Projects and a site's detail all lead
  back to the folder, its runtime and its address.
* **Sites are a first-class destination.** A site answers "how do I reach this?",
  so it is not a tab inside a server console.
* **The machine is always visible.** Home carries a compact environment ledger,
  and the rail keeps the machine's screens one click away.
* **Restraint in the surface.** Graphite surfaces, one blue signal, hairlines
  instead of shadows, type instead of boxes, and colour only for state.

The emotional target is *quiet confidence*: a well-made tool that already knows
the state of the machine, not a dashboard asking to be admired.

## Information architecture

```text
Home          environment ledger, attention, projects, sites, recent work
Inbox         approvals, questions and messages that need a decision
Projects      all projects, new project; project detail: files, changes,
              history, branches, reviews, memory, terminal
Sites         all sites, hosts, certificates, mail; site detail
Assistant     conversations and the composer
Runs          agent runs; Team: roster; Schedules, Automations, Skills, Notes, MCP
Models        providers, models, role assignment, usage
Machine       Environment (services, runtimes, databases, tools, remote),
              API and tools, Activity, Diagnostics
Create        a new project from a stack or template
Develop       guided setup for an existing project
```

The rail groups destinations by the question they answer: **what am I building**
(Work), **who does the thinking** (Assistant, Runs, Team), and **what is the
machine behind it** (Machine). Sites sits beside Projects because it is the
access layer of the work. A terminal is a tab of the project whose folder it runs
in, not a destination of its own.

Every tab strip is the same control whether it switches a section, a project
group, a site's page, a mail view or Form/JSON: a row of quiet pills where the
current one is raised. Filters and tabs are both a choice, so they do not get two
different shapes. A terminal with one session has no session tab; the panel head
already names the folder and the shell.

Every table shares one grid: the same uppercase header, cell padding and divider,
tabular numbers for counts and times, and actions in a named right-hand column.
That holds in a panel, a run's evidence, the task list, the models screen and a
table written in Markdown. A row laid out on one line does not inherit the top
margin used to space stacked blocks.

Motion follows frequency. Tabs and chips press in with a short scale; hover
styles run only on devices that can hover; popovers grow from their trigger;
dialogs and drawers move because they are occasional. Every transition names the
property it changes, and reduced-motion users keep the fade that explains a
change but lose the movement.

## Tokens

Tokens are defined in one authoritative block near the end of
`crates/devd-server/ui/style.css`. Components use the semantic names; the raw
palette is only the source of their values. A raw hex value, a pixel radius or a
duration inside a component rule is a defect.

### Colour

`primary` / `primary-content`, `secondary` / `secondary-content`, `accent` /
`accent-content`, `neutral` / `neutral-content`, `base-100`, `base-200`,
`base-300`, `base-content`, `info` / `info-content`, `success` /
`success-content`, `warning` / `warning-content`, `error` / `error-content`.

All twenty variables exist. Renderings that predate the system map onto them;
the contract test `components_use_the_semantic_colour_names` fails if a
component rule reaches for the old palette variables directly.

Surfaces are true neutrals. One blue marks the primary action, the current
location and live work. Cyan is the second signal (live execution, running
services) and violet appears once at most in a composition, never as a second
action colour. Status colour appears only beside a word or an icon: success,
warning, danger, information and neutral are never colour alone.

### Surface and atmosphere

The canvas is graphite rather than black: `--bg #0b0c0e`, with raised surfaces
at `--bg-surface #121417` and `--bg-raised #181b1f`. Depth comes from tonal
separation and a hairline, not from a shadow around every panel; the only
floating surfaces are menus, drawers, dialogs and toasts.

The atmosphere is painted once on `body` with two radial gradients (blue from
the top left, cyan from the top right, a trace of violet at the bottom) and a
160px SVG grain tile. It is static, has no animation loop, no canvas and no
WebGL, and is removed by the grain/gradient tokens. It shows through the
workspace canvas and is deliberately covered by the reading and terminal
surfaces. Rich atmosphere belongs to Home and empty states, never behind code,
logs or a conversation.

### Shape, size and depth

| Token | Value | Use |
| --- | --- | --- |
| `--radius-selector` | 6px | checkbox, radio, toggle, small badge |
| `--radius-field` | 8px | input, select, tab, button |
| `--radius-box` | 12px | panel, modal, alert, picker |
| `--size-selector` | 14px | the square of a checkbox or radio |
| `--size-field` | 28px | the height of a field or button |
| `--border` | 1px | every hairline and control edge |
| `--depth` | 0 | no drop shadows by default; 1 is reserved for floating surfaces |
| `--noise` | 0 | no texture; kept so a future surface can opt in |

`--radius` and `--radius-lg` are aliases of field and box. Borders separate;
shadows belong only to dialogs, drawers and toasts.

### Type

| Token | Size | Use |
| --- | --- | --- |
| `--text-2xs` | 11px | metadata, status bar |
| `--text-xs` | 12px | labels, row sub-lines |
| `--text-sm` | 13px | body in dense surfaces |
| `--text-md` | 14px | default body |
| `--text-lg` | 16px | section lead, notices |
| `--text-xl` | 18px | page titles |
| `--text-2xl` | 22px | KPI values |
| `--text-display` | 28px | Home greeting |

Families: `--sans` is Inter Tight, bundled with the application, with system UI
fallbacks; `--mono` is for paths, ids, code and numbers. Weight carries hierarchy
before size does: 400 body, 500 controls, 600 headings, 650 only for a KPI value.
Density comes from leading and the 4px spacing scale, not from shrinking the body
copy.

### Space and layout

`--s1` 4, `--s2` 8, `--s3` 12, `--s4` 16, `--s5` 24, `--s6` 32, `--s7` 48.
The page edge is `--edge`, section gaps use `--s5`, and row padding uses
`--s3`/`--s4`.

### Motion

`--fast` 120ms, `--base` 180ms, `--slow` 220ms, with the shared ease-out and
ease-in-out curves. There is no animation on a keyboard-driven action or on a
high-frequency surface. `prefers-reduced-motion` removes movement and keeps the
fade.

### Layering

`--z-sticky` 10, `--z-popover` 40, `--z-drawer` 60, `--z-modal` 70, `--z-toast`
80, `--z-palette` 90. A surface never invents a z-index.

## Components

The canonical components live in `app.js` and are styled once in `style.css`.
They are grouped by purpose:

- **Foundation**: tokens, reset, typography, icons, focus ring, scrollbars.
- **Primitives**: `btn`, `input`, `select`, `textarea`, `ds-check`, `ds-radio`,
  `ds-toggle`, `ds-range`, `ds-progress`, `ds-radial`, `ds-kbd`, `viz-steps`,
  `ds-timeline`, `ds-validator`, `pill`, `dot`, `face`, `skel`.
- **Composed**: `panel`, `ui-row`, `summary-band`, `kpi`, `notice`, `toast`,
  `drawer`, `modal`, `picker`, `palette`, `subnav`, `table`.
- **Domain**: `rail`, `ctx-panel`, `statusbar`, `env-cell`, `ops-*`, `site-*`,
  `run-*`, `decision`, `team-*`.
- **Conversation**: `turn`, `turn-who`, `turn-body`, `tool`, `worked`,
  `chat-plan`, `team-note`, `md`, `md-code`, `diff`.

Key rules:

* `page()` and `page-head` frame every screen; tabs appear only where an area has
  peers.
* `uiRow()` is the resource row: state dot, title, one supporting line, one
  trailing fact, and actions on hover (always visible on touch).
* `summaryBand()` is one surface of labelled facts, not a grid of dashboard cards.
* `panel` is a genuinely bounded group, never a box around one row. Its head and
  body share one inset; a flush body keeps that inset for ordinary content and
  lets only tables use the card's full width.
* A disclosure is one control everywhere: the same chevron, label size, colour
  and indent on Models, Skills, Diagnostics, a run report and an event's
  arguments. A summary band is a set of readings separated by space, not rules.
* A hovered list row paints one rounded, inset band a step lighter than its
  surface (`--bg-hover`). The row itself stays transparent, so text never moves
  and no square background can show behind the curve. Table rows round their
  first and last cell.
* `pill` names a state, `uiDot` marks it, `notice` explains it.
* `uiEmpty()` is an empty list plus the action that resolves it.
* `drawer` keeps detail without losing the list's context; `modal` is for a
  consequential decision, with the plan and its effect spelled out; `toast`
  confirms a completed operation and is never the only record of a failure.
* A component exists because the same problem is solved in more than one place.
  Screen-specific layouts keep their own class and use the foundation.

### Adding a component

1. Add the token or use an existing one. No raw hex, pixel radius or duration.
2. Add the CSS next to its category, with hover, focus, pressed and disabled
   states.
3. Apply it on the screen that needs it, and verify it there at the supported
   widths.
4. Add it to [`design-manifest.json`](design-manifest.json) with its props and
   variants.
5. Add a contract assertion if the rule can drift.
6. Verify at 1600, 1280, 1024, 800 and 390px, and with reduced motion.

## Workspace surfaces

The Assistant is a conversation *and* a workspace. Beside it, a project card
(Environment) carries the project, branch, Changes and Outputs. Two surfaces can
open next to it:

* **Browser** — a sandboxed preview of the project's site, with an address bar,
  reload, open-externally and a real empty/stopped/no-site state.
* **Terminal** — the same xterm and the same server-owned shell as the project's
  Terminal tab, opened in the chat's project without leaving the conversation.

Both are opened from the card's Workspace rows and, below 1440px where the card
folds into a drawer, from the three icon toggles in the conversation header. The
Browser always takes the wide column, replacing the card. The Terminal may sit
beside the card at 1600px and above; below that it replaces it. When both are
open they share one work column, split vertically with a draggable divider
(58/42 by default). Below 1280px a surface becomes a sheet over the conversation;
below 700px of height the pair becomes tabs.

The conversation header has one order: status chips (queued, pull request,
settled), then context links (Back to request, See the run, Chapters), then the
actions — the surface toggles, search, Evidence, Continue and the menu. Chips sit
on the same 28px row as the buttons, and the primary Continue is the last action
before the menu.

The surfaces belong to the conversation's project. Switching chats re-points the
browser and leaves an old shell running with a banner saying which project it
belongs to. Closing a surface never kills a shell. A full reload starts with both
closed.

## Screen decisions

* **Home** is a two-column workbench on wide windows. The environment ledger
  (services, sites, runtimes) is full width; projects and their addresses sit
  side by side beneath it. On narrow windows it is one column in reading order.
  The assistant prompt is present but not the page's only purpose.
* **Projects** separates registered projects from folders known only because a
  site serves them, and shows serving mode and Git state in each row. Loading,
  error and empty states are distinct. **Remove** and **Remove site** use the
  danger treatment; a reversible project action does not need a confirmation
  dialog, but it must look destructive.
* **Sites** leads with a status ledger (sites, attention, hostnames, local
  HTTPS), then a searchable, filterable list. Serving mode is a pill; the
  effective runtime is stated only where it exists; the right column carries
  only exceptional facts (not in hosts, no certificate).
* **Environment** keeps services and runtimes, names the stack separately from
  optional tools, and links to Sites instead of duplicating the list.
* **Runtimes** names the default, the installed path and how many sites use each
  version. A default version not used by a site is allowed to say so.
* **Databases** explains a stopped MariaDB and offers to start it, instead of
  printing a command failure to an ordinary reader.
* **Assistant, Runs, Team, Activity, Diagnostics, Models, MCP, Create and
  Develop** keep their structure because they already follow the system.

## Agent-generated interface

An agent writes a conversation turn, not a web page. The renderer chooses the
form from a small, declared set of blocks, and styles them with this system.

### What an agent may emit

Markdown, and only these blocks:

- headings, paragraphs, ordered and unordered lists
- tables with a header row; numeric columns align to the end
- fenced code with a language name
- a fenced `chart` block holding JSON:
  `{"title":"...","unit":"...","bars":[{"label":"...","value":12,"tone":"success"}]}`.
  The application validates it (at most 12 bars, finite non-negative values) and
  draws the shared bar chart; invalid data stays a code block
- blockquotes
- links to public `http` and `https` addresses
- inline code, strong and emphasis

Everything is sanitised by `md()`. Long code folds; tables use the shared grid;
there is no other path into the DOM.

### What an agent must not emit

Raw HTML, `<script>`, `<style>`, `<iframe>`, `<object>`, `<embed>`, `<form>`,
`<input>`, `<canvas>`, `<video>`, inline event handlers, `javascript:` or `data:`
URLs, external stylesheets, fonts or widgets. There is no component syntax that
executes.

### Choosing a shape

| The agent has | It writes |
| --- | --- |
| A result with named columns | a Markdown table |
| A command or a file | a fenced code block with its language |
| A comparison | a table, or two short lists |
| A sequence | an ordered list |
| A warning that changes the decision | a paragraph beginning with the word, then the detail |
| A patch it applied | a fenced `diff` block |
| A long report | a short answer first, then the detail, because the turn folds long work |

An agent does not invent status glyphs or colour words. It says "the check
failed" or "all 72 tests passed"; the renderer carries meaning that colour alone
cannot.

Buttons, fields, toggles, drawers, modals, progress, steps and timelines are
application components. An agent does not generate them. An agent that needs a
decision writes the question and calls the approval or inbox capability; the
application renders the real control. Generated controls would bypass the
permission, evidence and audit model.

The machine-readable list of what is allowed is
[`design-manifest.json`](design-manifest.json), served to agents as
`design.manifest`.

## Accessibility and width

Every interactive control is reachable by keyboard and has a visible
`focus-visible` ring. Icon-only controls carry an `aria-label`. State is never
communicated by colour alone. The supported widths are 1600, 1280, 1024, 800 and
390px; below 1024px the context lists fold into an overlay, and on a phone the
rail becomes a bottom bar with a More sheet.
