# Product audit and decisions

This is the working audit behind the assistant-era redesign: what was found by
following each part into its code, and what was decided.

## Navigation

| Was | Finding | Decision |
|---|---|---|
| Home | Static cards, status text that duplicated Overview. | Rebuilt around work: what needs you, what is running, projects, getting started. |
| Assistant, Inbox, Providers & models | New; Inbox was only reachable from the assistant. | Inbox is top-level (it is where agents reach you). Assistant gained Team runs as a tab. Providers became **Models** under Manage. |
| Projects, Create, Develop | Three pages for one thing: Projects listed git repos, Develop held stack projects, Create scaffolded. | One **Projects** entry with tabs: Projects, New project, Stacks and tasks. |
| Agent sessions, Shared notes | Notes exist only for agents sharing a project. | Shared notes became a tab of **Agents**; terminal sessions were later removed. |
| Processes | Distinct (supervised project processes and logs). | Kept. |
| Overview, Sites, Preview, Services, Runtimes, Toolchain, Certificates, Hosts | Eight rail items for one subject; Overview duplicated Home. | One **Server** entry; tabs for Overview, Services, Sites, Tools, Runtimes, Preview, Certificates, Hosts. |
| Capabilities, Activity, System | "Capabilities" is an API reference, not a daily destination; System was an unlabelled dump. | Moved to Manage as **API and tools**, **Activity**, **Diagnostics** (health model first, raw checks below). |

The rail went from 24 entries to 12.

## Search

"Search capabilities" searched the capability registry, which is an internal
catalogue. Replaced by a search and command palette over what developers look
for: places, projects, files (path match, walked live), literal text in a
project (`~`), conversations, models, tasks, services, things to do (`>`), and
asking the assistant (`?`). Capabilities remain, last. Nothing is indexed, and
the palette footer says which sources were searched and whether any was cut
short. Selecting a search result opens it; selecting an action runs it through
the normal approval flow.

## Health

The pill reported `doctor.ok`, a boolean over environment checks. Replaced by a
state model derived from components with a `required` flag, so an unset optional
provider is a setup step, and a failed provider test is `unavailable` rather than
silently healthy. See `docs/agents.md`.

## Logo and footer

The logo asset was a 96 px PNG with transparent rounded corners and a black
2 px frame, drawn inside a rounded box with a 1 px ring: the visible gap was in
the asset, not the CSS. The asset is now a full-bleed square cropped to the
photograph's own edges (128 px for the rail, up to 512 px for icons), clipped by
the container radius only. The rail footer now holds only the health status; the
version and data path moved to Diagnostics. The status bar shows context (branch
and changes inside a project, services up, agents working, what waits on you) and
the environment, instead of a capability count.

## Services

Two managers coexisted: `devd-services` (nginx, MariaDB, PHP listener, installed
into devd's directory) and the toolchain catalogue (databases through the host's
package manager and service manager). Both remain; `services.inventory` is the
single view over what is on the machine and records what the person decided.
Control through `toolchain.service` honours those decisions.

## Architecture decisions

* One scheduler in code, one planner in a model: scheduling and acceptance must
  be reproducible.
* Evidence comes from tool results devd produced, never from agent text.
* Approvals are data produced by a person; agents queue, never decide.
* Skills are content, loaded on demand; the prompt carries only an index.
