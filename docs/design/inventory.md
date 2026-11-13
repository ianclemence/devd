# Surface inventory

Every place in the window, and the states each must be checked in. A review walks this list, in every theme, at three
window sizes (1280 × 800, 1440 × 900, 1920 × 1080), and marks each cell. Nothing is reviewed from memory.

States: **load** (first load, nothing cached), **empty**, **full**, **long** (very long names and text), **error**,
**offline** (devd not answering), **busy** (an action in progress).

Each element is scored on seven questions:

1. Is it aligned to the spacing scale and to its neighbours?
2. Does the most important thing win?
3. Does every interactive thing show hover, press and focus?
4. When something fails, does it say what happens next, with a button?
5. Is there a keyboard path?
6. Does it hold up with very long text?
7. Does it read in every theme?

## Frame

| Surface | load | empty | full | long | error | offline | busy |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Rail, with the active screen |  | – |  |  | – |  | – |
| Top bar: back, forward, breadcrumbs, faces, search |  |  |  |  | – |  |  |
| Status line in the footer, with the update notice |  |  |  |  |  |  |  |
| Command menu (Ctrl+K), empty and with a query |  |  |  |  | – |  | – |
| Toasts, success and failure |  | – |  |  |  | – | – |
| Confirmation dialog |  | – |  |  | – | – |  |
| Form dialog |  | – |  |  |  | – |  |
| Approval dialog |  | – |  |  | – | – |  |
| Tooltips |  | – |  |  | – | – | – |

## Screens

| Surface | load | empty | full | long | error | offline | busy |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Home: greeting, needs-you, what do you work with, recent work, the server |  |  |  |  |  |  |  |
| Inbox |  |  |  |  |  |  |  |
| Assistant: conversation list, start, conversation, error line, chain, composer and its menus |  |  |  |  |  |  |  |
| Assistant side panels: terminal, browser |  |  |  |  |  |  |  |
| Runs: list, run page (heading, right now, criteria, chain, timeline, tasks, result), agent detail |  |  |  |  |  |  |  |
| Team: roster, schedules, automations, skills, MCP servers, notes |  |  |  |  |  |  |  |
| Intelligence: providers, models, roles, cost |  |  |  |  |  |  |  |
| Projects: list, new project, stacks |  |  |  |  |  |  |  |
| Project: overview, files, editor, Markdown, changes, history, branches, stashes, tags, remotes, worktrees, recover, run, terminal, team |  |  |  |  |  |  |  |
| Sites: list, hosts, certificates, mail |  |  |  |  |  |  |  |
| Server: overview, runtimes, databases, tools, remote |  |  |  |  |  |  |  |
| Activity |  |  |  |  |  |  |  |
| Settings: settings, diagnostics, API and tools, connect agents, flags |  |  |  |  |  |  |  |

## Tasks walked end to end

These are walked as a person would do them, not screen by screen, because what feels unfinished is usually between
screens.

1. Install devd on a machine with nothing on it, choose what to work with, open the first site.
2. Start a project from a stack, edit a file, commit the change.
3. Give the team a goal from a conversation, watch the run, step in when it is blocked, accept the result.
4. Close the window while a run is working, open it again, and find the run where it was.
5. Update devd from the footer.
6. Remove a site, a project, a conversation and a run.
