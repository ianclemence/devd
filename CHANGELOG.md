# Changelog

All notable changes to `devd` are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because `devd` is pre-1.0, each `0.x` release is a milestone, and a milestone
may change capability identifiers where the design was wrong.

---

## [Unreleased]

### Added

- **Themes of your own.** A theme is a folder in devd's `mods` folder: `mod.toml` says what it is, and
  `theme.toml` gives three colours (background, text, accent), any others you want, and a picture behind the
  screens from devd's scenes. devd works out every other colour in the proportions its own themes use. See
  [`docs/themes.md`](docs/themes.md).
- **A readability check.** devd measures the contrast of everything a person reads and refuses a theme that
  falls short, naming the two colours, the contrast they reach, the contrast they need, and which way to move.
  The five built-in themes pass the same check.
- **Your themes in Settings**, with the same cards as the built-in ones. A theme that cannot be used is listed
  with what is wrong and a button that asks the assistant to fix it. Saving a theme file shows the change within
  a second; a theme in use is repainted in place.
- **`/theme` in the message box.** Describe a look, or a change to the theme you are wearing ("make the accent
  greener"), and the assistant makes it. It writes themes only through `theme.save`, which checks a theme before
  anything is written. `theme.list`, `theme.check`, `theme.use` and `theme.remove` complete the set.

### Fixed

- **A model that thinks before it writes no longer looks stuck.** What it thinks is shown live, above the message
  box, as "Thinking" with the time it has taken and the latest of its thought. A reply that spent its whole budget
  thinking and wrote nothing is no longer treated as half an answer to continue, which made an agent think again
  and again: devd asks it once to act with less thinking, and stops the turn with a plain reason if it happens
  twice.
- **devd says what it is doing while it works.** A line at the foot of the reply shows the step running now
  ("Running go test"), the model thinking, or the words being written, with the time since you last spoke
  ("7m 17s"). Finished steps in a row fold into one line, such as "Read 8 files, searched 7 times and checked
  git", which opens to list them.
- **Icons and logos stand on their own.** The tiles behind the menu icons, the model and tool logos and the
  empty-state icons are gone; a single-colour logo is drawn in the theme's text colour, so it reads on every
  theme. The icon in each message box menu row sits closer to its name.
- **An agent's reply reads as one reply.** The model is named once, under the last step of a turn, not under
  every sentence between its tool calls.
- **Links in replies go where they point.** A written link to a file, such as `[main.rs](src/main.rs:42)`,
  `src/main.rs#L42` or a `file://` address, is a file, not a web address. A file in one of your projects opens
  in devd, in the project's files, scrolled to the line and with it marked; anything else opens as before. A link
  to a file that is not there is shown as its words instead of a link that does nothing.
- **Every skill can be reached from `/`.** The `/` and `@` menu showed the first eight matches and could not be
  scrolled; it now lists all of them, with the count, in a list that scrolls, and the arrow keys reach every row.
- **The face in the top bar is the agent at work.** A team member writing shows its own initial and colour (the
  Implementer's "I"), not devd's "D", and pressing it opens that member's conversation.

## [0.32.0] - 2026-11-14 - Runs that resume, screens that keep up, and one shape for every banner

### Fixed

- **Resuming a paused run carries it on.** A run paused because one task could not prove its work used to pause again
  the moment it was resumed: the same task was still stopped and everything else waited for it. Resuming now gives each
  stopped task another go, with what was wrong as its feedback.
- **The message box keeps its place.** In a conversation that belongs to a run, opening or closing the team chain no
  longer shrinks the box and moves it to the left.
- **A stopped conversation says so once, and can be resumed in one press.** The error was shown twice, at the top and
  at the bottom; it is now shown once, at the bottom beside the message box, with a Resume button, so a dropped
  connection does not mean retyping.
- **A paused run's page says why once.** The banner carries the reason and Resume; the same Resume in the heading and
  the same sentence under the banner are gone. A run's title that was cut half way through a word now ends at a word,
  with an ellipsis.
- **The Mail screen says when the mail catcher is not running**, with a button to start it, instead of the connection
  error.
- **The Server screen no longer calls a service that answers "stopped".** One started outside devd says so.
- **The team chain scrolls sideways without taking the page with it.** A plain mouse wheel scrolls the page past the
  chain; Shift and the wheel, a touchpad, dragging the chain, or the new bar under it move it sideways, so every agent
  in a long run can be reached. The same on a run's page.
- **The model and permission menus close when you click outside them**, on Home and in the Assistant.
- **The permission menu shows each mode's description in full**, under its name, instead of cutting it off.
- **Home counts PHP once.** A PHP version runs as several workers; Home now counts and names it as one service, as the
  Services screen does, and names the services that are not answering first.
- **The Databases screen says when MariaDB is not running**, once and in words, with a button to start it, instead of
  showing the client's error twice.
- **Screens keep up with the machine.** Starting or stopping a database server refreshes the database screens, and
  services, health and databases are read again every few seconds while they are on screen, so a change made outside
  devd shows up. A screen showing an error no longer flashes back to loading each time it is read again.
- **An install that cannot run here is not offered for approval.** The dialog says why and offers Close; on Windows
  without the Windows Package Manager it says so and how to get it. Long lines in every dialog now wrap inside it.

### Changed

- **Every banner has one shape.** Warnings, failures and notices across the app (a conversation, a run's page, an
  approval, a file that could not be saved, a screen that could not refresh) share one component: a hairline and a faint
  wash in their tone, the mark level with the first line, an optional title, and their actions under the words.
- **Empty states and section labels read the same everywhere.** A wrapped explanation under an empty state is centred
  like its title, and the Stacks groups use the same small-capital labels as every other screen.
- **Inbox messages lead with what needs you.** A paused run's message now shows why it stopped as its headline, the
  run as the line under it, who sent it in words, and Resume beside Open the run.

## [0.31.0] - 2026-11-14 - Runs that keep going, stay bounded, hand work on, and show how they are going

### The team keeps working, and says how it is going

- **Runs no longer stop when the window closes.** The window starts devd in the background, or attaches to the one already running, and the team works there. "Quit devd completely" in the command menu stops it too.
- **A run that nothing is carrying says so**, and `team.drive` (or the button on its page) puts it back to work from where it stopped. `team.get` reports whether anything is carrying the run.
- **`devd team show` and the other reads no longer take the agents.** With no devd running they used to become the owner, restart half-finished work and exit a moment later.
- **`team.status`** answers "how is it going?" in the same sentence the window shows: who is working and since when, what is ready, what is waiting, what needs you, and what the run has spent. The assistant is told to use it.
- **`team.timeline`** lays out where each agent's time went: model turns, tool calls and checks, with what happened along the way (sent back, retried, waiting on the provider, held, asked to wrap up).
- When a task is accepted, a line goes into the conversation that asked for the run ("cron finished: … (3 of 11 tasks done)"), without asking a model to write it.

### Runs that are bounded

- **A run can be given a ceiling** on what it spends and on how many model turns its agents take (`budget_usd`, `max_turns` when starting it, `team.budget` afterwards; raising it is for a person). devd says when a run nears its ceiling and pauses it, with its work kept, when it reaches it.
- **One task cannot grow without limit in one conversation.** Past a large share of what a conversation usefully holds, its agent is asked to commit what works and hand over what remains, so a fresh agent continues from its notes.

### Work handed on, not lost

- **What a finished task hands on is kept in a shape**: what it did, the files it changed (from the changes it made, not from its claim), what it decided and what it left open. The next task is given that instead of a report cut off at a character count. `team.report` takes `decisions` and `open_questions`.
- **A progress file** (`.devd/progress.md` in the project, outside version control) records every accepted task, and every agent is given its latest entries before it starts.
- **`project.verify`** runs every check the project has (format, lint, type check, build, tests) in one call and is recorded as the proof. The engineering roles are told to use it before they report.
- **A plan is checked against its roles before any work is paid for.** A task that asks a role which cannot run commands to run one is sent back to the planner with the reason. The planner is also told to split large pieces into tasks an agent can finish in one sitting.

### A run's page, rebuilt

- **The list is grouped by project** and narrows to the runs that need you, the ones working and the ones done, with how many of each. A row says where the run stands, how many of its tasks are done and when it last moved.
- **The heading says what the run has cost**: its state, project, tasks done, money spent against its ceiling, model turns and when it started, in one line.
- **A run that nothing is carrying says so on its page**, with "Carry it on". An error has a line of its own.
- **"Done when"** lists what the run was asked to meet, each line marked with what the final check found or "not checked yet".
- **Time**: one line per agent showing where its time went (thinking, tools, checks) and where it was sent back or held up.
- **Choosing an agent**, in the chain or in the task list, shows it beside the page: its brief, what it may change, the files it changed, the checks it ran, what it decided and left open, its report and what it cost, with "Open its conversation", "Try again" and "Accept as it is".
- **A finished run leads with its result**: the final check's verdict, how many checks passed and the report. The goal comes last.

### Quieter screens

- The chain of a run is drawn without decoration: one glyph per task for where it stands, a title and one line of facts, hairline connections, and colour only for the hand-over happening now and for work held up.
- A conversation's heading is its title, one line of facts and its actions. An error is no longer squeezed into it: it has a line of its own below, in full.
- The sentence for what a run is doing now sits level with its mark and has no box around it.
- `docs/design/rules.md` and `docs/design/inventory.md`: the rules every screen follows, and the list of every surface and state a review walks.

### Fixed

- **The browser panel starts again after a browser was left behind.** When Edge restarted itself on launch, the copy it started held devd's browser profile, devd could not find it to stop it, and every later start failed with "the browser did not open its DevTools port". devd now finds a leftover browser however its command line is written, reads the port from the profile when the first process hands over to another, and stops the restarted copy when the panel closes.

## [0.30.0] - 2026-11-13 - See the team work, delete what you are done with, and nothing forced on you

### The team, in view

- **A run is drawn as a chain**: a card for each agent, a line for each thing one waits for. Lines say what they carry (settled, handed on now, held up, not yet), a card says how long its agent has been quiet and whether it is on another try, and the chain opens scrolled to whatever is working. It sits under the conversation a run belongs to, and at the top of the run's page, where a sentence says what is happening right now: who is working and since when, what is ready to start, what needs you.
- **Every agent has a colour of its own** wherever it appears. The faces in the top bar show who is working, say so when the pointer rests on one, and open what that agent is doing when pressed.
- **One count of progress.** A run read "2 of 12" in one place and "4 of 12" in another because two rules disagreed about the planner and about work accepted by hand. The planner is not counted and accepted work is, everywhere, as the server counts them.
- A run that is still going is read again every couple of seconds, whoever is driving it, instead of waiting to be told.

### Delete and clear

- **Conversations and runs can be deleted**, from the list (an icon that appears when the pointer is on a row) and from the conversation or run itself, each after a confirmation that says what goes and what stays. "Clear finished" removes every finished one. A run still going is paused or cancelled from its own page first. A conversation deleted elsewhere says so and offers a new one.

### Nothing forced on you

- **`devd setup` installs the web server and nothing else.** PHP, databases and the other runtimes come when asked for (`--php 8.4`, `--services mariadb`), here, in the app or from the assistant. The first site is a plain page when there is no PHP. A fresh Home asks "What do you work with?" and offers PHP, Node.js, Go, MariaDB and Redis, one click each.
- PHP is a requirement only when a PHP site exists, so a machine without PHP is no longer told its setup is unfinished. The installer says what it will and will not install.
- **The health line names every problem**, not only the first ("2 problems to fix: nginx and PHP 8.4").

### Runs that stalled

- `devd team show` and the other reads no longer take over the agents. With no devd running, a read became the owner, restarted whatever was half done and exited, so a run was restarted every time anyone looked at it. Commands that start a run and return now say that nothing will carry it on until devd is running.
- Work accepted by hand is committed like work accepted any other way. Left uncommitted it blocked every later merge into the project.

### Smaller things

- Popovers in the message box close when you press anywhere outside them, on Home and in the Assistant.
- Screens, popovers, dialogs, the command menu, toasts and the side panels now fade and settle into place with Tailwind's own ease-out and ease-in-out curves.
- The command menu opens on your latest conversations and projects, then the screens in the order of the rail, not on whichever command you used last.
- Updating the model catalogue gave "Internal error: could not read the response from models.dev" on a slow connection: it was given two seconds. It now waits as long as a file that size needs, tries three times, and says what happened in plain words.
- Removing a project that was only listed because a site served its folder removes the site.

## [0.29.1] - 2026-11-13 - Removing a site, and a project that only a site keeps listed

- **Sites can be removed from the Sites screen**, with a confirmation that says the folder is not touched. There was no way to do it from the window.
- **Remove on a project that is listed only because a site serves its folder** (a deleted worktree, for example) removes the site instead of failing with a conflict about `site.remove`.

## [0.29.0] - 2026-11-13 - A page for every project, a terminal and a browser beside the assistant

### Projects

- **A project has its own page**, with a tab for everything you do in it: Overview, Files, Git (changes, history, branches, stashes, tags, remotes, worktrees, recovery), Run, Terminal and Team. Every control is a capability call, the same ones the assistant uses, so a change either of you makes shows on the other's screen at once.
- **Files opens and edits.** A tree that opens folders as you ask, a viewer with line numbers, Markdown drawn as a document (with a Source toggle), and an editor with Tab and Shift+Tab, automatic indentation, a gutter, Ctrl+S, and an Unsaved/Saved mark. Saving keeps a file's line endings and final newline exactly; a file with mixed line endings is shown but not opened for editing, and says why.
- New file, rename and delete from the tree. Delete asks first.
- Staging, committing, discarding, switching branches, stashing, fetching, pulling and pushing from the window, with diffs.

### The assistant's panels

- **A browser and a terminal beside the conversation.** A strip at the edge of the assistant opens each; both can be open, the browser above the terminal. Each has its own tabs, a button for another, and a way to close it.
- **The terminal is a real shell inside devd.** Open as many as you like, type into them, paste, and leave and come back: they stay open. The assistant can read what a terminal shows but only you can type into one. The Terminal tab of a project shows the same shells.
- **The browser is the one the assistant uses.** A page it opens appears here (the panel opens by itself); you can load an address, go back and forward, reload, click, scroll and type on the page, and see the console, the requests and the page as the assistant reads it. It starts only when asked for.
- `browser.status` says whether the browser is running without starting it. A browser left behind by an earlier devd no longer stops the next one from opening, and closing the window closes the browser it started.
- `project.terminal` takes a size, so a shell opens as wide as the panel that shows it.

### Updates

- **The footer says when a newer devd is out** and installs it from there: one click, a confirmation, then devd verifies the release's signature with the key built into the program, replaces itself and restarts. A release that is not signed is not installed from here; the footer opens its page instead.

### Looks

- The Intelligence screen has a brain icon.
- Tools, runtimes, databases, services and stacks show their own marks (PHP, Node.js, Go, MariaDB, Redis, nginx, Composer and many more) on the Server, Runtimes, Tools, new-project and connect pages; one with no mark gets its initial.
- Hub pages gain "Open project" and "Open folder".

### The assistant can draw

- **Answers can contain real tables, charts, metrics, steps, timelines, notices and buttons** instead of describing them, in the same way Vercel's json-render lets an agent write a spec the chat renders. The assistant writes a fenced `devd-ui` block; the window draws it from a fixed catalog (`devd-genui`), as it arrives, in every theme. See [docs/generative-ui.md](docs/generative-ui.md).
- **A drawing that does not check is sent back** to the model with each problem, once or twice, so it writes it again; the transcript says so. Part of a finished spec that still cannot be drawn is left out with a short note, and the rest is shown.
- **Buttons the assistant draws never act on their own.** A button can reply, open a devd page or file, or run a real capability; running shows what will run, with its kind and arguments, and asks first unless it only reads.
- Right-aligned Markdown table columns no longer collapse to a sliver (a numeric column's header wrapped one letter per line).
- Flags `agent.draw`, `agent.draw_fixes` and `ui.site_icons`.

### Site icons before links

- A web link in an answer starts with the site's own icon, fetched from the site through devd's guarded fetch rather than a third-party icon service, kept on disk, and shown as a letter tile until it arrives or when a site has none. Local and private addresses get a globe and are never looked up.
- `docs.fetch` and `web.open` followed no redirects at all (the HTTP library hands a `307` back as a response, not an error, so an empty body was accepted). Redirects are now followed, each hop re-checked as before.

### Themes

- **Five themes, each with its own name and its own background scene**, replacing three themes in two forms: Sling, Dawn and Psalm for the evening, Sharon and Daybreak for the day. Each background is a noise-driven dither that fades at its edges (ripples, dawn over hills, folds of silk, a rose-tinted map, rays of morning light). Settings shows each as a small window in its own colours. A choice saved by an earlier window opens the matching theme.
- The + menu in the message box starts each row with an icon, a `/` or an `@`, and opens below the box on Home.

## [0.28.0] - 2026-11-12 - The native window, and an assistant that checks itself

This release replaces the web-view window with a native one, drawn on the GPU and with no browser inside it, and adds packages for macOS and Linux for the first time. The assistant is also held to account before it stops, and the record of what it did can show whether it was changed.

### The native window

- **Every screen is native**: Home, Inbox, Assistant, Runs, Team, Projects, Sites, Server, Intelligence, Activity and Settings, with a command menu (Ctrl+K), keyboard navigation, light and dark themes, and a cache that is kept true by events, so screens open with what they already know and then catch up. The 18 pages behind the areas are restored: Hosts, Certificates, Mail, Schedules, Automations, Skills, MCP servers, Notes, Runtimes, Databases, Tools, Remote, New project, Stacks, Diagnostics, API and tools, Connect agents and Flags.
- **Projects are project-first.** The Projects screen lists your projects with their stack and tasks; each project's tasks fold away until you ask. New project offers three ways to begin — a stack, an empty folder with git, or a folder you already have — instead of assuming every project starts from a template. Stacks is the catalogue, with each stack's versions and what it makes.
- **Intelligence shows performance, not just price.** Per model: tokens, estimated spend, conversations, errors, cache use, accepted tasks and cost per accepted task. Per member: acceptance on proof, rework, checks, median time and cost per accepted task, with a rank that says how it is calculated and refuses to rank fewer than three settled tasks. A model or member whose model was not recorded says so rather than guessing.
- **Three themes, each with its own background.** Stone (the workbench), Shepherd (wool, olive and dawn) and Harp (indigo and gold), each in light and dark, with a distinct dithered field behind every screen. Settings switches and remembers the choice.
- **A new message box.** Type `/` for the skills devd can load and `@` for a member of the team (including ones you added); `+` attaches files and folders; the row underneath shows the model (with its vendor's mark), the effort and how much the assistant may do without asking. It grows with what you write and underlines slips as you type.
- **Files and addresses are clickable** in replies, in your own messages and in the message box: a path opens if the file exists, a web address opens in the browser, and `devd://runs/ID`, `devd://projects/DIR`, `devd://assistant/ID` and `devd://sites/NAME` open that place in devd as a chip. A click never starts a program: a script or executable is shown in its folder instead.
- Text that is too long for its place now ends in an ellipsis instead of being cut mid-letter, and a path keeps its end. A network failure is explained by its cause ("api.deepseek.com closed the connection before it answered. This is usually the network, a VPN or a firewall, not your key") instead of an operating-system message.
- Activity hides what only looked and shows what changed. A refresh that failed says what is on screen may be out of date; a tool that runs for a while says how long.
- A new logo and a faint dotted background.

### The assistant

- **It cannot end a turn on a promise.** Before a turn ends, devd checks the assistant's own plan for open items, checks that the run's policy has the proof it asks for, and asks any tool server you allowed (`_Stop`) whether it objects. It sends the assistant back to finish up to three times per prompt, says when it is on the last reminder, and tells you if it still ended with things open. `agent.end_gate` and `agent.stop_budget` switch it.
- **Private data is held at the door.** A conversation that read an `.env`, a key or devd's own secrets cannot send a secret to a tool server, a push, a public address or a network command without your approval, for that exact call. The label is rebuilt from the record every time, so a restart cannot clear it. It matches the secret itself, so reading an `.env` does not make every later call suspect. `agent.flow_guard` switches it. It does not sandbox arbitrary code and cannot see a secret that has been rewritten before it is sent.
- **Packs.** `devd call packs.install --arg source=owner/name` (or a folder path) installs roles, skills and tool servers together from a `devd-pack.json`; `packs.list` and `packs.remove` show and take them out. Nothing in a pack runs: a tool server is saved disabled. A name that is built in, yours or another pack's is a refusal. Removal takes out exactly what the pack added.
- **Lifecycle hooks** (`_Stop`, `_PostCompact`) on external MCP servers, off unless you list the servers in `mcp.hook_servers` or `DEVD_MCP_HOOK_SERVERS`.
- A team of 21 roles with a hand-off flow and roles of your own, a motion designer with built-in render tools and the Remotion skills, and every built-in skill rewritten to one standard that a test enforces. A new skill, `defects-that-keep-coming-back`, gives the reviewer, QA and implementer the ten defects that reviews find over and over.
- Spend is priced with prompt-cache prices, context is shortened in steps that keep a provider's cache valid, and the model catalogue comes from models.dev.

### Trust

- **The audit record is a hash chain.** Each entry carries the hash of the one before it, and the newest is also saved apart, so editing, removing, reordering or cutting off the end of the log is found. `devd call audit.verify` checks it, and Activity shows "record intact" or where it broke. Entries from before this version are counted, not blamed, and pruning leaves an anchor so a pruned log still verifies.
- **Feature flags**: `devd call flags.list` and `flags.set`, with `DEVD_FEATURE_*` in the environment taking priority for one run. A value that does not read is passed over and reported, never silently wrong.

### Automation

- `devd run --task task.json` runs one prepared task with the assistant and exits, for a script or a CI job: strict input (unknown, duplicate and extra fields are refused), one JSON line on standard output, and an exit code that says how it ended (0 completed, 1 failed, 2 invalid, 3 needs a person, 4 timed out, 5 finished but unproven). It does not retry, and it stops rather than wait for an approval nobody can give.
- Commands that create a run, a conversation or a project print a `devd://` link. `devd-app --register-links` makes the operating system open `devd://` links with devd.

### Providers

- A rejected key (401), an unpaid balance (402), a refusal (403) and a quota that is used up are no longer retried; overload and rate limits are, with a little jitter.
- OpenRouter: the model's reasoning is handed back between turns, effort is sent the way OpenRouter reads it, and `anthropic/*` models get prompt-cache markers.

### Installing

- Windows: a per-user installer (no administrator prompt) and a portable zip. macOS: a disk image for Apple silicon and one for Intel. Linux: a `.deb` that apt installs with its dependencies, and a `.tar.gz`.

### Not in this release

- The macOS app is signed ad hoc, not with an Apple Developer ID, so the first launch needs right-click, Open. The Windows installer is not code-signed, so SmartScreen may ask you to confirm.
- OpenAI's Responses API is not used; OpenAI models are reached through Chat Completions as before.
- Dragging files into the window is not supported yet; use the `+` menu.
- The macOS and Linux packages were built and checked on CI machines, and have not been run by hand on every distribution or Mac.

---

## [0.27.7] - 2026-11-09 - A shared browser, and an Update button in the footer

### The browser is a real tab, not a fetch

`web.open` answers what a page says. This adds what happens when you press
something, which needs a page both the agent and the person can see:

- `browser.open` starts a private headless Chromium (Edge or Chrome) with its
  own profile, so the person's own browser, cookies and windows are untouched.
- `browser.tabs.list/open/focus/close` give every tab a stable id. Every other
  command names the tab it acts on; there is no ambient "current page" to drift.
- `browser.snapshot` reads the accessibility tree and assigns `e1`, `e2`, ... to
  the actionable nodes. A navigation clears them, so a stale ref can never press
  a different control. `browser.click` and `browser.type` take those refs.
- `browser.back/forward/reload/navigate`, `browser.screenshot`,
  `browser.console` (console lines and fetch/XHR requests) and a guarded
  `browser.evaluate` escape hatch complete the set. Every command returns a typed
  state (`ok`, `empty`, `unavailable`, `failed`) and a reason.
- The Assistant's Browser surface has an **Agent** view: the agent's tabs, a
  screenshot of the focused one, and the refs it can act on. The person sees the
  same page the agent is using.

Screenshots are stored in the uploads directory and shown through the existing
token-checked asset route; the model gets the page's structure and the file name,
never a claim that something "looks right".

### Update from the footer

- The status bar now names the running version. When GitHub has a newer signed
  release it offers **Update to x.y.z**; when there is none, the version is a
  button that checks again.
- `update.check` now chooses the asset for the program that is running: the
  window program (`devd-desktop-...`) when devd is in its window, the command
  otherwise. `Check` reports which one it is.
- `update.apply` verifies the signature and checksum as before. For the window,
  it stages the verified download and starts a small updater that waits for devd
  to exit, keeps the old program as `.before-<version>`, swaps in the new one and
  starts devd again. For the command, it replaces the installed command as before.

### Packaging: the download a person is told to use exists

The README told people to run `devd-setup-<version>.exe`, but the release script
only built a raw exe and a zip, so the file the instructions named was not on
the release. The portable zip also carried only the window and its WebView2
loader: no command, no launcher.

- `release-windows.ps1` now builds the installer when NSIS is present, and says
  loudly when it is not. The installer is the documented Windows path.
- The portable zip now carries `devd.exe`, `devd-desktop.exe`,
  `WebView2Loader.dll`, `launch.vbs` and a portable README. `launch.vbs` runs
  `devd setup` on the first start, then opens the window.
- The README and `docs/desktop.md` now name the real files, explain that the
  window needs `WebView2Loader.dll` beside it, and give the desktop log and
  `devd ui` fallback. `docs/platforms.md` states plainly that published builds
  are Windows x86_64 only.
- An unsigned release (no signed `SHA256SUMS.txt`) is not installed by the
  footer; it offers **Download x.y.z** and opens the release page instead.

### The pane can be driven, not just watched

- `browser.mouse` clicks at a point in the page (left or right button),
  `browser.key` types text or presses a named key, and `browser.scroll` turns the
  wheel. A link can be followed and a form submitted in the page the person is
  watching, in the same tab the agent acts on.
- Chromium runs with `--force-device-scale-factor=1`, so one CSS pixel is one
  screenshot pixel and a click lands on the point the picture shows.
- The tab strip carries the page's own title and icon, read from the document the
  way a browser reads them (`link[rel~=icon]`, falling back to `/favicon.ico`),
  and the address bar follows the real page rather than the last navigation.

### Providers are put back on their vendor's own protocol

- A provider stored against a known official host is read back on its vendor's
  kind and base path. The live failure was a Gemini provider saved as
  `openai-compatible` against the native `/v1beta` URL: it sent
  `Authorization: Bearer` where Gemini wants `x-goog-api-key`, and the resulting
  401 read as a bad key. A custom host — a proxy, a local server, Azure — is left
  exactly as the person configured it.
- Gemini is a first-class `gemini` kind and its models are listed with
  `x-goog-api-key`.
- Zhipu AI (GLM) is removed as a preset and from the window's brand marks.
- The maintained model catalogue is gone. A preset now only says how it finds
  models (`anthropic`, `openai`, `ollama`), so a hand-written list can no longer
  disagree with what the vendor actually serves.

### The Inbox stops asking about a pause that is over

- Approving a plan, resuming a run, accepting a task and deciding the task that
  paused a run now answer the Inbox message that asked for that action. It used
  to stay open, so the same pause was listed again after the run had moved on.
- Home shows a paused run as **Run blocked** with **Open the run**, instead of a
  generic **Question** whose only action was *Answer*.

### Verification

- The browser's URL fence, actionability table, result clipping and missing-
  browser error have unit tests; `cargo test -p devd-agent --lib browser` passes.
- The updater has a test that the desktop and command assets are different files.
- The footer has a UI contract test for the version, the check, the update
  action and the style.
- The Browser surface has a contract test that the pane drives a real page —
  click, key and wheel — and shows that page's own title and icon.
- `clippy -D warnings` had been failing on every push since 6 November: the step
  aborted at the first crate and hid everything behind it. Those findings are
  cleared across the workspace, so CI can pass again.
- Workspace tests pass.

---

## [0.27.6] - 2026-11-09 - Agents verify finished work and can really use images

### Completion is independent, not self-reported

The live Nairobi Unwind design run was resumed and observed. It had one designer task, a clean commit
`84ac57c`, and was paused by a person after acceptance. On resume it went straight to `done /
approve` with **no gatekeeper and no reviewed commit**: a task that changed files verified its own
work, and `completion::check` only required an independent check for implementer/docs-like roles.
That is the failure the completion invariant exists to prevent.

- Any task that recorded a file change now requires an independent final check, whatever its role.
  A designer, architect or researcher deliverable is no longer approved on its own lint.
- A read-only answer or research note still does not create a pointless two-agent run.
- `final-verify` is added for the same rule, so a file-changing run cannot reach `done` without it.

### Image understanding is wired, not claimed

- The model catalogue now stores what a provider listing says about tools, images and reasoning
  (schema v24), and the provider listing wins over a name guess.
- More vision model families are recognised (`*-vl`, `*vision`, Pixtral, LLaVA, InternVL, GLM-4V,
  Qwen-VL, Llama 4, Gemma 3, and others). DeepSeek remains correctly text-only.
- A model whose image support is unknown is **given the images** rather than silently sent the path.
  If the provider rejects them, the turn is retried once as text and the person is told which model
  could not read the picture, instead of losing the turn or pretending.
- The prompt tells each model exactly what it can see for that turn.
- The composer marks models `vision` / `no images`, and warns before sending a picture to a
  text-only model.

### Skills and tools can be discovered without abandoning the task

- `skills.discover` searches devd's skills and the skill folders of other agents installed on this
  machine (Claude Code, Cursor, Codex, OpenCode, ...).
- `skills.read_external` reads one of those skills in place, from a folder inside the person's home,
  without executing it or installing it globally. Installing for future turns remains a human-only
  `skills.import`.
- The agent prompt now says to search before claiming a capability is unavailable, and the same rule
  applies to tools and model capabilities.

### Verification

- Live run `5157aa4f…` resumed with its history intact and completed; the missing-gate case is now a
  regression test.
- Image content is asserted at the wire level for Anthropic, OpenAI-compatible and Gemini adapters;
  provider metadata and family detection have tests.
- Skill discovery/recovery has tests for a local external skill and for refusing a folder outside the
  home.
- Workspace tests pass, including **123** UI contract tests, **178** agent tests and **269** core
  tests. The Nairobi Unwind Content Assistant suite passes **168** tests, including new poster
  preview and PDF-document tests.

---

## [0.27.5] - 2026-11-09 - The Browser and Terminal read like a desktop app

### What changed

The Browser and the Terminal were three stacked bars of outlined mini-buttons: a tab strip, a
title row and a separate address row, with the active tab almost invisible. They now follow the
Claude Code desktop surfaces:

- **One compact chrome, two rows.** A tab strip with a filled active tab, a close action on every
  tab, one new-tab button, and the surface actions at the right. Below it, one context/address row.
  The old title row and the separate "Open" button are gone.
- **Browser.** Reload, a globe, the `Browser` name, a live/off dot, an address field with the default
  port hidden (`https://nairobiunwind.test`, not `:443`), and a go arrow. A wide pane names
  **Open externally**; a narrow pane drops the redundant identity. Empty states are quiet: a glyph,
  a title, one sentence, and primary/secondary text links instead of heavy buttons.
- **Terminal.** Tabs, new, copy and close in the strip; one folder row; the xterm palette matches the
  app, with an amber cursor. Copy strips ANSI escape sequences and copies the selection when there is
  one, otherwise the screen. A full split terminal drops the folder row because the prompt says where
  it is.
- **A new browser tab stays empty** instead of immediately reopening the project's site; the site is
  one link away.
- **Responsive at all five widths** (1920, 1440, 1280, 768, 390). Narrow panes get finger-sized
  controls; the tab strip scrolls horizontally; nothing overflows.

Measured on the installed build at 1920 × 1080, 1440 × 900, 1280 × 800, 768 × 900 and 390 × 844:
two chrome rows, no horizontal overflow, and the same interactions (new tab, navigate, switch tab,
close tab, new terminal, copy, close, drag the split) on every size.

---

## [0.27.4] - 2026-11-09 - The Assistant composer has no status pill

The Assistant's composer carried a `Ready` / `Working` / `Waiting` pill above it. The conversation's
header and the stop control already say what it is doing, so the pill is gone. Home keeps its
Assistant/Agents mode switch, which is a control rather than a status.

---

## [0.27.3] - 2026-11-09 - Every screen paints once, then patches

### The measured cause

Navigating to a screen wrote `#main` whole, then a loader or a timer wrote it again when its data
arrived, and often a third time from an independent read. Measured on the installed app at 1440px:
Assistant 2 full rebuilds, Team 2 plus a whole run-detail replacement on every revisit, Team room 3,
Models 3, Diagnostics 3, and every other screen 2. Clicking the screen already showing still rebuilt
it on Projects and the Machine overview. Team reset `TEAM.lastHtml` on every render, so even the
1.2 s timer rebuilt the entire run. Projects set every Git status to `null` on refresh, so the facts
vanished for ~1.5 s and then reappeared. The crumbs, statusbar and health chip were assigned on every
poll even when they said the same thing.

### The fix

- **One paint per screen entry, a patch afterwards.** A new screen is written whole once. Every later
  paint of the same screen — a read landing, an independent loader, a live tick — compares the markup:
  identical markup is not touched; changed markup is walked against the live DOM with `morph`. The
  terminal keeps its whole-subtree swap because xterm owns it.
- **Live subtrees are owned by their own painter.** The conversation, the run detail, the header,
  the project card, the workspace surfaces and the attachment strip are marked `data-morph="skip"`,
  so patching the frame can never wipe the conversation.
- **Team reuses its frame and keeps its runs in memory.** The frame is written once; the chosen row
  is highlighted before the read; a run seen in the last two minutes paints from memory and refreshes
  behind the reader; a different run is one instant swap, not a morph and not an animation.
- **The Team room loads in one pass.** `team.stats` joins `team.roles` in the same read instead of
  painting the roster twice.
- **Projects keep their Git facts while refreshing.** A refresh replaces the list, not what it
  already knows.
- **The conversation cache stays complete.** An incremental poll merges into the complete snapshot
  instead of storing its delta as the whole conversation; switching back reuses the markup painted
  from that exact snapshot instead of re-rendering every turn.
- **Crumbs, statusbar and the health chip are rewritten only when they change.**
- Opening another project or site starts at its top even though the destination screen is the same.

### Measured after (installed app, 1440px)

| | Before | After |
|---|---:|---:|
| Screen entry: full `#main` rebuilds | 2–3 | **1** |
| Same screen clicked again: rebuilds | 1–3 | **0** |
| Idle DOM churn (25 s Home, 12 s Sites/Projects, 20 s Team, 8 s Assistant) | periodic | **0** |
| Team run switch | detail replaced, animated | **1 instant swap, 0 layout shift, 10–160 ms** |
| Conversation switch, 672-event thread | full rebuild | **first open 365 ms, cached revisit ~150 ms** |
| Projects status facts on revisit | blanked ~1.5 s | **kept** |

Every route, all 12 project tabs, the site tabs (Overview, Environment, Deploy, Cron, Queue, Logs,
Redirects, Web server, Database, Mail, Preview), the conversation list, the Models form and the Team
form were exercised: no stuck loading state, no error dialog, no full `#main` rebuild after entry.
The one long first load is Tools: `toolchain.list` probes every tool (3.1 s, cached by the server for
30 s), and the screen says so while it waits.

---

## [0.27.2] - 2026-11-09 - Sending a message never rebuilds the Assistant

### The measured cause

Sending to the open conversation replaced `AI.thread` with a bare object (no project, title or
team), then called `render()` and, after the poll, `renderAssistant()`. That was **two full
`#main` rebuilds per send**. Each rebuild removed `#chat-stream[data-thread]`, so `paintChat` took
the "fresh conversation" branch and replaced the whole stream, then played a 180 ms
`thread-swap` fade/slide — **twice**. The bare thread also dropped the project card for a frame, so
the conversation column widened and snapped back. Switching a conversation did one full rebuild and
one animated stream swap.

### The fix

- **A send to the open conversation patches it in place.** The thread, its project card, its scroll
  and its composer stay; the sent message and the working state arrive through the normal poll.
- **A new conversation** takes its id and project and reads its first page without a full render;
  only a send from another screen crosses to the Assistant.
- **Switching conversations reads the destination first.** The current conversation stays visible
  while every page is fetched, then one instant swap paints the new one. The chosen row is marked
  while it loads. No empty intermediate page, no full rebuild.
- **The `thread-swap` entrance animation is gone.** A conversation change is one frame, not a fade.
- **The project card column is reserved for every existing conversation**, so switching between
  chats cannot change the column width; a chat with no project shows its conversation card.
- **Composer status, attachments and the send icon are rewritten only when their content changes**,
  so idle polls no longer blink them.

Measured with a DOM/layout observer on a 23-turn conversation: sending now performs **0 full page
replacements, 0 stream replacements, 0 layout shifts**; switching performs **0 page replacements,
1 instant stream swap, a 0.0006 layout shift**. Before: 2 page replacements and 2 animated swaps on
send, 1 + 1 on switch.
- A UI contract test now pins the no-rebuild send, the preloaded switch and the removed swap
  animation.

---

## [0.27.1] - 2026-11-09 - The conversation opens on its current message

### The window

- **Opening a conversation shows its current messages immediately.** `chat.get` returns 500 events
  per page; the window used to paint only the first page, so a long conversation looked frozen on
  old history until the next timer tick or a sent message. It now reads every page and paints once.
- **No Home flash on a linked conversation.** The address is read before the first paint. The
  capability-summary load no longer repaints the Assistant, and the post-load paint is in place
  instead of a full rebuild of the screen.
- **The Workspace section is gone from the project card.** Browser and Terminal live in the
  conversation menu; the card no longer repeats them.

### Orchestration

- **A structured approval wins over the words around it.** A report that passed `verdict: approve`
  was still sent back because its prose contained the word "changes" (the live content-assistant run
  hit exactly this). The explicit verdict is the conclusion; prose is only the fallback.
- **A final check that asked for changes can be run again.** `team.retry` now reopens a finished
  gatekeeper whose verdict is not an approval, so fixed work can be re-verified without accepting it
  by hand. The paused-run need offers “Check the current revision again” before the dangerous accept.
- **A nudge is not an attempt.** The stall heartbeat no longer increments `attempts`, so a long task
  cannot exhaust its retry budget by being watched. A task whose last event is a tool call is not
  nudged or stopped: a running test or build is work.
- **Machine completion requires the delivered revision.** An approving run now pauses if the tree
  moved after the check or has uncommitted changes; the verified work must be committed on the branch
  (never merged). A person's explicit accept is still `accepted_by_hand` and remains visible.
- **Tests:** the failing-verification/accept test, the structured-approval precedence, the attempts
  counter, the lease/fencing and baseline store tests, and the updated UI contract.

---

## [0.27.0] - 2026-11-08 - A run finishes only when an independent check approves it

### Completion is a verdict, not a task state

- **Every run now carries a canonical verdict** (`approve`, `changes_needed`, `blocked` or
  `accepted_by_hand`) with the exact commit it applies to. `conclude` refuses to set a run `done`
  unless an independent gatekeeper approved that revision, every task is terminal, every acceptance
  criterion passed and no approval is waiting. Three historical runs that ended `done` with a failing
  suite would now pause with `changes_needed`.
- **`team.report` accepts a structured verdict and criteria**, and the prose classifier understands
  explicit failures (`FAILS`, `3 failed`, `defects remain`) instead of only review phrases. A
  sentence that names a failing check can never be read as an approval; `0 failed` and `no
  failures` still pass.
- **A human override is explicit and visible.** `team.accept` can accept a check that asked for
  changes; the run finishes as `accepted_by_hand`, the summary says “Finished by hand”, and the
  override is never presented as machine approval.
- **A paused non-approving run asks one question**: the final check, the criteria it could not pass,
  “Read the final check” and a red “Accept the work as it is”. The Inbox and the run page read the
  same `needs` object.
- **An approval is for a revision.** If the tree moved after the final check read it, the run pauses
  and asks for a review of the current commit instead of accepting the old one.

### Verification is baseline- and criterion-aware

- **The run records its starting revision and the project's most recent own checks** before the first
  task changes anything, as a clearly labelled *recent* baseline. A gatekeeper is told to call a
  failure pre-existing only when the command and the result match; otherwise it is new or unknown.
- **Acceptance criteria are first-class.** The planner can submit `criteria` per task; writer tasks
  without one get an explicit criterion from their title, and the final check receives the run's
  definition of done. The run scorecard shows the verdict and the reviewed commit.
- **`models.price`** records a real input/output price per million tokens for an unpriced model, so
  `max_cost_usd` and the usage estimate are not silently computed from zero.

### Writers are isolated, and a review sees a fixed commit

- **Any writing role, not only `implementer`, now triggers a worktree** when another writer could
  run; the scheduler also starts at most one non-isolated writer per tick. The content-assistant
  overlap came from both gaps.
- **A per-project write lease with fencing** admits one non-isolated writer at a time and survives a
  crash through expiry; a stale holder is stopped when ownership moves. The lease row is kept across
  releases so the fencing value never goes backwards, and a lease table test covers it.
- **A review records the HEAD commit** it started with, and the gatekeeper prompt names that commit
  and the base revision. A review verdict is attached to the revision it read.

### Execution keeps moving

- **A turn or output limit is a checkpoint, not a wall.** The same task continues in the same
  conversation with a durable checkpoint (objective, last result, next action) instead of reporting
  blocked; checkpoints are written when a task starts, is nudged, retried or turn-limited.
- **Liveness is progressive**: a nudge after ~3 quiet minutes, a stop at the existing threshold, and
  an unfinished long tool call is treated as work, not as a dead agent. The watchdog no longer
  nags every tick between the two thresholds.
- **Failures carry a classification** (`transient`, `policy`, `incomplete`, `environment`,
  `pre-existing`, `unknown`) on the task, and `team.report` after acceptance is idempotent: it
  returns the authoritative state instead of “this task is not running”.
- **A resumed run always gets a driver.** Resuming a just-paused run used to race the old driver's
  exit and could leave the run running with nobody driving it; the driver restart waits for the old
  one to leave.

### Tooling and hygiene

- **Pint runs as `php vendor/bin/pint`**, which works on Windows and Unix, instead of launching the
  PHP script as an executable.
- **A wrong-capability hint names the right tool**: `git` points at `git.status`/`git.diff`/…,
  `curl`/`wget` at `web.open`/`docs.fetch`, and a shell at the allow-list or a project task.
- **Schema errors include a filled-in example** built from the capability's own schema, not invented
  fields.
- **Routine window polling is no longer audited.** `terminal.list`, `team.get`, `health.status`,
  `approval.list` and the other timed reads stop filling the audit table with hundreds of thousands
  of rows; mutations, agent calls and approvals are still recorded.
- **The run page has a scorecard**: verdict, verified/total, accepted by hand, retries, stalls and
  baseline failures, plus the verdict and reviewed commit on each task.

---

## [0.26.4] - 2026-11-08 - If you told it to, it asks once and does it

### Changed

- **Destructive project operations are approval-gated, not human-only.** `project.run_destructive`
  and `project.exec_destructive` can now be requested by the assistant: the person sees the exact
  task or command in the conversation and approves that one run. The approval is single-use, so the
  plan a person read is the command that runs. A task like `migrate:fresh --seed` no longer ends in
  "cannot be performed by an agent" with no way forward.
- **Interpreter one-liners take the same path.** `php -r`, `php artisan tinker` and `npx -c` are
  refused by `project.exec` with a pointer to `project.exec_destructive`, which a person can approve;
  the hard "an agent may never run this" wall is gone, and the exact argv is what is approved.
- **The assistant can steer a running team.** `team.say` is available to agents, so a correction the
  person gives devd reaches the plan or the tasks instead of being refused as "not available to
  agents". Pause, resume and retry were already agent-safe.
- **`fs.write` creates missing parent folders inside the project.** Writing
  `app/Services/ContentAssistant/Fetch/UrlFetcher.php` no longer costs a turn to learn that
  `create_dirs` was needed.

### Changed in the window

- **Browser and Terminal moved into the conversation menu.** The header keeps search, Evidence and
  Continue; the two workspace surfaces are menu items, with a check when one is open. The menu reads
  surfaces first, then the conversation's housekeeping (Rename, pull request, settled), then Delete.
- **"Needs you" is the app's banner, at the bottom of the conversation.** The run page states the
  current need with the same tone icon, title, words and actions as every other banner, after the
  conversation and before the composer. The run's actions (Evidence, Pause/Resume/Cancel) sit in one
  row at the bottom, so they stay in reach as the log grows.
- **A long run title is clipped quietly.** The conversation header no longer appends
  "… N more characters (full output is in the transcript)" to a title; it ends in an ellipsis.
- **The Assistant's empty state has no glow.** The atmosphere belongs to the page, not to one block
  inside it.

### Added

- **Tabs in the workspace surfaces.** Browser and Terminal each carry a Claude-Code-style tab strip:
  switch tabs, close one with its ×, and open another with +. Terminal tabs are separate server
  shells in the same project (they keep running) and are named **Terminal 1**, **Terminal 2**; blank
  browser tabs are **New tab**, **New tab 2**, and a tab that opens a known site takes that site's
  name. The active tab has no bottom border, so it reads as one surface with the panel beneath it.

---

## [0.26.3] - 2026-11-08 - A new chat has no empty card

### Changed

- A new conversation no longer shows the empty **Conversation / No project / Open projects** card. The
  prompt and its starting points stand on their own until the conversation belongs to a folder. An
  existing conversation that still has no project keeps the explanation and the link.

---

## [0.26.2] - 2026-11-08 - One hover, one inset

### Fixed

- **Every hovered list row uses the same rounded, inset band as a `uiRow`.** The API registry,
  stack picker, notes, files, mail and sites no longer paint a square band from edge to edge. The
  row stays transparent and a shared layer behind it supplies the tint, so text never moves.
- Table rows round their first and last cell, and the model roles table inside the **Which model
  does which work** fold keeps the same 16px inset as a panel body.
- **The three workspace toggles retire on a large window.** At 1440 and wider the project card is
  open beside the conversation and its Workspace rows already open the Browser and a shell, with
  Environment being the card itself. Below 1440 the card is a drawer, so the header toggles stay.
- **The conversation header reads state, context, actions.** Status chips come first, then Back to
  request, See the run and Chapters, then the surfaces, search, Evidence, Continue and the menu.
  Chips sit on the same 28px row as the buttons, and a specialist's subtitle no longer repeats a run
  title that is already the heading.

---

## [0.26.1] - 2026-11-08 - Containers, folds and rules

### Fixed

- **Home's needs cards stay in the reading column.** An implicit grid column sized itself to the
  longest title, so the paused-run banner and service cards ran past the composer and off the page.
- **One inset for a container.** A panel's head and body now share the same left and right edge.
  Flush bodies inset ordinary content and let only tables use the full card width. Forms can no
  longer make a card wider than its page.
- **The team room reads as one card.** Its head, charter and project context share one edge, and the
  internal vertical rule between them is gone.
- **Summary bands carry no rules.** Projects, Sites and every other screen show readings separated
  by space, not by top, bottom and column lines.
- **The run toggle is a switch again.** Generic checkbox sizing had shrunk the 36×20 control into a
  white knob over a tiny ring; the schedule form and MCP dialog now show the real track.
- **Every collapsed section looks the same.** Models, Skills, Diagnostics, run reports, agent notes,
  mail setup and event arguments use one chevron, one label size, one colour and one indent.
- Wide tables scroll inside their own frame on a phone instead of pushing the page sideways.

---

## [0.26.0] - 2026-11-08 - The workbench, with surfaces

### Added

- **Browser and Terminal surfaces in the Assistant.** The conversation header has three toggles —
  Environment, Browser and Terminal — and the project card's Workspace rows offer **Open browser**
  and **Open terminal**. The Browser is a sandboxed preview of the project's site with an address
  bar, reload, open-externally and honest empty, stopped and no-site states. The Terminal is the
  same server-owned shell and xterm as the project's Terminal tab, opened without leaving the chat.
- At 1600px the Environment card and Terminal can sit side by side; the Browser takes the wide
  column. Both surfaces stack in one work column, split 58/42 with a draggable divider. Below
  1280px they become sheets over the conversation; a full reload starts with them closed.
- Surfaces belong to the chat's project. Switching chats re-points the browser; a shell from
  another project says so and offers a shell in the current one.
- The old **Terminals** section and the “Terminal here” row are gone.

### Changed

- **Graphite, atmosphere, spectral colour, texture, precision.** The palette is true graphite
  rather than navy; the canvas carries two static radial gradients and a 160px SVG grain tile, with
  no shader, animation loop or WebGL. Blue is the action colour, cyan the live signal, violet a rare
  spectral note.
- A shared component pass: exact-property transitions, pressed buttons, focus rings, tonal panels,
  quieter menus, precise inputs, one rich-content Markdown scale, refined code and table treatment,
  and far fewer list borders — rows are separated by space and hover, tables keep a header rule.
- The Assistant conversation: calmer user bubbles, assistant text as reading text, better heading
  and list rhythm, code labels and copy, and streaming-safe surfaces.
- The Inbox groups Needs you, proposals and waiting decisions with a clear kind; Updates and Earlier
  are compact lists rather than fences of lines. Runs keep the execution record as a timeline and
  mark Cancel as destructive.
- Conversation groups now say **3 requests** instead of an unexplained number, with the total
  conversation count in the tooltip.
- **MCP no longer prints machine-specific settings paths.** The coding-agent list says what it is and
  what to do, without a Windows path leaking into a screen used on Linux and macOS too.
- **Outputs says what it is.** It lists files this conversation wrote or edited, with `edited` and
  `created`, and explains that Changes above is the working tree now — so a clean tree beside a long
  Outputs list is no longer a contradiction. Source files use the file mark, not the terminal glyph.
- Home says things **need a look**, leaving “Needs you” for the actionable Inbox decisions.
- Selecting a chat paints its row's title and project immediately, clears the previous run and
  chapters, rejects a late response for a chat the person has left, and opens a deep link by id even
  when the list has not caught up.
- Home keeps its composition — greeting, assistant composer, project chips, attention, Your work —
  and receives the type, surface, spacing and atmosphere refinements rather than a new layout.

### Fixed

- The Assistant work surface fills the content height above the status bar; the both-open split no
  longer overflows by the divider's height.
- The work column has one continuous left edge, and its header aligns with the conversation header.
- The Environment panel is a drawer below 1440px with a visible close action.
- `docs/design.md` now documents the shipped tokens, atmosphere and workspace surfaces.

### Verified

- Workspace test suites pass, including 103 UI contract tests.
- Browser screenshots at 1024, 1280, 1440, 1920 and 2560 for Home, Assistant, Inbox, Runs, Projects,
  Sites, Machine and Diagnostics, plus 390px overflow checks and all four surface states.

## [0.25.6] - 2026-11-07 - The tray tells the truth

### Fixed

- **The tray menu is no longer a startup snapshot.** It is rebuilt from the same capability API the
  window uses, so a service row names its real state (`running`, `stopped`, `running, not
  answering`) and only offers Start, Stop or Restart when that action would do something. The menu
  refreshes every thirty seconds from one `service.list` read, and only when something visible changed.
- **Start All and Stop All use `stack.up` and `stack.down`**, so they start and stop in dependency
  order and never touch a process devd does not own.
- **Per-service tray actions call `service.start` / `service.stop` / `service.restart`**, which are
  the audited capabilities that do not wait for a second approval the tray cannot show.
- **Active Quick Shares are in the tray**: each open link shows its real public address, can be
  opened, and can be stopped; the row also says how many are active.
- **Health is always reachable from the tray**: recent self-healing incidents are listed with what
  was done, a healthy state says so, and Diagnostics is one click away.
- **Services offers Add or register a service…**, which opens the screen that installs or adopts one
  rather than pretending a menu click can do it silently.
- Restoring the window now goes to `/diagnostics`, the tray's Runtimes entry to
  `/server/runtimes`, Quick Share to `/sites`, and Services to `/server/services`; three older
  entries pointed at addresses the router does not serve.

### Verified

- A **relocation test** moved a devd home from `C:\dthome-026` to `C:\dthome-moved`, removed the
  old folder, and ran the installed binary with `DEVD_HOME` pointing at the new home:
  `portability.status` reported `moved: true`, the previous home, and the one missing project;
  `portability.repair` rewrote exactly that project; `project.list` then showed
  `C:\dthome-moved\www\portable-demo`.

## [0.25.5] - 2026-11-07 - The whole machine in one answer

### Added

- **`team.overview` is the Assistant's orchestration view.** One read returns every recent run in
  every project with its counts, every specialist with its role, conversation, model, worktree,
  scope, dependencies, attempts, evidence, **tokens spent and cost**, and last activity, which roles
  are busy, the projects grouped with their open work, and the one question each run would ask a
  person. The Assistant is told to read it before planning or assigning, and that it is accountable
  for the outcome whichever specialist does the work — retry, reassign or do it directly instead of
  forwarding a failure.
- **The machine heals itself.** A supervisor in the process that owns the agents checks every minute
  and restarts a service a project needs (or one set to always) through the registry — audited like
  any other operation, respecting a person's "observe" decision — with three attempts inside fifteen
  minutes and then a recorded stop instead of a restart loop. `health.status` carries the recent
  incidents, and the status panel lists them under **Self-healing**.
- **Token discipline in the agent prompts.** Read narrowly (glob and limit), quote the line rather
  than the log, and read the task brief and the run's memory before investigating what earlier
  agents already found. Per-agent input, output and cached tokens and cost are in the overview.
- **Quick Share through ngrok.** A project's Share button (and the tray's Quick Share submenu) finds
  the project's site and local port, opens a verified ngrok tunnel, reads the real public HTTPS
  address from the agent's own log, copies it, and draws a QR code locally. The agent is managed
  under `tools/ngrok`; the authtoken lives in an owner-only file and reaches the child as
  `NGROK_AUTHTOKEN`, never in argv; regions are auto/us/eu/ap/au and are remembered. Several
  projects can be shared at once, each with its own record, and stopping one never touches another.
- **Portability.** `portability.status` reports where devd is, where it was last seen, whether the
  home moved, which known project and site paths no longer exist, and how secrets are stored;
  `portability.repair` rewrites the projects and sites that were inside the old home, and reports
  anything outside it. The Diagnostics page has a Portability panel for both.
- **A Laragon-shaped tray.** www (projects), Quick Share (sites), Services (start/stop/restart/logs
  per service), Start All / Stop All, Quick app, Tools, Terminal, Preferences and Exit — with the
  actions calling the same local API, and so the same permissions and audit trail, as the window.
- **Start at login in the tray.** The desktop writes the per-user Run key with `--start-hidden`, so
  signing in puts devd in the tray instead of a window, and unchecking it removes the entry.
- **Open agent runs in the tray**, next to Open Assistant and Open in browser.

### Fixed

- **The Assistant and Runs badges no longer double-count.** A specialist conversation is part of a
  run, so the Assistant badge now counts only the person's own conversations while Runs counts the
  team; the two agree with what each screen shows.
- **DeepSeek no longer refuses the assistant's turn.** A tool result whose call had been trimmed
  from the history was sent as a `tool` message without a preceding `tool_calls`, which
  OpenAI-compatible servers reject with HTTP 400. The request now emits a result only while its call
  is still owed, closes an unanswered call with a placeholder before the conversation moves on, and
  drops orphaned results. Covered by two wire-format tests.
- **Approvals an older policy queued for a read are withdrawn at startup.** `fs.search` and its
  siblings are reads now; a pending row from before that change was still offered in the Inbox as
  an actionable decision. It is cancelled once, with a line in the conversation, instead of waiting
  out its lifetime.
- **The Needs-you card is one calm card, not a warning stripe.** The 3px left bar and the
  all-caps label are gone; it uses the same hairline, tint and radius as banners and notices. The
  question is the title (with a fallback so it is never blank), the task key is the meta line, and a
  blocked report renders as markdown instead of one escaped wall. The run view no longer repeats the
  same question above the card.
- **The tray cannot be duplicated, and it comes back reachable.** A single-instance plugin makes a
  second launch focus the running window and exit (one tray icon, one backend). Restoring from the
  tray reloads the page only when the webview is showing the browser's network-error page, so a
  hidden window never returns as "could not be reached" while its conversations are left intact.
- **Collapsible content is indented** under its heading on every screen, and nested disclosures step
  again.
- **Remove and Remove site on Projects are danger-coloured**, like every other destructive action.

---

## [0.25.4] - 2026-11-07 - One bubble

### Fixed

- **The Assistant and Runs icons show one indicator, not two.** A working conversation drew both
  the count badge and the small live dot in the same corner, which read as two bubbles. The badge
  is the indicator (blue working, amber waiting, red stopped); the live dot now appears only where
  there is no count, and CSS refuses a second circle if the markup ever allows one.

---

## [0.25.3] - 2026-11-07 - Nothing swallowed, nothing left unsaid

### Fixed

- **An unclosed code or visual block no longer eats the rest of the reply.** A model that forgets the
  closing fence used to have everything after it disappear into the last block; the rest is now
  rendered as the message it was.
- **The rail tells the truth about the Assistant and Runs from every screen.** Both read the live
  conversation and run state on a calm cadence: a blue dot while work is in flight, an amber count
  when something is waiting for a person, a red count when something stopped. The hint names it.
- **The tray menu is a whole app's.** It leads with the running version and address, then Open devd,
  Open Assistant, Open in browser, Open devd folder, Open projects folder and Open desktop log;
  closing the window hides devd to the tray and Quit is the explicit act.
- **"Needs you" says it once.** A long blocked report was used as both the question and the detail,
  so the card read `"X" is blocked: <report> — <report>`. The question now carries the first
  sentence (capped) and the detail the full text, once; the one-line status stays one line.
- **The last one-word alerts are sentences.** Deleting a run, a conversation, a message or a file,
  stopping the assistant, unpinning a reply, updating a conversation, sending a team instruction and
  recording a report each say what happened to what.

---

## [0.25.2] - 2026-11-07 - Work that finishes without you

### Fixed

- **Reply visuals render again.** A declared `chart`, `metrics`, `steps`, `timeline` or `details`
  block whose JSON sits on the opening fence line (` ```steps {"items": [...]}`) rendered as an
  empty card with a language name. The payload is read from the info line, the body or both, and an
  unparseable block shows its source instead of nothing.
- **The "Jump to latest" pill stays above the composer.** It was measured once from the bottom of
  the column, so a growing draft left it floating in the middle of the conversation. It now
  measures the composer's top edge and follows the composer as it resizes.
- **The composer has one stable action while devd works.** The primary slot is the red Stop button
  at every keystroke; typing no longer swaps it for Send. Enter still queues the draft for the next
  step.
- **A failed test is red everywhere.** A command that returned normally but reported `3 failed`
  now colours its step dot, its tool group and its summary red, matching the chip.
- **A run continues by itself.** A reply that reaches the model's output limit is continued in the
  same turn, a restart resumes work that was running, and the 40-step pause with "Say continue" is
  gone (240 is a runaway guard, not a work budget). Devd can also pause, resume and retry its own
  team run without asking a person to press the button.
- **Agents own their project.** Inside the project a conversation or task is scoped to, every
  agent may read, create, edit, move, delete, restore, stage, commit and run the project's own
  tasks without an approval prompt. Other projects are read-only; the Assistant assigns work to
  one with `team.assign`/`team.start` and the task's directory becomes that agent's boundary.
  The task directory, not a role title, decides file permissions, so a security or reviewer task
  that asks for the fix can implement it.
- **Machine and remote actions still ask.** Installs, services, hosts, certificates, force-pushes,
  deploys and database drops keep their approval and stay visible as waiting, while the agent
  carries on with everything that does not depend on them. Production mode keeps its own gate.
- The side card's **Changes** link opens the project's Git → Changes tab directly.
- The specialists disclosure is only visible when its row is hovered, focused or on a touch
  screen, instead of sitting on every row.
- The blocking **Needs you** decision is a contained card with its actions, rather than a label
  column stretched across the page.

### Added

- **A hidden-icons tray.** The desktop app keeps running when its window is closed; left click
  restores it and right click offers Open devd, Open in browser, Open devd folder, Open desktop log
  and Quit devd.
- **The conversation's right column is a real panel.** It mirrors the conversation list: the same
  head height, the same rail, and a divider on the side that faces the conversation. Under the
  project's branch it lists **Outputs** — the files the conversation created or changed, read from
  its own tool calls, with document, image, media and code marks — and **Terminals**, the shells
  running in that project, each one a click from its screen.
- **Effort in the model picker.** Low, Medium (recommended), High, Extra and Max. The level raises
  the reasoning effort a provider accepts and the room the answer has, so it is a real dial rather
  than a label.
- **Speech to text in the composer.** The microphone records 16 kHz mono locally and transcribes it
  with whisper.cpp on this machine; nothing is sent anywhere. The first press offers the one-time
  download, with its size, instead of failing after you have spoken.

### Changed

- **A goal starts working.** `team.start` and `template.run` make the plan and begin at once; the
  plan stays visible in the run, and a person can pause or cancel it. Production mode still holds
  the plan for a person.
- **Only the work that needs you asks.** In local mode, reads, writes, edits, deletes, project
  commands, dependency installs and an in-project `artisan`/`script` call run through; remote
  operations (deploy, push, ssh) and destructive machine work still ask. A blocked task is retried
  inside the same task before it becomes a question.
- **Expired approvals never reach the Inbox.** A pending approval past its lifetime is not listed,
  and a decision on one that expired while it was on screen withdraws the card with a plain
  sentence instead of an error.
- **Toasts are sentences.** Every toast is capitalised and ends with a stop, and the capability
  messages behind them were rewritten to name the thing they changed.
- **Rounded surfaces share one scale.** Hover fills and interactive rows use the fixed radius
  tokens (`--radius-sm`, `--radius`, `--radius-lg`, `--radius-xl`) instead of ad-hoc pixel values;
  the conversation and run lists now round their hover wherever they are drawn.

### Performance

- **The interface ships compressed and revalidates.** The script, stylesheet and vendored runtimes
  are gzipped once and tagged with an ETag, so a reload is a 304 instead of a megabyte.
- **Diagrams, maths and the terminal runtime load on demand.** Mermaid (2.5 MB), KaTeX and xterm are
  fetched only when a screen shows a diagram, a formula or a shell, which removes most of the
  first-paint parse time.
- **The health check no longer walks the whole database.** It ran `PRAGMA integrity_check` on every
  poll, which took ten seconds on a database with history and made the status chip — and anything
  queued behind it — feel slow. Health now reads the schema version; the deep check belongs to
  `devd doctor`, where it was asked for.
- **Service probes run together.** The Services screen asked each service in turn and waited for the
  sum of their timeouts; the independent probes now run at once, and the answer is shared for 30
  seconds and discarded by any change.
- **API answers are gzipped.** Capability lists, audit pages and health are a quarter of the bytes
  on the wire; the live event stream is left uncompressed so it still arrives at once.

---

## [0.25.1] - 2026-11-07 - A conversation list that holds its shape

### Fixed

- **The conversation list keeps its margins while you click through chats.** Opening a specialist's
  conversation replaced its entry in the list with a copy that did not carry the team link, so the
  specialist dropped out of the request it belonged to and was drawn with a different indent. The
  open conversation is now merged into the list entry, a request and its specialists are one keyed
  item the live patcher cannot pair up wrongly, and only the specialists' list indents them.
- The header no longer offers "Continue" on a run that has already finished.

### Changed

- **Charts, metrics, steps, timelines and details are drawn as coloured artifacts** on a dark
  surface in a named data palette, with a takeaway and source line on charts. Code blocks name
  their language, diffs are green and red, and numeric table columns line up on the right. The
  assistant prompt, the `interface-contract` skill and `design.manifest` describe the blocks, so
  agents use them.

Releases remain unsigned: there is no signing key on the build machine, so `devd update` will not
install them. Download the build and replace the files by hand.

---

## [0.25.0] - 2026-11-07 - A team you can watch work

### Changed

- **Navigation is a rail and a context panel.** The top bar and the section tabs are gone:
  a rail says where you are, a panel lists what is in that area (its pages, its projects or
  sites, or the conversations and runs, grouped by project), and the page starts at the top
  of the window. The panel folds over the page below 1024 px; phones get a bottom bar.
- **Graphite and one blue**, one type scale, one icon line weight, and semantic colour names
  (`--color-primary`, `--color-base-100`, `--color-success`...) for new work.
- **Conversations read as a team at work.** Each role has its own mark; steps say what was
  done to what ("Read Meetup.php", "Ran test ... 41 passed"); edits show their diff; a finished
  turn folds its work behind "Worked for 12m 55s" with result tiles; a specialist's
  conversation sits under the request it works for.
- **Plain words for operations.** Activity, Home, approvals and evidence name an operation
  the way a person would ("Start a new conversation from a message"), not by its id.

### Added

- Search inside a conversation (Ctrl F), including folded steps and reports.
- A project card beside a conversation: branch, changed files, and lines changed split into
  code, tests and docs, counted from the real patch.
- `team.stats`: how the team is doing, per role and project, from every task's record:
  accepted on proof, rework, reviews asking for changes, verification pass rate, time to finish.
- `models.usage`: tokens by model and by member, and the heaviest conversations.
- Built-in skills `agent-browser`, `agent-desktop`, `hyperframes` and `stageshow`; these are
  instructions, not tool-catalogue entries. FFmpeg remains the only media tool.
- Long code blocks fold; a "Jump to latest" pill appears when you have scrolled up.
- **One design system, applied in the product.** Shape, size and depth tokens
  (`--radius-selector`, `--radius-field`, `--radius-box`, `--size-selector`, `--size-field`,
  `--border`, `--depth`, `--noise`) join the semantic colours; every existing control uses them.
  A 68-component reference review and the existing-component audit are recorded in
  `docs/design.md`; the principles and token contract are in the same document.
- **An inline chart an agent may produce.** A fenced `chart` block carrying a small JSON object is
  validated (labels, finite non-negative values, at most 12 bars) and drawn with the shared
  stylesheet inside the conversation. Invalid data stays a code block.
- **An interface contract for agents.** `design.manifest` returns the Markdown blocks an agent may
  produce, the components it must not fabricate, the token names and the refused constructs. The
  `interface-contract` skill puts the same rules in front of the agent while it works.
- **Home is a launchpad.** One centred column: the ask box, the projects you work in as one-click
  targets, what needs you as cards, your work as two-line rows, and the machine as one quiet status
  line. The project picks wrap on a phone instead of hiding behind a sideways scroll.
- **A conversation list that reads like the work.** Two lines per request: title, then the
  specialists' faces and count, working / waiting / stopped / settled, the PR number and the time.
  Specialists fold under their request; settled conversations sink.
- **Rich agent replies, offline.** Math and diagrams are rendered by vendored runtimes, with no CDN:
  KaTeX, Mermaid, and declared `chart`, `metrics`, `steps`, `timeline` and `details` blocks. Code and
  diffs are highlighted in the application's own colours; the agent prompt and the interface
  contract teach the formats.
- **An offline outbox.** A message written while devd cannot be reached stays in the browser with a
  visible state (waiting, sending, not sent), is retried when the connection returns, and is
  delivered once because every message carries a client id the server accepts once. Home and the
  Assistant share one draft.
- **Pull requests and settled conversations.** A conversation can be linked to a pull request, and
  settled or reopened; the list and header show both. `chat.link_pr` and `chat.settle` are the
  capabilities behind the controls.
- **Incoming hooks.** `automation.*` stores a token, an event, a filter and one action;
  `/hooks/task/<token>` accepts a GitHub-shaped `pull_request` or `push`, starts a team run, messages
  the assistant or posts to the Inbox, and records the last result. Team > Automations has presets and
  rotates the address.
- **Usage that reflects the bill.** Cache read/write tokens are recorded per call for Anthropic,
  OpenAI-compatible, Gemini and DeepSeek; spend uses the price in force and the provider's cache
  multipliers. A provider limit now pauses the conversation, says when it will continue, and the
  guardian resumes it after the reset.
- **Faster catch-up.** `chat.get {brief:true}` trims tool output to head and tail, `chat.result`
  fetches one step's full output when it is opened, and idle polling is reduced.

### Fixed

- **The diagnostics status icon sits on its sentence.** The green check in "Healthy, 2 optional
  notes. Checked just now..." was drawn in the top corner of the larger notice; it is now centred
  on the first line.
- **A new conversation says only what matters.** "What are we working on?" shows the project, the
  model and whether changes are asked about first, without the instruction on choosing a project,
  the provider's remote host, or the paragraph about auditing.
- **Adding a folder is one clear step.** The picker opens on a real folder, lists only folders,
  takes a typed path on Enter, keeps its confirm button disabled until there is a folder to use,
  and says "Add this folder" or "Browse..." consistently with Clone and Import.
- **The Diagnostics registry is explained.** The cloud of group-count pills is gone; the panel says
  what the registry is for and links to API and tools, where the same list can be searched.
- **Form buttons reflect the form.** Clone, Start a member, Save a rule and Create the schedule stay
  disabled until their required fields are filled, instead of accepting a click and answering with
  a warning.
- **Colour is named by role throughout.** Every component rule uses `--color-primary`,
  `--color-base-200`, `--color-success` and the rest of the semantic set; the palette values and the
  app background are unchanged. A contract test keeps new rules on the same names.
- **A conversation can be renamed** from its header (`chat.update`), so a title the agent chose can
  be corrected without starting a new conversation.
- **Home has dictation too.** The composer's microphone was only on the Assistant screen; both now
  record, transcribe and insert at the cursor, and Home's send button wakes when the text arrives.
- **Usage shows estimated spend** where the model catalogue has a price, with the unpriced
  conversations counted separately instead of hidden.
- **Projects agree with their own summary.** A project with a site showed "no address" while the
  tiles counted it as exposed; the row now shows the site name and keeps the Site action.
- **A "Plan ready" message asks for a decision, not an answer.** The Inbox and its sheet offer
  "Review the plan"; the free-text answer box belongs to agent questions, not approvals.
- **The Environment page has one setup action.** The empty Services block no longer repeats the
  header's primary "Set up devd" button.
- **nginx accepts many local names.** The rendered http block sets
  `server_names_hash_bucket_size 128`, so `nginx -t` no longer fails before the server starts.
- **Service logs are not empty.** nginx writes its error log to the file the Logs sheet reads, and
  nginx/MariaDB fall back to the file they actually write. A refused `nginx -t` is appended with a
  timestamp.
- **An install no longer says it can destroy work.** Only a destructive plan gets the red warning; an
  admin change says it changes the machine and that nothing is removed unless the steps say so.
- **Dropdowns survive a page refresh.** Choosing an option resolves the control that is on screen
  now, by id or label, instead of writing to the detached select from before the refresh. A contract
  test now fails if a select has no handler or a `data-change` value has no action.
- **A handoff's conversation appears without a refresh**, on every screen, and the Inbox stays
  in sync. Live events for any conversation or run now reach the window.
- **Reading a conversation no longer jumps.** Updates patch the page in place, so the scroll
  position, opened cards and loaded images survive them.
- **A reviewer's "changes needed" is acted on.** The findings go back for a fix and a re-review,
  at most twice, then the person decides; before, the run finished anyway.
- **Screens load in about 2 s, not 5-15 s.** A new event stream no longer replays all history;
  Git state loads in the background four at a time; machine probes are shared and cached until
  something changes.
- The person's own message was drawn as a note from the team.
- The Sites table's headings now sit over their columns.

---

## [0.24.1] - 2026-11-06 - The assistant hears back from its team

### Fixed

- **A team run reports into the conversation that started it.** The run already knew how
  to bring its plan, a pause or its result back to the conversation that asked, but only
  when the model passed its own conversation id to `team.start`, and a model does not
  always. The harness now attaches it, always to the conversation the call came from, so
  the assistant comes back with the outcome instead of the result landing only in the Inbox.
- **The team's note is not drawn as you.** The report reaches the conversation as a
  message; it is now shown as a note from your team, with the raw report folded away and
  the assistant's answer after it, instead of a "You" bubble.

---

## [0.24.0] - 2026-11-06 - Calmer pages, real uploads, mail that knows its site

### Added

- **Preview loop.** A one-file helper (`devd-inspector.js`, written by
  `site.preview_helper_install`) lets a site's Preview tab show the page's console
  errors and failed requests, pick an element, and send the page address, the element
  and the errors you leave ticked to an agent. It does nothing outside the preview
  frame or on a non-local address, and devd never edits the layout: it reports the one
  line to add (`site.preview_helper` reads the state).
- **Mail tied to its site.** Each message names the site it came from when that can be
  told, and how it is known: an `X-Devd-Site` header (proof), a tag, a sender on the
  site's hostname, or a link back to it. The Mail screen filters by site, every site
  has a Mail tab, and `devd mail list --site` / `mail.inbox` take a site.
- **Pictures reach a model that can see.** Attached pictures go to Anthropic,
  OpenAI-compatible and Gemini models as images, not only as a path; only files in the
  uploads folder whose bytes really are a picture, the three most recent.
- **Attachments.** Real thumbnails, paste a screenshot, a drop zone, per-file upload
  and retry states, files kept per composer, drafts that survive a reload, and files
  shown in the conversation they were sent in. Uploads are checked by their own bytes
  (programs and fake pictures are refused), kept 30 days, and read back by name with
  the token.

### Fixed

- **Open sections stay open** when a page redraws, and a page is not redrawn under a
  field that holds unsaved input.
- **Home refreshes** while its composer has focus, and repaints when what needs
  attention changes.
- **Boot.** A needed service with no process yet is "Starting" for the first 90 seconds
  after devd opens, not an error card.
- **Run templates** can be chosen: the new-run form was redrawn every 1.2 s.
- **Tables.** One style for every data table; the Actions head is a real table cell;
  heads are centred between their lines; numbers right-align.
- **Buttons in a row** are spaced wherever they appear, not only inside a sheet.

---

## [0.23.1] - 2026-11-05 - Table heads stay on one line

### Fixed

- **Table heads stay on one line.** The shared rule no longer lets header text
  wrap; the action column has a minimum width, so "Actions" cannot break into
  one letter per row. Applies to models, roles, skills, diagnostics, tasks,
  evidence, operations and Markdown tables.
- **No duplicate roles title.** The "Which model does which work" disclosure
  already names its section; the repeated heading above the table is gone.

---

## [0.23.0] - 2026-11-05 - One navigation, one tab language, one table

### Changed

- **One rail, seven places.** Home, Projects, Sites, Assistant, Agents and Models are
  top-level destinations, with Environment, API and tools, Activity and Diagnostics
  under Machine. Sites moved out of the Machine menu and now lives at `/sites`.
- **The terminal belongs to a project.** It is the project's own tab, lists only
  the shells started in that folder, and is no longer a destination in the rail.
  Opening a system terminal is still in the project's More menu.
- **Mail is read beside the sites.** It is a tab on Sites, where the apps that send
  the mail are already listed, rather than a seventh place of its own.
- Schedules, Skills and Notes keep their Agent addresses (`/agents/...`); links to
  the old `/schedules`, `/skills` and `/shared` addresses still open them.
- **One tab control everywhere.** Section tabs, project groups and sub-tabs, site
  and remote tabs, mail views, activity filters, Form/JSON and terminal sessions
  share the same height, radius, type and raised current state. Filters and tabs
  no longer look like two different controls.
- **A single terminal has no session tab.** The panel head already names the
  folder and shell; session tabs appear only when there is more than one.

### Fixed

- **Provider actions stay in the card header.** Test, Fetch models, Verify model,
  Edit and Remove sit beside the provider name at every width instead of falling
  under the facts as a second row.
- **Long code lines wrap in place.** The Files viewer gives every line its own
  row, so a wrapped line keeps its line number and no code hides behind a
  horizontal scrollbar.
- **The Files panes start on the same line.** The tree and the document are two
  columns, so the generic stacked-panel gap no longer pushes the document down.
- **Lists stop repeating their own counts.** The Changes view no longer shows
  "1 untracked" above "UNTRACKED 1"; Hosts and Certificates no longer repeat the
  summary count in the section heading.
- **Project and site rows read cleanly.** Branch rows use a plain current-branch
  marker instead of an empty checkbox-like tag, site addresses drop the default
  `:443`, site fact labels have colons, and the project overview no longer
  repeats the runtime in both "Serves at" and "Runs on".
- **Branches are a table.** The Git branch list now names its columns, keeps the
  current branch first, separates local from remote-tracking refs using Git's
  own `refs/remotes` namespace, omits `origin/HEAD`, and puts the secondary
  actions behind one per-row menu.
- **Tables share one grid.** Header rhythm, cell padding, borders and right-hand
  action columns are the same in panels, evidence, tasks, models and Markdown;
  no action header is left as an empty cell.
- **Inline rows line up.** Controls in a single-line form row no longer inherit
  the stacked-sibling top margin, which pushed every control after the first one
  down by 12px (the tag form was one example).
- **Motion is one language.** Tabs and chips give short press feedback, hover
  styles only apply to devices that really hover, transitions name exact
  properties (`background-color`, not `background`), and the composer and
  mention popovers grow from their trigger in 140ms instead of appearing from
  nowhere. Reduced-motion users keep the fades but lose the movement.

---

## [0.22.0] - 2026-11-05 - Branches get an address, and the mail is readable

### Added

- **A site for every git worktree.** `worktree.sites`, `worktree.create`,
  `worktree.serve` and `worktree.unserve` (and `devd worktree ...`) check out a
  branch under devd's own folder and serve it at `<branch>.<site>.test`, with a
  `.env` pointed at that address and a shared, copied or untouched database. The
  site page has a Worktrees section with the branch, its address and the
  dependencies a fresh checkout still needs.
- **Mail, in the window and for agents.** `mail.setup`, `mail.inbox`,
  `mail.read`, `mail.delete` and `mail.clear` read the Mailpit catcher devd
  already supervises. A message opens as HTML (scripts and remote requests
  blocked), text, raw source or headers, and can be handed to the assistant or
  the team with its id rather than its body.
- **The site a folder belongs to.** `site.current` names the site whose project
  contains a folder, preferring the most specific one, so a worktree is reported
  as itself.
- **Agent context in MCP.** `initialize` now carries the site for the working
  folder, and the server exposes `devd://sites`, `devd://site/current` and
  `devd://site/<name>` resources and a `debug_site` prompt.
- **Coding-agent setup.** `agent_client.list`, `agent_client.add` and
  `agent_client.remove` add devd to Claude Code, Claude Desktop, Cursor, VS Code,
  Windsurf or Codex by editing only devd's entry, keeping a backup and leaving
  the rest of the file byte-for-byte. Shown on Agents → MCP.
- **Project pins.** `site.pin` writes `.devdrc` (`php=8.4`, `node=22`) so a
  version belongs to the repository; `.php-version`, `.nvmrc` and
  `.node-version` are read too. Precedence is isolated, then pinned, then the
  machine default, and the site page says which one is in effect.
- **A guideline for a project's coding agents.** `project.agent_guidelines`
  writes `.ai/guidelines/devd.md` for Laravel Boost projects: the site is already
  served, versions come from the project, and mail is caught locally.
- **Two skills:** `check-the-mail` proves what a change sends by reading the
  caught message, and `work-on-a-branch` gives parallel work a branch and an
  address of its own.

### Changed

- **`devd site php` isolates a version on this machine; `devd site pin` pins it
  in the project.** The site page distinguishes “isolated”, “pinned by the
  project” and “follows the default”.
- **Removing a folder that is listed only because a site serves it** now names
  the site and explains that removing the site is what removes the row.

---

## [0.21.0] - 2026-11-05 - A design pass, and a terminal you can type in

### Added

- **Terminal is a main destination.** It sits in the navigation beside Home and
  Projects, takes focus when it opens so you can type straight away, and the
  Terminal button on a project no longer waits for a full refresh before it
  switches screens. The screen is read on every keystroke, so echo is about
  110 ms instead of up to a second.
- **A real 404.** An address that means nothing says so and offers the nearest
  real one, instead of quietly showing Home under the wrong URL.
- **Skeleton loading.** Projects, project tabs, Files and Diagnostics keep their
  shape while data arrives. Diagnostics says it is checking and no longer gives
  up on a slow check at 15 seconds (it waits 45).

### Changed

- **Navigation is four destinations.** Home, Projects and Terminal; Assistant
  and Agents; and Machine, which holds Sites, Environment, Models, Activity,
  Diagnostics and API and tools. Runs and the roster moved to `/agents/runs` and
  `/agents/team`; `/team` and `/agents` still open the right page.
- **The project page is Files, Git and Agents.** The eleven tabs are three
  groups with a sub-row. Git actions appear only when they apply. Remove and
  Delete are in a "..." menu instead of beside Terminal and Refresh.
- **Sites.** Rows are two lines on narrow windows. The Laravel tab is hidden for
  sites that do not serve PHP, and Preview explains a site with nothing to serve
  instead of showing a white page.
- **Counts agree.** Home and Environment count services the same way; Projects
  says what needs attention instead of how devd came to know a folder.
- **Words, not identifiers.** Inbox says what was declined and who asked;
  Activity says "Opened a terminal in a project".
- **Models, Skills and Home** are quieter: providers and skill stages fold, and
  "no model connected" is an invitation, not an alarm. Environment makes Start
  needed the primary action and Stop all a quiet one.
- **Diagnostics.** A port held by devd's own running web server is expected; one
  held by anything else keeps its warning, and the note says another program
  holds it. The state-database message is printed once.

### Fixed

- **Terminal input is no longer recorded.** `terminal.write` was written to the
  audit trail once per keystroke, which stored typed characters, including
  anything typed at a password prompt, and buried real changes. Opening and
  closing a terminal are still audited.
- **Cancelling a run could overflow the stack.** `kill_tree` ran `taskkill` through
  a call that also checks whether the run was cancelled; on a busy machine
  `taskkill` took longer than the 200 ms check, which called `kill_tree` again,
  and so on until the stack overflowed. The kill is no longer cancellable.
- **No more empty states before data.** History and Branches no longer say "No
  history loaded" or "No branches" for the moment a request is in flight.
- **The Assistant heading is no longer cut off** at about 785 px, and the status
  line and summary tiles no longer clip on a phone.

---

## [0.20.0] - 2026-11-04 - One inbox, one run view

### Fixed

- **The Inbox, the badge and Home now count the same thing.** An inbox row
  records the run and the kind of event that produced it (`run_id`, `task_id`,
  `kind`). The badge is derived from the rows the Inbox actually renders, not
  from a second raw count. One existing database repair removes the pre-0.20
  `Run paused:` / `Plan ready:` copies that duplicated a live run's Needs-you
  card.
- **Deleting a run deletes its notifications.** `delete_run` now removes the
  inbox rows and undelivered team reports that belong to the run. Cancelling a
  run resolves them, so a plan that can no longer be approved stops being
  offered. Previously an orphaned pause notice became a new Decision on Home
  the moment the run was deleted.
- **A cancelled run no longer offers Retry, Accept or "Needs you".** The single
  source of that question returns nothing for a finished run, and the run view
  only offers task actions while the run can still act.
- **Agents can read long runs again.** Tool results were compacted before the
  context budget was checked, blinding an agent after eight tool calls even on
  a large model. Results are cleared only when the prompt is actually over
  budget, oldest first, and the most recent results stay whole. This was the
  cause of the implementer reporting blocked and the reviewer looping.
- **A plain PHP project can run its own tool.** The exec allow-list now reads
  the same serving evidence as the site view: a directory served as PHP may run
  `php` (and `composer` when it has a manifest), even with no framework stack.
- **Home and the Assistant both have a working `/` and `@` menu.** Home renders
  its own menu, `openMentions` remembers the query, and one lookup serves both
  composers. The trigger itself is part of that identity: typing `/` and then
  `@` at the same position is a different list, so the old one is replaced
  rather than left on screen.
- **Refreshing a run no longer pulls the reader to the top.** The run detail
  keeps its scroll position, skips an identical repaint, and follows the bottom
  only when the reader was already there.
- **The run detail is a conversation, not a log.** It uses the Assistant's
  `.chat-stream`, `.turn`, `.bubble` and `.msg-meta` components instead of a
  64-pixel monospace gutter and plain-text rows.
- **The terminal gives its emulator the raw stream.** `terminal.read` can return
  the untranslated bytes for xterm (colour, cursor movement, full-screen
  programs), while readers and agents still get the escape-stripped screen.
- **The task strip counts work, not the planner.** `tasks_done` no longer
  includes the planner task against a total that excludes it.

---

## [0.19.2] - 2026-11-04 - The caret, the inbox, and a pid that lied

### Fixed

- **The Assistant composer lost the caret while typing.** The input handler is a
  chain of `if / else if`. The `home-ask` branch is one statement, so the chain
  ends there. The `chat-input` branch was written as a block and never closed, so
  every keystroke also ran the *filter* branch below it: it set
  `state.filters[undefined]` and scheduled `render()` 120ms later. `render()`
  replaces `main`, the composer lives inside `main`, and the textarea was
  therefore rebuilt twice a second with the caret going with it. Home was never
  affected because its branch ends the chain. The block is closed, and the filter
  branch now requires a real filter box.

- **The Inbox stayed on "Loading" whenever a run was paused.** `needsCard` read
  `n.primary.id` for every action, but the server has no `primary` field - it is
  a Rust method that is never serialised. So the first paused run threw
  `Cannot read properties of undefined (reading 'id')` *inside the render*, the
  markup was never written, and the error was visible only in the console. The
  primary is now computed the way the server decides it: the first action that is
  not dangerous, else the first.

- **The Inbox showed four cards for two paused runs.** Two separate causes:
  `loadInbox` reset `AI.needs = []` and then pushed from inside each per-run
  read, so two overlapping loads appended to each other's result; and the
  `Run paused: <title>` inbox message became a second "Question" card about a run
  already shown above. The list is built locally and de-duplicated by run, and a
  message that is about a shown run is dropped - the same rule Home already used.

- **A pid file naming a recycled process read as a live owner.**
  `live_devd_holder` checked only that the pid was alive and its image name
  contained "devd". A pid file left by a dead process, whose number a later devd
  run reuses, therefore blocked recovery while naming an unrelated process. The
  claim files now carry the writer's start token beside the pid - the same
  identity the daemon record already used - and a file with no token is treated
  as stale rather than as a claim, so a legacy file cannot block recovery for
  ever.

## [0.19.1] - 2026-11-04 - The screen that waited for ever

Four fixes, all reported after living with 0.19.0.

### Fixed

- **The Inbox could sit on "Loading" for ever.** `renderInbox` draws nothing
  until `AI.requests` is set, and that happened only when all six reads in
  `loadInbox` succeeded. Two of them already had `.catch` handlers; the four that
  carry the screen's actual content did not, so one failed request rejected the
  whole batch and the screen waited on a promise that had already settled - with
  no error shown and nothing to retry. Every read is now guarded to its own empty
  value, so the screen draws with whatever could be read.

- **A run could not be deleted the way a conversation can.** The Assistant list
  offers delete on every conversation; the Agents list offered it only on runs
  that were already over, so the same gesture existed in one place and not the
  other. The reason was sound - `team.delete` refuses a run that is still working
  - but hiding the control is not how to say so. Every run row now carries it:
  on a finished run it deletes, on one still going the server answers with what
  to do first.

- **A project opened on Changes instead of Files.** `files` is the first tab in
  the strip and `openProject` sent a repository to the second, so opening any
  repository put the person one tab along from where the strip implies the
  screen begins. It now opens on Files.

### Removed

- **The "add a model id this provider does not list" field** on each provider
  card, with its handler. The model picker already offers "use the id you typed"
  for ids the provider does not list, so the card carried a permanent text field
  and a button to reach a place one click away.

## [0.19.0] - 2026-11-04 - The team reports back

This release is about a single complaint: asking devd for something meant
reading the answer on another screen, if it arrived at all.

### Added

- **A run reports into the conversation that asked for it.** A team run had no
  field naming a conversation, so it could not address the person who started it
  and its result went to the inbox - a separate channel. `team_runs` now carries
  the conversation, `team_reports` queues a report for it, and `team.start` takes
  an optional `thread` the assistant passes with its own conversation id. A run
  started from a schedule or the command line has no conversation to address, so
  it still posts to the inbox. This also removes the duplicate on Home: a paused
  run was derived from `team_runs` *and* listed again as the inbox message the
  team posted about itself, so every pause counted twice.

- **The assistant answers in its own words when its team reports.** Reports were
  queued but nobody spoke them. The assistant is now told about one in that
  conversation and asked to answer - what was done, how it was proved, what it
  needs - with the ask shaped by the kind of report: a finished run wants an
  outcome, a paused one wants what is blocked and what the assistant will do
  instead. It goes through `hub.tell`, so a message arriving mid-turn waits for
  the next step boundary instead of splitting a tool call in half.

- **What a project declares, in `devd.toml`.** The ecosystem's own files still
  say which runtime a project wants - `.nvmrc`, `.python-version`, `go.mod`,
  `Cargo.toml` and the rest - because they are the truth and a person already has
  them. What none of them can say is which *services* the project needs, which
  port, how to start it and how to know it came up. `devd.toml` declares exactly
  that. Two rules keep it safe to commit: a literal secret is refused by name,
  because the file belongs in git, and `run.health` must be a URL rather than a
  command, because it is polled and not executed.

- **`.tool-versions`**, asdf's and mise's single file for every runtime.

- **The local inbox is readable.** Mailpit was already installed and pointed at
  by PHP, so an application's mail never left the machine - but nothing in devd
  could say what had been caught. `mail.inbox`, `mail.read` and `mail.delete`
  read Mailpit's own API. A message body is data, never an instruction.

- **Memory: what devd learns, and when it recalls it.** Layered Markdown under
  the devd home, not a vector store: `notes.md` written as runs end, `facts.md`
  holding one convention each, and a `digest.md` generated from those two so a
  wrong digest is a bug in the generator and the facts behind it are still there.
  Recall is gated by a cheap keyword test - without a cue a turn gets the
  conventions, which apply to everything, and not the recent work. A missed recall
  costs one tool call; a false one costs the model's attention on every turn.

- **A teammate remembers what it learned.** Role files are identity and live in
  the binary; what *this* QA learned in *this* project accumulates in
  `souls/<project>/<role>.md`. Only devd writes it, from a lesson a task reports.
  An agent that writes its own identity is drifting, not learning.

- **A persona you accept, never one devd decides.** `memory.persona_propose`
  puts a line in front of you; `memory.persona_accept`, which only a person can
  call, moves it into the persona. A proposal reaches no prompt.

- **A provider seam.** `devd` runs native processes and always will; what it
  lacked was a place to decide anything else. `provider.rs` reports what each
  engine is and what it can actually enforce, and refuses rather than degrading:
  asking for a container on a machine with no engine is an error naming the
  remedy, never a silent native run with less isolation than was asked for.
  `runtime.providers` shows the answer. Nothing is installed on your behalf.

### Changed

- **`project.terminal` starts a session in devd** instead of handing a folder to
  the operating system's terminal and reporting that it opened one. That was true
  on a desktop and silently false from a service, and it is the wrong act now that
  devd has a terminal the assistant can read. The system window survives as
  `project.terminal_external`.

- **The composer has two triggers**, as its placeholder has claimed since it was
  written. `/` offers skills and tools; `@` offers files from the open project, a
  project, or a teammate. A path query may contain the characters a path is made
  of; the old trigger rejected anything containing a space.

- **Destructive buttons say so with a class.** Nineteen were coloured red by
  `[data-action*="remove"]` - colour chosen by the spelling of the action - and
  `.btn-danger` was declared five times with different backgrounds, so the late
  ones largely won and danger looked like neutral. Seventeen now carry
  `btn-danger` explicitly.

- **The pane headers line up.** `.threads-head` rendered 44px and `.chat-head`
  46px plus padding, set across six rules that each overrode the last. Both take
  one token now.

### Fixed

- **The Agents conversation scrolls.** `.chat` had no height, so `flex: 1` on the
  scroller had nothing to fill and the page scrolled instead of the pane. The
  Assistant screen was unaffected only because the grid they share sets a height.

- **`@.tool-versions` lost its first runtime on Windows.** PowerShell's
  `Set-Content -Encoding utf8` writes a UTF-8 BOM, so the first line parsed as an
  unmatchable tool name and the runtime silently vanished. Found by testing
  against a real file rather than a fixture.

- **The Projects screen offered the same action twice**, with the header and the
  empty state disagreeing about which was primary.

- **The terminal screen** read `state.project.path` unguarded and crashed on a
  first visit; `terminal` had no URL route, so a reload landed on Home; every
  render threw away the element xterm owns; and `Terminal.open()` ran on every
  paint, discarding the screen. All four are fixed, and the terminal renders with
  xterm.js.

## [0.18.1] - 2026-11-03 - Home opens the project

### Fixed

- **A project on Home opens that project.** Its row, and the Open button beside
  it, went to Develop's generic Stacks and tasks screen - every stack in the world
  listed for a project you had already chosen. Both now call `openProject`, the
  same function the Projects screen uses, so either route reaches the same screen.
- **No dead band under a site's tabs.** A section head carries a 32px top margin
  to separate chapters; the first one under a tab strip is not a new chapter, so
  it left a gap between the tabs and what they had just revealed. The strip and
  its sections are now one block and the first head inside it sits flush.

### Changed

- **Effort belongs to the model.** It was a second chip beside the model in the
  composer. It is now the last row of the model picker, offered only for models
  that accept a reasoning effort, and it opens in place with a way back. An inert
  control for a model without a reasoning dial is gone rather than shown disabled.
- **An open menu survives a repaint.** The rail is painted by every timer, which
  closed an open menu, replayed its animation and moved the pointer's target. A
  repaint now leaves an open menu and an open dialog alone, and a popover whose
  trigger was replaced finds the trigger that took its place.
- **Agent tool runs collapse into one line.** A turn that called six tools used to
  be six blocks; it is now one group that opens on demand, so the conversation
  reads as a conversation.
- **Files folders look like doors.** A folder row opens into its own listing and
  now carries a chevron, dimmed until hover or focus. A skipped folder has none,
  because it does not open.
- **Files state belongs to its project.** Opening a second project no longer
  leaves the first one's tree and open file behind.

---

## [Unreleased]

Nothing yet.

---
---

## [0.18.0] - 2026-11-03 - The window sees the work

### Changed

- **Skills are grouped by the stage of work, not by kind.** The old `playbook`,
  `technique` and `principle` kinds said what shape a skill had, which helped
  nobody choose one. The 63 built-in skills now name the stage they belong to:
  `discover`, `design`, `build`, `check`, `review`, `ship`, `words` and `house`.
  The prompt index and the **Skills** page show the stages in the order of the
  work, so an agent finds the right skill without reading all of them.
- **No skill name carries a type prefix.** The 24 `principle-*` skills are named
  for what they say (`prove-it-works`, `fix-the-root-cause`,
  `make-the-narrowest-change`, `validate-at-the-edge`, ...), and every
  cross-reference between skills was updated to match. `skills.save` now accepts
  the new stages; a person's own skill may still use a word of its own, and it is
  shown under that word rather than dropped from the index.
- **The eight core rules name the skill that holds the full text**, so an agent
  that knows the rule can read the reasoning behind it.
- **The README and the repository description no longer call devd "local".** It
  is a development environment, not a local-only one: the module descriptions in
  `Cargo.toml` and the desktop metadata say the same.
- **Documentation.** `docs/limitations.md` now exists (the README linked to it and
  it was never written); the reference covers the new capabilities and the queue,
  and the daemon documents the retention settings; the facts that had drifted -
  capability counts, the MCP transcript and two Git commands that do not exist -
  are corrected. `scripts/check-docs.ps1` checks every capability and link a doc
  names, so the next drift is caught rather than read.

### Added

- **A terminal in the window.** A shell in a project, running in devd rather than
  in the page, so it keeps running when the window closes and `terminal.read`
  returns the same screen to an agent that the person sees. `terminal.open`,
  `terminal.read`, `terminal.write`, `terminal.resize`, `terminal.list` and
  `terminal.close` are the whole surface: a person opens and types, an agent may
  read, and closing a shell is destructive and approved.
- **The terminal answers its shell's questions.** The Windows console host asks
  "where is the cursor?" (`ESC[6n`) as soon as it starts, and blocks until it is
  answered - so a shell whose output is merely stripped never runs a command at
  all. devd keeps the small part of a terminal emulator that answers it.
- **`portable-pty` is the one native dependency.** It is the only cross-platform
  way to get a PTY, it is MIT, and it is compiled in. The README says so instead
  of repeating that devd bundles no third-party code.
- **The README no longer claims the window vendors xterm.js.** It did not, and the
  folder it pointed at did not exist.

- **A project's files, in the window.** A **Files** tab lists a project's folders
  and opens its files: `fs.tree` asks for one folder at a time, so a large project
  costs one request per folder opened rather than one per visible row, and a file
  is opened with `fs.read`, so a person sees what an agent would see - the same
  text, bounds and refusals. A secrets file is **listed and marked** rather than
  hidden, so a person can see it is there and will not open; a dependency folder is
  listed and marked skipped; `.git` is refused. Files are syntax-highlighted for
  the languages devd's own stacks use and shown as plain text otherwise, and
  **Copy** leaves the line numbers behind.

- **`web.open` reads a page's structure, not only its prose.** `docs.fetch`
  answers "what does this page say?"; `web.open` answers "what is on this page?"
  with the title, the headings, the links (resolved to absolute addresses) and the
  form controls with their name, type and label, alongside the readable text. Both
  share one fetch, so the address rules cannot drift: public https only, private,
  loopback and metadata addresses refused, and every redirect re-checked.
  `javascript:`, `mailto:` and fragment-only targets are not reported as links,
  because they are not pages an agent can open.
- **Preview any address on this machine.** The Preview tab used to show a site
  devd serves. It now takes any loopback address in a bar above the frame, so a
  `php artisan serve` on 8000 or a `next dev` on 3000 can be looked at without
  leaving the window. The address is checked by the server (`POST /api/preview`)
  before it reaches the frame, because a frame is a place a page can run script:
  http and https only, a host on this machine only, no credentials, fragment
  dropped. The rule lives in `devd_core::preview` so the window holds no second
  copy of it.
- **One question, not a status report.** A run that cannot continue now answers
  with a structured `needs` object - the one task to look at, what evidence is
  missing, and the actions that resolve it (retry with a note, accept by hand
  after a confirmation, read what it did). The Inbox leads with it, the run view
  shows the same object, and `team.get` returns it, so no two screens can word it
  differently. A run with nothing waiting has `needs: null`.
- **A stall watchdog.** Every run turn and tool call moves the task's
  `last_step_at`. A task that moves no heartbeat for twice the provider's own
  request timeout (never less than three minutes) is asked where it is; if that
  gets no answer it is stopped as `unverified` with the reason. A worker waiting
  on a slow model is slow, not stuck, and the window is set so it cannot be
  mistaken for one.
- **Proposals instead of work.** `team.suggest` records work devd would do -
  what it noticed, what it would change, how that would be proved - and starts
  nothing. One row per idea; a person turns one into a run with
  `team.accept_suggestion`, which starts an ordinary run that still waits for
  plan approval. `team.dismiss_suggestion` keeps the record that this was not
  wanted.
- **A rule for when to think about starting work.** `intake.set` writes a
  project rule - "when the nightly check has failed three times in a row, at most
  once an hour, consider a run that fixes it" - and `intake.list`/`intake.delete`
  manage them. A new rule starts in **observe** mode, which records a proposal
  and nothing else; `auto` starts the run, and never in production. The daemon
  evaluates the rules against facts devd itself recorded, never a model's
  opinion. The team room shows them and the project's prompt describes them, so
  an agent does not propose the same thing again.
- **A mandate on every team task.** Each task records why it may run
  (`explicit`, `schedule`, `inferred` or `system`), who asked, its scope, its
  role and a hash of the rules it was accepted under (environment mode,
  definition of done, parallel limit, working agreement). A resumed task whose
  authority no longer holds pauses the run with the reason instead of continuing
  blindly, and a person resuming a run resets the watchdog for its tasks.
- **A durable queue for work that leaves the process.** Notifications (a site
  going down or coming back), background reviews and scheduled starts are rows
  with attempts, a last error and a retry time, drained by the daemon, retried
  with backoff, and reported on the health screen only after a run of three
  failures. `notify.test` still delivers synchronously, where the person is
  waiting for the answer.
- **The kind of work is decided in code.** `RunKind` - `answer`, `plan`,
  `assign` or `schedule` - is chosen from the words (or by the composer, or
  `chat.send`'s `kind`) before any model is asked, stored on the conversation,
  and chooses the model chain: a plan uses the planner's models, an answer the
  default ones. `models.roles` now lists what each kind of work reaches.
- **Prompt provenance.** Every turn records a manifest of the role, loadout and
  skill text it ran under, by name and hash, on its `run_steps` row. The content
  is still compiled in and still cannot be edited; the fingerprint is what makes
  "the run that worked and the one that did not" comparable.

- **A health check that flips is queued, not sent from the check.** The same
  message is delivered by the queue, so a dead webhook no longer loses the
  warning.
- **`review.start` is a queued row**, not a thread, so a process that stops with
  a review pending tries it again and a review that keeps failing is visible.
- **The assistant is told to spend less of a person's attention**: confirm once
  and work quietly, ask one concrete question rather than a list, and record an
  unrequested improvement as a proposal with `team.suggest` instead of doing it.
- **Retention covers the new records**: finished queue rows after 14 days and
  decided proposals after 90 (`effects.retention_days`,
  `suggestions.retention_days`).
- **Servers are the only remote-server path.** The separate third-party control-panel
  integration and its API capabilities were removed. Use **Server → Remote → Servers**
  with SSH for every provider.
- **Server PHP limits live with servers.** `server.limits` and `server.limits_set`
  read and record the limits `deploy.check` compares against; they are shown on the
  Servers tab instead of a separate integration.
- **Service discovery no longer names third-party local stacks.** Find and adopt scan
  plain install folders and `PATH`.

---

## [0.17.1] - 2026-10-30 - Setup says what it needs

### Changed

- **Setup** groups its steps into "Needed" and "Whenever you like", with an icon
  and a state word on each (Not installed, Not working, Ready), and a plain
  sentence at the top saying how many are still needed.

### Known

- Starting a service whose port a program devd did not start is already
  answering can still start a second copy beside it on Windows. It is not changed
  here; it needs a decision in the service layer.

---

## [0.17.0] - 2026-10-30 - The environment, not the chat

The app is redesigned around the machine it manages. Everything below is the
interface; no capability, approval or recovery rule changed.

### Changed

- **A command bar and one centred column.** Work (Home, Projects, Sites,
  Processes), Agents (Assistant before Agent runs), Machine and Records. Needs you
  is always in the bar, with a live chip for a run in flight. The bar, tabs, page
  and footer share one centred column.
- **Dark only**, with Inter Tight bundled with the app. There is no theme switch.
- **One composer** for Home, the Assistant and the team: a round send button, the
  assistant or the team chosen inside it, and a strip along its top that holds the
  paperclip and the files and photos attached.
- **Health lives in the footer**; the search field is a pill.
- **Plain language throughout**: a warm greeting, empty states that say what to do,
  Activity written as sentences with the exact capability beside each.
- **Tables keep their columns**: Cron, Workers, Redirects and Laravel are tables,
  and the Models screen lines providers up on four shared columns.
- Live runs and the Needs you count are re-read every 4 seconds while a run is in
  flight, and every 10 otherwise.

### Fixed

- Stopping a run no longer waits on pipes that a second program was holding open.
- Attaching a file never worked from the browser UI: the upload request carried no
  session token and was refused. It now carries it.

---

## [0.16.0] - 2026-10-29 - A resident process, and the gaps closed

The limitations page is gone: each entry on it is either built, or stated where it
belongs (`docs/platforms.md` says what has been run on which platform).

### Added

- **A resident daemon** (`devd daemon run|start|stop|status`). One process per user
  owns scheduled work and the local API. It is found through an instance record
  that includes the process's start time, not by scanning ports; the window, `devd
  ui` and `devd serve` attach to it. `devd autostart enable` starts it at sign-in
  (a `Run` value, a `systemd --user` unit, or a LaunchAgent).
- **Service ownership by pid and start time.** A pid file now records when the
  process started; a recycled pid is never taken for a service. A reconciliation
  pass reports adopted, stale, ambiguous (a stranger on the port, left alone), down
  and unexpectedly running services.
- **Maintenance**: log rotation, audit retention, download-cache eviction and
  undo-snapshot pruning, hourly, each with a setting.
- **Stopping reaches the program in flight.** Cancelling a run kills the process
  tree a handler is waiting on, and the call returns `cancelled`.
- **Changes the registry did not see** are evidence: a before/after comparison of
  the project becomes `observed.diff` and counts as an inspectable change.
- **Native Gemini adapter**; a **per-model capability table**; **privacy and cost
  policy** (`policy.get`, `policy.set`: local-only, allow/block providers, a
  per-run spending limit).
- **MCP over the older SSE transport**, and request headers kept in the private
  environment file.
- **Background reviews** (`review.start`) that leave an inbox message; **three-way
  merge** of memory proposals; **full-text search** (`search.index`,
  `search.indexed`); **merge-conflict resolution with merged text**
  (`git.conflicts.resolve_with`), classified against the real stages.
- **An evaluation suite** (`crates/devd-agent/tests/eval.rs`): scripted scenarios in
  CI, a cassette recorded from a real provider replayed through the real loop, and
  an opt-in live-provider mode (`DEVD_EVAL_LIVE=1`).
- **A local DNS server** for `.test` and the commands that point the system at it
  (`dns.status`, `dns.enable`, `dns.disable`).
- **HTTP/2** on the TLS listener, spelled for the installed nginx or Apache.
- **Resumable downloads** and a bounded cache.
- **Services are asked in their own protocol** whether they are ready (HTTP, MySQL
  handshake, FastCGI, Redis, memcached, PostgreSQL); `listening` and `ready` are
  separate facts.
- **Undo snapshots** for `fs.write`, `fs.edit`, `fs.delete` and `db.drop`
  (`snapshot.list`, `snapshot.restore`).
- **PostgreSQL and MySQL install on Windows** from builds whose SHA-256 devd pins
  (neither vendor publishes a strong checksum; the install result says so). MySQL
  8.4 gets a `root@127.0.0.1` account, a graceful stop, and the same database
  tools, dump/restore and undo as the others; `engine: mysql` means it when it is
  installed and MariaDB otherwise.
- **Signed self-update** (`devd update check|apply`, `keygen`, `sign`).
- **The window**: argument forms generated from each capability's schema; the
  drawer takes and returns focus; failures are announced to screen readers; a
  run's sheet shows the project's diff and environment.

### Changed

- `autostart enable` now starts the daemon, which brings the stack up, instead of
  running `devd stack up` once.
- `docs/limitations.md` and `docs/branding.md` were removed.

### Not done

- Pointing Windows at the DNS server without port 53; the
  Linux and macOS code paths that need a real machine (see `docs/platforms.md`).

---

## [0.15.1] - 2026-10-28 - A mutation the lists do not name is still a change

The evidence classifier recognised changes and reads by name. 102 of 415
registered capabilities were in neither list - `git.push`, `git.clone`,
`composer.install`, `deploy.run` and others - so a successful call left only
generic "completed" evidence and could not satisfy a change gate.

### Fixed

- A capability the name lists do not know is classified by the effect it
  declared when it was registered: a read is read evidence, any other effect is
  a change. `evidence_for_effect` carries the effect from the dispatcher.
- A mutation whose name ends in a read-looking suffix (`git.conflicts.resolve`)
  is a change; the exact change list is checked before the read suffixes.

---

## [0.15.0] - 2026-10-27 - A read is not a change, and a claim is not a check

0.14 said completion was decided from evidence. Reading the implementation
against that claim found places where the record could say more than it knew: a
file read counted as a change, a successful call counted as readiness, a large
result was stored truncated with nothing behind it, and an idempotency key was
looked up without the capability that produced it. This release makes the
evidence match the observation, and adds the check that a site goal was missing.

### Added

- **`site.verify`.** One capability answers the question a site goal actually
  asks: is the hostname in the hosts file, does it resolve to loopback, is the
  listener accepting connections, does TLS validate against devd's own CA, and
  does the site answer with a status below 500? It returns `ok=false` with the
  failing checks in the payload rather than failing the call, so a run records
  what was observed instead of only that the probe returned. It reads no
  environment proxy, follows no redirect and trusts only `certs/devd-ca.crt`.
- **Artifacts for output that does not fit.** A result larger than 24 KB is
  written under `<root>/artifacts/<run>/` (8 MiB cap) with its SHA-256, byte
  count and a preview; the tool call and every derived evidence row point at the
  artifact. Replaying the call returns the stored result, including the
  artifact reference, instead of a truncated preview.

### Fixed

- **A read is not a change.** `fs.read`, `fs.list`, `fs.search`, `git diff`,
  `git status`, `git log`, `project.detect`, `project.tasks` and
  `environment.manifest` now record `read` evidence. They can give a review its
  basis, but they cannot satisfy a change requirement or turn an unverified run
  into a completed one. Change evidence comes from a write, a commit, a site or
  stack change, or a setup that reported a change.
- **Readiness comes from the payload, not from the call returning.** A service
  is ready only when it reports `running` and `listening`; a stop is complete
  only when it reports not running; `site.verify` passes only when its own `ok`
  is true; `workflow.setup` verifies only when `verified` is true. A capability
  that returns `ok` with a negative payload is recorded as evidence that did not
  pass.
- **`code` and `test` policies require a real change.** A passing test with no
  recorded change no longer completes a code goal, and a failed change is never
  treated as a completed one.
- **An idempotency key is scoped to its capability and arguments.** The lookup
  matches key and capability, compares the stored argument hash, and returns a
  `conflict` for a different call under the same key. A recorded result that
  cannot be read is also a conflict: the operation is not run again.
- **An approval that was already consumed is not dispatched twice.** `Hub::decide`
  adopting a `consumed` or `failed` approval returns the result the operation
  actually produced instead of re-running it, and only the surface that
  dispatched the call finishes the approval.
- **The context budget uses the catalogue's window.** The model's real context
  window is read from the catalogue by exact model id (the largest positive
  value when a model has several entries) before falling back to the name
  heuristic, so compaction happens at the right size.
- **The event stream applies the same Host check as the API.** `GET /api/events`
  now requires a loopback `Host` in addition to a local `Origin`. The middleware
  already refused a foreign host; the route now refuses one itself, so a future
  mount cannot reopen the hole. The origin comparison is an exact match against
  the local prefixes rather than a prefix test.

## [0.14.2] - 2026-10-27 - The stack outlives the command that started it

Installing 0.14.1 over a real machine and then running `devd stack up` showed the
web server starting and dying within a second. Each service was put in a Windows
Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; a one-shot command exits
immediately after starting the stack, so closing the job killed the service it
had just started. `devd service start nginx` and the sign-in entry that runs
`devd stack up` were therefore unable to leave anything running on Windows.

### Fixed

- **A service and a public link outlive the command that starts them.** Both are
  detached by contract and are no longer placed in a kill-on-close Job Object.
  Stopping them still terminates the whole process tree, and they are still
  tracked by pid file. Supervised project processes and stack tasks keep the Job
  Object, because their lifetime is the devd process that supervises them.
- A test now starts a persistent process, drops the supervisor, and asserts the
  process is still alive before killing its tree.

## [0.14.1] - 2026-10-27 - Every agent call is a run

0.14.0 said an MCP tool call left the same trail as a direct call. It did not:
an outside agent driving `devd mcp` had its calls audited, but no run, tool call
or evidence row was written, because the server had no run to attach them to.
This release makes the claim true.

### Fixed

- **An outside MCP call is a run.** `devd mcp` starts one headless run per call
  when the caller did not bring one, so the capability, its redacted arguments,
  the result and the derived evidence are recorded where the assistant and the
  team record theirs. A dry run still opens none, and a caller that already has
  a run - the built-in assistant - keeps it rather than opening a second,
  competing timeline.
- **A queued approval belongs to that run.** The request an outside agent queues
  now carries the run id, so approving it records the operation in the run that
  asked and settles that run when the operation returns. Declining it cancels
  the run, which is the truth: the work can never proceed. Neither case leaves a
  run executing after the decision.

## [0.14.0] - 2026-10-27 - One run behind every piece of work

A conversation, a team run, a scheduled job, an HTTP call and an MCP tool were
five ways to start work and five ways to describe what happened. They are now
one record: a run with steps, capability calls, evidence, artifacts, approvals
and a sequenced event log. This release adds that spine and moves the existing
surfaces onto it. History is kept: `chat_*` and `team_*` rows are linked to
their run, not rewritten, and the migration is additive.

### Added

- **Runs, with evidence rather than claims.** `runs`, `run_steps`, `tool_calls`,
  `evidence`, `artifacts`, `approvals` and `run_events` (migration
  `017_runs.sql`). `Registry::dispatch_with` records a normalized call at the
  boundary, so a direct alias, `devd_invoke`, an MCP tool and an internal call
  leave the same trail. Arguments are redacted by key name and hashed for
  comparison.
- **Completion is decided, not announced.** `CompletionPolicy::for_goal` reads
  the goal and acceptance criteria and requires the matching evidence: a change
  for a code goal, a passing check for a fix, readiness for a service, runtime,
  database or site. A change that was never checked ends `unverified`, with the
  missing evidence named. The model's sentence that it finished is not
  evidence.
- **One durable approval service.** A decision is a row, not a message in one
  window: 256-bit token, only its SHA-256 hash stored, single use, scope,
  expiry, decider and reason. The conversation, the token flow, the by-id
  endpoints, the desktop window and `Hub::decide` all consume the same row; an
  agent cannot approve its own operation. Changed arguments or a changed plan do
  not match the stored hashes.
- **A context budget the model can live in.** Model-aware windows, project
  instruction discovery, preserved notices, errors and decisions, older tool
  results summarized and their raw output kept as artifacts, and a
  `ContextManifest` that records what was included, summarized or dropped.
- **Replayable events.** `run_events` is append-only with a global cursor and a
  per-run sequence. `GET /api/events` replays after `Last-Event-ID` and then
  tails; a lagging subscriber gets `stream.lag` instead of silently losing
  frames. `GET /api/runs/:id/events` pages both ways.
- **Environment manifests and setup.** `environment.manifest` describes a
  project's stack, runtimes, services, sites, tasks, variable *names*, ports and
  isolation, with a fingerprint that ignores `generated_at`. `workflow.setup`
  applies what is missing in a fixed order, dry-run first, and reports each step
  as done, skipped or failed.
- **An evaluation suite for the hard paths.** `crates/devd-core/tests/evaluation.rs`
  covers a full workflow, recovery, approval scope and single use, timeouts,
  malformed arguments, cancellation mid-run, duplicate idempotent calls, event
  replay and multi-criteria acceptance.
- **A Run sheet, not a new page.** The durable record opens from the
  conversation header and from the team run it belongs to. There is no
  destination of its own, because a run is not a second kind of process; it is
  the record behind work that already has a place.

### Changed

- **Assistant turns report their run.** The conversation header shows one state
  for the turn, with Continue offered only when the server would accept it.
  Messages reopen a finished conversation deliberately; a cancelled run stays
  cancelled and a continuation is a new run.
- **Retry is a person's decision.** `run.retry` refuses a team run, an active
  run and a run with a pending approval, and a cancelled run starts fresh rather
  than pretending to resume.
- **Supervised processes on Windows are job objects.** A child cannot outlive
  devd, and its environment is built from an allowlist rather than inherited
  whole.
- **HTTP effectful calls are runs.** An API write creates an `api` run; a read
  or dry run does not. `deploy_hook` now dispatches `deploy.run` through the
  capability registry instead of starting a second execution path.
- **`team.get` returns the unified run id,** so the team surface links to the
  record rather than reconstructing it.

### Fixed

- **A live stream gap is not a lost history.** Reconnecting rebuilds the
  conversation from durable rows, and the durable timeline is read from the
  server rather than inferred from the frames that arrived.
- **A pending request survives a restart.** Decisions made in another window or
  after a restart read the same approval row.
- **An unknown model window no longer overflows first.** The budget is chosen
  from the catalogue or the model name, with output and tool-call reserve
  subtracted, before the provider has to reject the request.

### Not claimed

- Evidence records what devd observed; it does not prove that an external
  service behaved correctly afterwards.
- A replay reconstructs state from durable rows; it does not reverse an external
  side effect such as a deployment or a payment.
- Cancellation reaches handlers that check `RunContext` and processes devd
  supervises. A third-party program that ignores termination is reported as
  still running.

---

## [0.13.8] - 2026-10-26 - One reply, written as it happens

### Fixed

- **One question is one assistant turn.** A reply is the whole answer to a
  person's message, however many model calls it takes: a line of intent, a tool
  call, the result and the final explanation. Those calls used to render as
  separate assistant messages with their own avatars and timestamps, so one
  question could look like two assistants answering. They now share one turn and
  one attribution, in the order the agent produced them; a person's message is
  its own row.
- **The same answer is no longer shown twice.** While a reply was being written,
  the live text and the persisted reply could both be on screen. The live text is
  now an overlay that is cleared as soon as the reply is logged, leaving the
  persisted event in exactly the same place.
- **Overlapping reads cannot duplicate a reply.** The timer, the live channel and
  a person's click can all ask for new events at once. An event at or below the
  last id already shown is ignored, so whichever answer arrives second cannot
  append it again.

### Added

- **Replies stream as the provider writes them.** The agent asks Anthropic and
  OpenAI-compatible providers for their event stream, reads it as it arrives, and
  publishes batched `chat.delta` frames. The window paints the growing text into
  the turn being written instead of waiting for the whole paragraph. Providers
  that ignore `stream` still work: their answer arrives whole and is logged once.
- **`GET /api/events` is the window's live channel.** Server-sent events carry
  `chat.delta`, logged chat events, approvals, inbox messages and team activity.
  The stream is read-only and carries only what the screens already show;
  `EventSource` cannot send the bearer token, so this one route is fenced by the
  loopback `Host` and `Origin` checks instead. Noisy process output is not
  forwarded.
- **The turn settles before the dots return.** The working indicator gives way to
  the streamed text, and the caret belongs to the last paragraph rather than to a
  line of its own.

---

## [0.13.7] - 2026-10-26 - One dropdown, and the truth about the project

### Fixed

- **Every dropdown is the composer's dropdown.** The provider and model pickers
  were the reference; every other select in the application kept a different
  shape. `.input`, `.select`, `.select.compact`, `.role-provider` and a bare
  `<select>` now all resolve to one control: the same surface, border, height,
  chevron, padding, hover and focus ring, on every screen. The conversation's
  project picker stays one step smaller because it sits in the one-line status
  strip, and it is still the same control.
- **Stop, then send, then stop again.** While the assistant works, the primary
  control in the composer is a filled stop square; typing replaces it with the
  send arrow in the same place; clearing the draft brings the stop back. The
  behaviour was already computed, but a second rule ran after it and hid the
  stop again, so the state now has one owner.
- **A project inside the devd home is readable again.** The file fence tested
  whether a path started with the devd home, so with a portable install at
  `D:\devd` every file under `D:\devd\www\...` was refused as "devd's own data
  directory". Agents fell back to `git show`, and the assistant told the person
  it "cannot read files here". The fence now names devd's own folders (state
  database, secrets, logs, services, runtimes, backups, ...) and leaves the rest
  of the home - the projects folder included - to the person. Secrets files are
  still refused by name, and `.git` is still git's.
- **The assistant no longer infers a project from what devd serves.** It has the
  project's detector output in its context (stack and evidence, package manager,
  runtime requirements, ambiguity) and is told plainly that a site is an address,
  not a property of the project. A folder served as PHP that is really an Expo
  app is now described as an Expo app.
- **A refused read is reported as a refusal.** The prompt says to check the path
  and name what was refused, never to conclude that it cannot read files and
  never to reach for git as a way around the policy.

### Changed

- **The "Used N" line belongs to the steps it counts.** Consecutive actions are
  grouped under one heading inside the same block, instead of a caption sitting
  a hundred pixels above them in the stream. The heading is the same width as
  the rows and one type step smaller.
- **Longer text areas where a line-oriented list goes.** MCP arguments (2 → 5
  rows), MCP environment (2 → 4), MCP edit arguments (3 → 5), the team's
  definition of done (3 → 4), a schedule's extra proof (2 → 4).
- **The stop square is filled.** It reads as "stop" from the glyph alone, not
  only from its colour.

---

## [0.13.6] - 2026-10-25 - The model picker is a selector, not a text box

### Fixed

- **The assistant model picker is now a real selector.** It uses the same
  native `<select>` surface and interaction as the provider beside it. The
  options are the models the selected provider actually lists, the current
  choice is selected, and an unknown current model is shown as such rather than
  silently disappearing. A provider change selects that provider's default
  model, and a choice made before the first message is carried into that
  message instead of being lost when the composer repaints.
- **Sending a message no longer raises a toast.** “sent; the agent is working”
  was the capability's result message being shown as a success notification on
  every submission. The conversation's running state is the acknowledgement;
  failures still report normally.
- **Pin as chapter uses a pin.** The layers icon was the wrong metaphor for the
  action and for the pinned state. A dedicated pin symbol is used for both,
  with the same tooltip, accessible name and chapter behaviour.
- **The terminal belongs to the project, not Stacks and tasks.** The action was
  removed from the Develop page and added to the project page and the command
  palette, where it opens the person's terminal in the project folder with its
  pinned runtimes and `devd` first on `PATH`. The Develop page still detects and
  runs tasks.
- **Tools no longer prints machine implementation details.** The
  `windows x86_64 · package managers: npm` line is gone; the screen says what
  the tools are for and leaves runtimes, services and databases to their own
  tabs.

### Added

- **A startup set for services.** `service.set_startup` records whether an
  installed service is included when `stack up` runs, including the sign-in
  entry. The Services screen shows a Startup checkbox for the services a person
  may choose; the web server remains controlled by `web.use`, and PHP listeners
  follow installed runtimes. Newly installed optional infrastructure (Redis,
  Mailpit, a second database) starts out of the startup set instead of joining
  every startup silently.
- **The active web server is visible and selectable.** When both nginx and
  Apache are installed, the Services screen marks which one serves sites and
  offers “Use for sites” on the other; choosing one updates the setting and the
  startup set together, so the next stack starts the server the person chose.

### Changed

- **Project detection says what it knows.** The Develop project list shows
  “not detected yet” rather than “unknown stack” for rows the list has not
  inspected. Runtime requirements are grouped by tool and keep every source
  (for example `package.json engines.node` and `.nvmrc`) instead of repeating
  the same tool as if it were several unrelated requirements. An unsatisfied
  requirement links to Runtimes rather than implying that a generic install
  will choose the exact version the project asked for.
- **The README and limitations page are UTF-8 again.** An earlier Windows text
  round-trip had left `â€”`, `â€¦` and `â†’` in the first documents a user reads.
  A contract test now refuses that class of regression there too.

---

## [0.13.5] - 2026-10-25 - One control group in the composer

### Fixed

- **The model picker now matches the provider picker.** In the Assistant
  composer the editable model/datalist input now uses the same surface, border,
  height, padding, text colour, hover and focus treatment, and chevron as the
  provider select directly beside it. The two read as one control group rather
  than a select followed by a bare text field.

---

## [0.13.4] - 2026-10-25 - The team room

### Added

- **A written team room for every project.** `team.charter.get` and
  `team.charter.set` store the north star, the human owner, the working
  agreement and the roles invited to propose next work. The charter is
  human-only, persists under the devd home, and is shown on Team > Team room.
- **One shared context for the assistant and the team.** `team.context` assembles
  the project's sites and addresses, managed services, installed runtimes,
  `AGENTS.md`/`CLAUDE.md` knowledge, recent lessons and the trust record. The
  same bounded brief is injected into every assistant turn and every specialist
  prompt; the window shows it rather than hiding it in a prompt.
- **A trust record.** The team page counts accepted work, work accepted by hand,
  unverified work and repeated attempts per role. It informs the person; it does
  not bypass approval or the role's tier.
- **Lessons and missteps in the record.** A finished run appends the failures,
  unverified tasks, hand-accepted work and repeated attempts to its summary and
  to the context of the next run. These are derived from task records, not from a
  model's recollection.

### Changed

- The `Team` screen is now the room plus the roster: north star, owner,
  agreement, sites, services, runtimes, project knowledge, trust and lessons,
  followed by the people who do the work.
- Assistant and Team runs both offer a direct link back to the room.
- PHP FastCGI workers are grouped as one `PHP 8.x` service in the shared context
  instead of repeating every worker.

### Fixed

- A project with no site no longer shows a visible “no site” label; the absence
  is represented by the absence of a serving mode, not by a label that competes
  with real status.

---

## [0.13.3] - 2026-10-25 - The interface is a workbench

### Changed

- **Sites is a first-class destination.** Projects, Sites and Environment now
  answer the three questions a person actually has: what am I building, at what
  address is it reachable, and what is running underneath it. Hosts and
  certificates live with Sites; services, runtimes and databases live with
  Environment.
- **Home is the workbench.** A compact environment ledger shows services, sites
  and runtimes beside the projects and their addresses, followed by what needs a
  person and what recently happened.
- **Lists tell the truth.** Projects distinguish loaded, empty and failed; a Git
  repository the operating system will not let Git read says so on the row; a
  site with no entry point is not presented as if it can serve; a stopped stack
  is a state, not automatically a warning.
- **The environment screen no longer duplicates the site list**, and Databases
  explains a stopped MariaDB instead of printing the failed `mysql` command.
- **Every screen names itself.** Pages that previously hid their title because a
  tab repeated it now show it, including Projects, Services, Runtimes and
  Databases; the Assistant and Team workspaces have the same page heading.
  The divider under the page header is gone.
- **List search is one control.** Sites, Skills and Tools now use the same flat
  search field as Projects instead of a second icon-and-input design.
- **The model catalogue is the configured catalogue.** It lists the models of
  the providers in this installation, marks each provider's default, and allows
  changing it. The official Claude list is only a reference and appears only
  when Anthropic is configured.

### Fixed

- **Double-encoded UI text is repaired.** A Windows text rewrite had encoded
  `…`, `·`, `→`, `—`, `≥`, `≈`, `✓`, `★` and `–` as their UTF-8 bytes read through
  Windows-1252, so the window showed strings such as `â€¦` and `Â·`. The UI source
  is now plain UTF-8 again.
- **A UI contract test now refuses that class of regression.** Every embedded
  interface file is checked for `Ã`, `Â`, `â€`, a UTF-8 BOM and U+FFFD, so a future
  ANSI round-trip fails the build instead of shipping.

---

## [0.13.2] - 2026-10-25 - Approval is the person's authority

### Fixed

- **An approval now runs with the person's authority.** devd queued the request,
  took the person's approval, then re-dispatched the operation as the agent — so an
  approved path outside the project was refused by the same fence that raised the
  request. The decision is recorded against the person and the operation now runs as
  the person approved it.
- Two regression tests now prove both sides: deleting inside the project is
  automatic, and deleting outside waits for a person and actually executes after
  approval. Declining still leaves the file and tells the agent not to retry.

---

## [0.13.1] - 2026-10-25 - The project is the workspace

### Added

- **A portable install can put sites under the devd home.** When `DEVD_HOME=D:\devd`,
  a project at `D:\devd\www\app` is the person's work, not devd's database, keys or
  logs. File tools, Git and project tasks now cross that boundary; state, services,
  keys and logs stay fenced. This was the reason the assistant refused every file
  operation in a normal portable install.
- **An implementer may clear files in its own task worktree.** `fs.delete` there is
  ordinary work, not a destructive escalation; read-only roles still cannot, and
  every path outside the worktree keeps the role's normal ceiling.
- **`delete_file` is a direct tool**, so clearing a worktree is one obvious call
  rather than a discovery trip through `fs.delete`.
- **Reviews and Agent memory are tabs of a project.** The separate Studio screen is
  gone from the rail and routes. Every project carries its own review history,
  memory proposals, instruction-file audit and the action to start a review.
- **Uploads are readable by the agent.** Attached files live under the devd home's
  `uploads` folder, which is now a workspace root; the rest of the home is still not.
- **The composer accepts dropped files** as well as the file picker.

### Changed

- **Stop, send and microphone share one primary slot.** While it works and the box
  is empty the slot is Stop; once there is text it is the queue arrow, in the same
  place. Stop no longer sits beside the slot.
- **One select style everywhere.** The Home project picker's surface, border,
  chevron and focus treatment now apply to the conversation project, provider and
  form selects.
- **Home → Ask attaches the project.** The row's Ask action opens a new Assistant
  conversation with that project selected and the message field focused.
- **The queue acknowledgement is quiet.** Sending while the assistant works no
  longer raises a toast; the header count is the feedback.
- **Tool actions in a reply are tighter.** The vertical gap between calls was
  reduced without losing the connecting rail.
- **Remove and Forget use the destructive text colour**, not the neutral ghost one.
- **Every overlay has the same close grammar.** The approval modal and dialog gained
  a top-right X; the drawer's Close is pinned to the far right of its header behind
  an ellipsised title; a drawer scrim, the modal backdrop and the folder-picker
  backdrop all close on outside click, and focus stays inside.
- **The Implementer role is `exec`, not `admin`.** It changes code and runs checks;
  installing machine-wide tools belongs to Infrastructure and still asks a person.

### Fixed

- A file-tool path was rejected as “devd's own data directory” whenever the project
  lived under the devd home. A site under `www` is now a project; `state.db`,
  services, keys, logs and runtime files are still refused.
- The role gate refused `fs.delete` for an implementer even inside its own worktree.
- The project page had no reviews or memory; the Studio route was the only way in.

---

## [0.13.0] - 2026-10-24 - Other programs' MCP servers; a conversation that holds still

### Added

- **devd connects to other programs' Model Context Protocol servers**, the way Claude Code connects to
  the servers configured for it. `mcp.add` stores a stdio server (a program devd starts and speaks
  JSON-RPC to) or a Streamable HTTP endpoint; `mcp.update`, `mcp.remove`, `mcp.enable` and `mcp.trust`
  manage it; `mcp.test` performs the initialise handshake and records the tool catalogue; `mcp.list`
  shows it. The assistant sees each tool as `mcp__<server>__<tool>` and calls it through the same
  capability registry, approval queue and audit trail as everything else. `mcp.import` reads
  `~/.claude.json`, Claude Desktop's `claude_desktop_config.json`, Cursor's `mcp.json` or a project
  `.mcp.json` and adds the servers it finds.
- **A server's tools are untrusted by default.** `mcp.trust` records `ask` (every agent call waits for a
  person), `allow` (a person opted in) or `deny` (agents never see it). Only a person may change it.
  The handler for the trusted path checks the stored decision itself, so the permission is enforced
  where the call runs, not in the prompt. A stdio server is started with a small, named environment
  (never devd's), inherits no API keys, and its environment values live in an owner-only file outside
  the database. Tool results are data: the prompt says so, and a result that is marked `isError` is
  recorded as a failed tool result rather than a success.
- **The Projects list includes every folder a site serves**, not only the folders someone imported or
  indexed. A folder added through the Sites screen is a project the person is working on.
- **The assistant knows the calls it uses most** (`project.detect`, `project.run`, `fs.read`/`fs.edit`,
  `git.status`, the todo list) without spending three turns discovering them, and its open todo list is
  carried into every turn.
- **Common operations are direct tools, with schemas from the registry.** `read_file`, `write_file`,
  `edit_file`, `list_files`, `search_files`, `project_info`, `project_tasks`, `run_task`, `git_status`,
  `git_diff`, `todo_list`, `todo_create` and `todo_complete` are offered beside the five discovery tools.
  Each is a name for a registered capability: its schema, description and permission rules are the
  capability's own, and the call is dispatched through the same registry, approval queue and audit
  trail. An agent no longer spends a turn describing `fs.read` before reading a file.
- **A message written while the assistant is working is queued, not refused.** It is kept in a durable
  queue and appended as an ordinary user turn at the next step boundary, where no tool call is waiting
  for its result; the conversation header shows how many are waiting. Stopping discards what was
  queued, and the queue survives a restart (the host delivers it and resumes the conversation).
- **The plan is visible above the conversation.** The todos the assistant writes with `todo.create` are
  shown as a compact strip (done/total and the next few titles) with a sheet listing every item, and are
  re-read while the conversation is open. It is the same project state shown in Agents → Notes.
- **The screen stays in step with the disk.** Projects, Sites, Inbox, MCP, Home and a project page are
  re-read on a cadence (5-20 seconds by view) and repainted only when the answer changed; nothing is
  replaced while a person is typing, has a sheet open or is selecting text. A changed page keeps its
  scroll position.
- **A site's web-server configuration is reconciled when another program changes what it serves.**
  A background loop in the process that owns devd's background work compares each site's effective
  serving and rewrites the web-server configuration when a file appears or disappears, checks it with
  `-t`, and only then reloads. A failed check leaves the running server on its working configuration.
- **The message box grows with the draft**, up to 40% of the window, and then scrolls inside itself. It
  returns to its resting height when emptied, so the conversation is not left cramped.

### Fixed

- **The conversation no longer jitters while it streams or while you read it.** The stream rebuilt its
  entire DOM every 800ms, changed or not, which dropped the browser's scroll anchor and could pull the
  view under a reader or a text selection. It now repaints only when something it shows has changed,
  puts the scroll position back exactly when the reader is not at the bottom, and leaves a selection
  alone. Idle conversations do no DOM work at all.
- **Removing a project from devd now sticks.** `git.status` is a Read capability and wrote a repository
  index row, so opening a project's page re-added it after `project.forget` removed it. Reading a status
  no longer writes, and `project.forget` matches the folder by identity rather than by spelling, so
  `D:\x`, `D:/x` and a different case all remove the same row. The Projects list has a Remove action.
- **Buttons have no underline**, including a link styled as one ("Choose a provider") and the text
  buttons under a page.
- **An agent cannot hand an interpreter code through `project.exec`.** `php -r`, `node -e`,
  `python -c`, `npx -c`, `ruby -e` and `php artisan tinker` are refused for an agent, with the flag
  named; a person at the command line is not restricted. The plan says it before anything runs.
- **A credential in an MCP server's arguments is redacted** wherever the server is listed or described,
  including in plans a person is asked to approve. An MCP address carrying a username or password is
  refused; a credential-looking URL query value is redacted. Secrets belong in the owner-only
  environment file. An external server's own tool description and schema are capped before they reach
  the model.
- **An MCP server can be edited from the window**, and an update keeps every field it was not given:
  trust, enabled, timeout, arguments and the stored environment are no longer reset by changing one
  thing. "Keep" is the default, so a partial edit cannot silently un-trust a server.
- **The interface was audited as a system**: six unused icon symbols were removed and every remaining
  icon resolves; the tertiary text colour now clears 5:1 and control borders have a 3:1 token; selects
  have a consistent chevron, dark option menu, hover and focus state, and no longer fall back to the
  system's white dropdown. Create/read/update/delete was exercised against a disposable home for
  projects, sites, env, cron, schedules, todos, notes, MCP servers and providers (23 paths), including
  that removing a site or project never deletes its folder.
- **A PHP pin left on a site that does not serve PHP no longer steers the terminal.** The launcher
  follows the site's effective serving, so an unused pin falls through to the project pin or the
  default.
- **The web server's configuration is re-derived every time it starts.** It used to be rendered only
  when the file was missing, so an upgraded devd kept starting nginx on the file the previous version
  wrote. Rendering is atomic and idempotent, and a test proves a stale file is replaced.

### Changed

- Migration 015 adds `mcp_servers`. It is additive: an older devd ignores the table, and an existing
  database gets it the first time this version starts.
- Migration 016 adds `chat_queue` for input written while the assistant works; both migrations are
  additive and an older devd ignores them.
- `project.list` rows say `source: "site"` for a folder that is only known because a site serves it,
  and every row lists the sites that serve it.

---

## [0.12.0] - 2026-10-24 - Sites know what they serve; agents wait for code they write

### Security

- **An agent can no longer have devd run code it wrote without a person's approval.** The Exec tier needs no
  approval by design (it starts what devd already manages), but `cron.add`, `queue.add`,
  `deploy.script_set`, `deploy.vars_set` and `laravel.artisan` take a command line or
  script from the caller and were Exec and agent-callable: an agent could add a cron job running any command
  every minute. They now carry `agent_needs_approval` and support dry runs, so the approver reads the code in
  the plan. A person calling them is not asked. A test walks the shipped registry and fails when a new
  agent-callable capability takes a `command`, `script`, `steps`, `argv`, `program` or `directives` argument
  and is neither gated nor on the reviewed list. `project.exec` / `project.run` remain open, recorded in
  `docs/limitations.md`.

### Fixed

- **A Go module or a Node app was listed as "PHP 8.5", and its source folder was served.** devd had no idea
  what a site served: every directory site got PHP in its nginx and Apache configuration, `site.sync` made a
  site of any folder, and `site.create` pinned today's default PHP into the record, so six of twelve sites
  froze at 8.5.12 while the default moved to 8.4. A site is now served as what it is (`proxy`, `php`,
  `static`, or `no_entry`), read from the folder unless a person chose; PHP is configured only for a site that
  serves PHP, a PHP version is shown only there and says whether it is pinned or follows the default, and a
  directory with nothing to serve has no document root and answers with an explanation. A leftover pin on a
  site that does not serve PHP is reported as unused and can be cleared.
- **Parking a folder made a site of everything in it.** `site.park` now only records the folder and reports
  what is in it; `site.sync` registers only projects with a web entry point, names what it will create in its
  plan, and skips the rest with the reason. New: `site.discover` (read only), `site.set_serving`, and
  `site sync <names>`.
- **`site.create` wrote an `index.php` into any existing project that had no index file.** A starter page is
  now written only into an empty directory.
- **`doctor` changed the database it was diagnosing.** Opening it created the file and folder, switched the
  journal mode and migrated the schema. Diagnostics (`doctor`, `health.status`, the `php`/`composer`
  launchers) now open it read-only; `doctor` reports a database that is behind the build as waiting to migrate,
  and the window's Diagnostics checks the database the process is actually using.
- **Two devd data folders on one machine went unnoticed.** A process that keeps an old `DEVD_HOME` works on a
  different devd than a new terminal or the window. `doctor` now reports it (`home.conflict`, `home.other`);
  it never picks, moves or deletes one.
- **Nobody was told that scheduled jobs were not running.** Diagnostics has a Scheduled jobs component:
  healthy when a devd process runs them or none is scheduled, degraded and explicit otherwise.
- **The no-entry explanation and its docs named a command that does not exist** (`set-upstream`); it is
  `devd site upstream`.

### Changed

- **Sites know their project.** `Site::project_dir()` is the one definition of which project a site serves;
  `site.list` returns `project` and `project_known`, and `project.list` returns each project's `sites`.
- Migration 014 adds a nullable `sites.serving` column. Existing rows are untouched, an older devd ignores the
  column, and it is applied the first time a new devd starts.

---

## [0.11.1] - 2026-10-24 - Touch rows, honest diagnostics, scoped CA trust

### Added

- **`php` and `composer` work from any terminal.** Composer reported "'php' is not recognized"
  because devd's PHP is not on `PATH`, and putting each version's folder there would break when
  the default changes. devd now keeps one folder, `<home>/bin`, of launchers that ask devd at the
  moment of the call which PHP applies: the PHP the site in the current folder is served with, else
  a pin, else the default. `devd path status|enable|disable` (also **Server → Runtimes → Command
  line**, and the Windows installer) manages the one `PATH` entry; the launchers appear when PHP or
  Composer is installed, follow devd if it moves, and go when what they run does. Arguments and exit
  codes pass through, including `--json`, `-y` and `--plan`. New capabilities: `shell.status`,
  `shell.resolve`, `shell.path_enable`, `shell.path_disable` (human only), `composer.install`.
- **`devd composer install`** downloads the newest stable Composer and checks it against the SHA-256
  getcomposer.org publishes for that release; it runs with devd's PHP, so it needs no PHP on `PATH`.

### Fixed

- **`doctor` and the launchers no longer create a devd home.** A stale `DEVD_HOME` naming a folder
  nobody made left a second, empty home behind after one `doctor` run. `doctor` now reports that the
  folder does not exist and checks nothing in it; a launcher says where it looked and stops.
- **Row actions work on a touch screen.** Manage and Open on sites, and Stop,
  Restart, Logs and Remove on services, were revealed only on hover. A phone or
  tablet could see the row but not the controls. They are now visible where the
  device cannot hover, and they move to their own line in a narrow window.
- **Diagnostics no longer calls devd's own web server "another program."** The
  HTTP and HTTPS port notes name the process that is listening and explain that
  devd's own nginx or Apache holding the port is expected.
- **Trust checks and "remove trust" identify this installation's CA.** The
  Windows store was queried and changed by the shared common name, so a second
  devd installation could be reported as trusted when only an older CA was, and
  removing trust could remove every devd CA. The certificate serial now scopes
  both operations. A regression test covers a fresh CA on a machine that already
  trusts another devd CA.
- **The site Database tab says "No database managed by devd"** instead of
  implying the application has no database, so SQLite and external databases are
  not misreported.
- **Redis starts on a clean Windows install.** The Redis build devd downloads is an
  MSYS program that reads a `C:\...` configuration path as relative to its working
  directory, so it exited at once with an empty log. devd now passes it as
  `/cygdrive/c/...` (only for a folder that carries the MSYS runtime; an adopted
  native build keeps the ordinary path) and works the command out again at start, so
  a record written by an earlier devd starts too.
- **The first-run sheet's "Install PHP" works.** The requirement's fix called
  `runtime.install` without the `version` it requires, so the button failed with a
  400 and, because the message was a brief toast, looked like it did nothing. The
  sheet also redraws after a fix instead of still listing what was just installed.
- **"Set up devd" no longer tries to reinstall PHP that is already there.** It read
  the runtime kind from the wrong level of `runtime.list`, never saw PHP, hit
  "already installed" and stopped before nginx, MariaDB and the first site.
- **Redis and Memcached offer Install on Services**, as they have since 0.10.0, and
  `php-cgi` is no longer listed as "available" beside the PHP pools that run.
- **Port notes in Diagnostics say it once.** The explanation no longer repeats the
  line that names the program holding the port.
- **The starter page names the web server that answers**, not always nginx.
- **The install-failure message lists memcached** among the installable services.

### Changed

- **Version 0.11.1.** Capabilities were added (below); none was renamed or removed.

## [0.11.0] - 2026-10-24 - Server tabs by scope, one page per site

### Added

- **A page for each site.** `/server/sites/<name>` has its own tabs: Overview, Environment, Deploy,
  Cron, Queue, Logs, Laravel, Redirects, Web server, Database and Preview. The Sites list and the
  server Overview now offer **Manage**; share, remove, PHP version, PHP limits and every site-scoped
  operation live on that page instead of a site picker.
- **Server -> Databases** holds database users and scheduled backups for the whole machine.
- **Server -> Remote** holds remote servers and SSH keys. Deploy links and remote servers are
  still linked from a site's Deploy tab.
- **Diagnostics** has a Notifications section for alerts, so Alerts is no longer a server tab.
- **Runtimes** now carries the PHP limits profile, worker count and extension list.
- **Overview** shows the machine's CPU, memory, disks and uptime beside services, sites, pending hosts and certificates.

### Changed

- **Server tabs follow scope:** Overview, Sites, Services, Tools, Runtimes, Databases, Remote,
  Certificates, Hosts. Operations and Preview are removed from the strip; their old routes stay as a
  one-release fallback for bookmarks.
- **Sites is a list.** Each row has Manage and Open. Inline share, remove and PHP controls moved to
  the site page.
- **Tools is only CLI tools and package managers.** Databases, services, containers, nginx and the
  runtimes managed by Runtimes (PHP, Node, Go, Python, Ruby) no longer appear there.
- **The site Database tab is read-only.** It shows the database name and whether a backup is
  scheduled; users, schedules and dump files stay under Server -> Databases.
- Health remains in Overview and Diagnostics; the old Operations screen still works but is hidden
  for one release.

### Fixed

- **MariaDB appearing twice on Services.** devd's MariaDB runs as `mysqld.exe` on Windows. The
  "Already on this computer" inventory saw that process holding port 3306 and listed MariaDB and
  MySQL as "Not installed, port held by mysqld.exe", next to the managed MariaDB row. Inventory
  rows whose port owner is a devd-managed process are now omitted, and programs that are not
  present are no longer counted under "Already on this computer".

---

## [0.10.0] - 2026-10-24 - Operations, remote deploys and PHP pools

### Added

- **Operations**, what a server panel shows for a site, as capabilities and a **Server → Operations** screen: the
  `.env` file with secrets hidden and a diff against another environment (`env.*`), cron jobs and the Laravel scheduler
  (`cron.*`), queue workers (`queue.*`), redirects written into nginx and Apache (`redirects.*`), the Laravel log as entries
  (`logs.*`), a deploy script per site with history, rollback, a repository link and a push-to-deploy hook (`deploy.*`,
  `repo.*`), and alerts to the inbox, Slack, Discord or a webhook (`notify.*`). See [`docs/operations.md`](docs/operations.md).
- **PHP workers**: a pool of PHP processes per version (`php_workers`, default 4) behind an nginx upstream, so a page that makes
  many requests no longer queues behind itself.
- **PHP settings** (`php.settings`, `php.configure`, `php.extensions`, `php.extension_set`): a `local` or `production` limits
  profile (128M / 5M / 30 s, like a typical server), single values, and an extension manager.
- **Redis and memcached are downloaded** on Windows and checked against the SHA-256 GitHub records, like Mailpit.
- **`devd setup --services`, `--php`, `--profile`** choose what a first run installs.
- `php` command group; `devd call` reaches every new capability.
- **A PHP limits profile per site** (`php.site_profile`): a site can run with the other profile, served by a second pool of PHP
  processes started with that profile's ini.
- **Deploy warnings** (`deploy.check`, `server.limits_set`): every PHP limit that is looser here than on your server, in the
  deploy plan and the deployment's log. devd is told the server's limits and assumes a typical production server until then.
- **Apache balances PHP workers** through `mod_proxy_balancer` when the build has its modules.
- **Set up devd** on the Overview screen: a first run shows what it will install, with sizes, and lets you choose.
- **Deploy to any server over SSH** (`server.*`, `remote.*`): register DigitalOcean, Hetzner, Vultr or any other SSH machine; devd pins
  each host key in its own file, provisions a fresh Ubuntu or Debian server for PHP sites, and deploys either as atomic releases
  (local build, upload, switch `current`, instant rollback) or by having the server pull and run the deploy script. See
  [`docs/operations.md`](docs/operations.md).
- **The Laravel tab** (`laravel.*`): artisan commands, maintenance mode, `storage:link`, clearing caches.
- **Database users and scheduled backups** (`db.user_*`, `backup.*`).
- **Per-site nginx directives** with presets, checked by `nginx -t` and rolled back on failure (`nginx.*`).
- **Deploy variables, scheduled deploys and staging sites** (`deploy.vars_set`, `deploy.schedule`, `deploy.staging_*`).
- **Health checks and machine monitoring** (`health.*`, `monitor.stats`) and **SSH key management** (`ssh.*`).
- PHP processes are shown as one row per version; the old single `php-cgi` listener is retired.

- **Apache** as a web server (`web.use`, `service.install apache`, `service.adopt`). An existing Apache is used as it
  is; Apache Lounge's build is installed against the SHA-256 it publishes. `.htaccess` works; PHP goes to the
  same FastCGI listeners nginx uses.
- **Park a folder** (`site.park`, `site.sync`): every project directory becomes `<folder>.test`.
- **Database moves** (`db.dump`, `db.restore`, `db.copy`) from another MySQL on this machine into devd's MariaDB.
- **Redis** (adopted) and **Mailpit** (downloaded, digest-checked) as managed services; PHP mail goes to Mailpit.
- **Hosts file with a consent prompt** on Windows (`hosts.sync --elevate`) and **start with sign-in**
  (`autostart.*`).
- **A definition of done** on team runs, judged by Jev (`judge.*`), with extra passes until it is met.
- **Run templates** and **schedules** (`template.*`, `schedule.*`): start the team by name and on a clock.
- **Your own roles** in `<devd home>/roles/`.
- **Studio**: reviews of a real diff by the reviewer model (`review.*`), `AGENTS.md` changes proposed as a diff and applied only
  by a person (`memory.*`), and an audit of the instruction files agents read (`compat.*`). See [`docs/workflows.md`](docs/workflows.md).
- **PostgreSQL, MongoDB and Memcached** as adopted services; `db.*` takes `engine: postgres`.
- **Python and Ruby runtimes** (`runtime install python@3.12`, `ruby@3.3` on Windows) and `.7z` archives; **per-project pins**
  (`runtime.pin`, `runtime.pins`) so a project's commands run with the version it names.
- **Public share links** (`share.start`, `share.stop`, `share.list`): a Cloudflare quick tunnel with an expiry, shown on the site.
- **`project.terminal`**: the system terminal in a project folder with its pinned runtimes first on PATH.
- `site.fix_htaccess`: guard `php_value` lines Apache cannot read; PATH_INFO works under Apache.
- `Registry::define`: capabilities are declared with named properties instead of a twelve-argument helper.
- **Changing the ports sites are served on** (`web.ports`, `devd web ports --http 8080 --https 8443`): writes
  `settings.toml`, rewrites the generated configuration, moves the web server's recorded port with it and restarts
  the running server. A port another program is listening on is refused.

### Changed

- Only the process that owns the agents recovers interrupted conversations and runs; a one-line command beside the
  application no longer restarts its live work.
- A person who starts a run on a folder imports it as a project so the team may work there.
- The window shares one spacing rhythm and one set of controls across every page: page blocks are spaced by the page
  rather than by each component, panels inside a page no longer stack double margins, tables scroll instead of
  stretching the layout, chat prose uses the reading column, and the working tree, commit box and empty states are
  built from the same tokens as the rest of the application.

### Fixed

- **Composer version constraints such as `^13.0` reached Composer as `13.0`** when run through a `.bat` shim on Windows, because `cmd.exe` eats `^`. Arguments with cmd metacharacters are now quoted.
- Composer's coloured `--version` output hid its version from detection; colour codes are stripped first.
- **A port changed outside devd no longer makes a correct start look broken.** `service start` reads the HTTP port
  from `settings.toml` before it verifies the start, instead of waiting on the port recorded at install time, and the
  nginx and Apache configurations are rendered with the current ports.
- **Apache's current checksum format.** Apache Lounge now publishes `SHA256-Checksum for: <file>:` followed by the
  digest on the next line; the catalogue reads the digest token instead of the old `SHA256:` label.
- **`service install --plan` names the build the install will actually use.** An nginx release with no recorded
  digest is skipped for `latest`, and an explicitly requested unverifiable build is shown as refused rather than
  promised.
- The installer's launcher and uninstaller honour `DEVD_HOME`, so a portable install such as `D:\devd` reads and
  keeps the right data folder. The Linux uninstall also removes `devd-desktop`, with or without `--desktop-binary`.
- The runtime catalogue names the executable each archive actually yields after devd strips its top-level folder
  (`bin/node`, `bin/go`), instead of a path that only happened to be unused by the installer.

### Removed

- **The "Use existing" button beside an available service.** The Available list offers Install; a service that is
  already on the machine is still listed under **Already on this computer** with **Run it with devd**, which records
  the choice without moving or changing the existing folder.
- The **Terminals** tab and everything behind it: terminal sessions, delegation to Claude Code and OpenCode
  (`session.*`, `agent.*`), the PTY manager, xterm and `@agent` mentions. devd has its own agents.

---

## [0.9.0] - 2026-10-21 - a folder picker, official brand marks, and a stylesheet that works

devd now lets you find a project by looking at your device instead of typing
its path, shows the owners' own marks beside the services and providers it
supports, and — after a one-character fault in the stylesheet — actually
adapts to the window you are in.

### Added

- **Choosing a project folder by looking at your device.** The window and the
  web application now have a folder picker, so no project ever has to be added
  by typing a path.
  - The desktop window opens the **operating system's own folder dialog**
    (`POST /api/pick-folder`, implemented with `rfd` in the shell). The picker
    returns a path and nothing else: it cannot read a file or run a program.
  - The web application draws its own folder browser, listing the places work
    is kept, the filesystem roots, the path as a trail you can step back
    through, and the folders that are there, with symbolic links marked.
  - The highlighted folder is inspected before you commit to it: the detected
    stack, the package manager, the runtimes it asks for, whether it is a Git
    repository, and anything that is missing.
  - The same picker is used wherever a folder is needed: the Projects screen,
    the Develop tab, the agent's working directory, and the destination for a
    clone.
  - In a terminal, `devd project browse [path]` prints the same listing and
    marks the directories that hold a project.
- **Official brand marks.** 51 marks - the model providers, the databases,
  caches, search engines, runtimes, tools and editors - are served from the
  binary and shown beside the thing they name. Each one is the owner's own
  artwork, taken from their own site or from a curated brand registry, and
  never recoloured or distorted.
- **A documented plan for the comment-debt remediation** in
  `docs/research/00-comment-debt.md`, with the evidence for each finding and a
  separation between the changes that are safe now and the ones that need
  architectural work.
- **Research notes** in `docs/research/`: the investigation of Laravel Herd
  2.0.2 with the evidence for each observation, a feature acceptance matrix
  that maps every Herd capability to its DEVD equivalent, and a full screen
  inventory.
- **A Linux installer**, `scripts/install-linux.sh`: a per-user install with no
  administrator rights, a desktop entry, and an uninstall that keeps your data
  unless you ask it to remove it.

### Fixed

- **The responsive half of the stylesheet was dead.** A malformed rule left a
  brace open, so the browser silently ignored everything after it - including
  every breakpoint below 960 pixels. The application was unusable on a narrow
  window and nobody had noticed, because the fault was in the file and not in
  the browser. A contract test now checks that the stylesheet is balanced.
- **The folder list could take the whole screen down.** A missing element id
  made the picker throw instead of rendering. A contract test now checks every
  id the script looks up, and another checks that no action handler is empty.
- **`hidden` did not hide anything** that had a `display` rule of its own.
- **`git.reflog` returned text where the interface expected structure**, so the
  Recover tab showed blank rows and offered to reset to an empty revision. It
  now returns the hash, the reference, the time, the action and the subject.
- **A provider whose id was not a preset id could not be edited**: the
  interface sent the provider's own id as a preset and the server refused it.
- **The Recover tab and the Changes tab gave a conflicted file different
  colours.** Untracked and conflicted are now different badges.
- **"Dry run" on a capability that cannot plan threw an error.** It now says
  so.
- **A failed request for the activity log looked like an empty history.**
- **The number of stacks was written into the interface.** It now comes from
  the server.
- **The Diagnostics screen claimed that services, sites, runtimes, DNS and
  certificates had no capabilities** - on the same screen that renders all
  five.
- **The runtime installer dropped its "upstream publishes no digest" warning**
  because a later binding shadowed it.
- **`devd setup` aborted on Linux**, where devd publishes no PHP or service
  binaries. It now uses the system's own PHP and services, says so, and reports
  what is left to do instead of failing.
- **Two collection screens were unbounded**: the activity log and the
  capability registry. Both are paged, with a real count of what is showing.

### Changed

- **One list of projects.** `project.list` returns the union of imported
  projects and indexed repositories, so the window, the terminal and an agent
  all see the same projects. The window's Projects screen used to read a
  different list from the Develop tab.
- **"Remove from devd" and "Delete from disk" are now different things.** The
  first forgets a project and keeps the folder; the second deletes the folder
  and asks you to type its name first.
- **One name per destination.** The rail, the tab strip, the command palette
  and the breadcrumb all call a screen the same thing; the page heading may be
  longer.
- **The installer can be repaired.** The Windows install is no longer marked
  "no repair", and the uninstaller offers to remove your data as an explicit,
  separate choice that is off by default.
- **The uninstaller says what it keeps.** Your projects, databases,
  certificates and settings stay in `%LOCALAPPDATA%\devd` unless you ask for
  them to go.

### Changed (earlier in this release)

- **One logo, everywhere, generated from one file.** The artwork now lives in
  `assets/devd-logo.png` and nothing else is a source of truth. `scripts/make-icons.py`
  generates the desktop icons (16 to 512, and a `.ico` with every size Windows asks
  for), the favicon and the brand mark in the window from that one file, so the icon
  on the taskbar, the mark in the sidebar and the favicon are the same picture
  instead of three near-copies. A test keeps them that way.
- **Motion, rethought against a stricter bar.** Hover states only apply to a device
  that can really hover, so they no longer stick after a tap on a phone. Buttons
  press with a scale rather than a nudge, dialogs and drawers settle with a
  transition instead of a keyframe, toasts can be interrupted and retargeted, the
  spinner turns faster, and reduced motion keeps the fades that explain a change
  while dropping the movement.
- **Phone behaviour.** No grey tap flash, no 300ms tap delay, no pull-to-refresh
  hijacking the scroll, fields large enough that the browser does not zoom into
  them, and content that stays clear of the notch and the home indicator.
- **Worst-case content.** Long names truncate with the full value in the title,
  identifiers and paths wrap instead of pushing the layout apart, and numbers
  that change do not shift the row they sit in.

### Added

- **Requirements, asked for at the right moment.** One capability (`setup.requirements`)
  lists what devd needs on this machine - Git, the PHP runtime, the services it
  manages, a model provider and its key, and the WebView2 runtime the native
  window needs - with the state of each, why it matters, and the exact operation
  that fixes it. The Windows installer asks before installing the WebView2
  runtime, because without it the window cannot open; `devd setup` prints the
  whole list and says what this run will install; the window shows it on the home
  page and in Diagnostics and installs a missing piece through the ordinary
  approval flow; the terminal dashboard lists it and installs the first one with
  `i`. Nothing is installed silently, and an optional thing that is missing never
  makes devd look broken. See `docs/setup.md`.
- **Model providers, completed.** Presets for Anthropic, OpenAI, DeepSeek, Moonshot AI
  (Kimi), Zhipu AI (GLM), Google Gemini, xAI, Mistral, Groq, Together, Fireworks,
  OpenRouter, Ollama, LM Studio and any OpenAI-compatible endpoint, in that order of
  usefulness. Each preset carries where to get a key, how its models are discovered
  (the vendor's own list, Ollama's native `/api/tags`, or a maintained catalogue for
  vendors without a listing API), and whether it runs locally. `provider.verify` runs
  one real request that must call a named tool, so "tool use works" is only claimed
  when the provider actually returned a tool call. `provider.discover_local` finds a
  running Ollama or LM Studio on this machine. Errors now say what is wrong: a
  rejected key, an exhausted quota, an unpaid balance, a rate limit with its retry
  window, a context window that is too small, or a model the account cannot use.
- **Agent team** (`team.*`): describe a goal, a planner proposes tasks with owners,
  dependencies and acceptance criteria, you approve the plan, and specialists
  (architect, researcher, implementer, QA, reviewer, infra, docs) work in parallel
  where it is safe, in isolated git worktrees that are merged only after the task is
  accepted. A task is accepted only when the tool results prove it (files changed
  and a verification that passed, where the role requires it); otherwise it is
  `unverified` and the run pauses for you. Runs persist and resume after a restart.
  GUI (Assistant > Team runs), terminal dashboard (Team and Inbox views), CLI
  (`devd team`) and API share one backend. See `docs/agents.md`.
- **Skills** (`skills.*`): 55 playbooks, techniques and principles agents load on
  demand, plus your own in `<home>/skills/`. `devd skills`.
- **Built-in assistant** with provider management (Anthropic, OpenAI, Gemini, local
  runtimes, any OpenAI-compatible endpoint), official Claude model discovery from
  Anthropic's documentation, per-key model lists, model choices per kind of work
  with fallbacks, an approval queue and an inbox, `docs.fetch` and `search.query`.
- **Operational health model** (`health.status`): healthy, degraded, starting,
  setup incomplete, unknown, critical, per component, with what each means and the fix.
- **Services already on the machine** (`services.inventory`, `services.decide`):
  installed, running, required and managed are kept apart; adopt, observe, use your
  own instance, or ignore. Nothing is replaced as a side effect.
- **Production mode** (`environment.set`): every agent change needs approval.
- Global search and command palette; rebuilt Home; 12-entry sidebar with tabs;
  new logo asset that fits its container exactly.

### Added (earlier in this release)

- **A shared stack engine** (`devd-stacks`). Stacks are structured YAML definitions
  validated when they load: 26 stacks (Next.js, Nuxt, Astro, React/Vite, Vue,
  SvelteKit, Express, Node.js, Laravel, Symfony, CodeIgniter, CakePHP, Slim, Django,
  Flask, FastAPI, Python, Rails, Go, Rust, Spring Boot, ASP.NET Core, Expo, React
  Native, plus detect-only Ruby and Java), 40 tools and 14 package managers. Each
  stack carries its official documentation, version requirements, options mapped to
  the scaffolder's real flags, tasks, services, conventions and where its commands
  were verified. Add a stack by dropping a YAML file in `<devd home>/stacks/`.
- **One initialisation engine** behind `project.init` for the CLI (`devd new`, with a
  guided flow), the terminal dashboard (Create view), the window (Create view), the
  HTTP API and MCP: validation with field-level issues, prerequisite and version
  checks, exact commands shown first, progress events, cancellation, post-creation
  verification (files, git, a build or check command) and honest `ok`/`partial`/
  `failed`/`cancelled` results.
- **Existing projects**: `project.detect` weighs manifests, lockfiles, entry points and
  config to report stack, package manager, required runtimes, tasks, services,
  environment variable names (never values) and missing tools; `project.import`
  registers a checkout without touching it. Ambiguity is reported, not hidden.
- **Task runner**: `project.tasks`, `project.run`, `project.exec`, background dev
  servers with logs (`task.*`). Destructive tasks and commands need a human's
  approval and are refused to agents.
- **Toolchain management**: `toolchain.*` lists, plans, installs, removes and
  controls tools and services (Git, Node.js, PHP, Composer, Python, Ruby, Go, Java,
  .NET, Rust, MySQL, MariaDB, PostgreSQL, Redis, SQLite, Docker, GitHub CLI, adb,
  Android SDK, EAS and more) through the host's package manager. Success is verified
  by running the tool and checking service health, never by an exit code.
- `docs/stacks.md` (generated compatibility matrix, tested for staleness) and
  `docs/stacks-engine.md`.

### Changed

- **One design system across the window, the browser and the terminal.** The web and
  desktop interface is rebuilt on tokens: a single spacing scale, a type scale with
  a defined role for every size, semantic status colours that are always paired with
  a word, and one set of control styles for buttons, inputs, pills, tabs, tables and
  dialogs. The shell has an icon rail that collapses to icons on narrow windows and
  becomes a scrollable strip on a phone; every page has one heading, one action row
  and a content width that stops stretching on a wide display. The agent conversation
  is the workspace it should be: messages, tool executions, results and errors each
  have their own treatment, tool output is collapsed by default and can be copied,
  and the composer keeps the project selector on small screens. Tables scroll
  themselves instead of the page. Focus is visible, dialogs keep Tab inside them,
  the conversation announces new replies once instead of re-reading itself, and
  reduced-motion and forced-colour modes are honoured.
- **The terminal dashboard** now has a header that shows the repository state beside
  the identity, a footer that is cut to the terminal width rather than wrapping, empty
  panels that say which key to press, rounded frames, and `NO_COLOR` support.
- New visual identity: midnight-blue surfaces, azure/cyan/violet accents and a
  halftone dot-matrix logo, in the window, the terminal dashboard and the desktop
  icons.
- The HTTP API runs capabilities off the async workers so long operations never block
  the requests that poll their progress.
- Agent sessions start with `DEVD_PROJECT_DIR` and a hint to call `project.detect`.

### Fixed

- `devd-core` did not compile on Linux (a Windows-only function was called
  unconditionally).
- Port checks now work on hosts without `ss` by reading `/proc/net/tcp`.
- Processes started by devd get their own process group on Unix so stopping them
  stops their children too.


## [0.8.0] - 2026-10-20 - persistent agents, project processes, shared state and a native shell

The v0.7 roadmap is complete: sessions survive the window, agents can delegate
real work to other agents, projects declare their processes, people and agents
share durable notes and todos, and the application is a native desktop app.

### Added

**Persistent agent sessions**

* Every session has a stable id, harness, working directory, owner process,
  dimensions, lifecycle state, exit code, timestamps and a bounded transcript on
  disk. Metadata lives in SQLite (schema version 2).
* The backend owns the PTY. Switching views, opening Preview, switching tabs or
  reloading the page does not stop or restart the harness. Reopening devd
  restores the session list and reattaches to a running session.
* A session list, tabs, rename, stop, dismiss and transcript replay for completed
  and failed sessions. Output is replayed from a 2 MiB live buffer or the last
  256 KiB of the transcript, with a byte cursor and a reset signal.
* Honest reconciliation: a row whose owning process is gone is `stale`; a row
  owned by another live devd process is `unavailable`; neither is reported as
  running and attachable.

**Multi-agent orchestration**

* `agent.spawn`, `agent.wait`, `agent.status`, `agent.list`, `agent.result` and
  `agent.cancel`. A task is a real child PTY running its own harness, not a
  metadata record.
* Parent/child task relationships, depth limit 3, 8 unfinished tasks, wait
  timeout 900 s, task timeout 3600 s, structured results and non-zero exit
  reporting. A wait timeout does not kill the child.
* Claude Code tasks use `claude -p`; OpenCode tasks use `opencode run`; custom
  tasks take an argv array. The child's own permission prompts remain the
  authority.

**Project processes**

* Versioned `devd.yml` with strict validation: argv commands, project-contained
  working directories, loopback-only health checks, explicit environment,
  dependencies, restart policy, startup ordering and unknown-field rejection.
* `project.plan`, `project.approve` (human-only), `project.status`,
  `project.start`, `project.stop`, `project.restart` and `project.logs`.
* A semantic manifest hash must be approved before anything runs; editing the
  manifest invalidates the approval and blocks automatic restarts.
* Supervised processes with captured stdout/stderr, pids, exit codes, restart
  counts, health status, searchable log tails, secret redaction and whole-tree
  termination.

**Shared state**

* Project-scoped scratchpads with authorship, timestamps, optimistic revisions
  and `scratch.append` for agent findings.
* Structured todos with status, priority, assignee, task association,
  dependencies, transition history, completion, reopening and notes.
* `scratch.*` and `todo.*` capabilities are available to humans and agents;
  deletes are destructive and require human approval.

**Native desktop**

* `devd-desktop`, a Tauri 2 window over the existing server and UI. The backend
  starts in-process on an ephemeral loopback port; the window loads the same HTML
  and JavaScript.
* No Tauri commands, no shell/filesystem plugins, no remote IPC access. Top-level
  navigation is restricted to the devd origin; external links go to the system
  browser.
* Graceful shutdown stops PTYs and project processes.

**UI**

* **Agents** with session list, tabs, lifecycle pills, rename/stop/dismiss and a
  delegated-task panel with Wait, Cancel and Result.
* **Processes** with manifest review and approval, per-process start/stop/restart,
  health, pid, restart count, failure detail and filtered logs.
* **Shared** with todos and scratchpads, revision-aware saving and conflict
  reporting.
* Responsive layouts for narrow windows; the terminal remains usable when the
  session list stacks above it.

### Changed

* The CSP allows inline *style* so xterm.js can position its screen; scripts
  remain same-origin only, with inline scripts still forbidden.
* Manifest working directories strip Windows' `\\?\` prefix before they reach a
  child process, so `cmd.exe` and harnesses start in the project rather than
  falling back to `C:\Windows`.
* The registry composition moved into `devd-bootstrap`, shared by the CLI and the
  desktop shell.

### Security

* Sessions record their owning process; only the owner may write, resize or stop.
* Terminal input/resize/rename/dismiss are human-only capabilities.
* Project manifests are executable configuration and require a human approval of
  their semantic hash.
* Project environments are explicit; the daemon's own credentials are not
  inherited.
* The new capabilities are subject to the existing tier, approval, agent-safety
  and audit rules.

### Fixed

* A ConPTY session could remain "running" for ever when its child exited but the
  reader did not reach EOF; a separate child watcher now records the true exit.
* `cmd.exe` treated canonical `\\?\` paths as UNC and started in the Windows
  directory.

### Known limitations

* A PTY cannot survive the devd process that owns it. On restart the row becomes
  `stale` and only its transcript remains.
* MCP runs in its own process, so an agent-spawned session is owned by that
  process, not the window. It is genuine, and it is not attachable from the
  window.
* Project processes are owned by the process that started them; a second devd
  reports them as supervised elsewhere rather than adopting them.
* The Windows installer is not code-signed and the Tauri window requires WebView2.

---

## [0.7.0] - 2026-10-20 — agents in the window, and a built-in browser

The application window now runs the coding agent you already use and shows the
site you are building, without leaving the app.

### Added

**Agents**

* An **Agents** view that starts Claude Code, OpenCode or a plain shell in the
  selected project directory.
* A real PTY per terminal: full-screen harness interfaces, colours, keyboard
  shortcuts and resizing work. Output is polled over the loopback API; keystrokes
  go back the same way.
* The harness runs with its own models, credentials and permission prompts.
  devd does not proxy the model, store the conversation or mediate file edits.
* Harness detection is honest: a card says "not installed" when the CLI is not
  on `PATH`, and nothing pretends to be the tool.

**Preview**

* A **Preview** view that embeds the running site in the window, with a site
  picker, reload and open-externally.
* The iframe is sandboxed, so the previewed page cannot navigate the window.

**Design**

* A new palette: warm graphite surfaces and a single terracotta accent instead
  of the blue-grey default, with larger page titles and a calmer rail.
* The terminal theme matches the window.

### Security

* Every terminal endpoint sits behind the same bearer token and loopback
  defences as the rest of the API. The working directory must exist.
* The preview iframe is sandboxed (`allow-scripts`, `allow-same-origin`,
  `allow-forms`, `allow-popups`, `allow-downloads`, `allow-modals`) and cannot
  navigate the top window.
* The CSP now allows framing HTTP(S) for the preview and still denies
  everything else by default.

### Known limitations

* One agent terminal at a time; reloading the window loses the handle while the
  PTY keeps running.
* Terminal output is polled, not archived.
* Preview refuses sites that send `X-Frame-Options` or `frame-ancestors`.

---

## [0.6.0] - 2026-10-20 — the server dashboard

The application window now shows the machine, not just repositories: the
services, the sites, the runtimes, the certificates and the hosts file, with
controls for each and a live overview.

### Added

**The server views**

* **Overview**: a status banner (running, ports, start/stop all), four counters
  (services, sites, PHP, certificates), and panels for sites, services, hosts
  and the certificate authority.
* **Sites**: the table (URL, PHP, TLS, directory), a create form with hostname,
  HTTPS and database options, open and remove. Removal is destructive and goes
  through the approval dialog.
* **Services**: version, ports, state and PID per service, with start, stop,
  restart, logs in a drawer, and remove. Missing services offer install.
* **Runtimes**: installed PHP, Node.js and Go with their primary marker,
  "install latest", "use for new sites" and remove.
* **Certificates**: the local authority, its trust state, trust for the user or
  the machine, remove trust, and the issued certificates.
* **Hosts**: the managed file, pending additions and removals, what is
  published, and sync behind the approval dialog.

**`tls.ca.status`** reports whether the authority is in the user and machine
stores, by asking `certutil` rather than remembering it in the browser. The
overview's trust notice is now a fact rather than a guess.

**Design pass**, following Emil Kowalski's design engineering principles: press
feedback on buttons (`scale(0.97)`), a visible focus ring on every control,
custom easing curves instead of the built-in ones, no entrance animation on the
command palette (it is opened from the keyboard many times a day), a 200ms
entrance for the modal, tabular numerals for ports and counts, and a responsive
three-to-two-to-one column grid.

### Fixed

* **The environment check took 40 seconds.** Every liveness check and every port
  lookup started a `tasklist` process; a machine with a few dozen listeners
  meant a few dozen of them. Process state and image names are now asked of the
  kernel (`OpenProcess`, `GetExitCodeProcess`,
  `QueryFullProcessImageNameW`), and the listener table is read once per check.
  `devd doctor` went from ~45s to ~3s, and the service list from ~5s to under
  100ms.
* The hosts block no longer proposes entries for `localhost` and its
  subdomains, which resolve without the file. `hosts.show`, `hosts.sync` and the
  overview therefore agree with `site.create`.

### Changed

* The graphical surface is described as one application window, not a website.
  The loopback API remains as the window's transport and for scripts; a native
  webview wrapper is the planned shell, with no second UI implementation.

---

## [0.5.1] - 2026-10-19 — a Windows installer

`devd` now installs like an application: download one executable, click through
the installer, and the app opens with a site already served.

### Added

**The installer (`devd-setup-<version>.exe`, `installer/devd.nsi`)**

* Per-user install under `%LOCALAPPDATA%\Programs\devd`; no administrator prompt.
* Adds `devd` to the user's `PATH`, creates Start Menu entries (the application,
  a terminal, the uninstaller) and an optional desktop shortcut, and registers
  itself in *Apps & features*.
* The last page runs `devd setup` in a console with visible progress, then opens
  the application without a console window.
* Uninstall stops the services, removes files, shortcuts, `PATH` entry and
  registry key, and keeps the data directory.

**`devd setup`**

* Idempotent first-run provisioning: install PHP, nginx and MariaDB when
  missing, start the stack, create the first site, trust the CA for the current
  user with `--yes`.
* If ports 80/443 are held by another program it moves the listeners to
  8080/8443 and persists the choice.
* The first site is `localhost`, so it needs no hosts entry and no elevation.

**`scripts/release-installer.ps1`** builds the installer with NSIS; the release
scripts are local by decision, with no CI pipeline.

### Fixed

* `www_root` is honoured when creating sites and by the `--delete-files` guard;
  it previously only affected the doctor check.
* `dns_mode` and `auto_start_services` were serialised into `settings.toml` and
  read by nothing; removed.
* A site whose host is `localhost` no longer warns that its name does not
  resolve.

### Changed

* The README leads with the installer; the standalone binary and the source
  build are the alternatives.

---

## [0.5.0] - 2026-10-19 — M4: a real site over HTTPS

The fifth milestone is the first one where `devd` does what the product is for:
it installs a runtime, installs a web server and a database, creates a site, and
serves it over TLS with a chain that verifies. Everything is downloaded from the
vendor, checked against a digest, and started and confirmed by the same
capability dispatcher that has been underneath every surface since M0.

### Added

**Runtimes (`devd-runtime`)**

* `runtime.available`, `runtime.install`, `runtime.remove`, `runtime.verify`,
  `runtime.list` and `runtime.set_primary`, plus `devd runtime …` on the CLI.
* PHP from `windows.php.net`'s `releases.json`, Node from `nodejs.org/dist` with
  its `SHASUMS256.txt`, Go from `go.dev/dl/?mode=json`. Downloads are in-process
  (`ureq` + `rustls`), digest-verified, extracted in-process (`zip`, `tar`,
  `flate2`), and moved into place only after the digest matches. Nothing is
  bundled and no external `curl` is used.
* PHP is configured after it is unpacked: the curated extension list is
  intersected with the DLLs the build actually ships, `opcache` is enabled as a
  `zend_extension`, the interpreter is run, anything that reports "Unable to
  load dynamic library" is commented out, and it is run again. The install
  reports what was enabled and what was switched back off.

**Services (`devd-services`)**

* Nine service capabilities — install, start, stop, restart, status, list, logs,
  remove and available — for nginx and MariaDB, plus `web.render`, `web.test`,
  `web.reload`, `stack.up`, `stack.down`, six `tls.*` and three `db.*`.
* nginx from nginx.org; MariaDB from `archive.mariadb.org` with the per-file
  SHA-256 from MariaDB's release API. MariaDB is initialised by its own
  `mysql_install_db.exe`, without registering a Windows service, with its data
  directory outside the install directory.
* `php-cgi` as a service built from the installed PHP runtime, and `stack up`
  creates it if it is missing.
* Detached supervision: a pid file plus a process-image check, and a start is
  confirmed by the port opening before the command reports success. A failure
  kills the process and returns the tail of its own log.
* `nginx.conf` rendered from the site table: a PHP upstream, a default server
  that returns an explanatory 404, per-site HTTP→HTTPS redirect and TLS server
  blocks, and per-host access logs. Config content uses forward slashes; `argv`
  uses native paths.

**Sites and TLS (`devd-sites`, `devd-services::tls`)**

* `site.list`, `site.create`, `site.remove`, `hosts.show` and `hosts.sync`.
* An internal ECDSA P-256 certificate authority created on first use, leaves
  valid 397 days and a full chain for nginx, all issued in-process with `rcgen`.
  The CA key never enters a response or a log.
* `tls.issue`, `tls.list`, `tls.ca.show`, `tls.trust`, `tls.untrust` and
  `tls.leaf.remove`; trusting the CA is `Tier::Admin` and uses `certutil` on
  Windows or `trust anchor` on Linux.
* A managed hosts block that snapshots the file, preserves every line it did not
  write byte for byte (including CRLF endings), and is idempotent.

**CLI**

* `devd runtime`, `devd service`, `devd site`, `devd hosts`, `devd web`,
  `devd stack`, `devd tls` and `devd db`, each mapping one-to-one onto
  capabilities. 98 capabilities in 20 groups, up from 64 in 12.

### Safety

* Hostnames are validated before they can reach a config file or a path;
  `site.remove` refuses to delete a directory outside the `www` root.
* Publishing a name and trusting a CA are separate, explicitly approved
  capabilities — never side effects of creating a site.
* The database password is passed to the one-time bootstrap on its command line
  and to clients through `MYSQL_PWD`, and is never written to a file `devd` owns.
  `root` is passwordless and bound to loopback by default, which the install
  warns about.
* A fresh MariaDB data directory has its world-writable `test` schema removed.
* nginx publishes no digest; its install says so rather than implying a
  verification that did not happen.

### Fixed

* `Path::with_extension` collapsed `certs/blog.test.crt` and
  `certs/blog.test.key` onto `certs/blog.test`, so nginx was told to load a
  certificate that `devd` had never written and refused to start.
* A detached service inherited `devd`'s stdout, so `devd stack up | Tee-Object`
  printed its result and then hung until it was killed. `devd` now clears the
  inheritance flag on its own standard handles before spawning a service.
* The debug binary overflowed the 1 MB Windows main-thread stack while parsing
  the grown command line, so even `devd --help` died before running. The stack
  reserve for this target is now 8 MB, matching Linux.
* `mysql_install_db.exe` was given `--basedir`, which it does not accept; it
  exited 7 and created nothing.
* `mysql --batch` printed a header row, which made `schema_name` appear as a
  database in `db list`.
* The MariaDB catalogue resolved `latest` to a preview series with no Windows
  package; it now reads MariaDB's release API, offers only Stable series that
  have a `winx64.zip`, and knows each file's digest before downloading it.
* The MariaDB provisioning report — data directory, whether it was initialised,
  the password warning — was collected at install time and then dropped.
* `php-cgi` was reported as "unknown version" by `service list` and `doctor`.

### Tests

See the line below. The suite covers the
generated vhost and its certificate paths, the hosts block's preservation and
idempotence, hostname validation before a write, the refusal to delete outside
the `www` root, the chain verifying against the CA, and re-issuance replacing the
leaf without replacing the CA. The live downloads are `#[ignore]` tests.

### Known limitations (at 0.5.0)

* Apache, Redis and Mailpit are not implemented; there is no DNS server, SSH or
  deploy.
* Services are Windows-only: the vendor archives used here are Windows builds.
* nginx has no published digest.
* One `php-cgi` listener serves every site, so per-site PHP versions are recorded
  but not routed.
* The application and the dashboard have no Runtimes, Services or Sites panels.

---

## [0.4.0] - 2026-10-19 — M3: the terminal dashboard

The fourth milestone adds the surface that works where the others cannot: a
console with no browser to open, and an SSH session with nothing forwarded. It is
the third projection of the same registry, and it changes nothing about how the
product decides what may happen.

### Added

**The terminal dashboard (`devd-tui`, `devd tui`)**

* Nine views — Dashboard, Projects, Changes, History, Branches, Capabilities,
  Activity, Doctor, Help — reachable by digit (`1`–`9`) or by `Tab`.
* Git actions from the keyboard without a second implementation: `s` stage, `u`
  unstage, `a` stage all, `c` commit, `Enter` toggle a file in or out of the
  index, `n` new branch, `t` track a repository by path, `/` search the registry,
  `x` run any capability with arguments typed as JSON.
* A frame is a pure function of the state, which is what lets the tests render
  one and assert on the buffer.
* **An approval prompt that is the operation.** `d` (discard) is
  `Tier::Destructive`, so the dashboard asks the registry for the plan, shows it
  verbatim, and keeps the `InvokeRequest` it built. `y` replays *that* request,
  never a rebuild from the screen. `n` dispatches the same request as refused, so
  the refusal is recorded rather than discarded — the same two-step principle as
  the CLI and the application window.
* **A branch switch that would lose work becomes a confirm**, with the list of
  files at risk taken from `git.switch.check`.
* `devd tui` refuses to start when stdin or stdout is not a terminal, and says
  what to use instead. Entering raw mode on a pipe killed the process with
  `0xc0000005` and no output on Windows; that is now a sentence and exit code 2.

### Safety

* **Control characters are neutralised before they reach the terminal.** A branch
  name or path containing an escape sequence is a command being sent to the
  reader's console, not text. One function replaces them, and a test renders a
  branch named with `ESC [ 2 J` and asserts nothing reached the buffer.
* **A modal owns every key**, so a keystroke cannot both answer a prompt and act
  on the view behind it. `Esc` on an approval means *no*, not *cancel the
  question*.
* **The dashboard has no path to a shell.** It calls capabilities, and
  capabilities are typed structs turned into `argv` arrays.
* **A plan is not counted as an action.** The Activity view labels a dry run
  `plan`, an application `applied` and a refusal `refused`.

### Changed

* The footer's first line is the message line and starts empty, rather than
  defaulting to a hint; each view's key hints now carry the common tail.

### Fixed

* **The release binary linked by Zig crashed at startup.** On this
  `x86_64-pc-windows-gnu` host the final link goes through a `cc` that forwards
  to Zig, and the result faulted at `0xc0000005` inside its own code before
  `main` ran — release only, with no output, while debug and the test suite were
  fine. `.cargo/config.toml` now pins `linker = "rust-lld"` for that target
  alone, with the reason recorded beside it. C sources are still compiled by
  `cc`, and MSVC builds are unaffected.

### Tests

**226 tests pass** (up from 169), zero warnings, in debug and in release. The 57
new ones are in `devd-tui`: 52 unit tests driving the controller with real
keystrokes against a backend that counts what ran, and 5 integration tests
against the real registry and a real repository — including that staging from
the dashboard changes the real index, and that a declined discard leaves the file
on disk with the refusal in the audit table.

---

## [0.3.0] - 2026-10-18 — M2: the agent surface

The third milestone adds the Model Context Protocol server. It is the third
projection of the same registry, after the CLI and the application window, and it
makes the product's premise checkable: an agent driving `devd` reaches the same
functions as a person, with the same permission checks.

### Added

**The MCP server (`devd-mcp`)**

* `devd mcp` serves the Model Context Protocol on stdin/stdout;
  `devd mcp --client <name>` names the caller in the audit trail;
  `devd mcp --list` prints the tool list.
* **Five discovery tools instead of one tool per capability**:
  * `devd_groups` — the domains and their sizes;
  * `devd_capabilities` — search by text, by domain, or both, with a true total;
  * `devd_describe` — one capability, with the JSON Schema of its arguments;
  * `devd_invoke` — run one by id, optionally as a dry run;
  * `devd_doctor` — the environment check.
  A capability added later is discoverable and invocable without changing this
  crate, and a test asserts that two servers over registries of different sizes
  publish an identical tool list.
* The protocol handles `initialize`, `ping`, `tools/list`, `tools/call`, and
  answers `resources/list`, `resources/templates/list` and `prompts/list` with
  empty lists, because clients probe them. A notification is never answered. An
  unknown method is `-32601`; an unknown tool names `tools/list`; a missing
  required argument is `-32602`; malformed input is `-32700`.

### Safety

* **The actor is forced.** `McpServer::new` discards the actor in the context it
  is handed and installs an agent actor, so a caller cannot present a human
  context and inherit a human's authority. A test asserts it.
* **A refused operation hands the agent its plan.** When an approval-gated
  operation is refused, the server asks the registry for a plan — itself a dry
  run, recorded as a plan rather than an application — and returns it with the
  refusal, the error code, and an explicit instruction not to report the
  operation as having happened. That instruction exists because the failure mode
  being defended against is an agent that confidently describes work it was not
  allowed to do.
* **A capability that cannot describe itself gets no invented plan**, and the
  message says so. Both branches are covered by tests.
* `human_only` is refused for an agent before approval is considered, so an
  approval could never unlock it.
* Tool results are data, never instructions: the only text that reaches a model
  is written by the registry or by this crate, never echoed from repository
  content.
* The refusal tests assert against a counter incremented by the handler, so
  "nothing ran" is a measurement rather than an inference.

### Changed

* The Git doctor check moves from `devd-cli` to `devd-git`
  (`devd_git::doctor_check`), so the CLI, the application and the MCP server
  share one implementation instead of three that could disagree.
* `App::registry` is now an `Arc<Registry>`, because two surfaces move it into a
  long-lived task while the CLI still holds the application.

### Tests

169 tests, all passing (up from 148):

* `devd-mcp`: 21 — the protocol, the five-tool surface and the assertion that it
  does not grow with the registry, the forced actor, discovery, and the refusal
  paths (destructive, `human_only`, cannot-plan, dry run, unknown capability,
  unknown tool, missing argument) plus the doctor.

Verified over a real pipe against the built binary as well: the transcript and
the resulting audit rows are in [`docs/mcp.md`](docs/mcp.md#observed).

### Known limitations (at 0.3.0)

* The TUI is still not built.
* Services, sites, runtimes, DNS and TLS remain designed only.
* There is no conversational agent inside the application; an agent connects from
  outside over MCP, which is the same registry either way.

---

## [0.2.0] - 2026-10-18 — M1: the application

The second milestone turns the core into something a person can actually use: a
local application served by the `devd` binary on loopback, opened as a
chrome-less window. It adds a surface, not a second implementation — the server
is generic over the registry, and the interface is a pure client of it.

### Added

**The local application (`devd-server`)**

* `devd ui` starts an HTTP API on `127.0.0.1` and serves the interface from
  inside the binary, then opens it in an application window (Chromium in
  `--app` mode where available, otherwise the default browser). `devd serve`
  runs the API headless.
* If a `devd` instance is already running, `devd ui` probes for it, through the
  `X-Devd-App` response header, and **reopens it** rather than starting a second
  server. A port held by an unrelated program is left alone and the next port is
  tried. The chosen port is remembered in the settings table.
* Twelve pages, all composed in the browser from `/api/invoke` calls: Projects,
  a project workspace with Changes, History, Branches, Tags, Remotes, Stashes,
  Worktrees and Recover, plus Capabilities, Activity and System.
* A `Ctrl+K` command palette backed by the registry's own ranked search
  (`/api/capabilities?q=`), which runs read-tier capabilities inline and opens
  the inspector for everything else.
* Per-file diff rendering, hunk-level staging and reversing, commit with amend, a
  credential-looking-staged-file warning, branch switching that consults
  `git.switch.check` first, conflict resolution, the reflog, reset modes, squash,
  bisect, and continue/abort for an interrupted operation.
* An approval modal that renders the server's plan with a countdown, a red
  confirm button for destructive and remote tiers, and the arguments shown as
  the server recorded them.
* A **Quit devd** action calling `POST /api/shutdown`, so closing the window and
  stopping the process are not the same confusing thing.

**Two-step approvals over HTTP**

* `POST /api/invoke` returns `{ approval_required, plan, token, tier,
  expires_in_ms, args_hash }` and **changes nothing**.
* `POST /api/approve { token }` replays the **server's own stored arguments**
  with `Approval::Granted`. The client cannot assert an approval.
* `POST /api/deny { token }` dispatches with `Approval::Denied`, so a refusal is
  audited rather than invisible.
* A token is single-use, lives 120 seconds, is bounded to a fixed number of
  pending decisions, and is bound to the capability, the arguments and the actor.

**Loopback hardening**

* `Host` allow-list (`127.0.0.1:P`, `localhost:P`, `[::1]:P`) that defuses DNS
  rebinding, refused with `403`.
* `OPTIONS` is refused with `405` before any other check, so no cross-origin
  caller can complete a preflight.
* A foreign `Origin` is refused with `403`.
* A 256-bit session token, generated at startup, injected into the served HTML
  and required on every `/api` request with a constant-time comparison. It is
  never written to disk and no endpoint returns it.
* The actor for every request is forced to `human:http` by the server; a caller
  cannot choose its own actor.

**The interface itself**

* Three files — `index.html`, `app.js`, `style.css` — embedded with
  `include_str!`. No build step, no framework, no `node_modules`.
* A Content-Security-Policy of `default-src 'none'` with same-origin script,
  style and connect, and a test asserting the shell contains no inline script,
  no `style=` attribute and no `on*=` handler.
* Dark, calm, keyboard-first; monospace for machine data; reduced
  `prefers-reduced-motion` respected.

**Shared environment checks**

* The doctor checks move into `devd-core::doctor`, so the CLI and the server
  report the same thing from the same code. `devd`'s contribution is the Git
  check; the server appends nothing the CLI lacks.

### Changed

* `Surface::can_prompt()` distinguishes a surface that can *answer* a prompt from
  one that is a *person*. The application can answer prompts without being
  treated as an interactive human, and an agent cannot.
* `Store::open_in_memory()` now migrates, so an in-memory store is usable
  immediately instead of failing every table read.
* A Git command that fails now reports what Git said on **stdout** when stderr is
  empty. Git writes "nothing to commit" and similar to stdout, so the previous
  behaviour turned a real, explainable cause into an empty message.

### Safety

* A destructive operation over HTTP returns a plan and changes nothing until it
  is approved; the arguments that run are the ones the server stored, never the
  ones the client sent a second time.
* A declined operation is recorded with the actor, the tier, the redacted
  arguments and the reason, and its token cannot be replayed.
* The application never receives a capability it could not reach from the CLI,
  and the CLI never gains a rule the application lacks.

### Tests

148 tests, all passing (up from 98):

* `devd-server` unit: 17, covering the approval store (single use, expiry,
  capacity, binding to capability and arguments) and the request guards.
* `devd-server` HTTP: 22, driving the real router: the four loopback defences,
  the token requirement, capability search and description, an approval-gated
  invocation that changes nothing until approved, a decline that is audited and
  applies nothing, token replay, and the error envelope.
* `devd-cli`: 4, including port selection.
* `devd-core` and `devd-git` as before, plus the new doctor and failure-reporting
  tests.

Verified against the running release binary as well, not only in tests: the
security matrix and the Git approval flow in
[`docs/app.md`](docs/app.md#what-was-actually-observed).

### Known limitations (at 0.2.0)

* The TUI and the MCP server are still not built.
* Services, sites, runtimes, DNS and TLS remain designed only, and the
  corresponding panels say so instead of offering controls that do nothing.
* The window is a browser engine in application mode, not a native shell. Tauri
  remains the intended eventual wrapper; the UI code does not change when that
  happens, because it speaks only to the local API.
* A capability's arguments are edited as JSON in the inspector rather than by a
  generated form.

---

## [0.1.0] - 2026-10-18 — M0: foundation and Git

The first milestone. It delivers the part of the product that everything else
will be built on — the capability registry and its permission model — together
with one skill area, Git, implemented end to end so the model is proven against
a real, stateful, destructive domain rather than a toy one.

### Added

**The core (`devd-core`)**

* A capability registry. A capability carries an id, a group, a summary, a
  description, a permission tier, a side-effect class, a JSON Schema for its
  arguments, and flags for dry-run support, agent safety, human-only
  reservation, idempotence and reversal.
* Five permission tiers — `Read`, `Write`, `Exec`, `Destructive`, `Remote` —
  with approval required for the last two.
* A single dispatcher (`Registry::dispatch`) that every surface calls. It
  enforces, in order: an explicit decline, the human-only reservation, the
  agent-safety reservation, the tier's approval requirement, and dry-run
  support — before the handler runs.
* An audit trail. Every attempt is recorded with the actor, the surface, the
  tier, the redacted arguments, the outcome and the message, including attempts
  that were refused.
* Argument redaction, so a secret that reaches an interface never reaches the
  audit file.
* A SQLite-backed state store with a versioned schema, integrity checking and an
  in-memory mode for tests.
* Path layout, layered settings, an event bus, port allocation, health
  probing and a process runner that kills whole process trees.

**Git (`devd-git`)**

* A typed Git service over the user's own `git` executable. Every call is an
  `argv` array; nothing is ever assembled into a shell string, and revisions,
  refs, relative paths and remote URLs are validated before use.
* 64 capabilities in 12 discoverable groups:
  * `git.repo` — init, open, track, untrack, list, describe.
  * `git.changes` — status, diff (working tree, index, path, revisions,
    diffstat), stage, unstage, hunk-level staging and reversing from a patch,
    discard, ignore.
  * `git.commits` — create, amend, allow-empty, squash.
  * `git.history` — log, show, blame, search, reflog.
  * `git.branches` — list, create, rename, delete, switch, switch safety check,
    upstream, prune merged.
  * `git.tags` — list, create (annotated or lightweight), delete, local or
    remote.
  * `git.remotes` — list, add, remove, rename, fetch, pull, push, force push,
    sync preview, outgoing, incoming.
  * `git.merge` — merge, rebase (including full-history and `--autosquash`),
    cherry-pick, revert.
  * `git.stashes` — list, push (including untracked), show, apply, pop, drop.
  * `git.worktrees` — list, add, remove, prune.
  * `git.config` — get, set, with credential-bearing keys refused.
  * `git.recovery` — conflicts list and show, resolve a side, continue, abort,
    reset (soft/mixed/hard), bisect start/mark/reset.
* Safety properties enforced by construction:
  * **Force push is a separate capability** (`git.push.force`), not a flag, so
    no code path can reach a forced update without its own approval gate.
  * **Both `git.push` and `git.push.force` are `Tier::Remote`**, so publishing to
    a machine `devd` does not own needs a human.
  * **Remote URLs that embed a password are rejected**, because such a URL is
    written to `.git/config` in plain text.
  * **Configuration writes to credential-bearing keys are refused**, including
    `*extraheader`, `user.password`, `*.token` and similar.
  * **`LC_ALL=C`** is forced and the pager is disabled, so output parsing is
    stable and nothing blocks waiting for a reader.
  * **Interactive rebase is driven by a no-op sequence editor**, so an
    `--autosquash` rebase cannot hang waiting for a human who is not there.
  * `GIT_TERMINAL_PROMPT=0`, so Git can never stop to ask for a credential.

**The CLI (`devd-cli`)**

* `devd capabilities`, `devd describe`, `devd call`, `devd audit`,
  `devd doctor`.
* A Git command set that keeps Git's vocabulary: `status`, `diff`, `stage`,
  `unstage`, `discard`, `commit`, `log`, `branch`, `switch`, `fetch`, `pull`,
  `push`, `clone`, `merge`, `stash`, `worktree`, `remote`, `tag`, `conflicts`,
  `continue`, `abort`, `reflog`, `undo`, `revert`, `reset`, `cherry-pick`,
  `rebase`, `squash`, `bisect`, `repo`.
* `--plan` to see a plan and change nothing; `--yes` as the explicit,
  deliberate approval for a non-interactive run; `--json` for machine output.
* Stable exit codes: 2 invalid, 3 approval required or denied, 4 not found,
  5 conflict, 1 otherwise.

### Safety

* An agent cannot approve its own operation. The dispatcher refuses before the
  tier check, and no surface can supply an approval on an agent's behalf.
* Human-only capabilities are refused for an agent even when an approval is
  presented.
* A declined destructive operation is refused and audited, and the working tree
  is untouched. This is asserted by test, not by review.
* An aborted merge or rebase restores the previous state and preserves
  uncommitted work in files the operation did not touch.
* A plan that declares itself destructive now also declares that it requires
  approval; the two can no longer contradict each other.

### Tests

98 tests, all passing:

* `devd-core`: 43, including the registry tests that prove an agent cannot
  perform a human-only operation, that a declined operation never executes, and
  that secrets are redacted from the audit trail.
* `devd-git` unit: 9, covering revision, ref, path and remote-URL validation,
  option-injection refusal, redaction, and the credential-key rule.
* `devd-git` lifecycle: 30, against real repositories in temporary directories —
  the full Git lifecycle, hunk staging, amend, branches, rebase, autosquash,
  squash, conflict detection, resolution, abort and recovery, reset modes, the
  stash lifecycle, the worktree lifecycle, tags, cherry-pick, revert, clone,
  fetch, pull, push, force-push-with-lease recovery, diverged pulls,
  credential rejection, an unreachable remote, ignore rules, and paths and
  repository paths containing spaces.
* `devd-git` policy: 16, driving the registry — declined operations do not run,
  approval is required then honoured, an agent cannot self-approve, a
  human-only capability is refused for an agent, force push is unreachable from
  the ordinary push schema, a dry run changes nothing, destructive plans name
  what they destroy, every attempt is audited, the actor and surface are
  recorded, secrets never reach the audit trail, and a resolved conflict can be
  continued through the registry.

### Known limitations (at 0.1.0)

* Only the Git capability group is registered. Service, site, DNS, TLS and
  runtime capabilities are designed but not implemented.
* There is no TUI, no desktop GUI, no HTTP API and no MCP server yet. The
  capability declarations and JSON Schemas they need already exist.
* No macOS support. Windows and Linux are the target platforms.
* Windows process-tree termination uses `taskkill /T /F` rather than a Job
  Object.
* Git hooks and credential helpers are the user's own; `devd` neither installs
  nor executes hooks from a repository it merely opens.

---

[0.3.0]: https://github.com/ianclemence/devd/releases/tag/v0.3.0
[0.2.0]: https://github.com/ianclemence/devd/releases/tag/v0.2.0
[0.1.0]: https://github.com/ianclemence/devd/releases/tag/v0.1.0
