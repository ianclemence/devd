# Screen inventory

Every screen in the DEVD web application and the native window, what it is for,
what it shows, and what was checked and corrected.

The application is a single page: the "route" is a view name held in the client,
the "page" is written into `#main`, and the surfaces are the rail (the areas),
the tab strip (the screens inside an area), the command palette (every
destination), the breadcrumb and the status bar.

**How this was checked.** Every screen below was opened in the running
application and read from the DOM — its title, its subtitle, its panels, its
tables, its empty states and its controls — with the DevTools Protocol. The
figures in the "Measured" column are the numbers the running application
reported, not readings of the source.

**Naming rule, applied throughout.** The rail, the tab strip, the palette and
the breadcrumb all use the same short name for a destination. The page heading
may be longer, because a heading has room and a rail does not. Before this pass
the same screen was called "Sessions" in the rail and "Agents" on the page; that
is fixed.

---

## The areas

| Rail entry | Area | Screens inside it |
|---|---|---|
| Home | — | Home |
| Inbox | — | Inbox |
| Work | Work | Assistant · Chat, Team runs; Projects · Projects, New project, Stacks and tasks; Agents · Agents, Shared notes; Processes |
| Run | Server | Overview, Services, Sites, Tools, Runtimes, Preview, Certificates, Hosts |
| Manage | Manage | Models, Activity, Diagnostics, Capabilities |

---

## 1. Home

* **Entry:** the first thing shown; the rail's first entry.
* **Purpose:** what needs you, what is running, and where to go next.
* **Shows:** the greeting and the date; the ask box; four quick actions; what
  needs you (approvals, agent messages); your projects; what is running; recent
  activity; the requirements that are outstanding.
* **Actions:** ask the assistant; open a project; approve or decline; open the
  thread; re-run a health check; jump to a screen.
* **States measured:** 5 panels, 9 controls, 1 empty state ("No projects yet").
* **Checked and corrected:**
  * The quick action said "From 24 stacks" — a number written into the client.
    It now comes from the server's stack list, and reads "From a stack" until
    the list has loaded.
  * The ask box is hidden entirely when no provider is configured. It now says
    why.
  * The project count came from a different capability than the Projects
    screen used, so the two could disagree. They now come from the same one.

## 2. Inbox

* **Purpose:** how agents reach you. Approvals and questions.
* **Shows:** pending approvals; agent messages; decided history.
* **Actions:** approve and run; decline; reply; mark as read.
* **States:** loading; "Nothing to approve."; "No messages."; "Nothing yet."
* **Checked:** the empty states are three different sentences for the same
  idea. They are now distinct in meaning (nothing needs a decision / nothing was
  said / nothing ever happened) and each says which.

## 3. Assistant → Chat

* **Purpose:** talk to the built-in agent.
* **Shows:** the thread list, the conversation, the tool calls with their
  output, the token count, and the provider and model in use.
* **Actions:** new conversation; send and stop; choose a provider and a model;
  expand a step; copy a tool's output.
* **States:** loading; "What should we build?"; "No usable AI provider yet".
* **Checked and corrected:**
  * The thread list had no way to delete a conversation even though
    `chat.delete` exists. Added.
  * The page has no `h1` while every other screen does. It now has one.

## 4. Assistant → Team runs

* **Purpose:** the agent team: plan, approve, watch, read the evidence.
* **Shows:** runs, tasks, evidence, events, approvals.
* **Actions:** new run; start; approve the plan and start; pause; resume;
  cancel; retry; accept as is; send a message; open the conversation.
* **States:** "No runs yet."; "No tasks yet."; "Nothing yet."; loading.
* **Checked:** the copy says a task "only counts as done when devd can see the
  proof", and the "Accept as is" button says it accepts it "without the proof
  devd asked for". Two statements of one rule; they now agree.

## 5. Projects

* **Purpose:** the folders on your machine that devd knows about.
* **Shows:** one card per project: its name, its real path, the detected stack,
  the branch, the number of changes, how far ahead or behind it is, and whether
  the folder is still there.
* **Actions:** open a project; pin it to the top; clone a repository; **add an
  existing project**; search; sort by name, by recently touched or by what
  needs attention.
* **States:** loading; "No projects yet" with the two ways forward; "No project
  matches …" when a search finds nothing; a badge saying the folder is gone;
  a badge saying a repository is not imported.
* **Measured:** the search field, three sort buttons and a "N of M" count sit
  in one row above the grid.
* **Checked and corrected:**
  * **The page asked for a typed path.** It now opens the folder picker. This
    is the change the whole exercise turned on: *no typed path is needed to
    add, open or import a project.*
  * The list came from `git.repo.list` while the Develop tab used
    `project.list`, so the same project could appear in one and not the other.
    Both now read `project.list`, which returns the union of imported projects
    and indexed repositories.
  * "Forget in devd" became **"Remove from devd"**, and a separate
    **"Delete from disk"** was added with a typed confirmation. The two are
    never confused.
  * A project that had been deleted from disk used to look like any other; it
    now says "folder is gone".

## 6. Projects → New project

* **Purpose:** create a project from a stack.
* **Shows:** the stack catalogue, the versions, the options, the plan, the log
  and the report.
* **Actions:** pick a stack; set options; show the advanced ones; create;
  cancel; install a prerequisite; open it in Develop.
* **States:** "Checking…"; a plan with its problems; a report saying what was
  created and what failed.
* **Checked:** the plan lists its problems, one per line, each naming the
  field and the reason. That is the right shape and was left alone.

## 7. Projects → Stacks and tasks

* **Purpose:** detect any project's stack and run its tasks.
* **Shows:** the detected stack and its confidence, the runtimes it asks for,
  the environment variables it needs, the tasks it has, and the running
  processes.
* **Actions:** detect; import; run a task; install a tool; read a task's log;
  stop a task; **choose a folder** (the picker).
* **States:** "No projects yet…"; "Select a project."; "Inspecting…".
* **Checked and corrected:** the import field asked for a path. It now has a
  **"Choose folder…"** button beside it, and the path field remains for the
  advanced case.

## 8. Agents

* **Purpose:** the coding agents, as real terminals.
* **Shows:** the session list, the harnesses that are installed, a live PTY, and
  the delegated tasks.
* **Actions:** new session; stop; dismiss; rename; delegate a task; wait;
  cancel; read the result; **choose a working directory**.
* **States:** "Starting the harness…"; "No sessions yet."; "No harnesses were
  found".
* **Checked and corrected:** the working-directory list ended with **"Choose a
  folder…"**, which opens the picker, in both the side panel and the new-session
  dialog. It used to be a fixed list of known projects, which made an unusual
  project unreachable.
* The rail called this "Sessions" and the page called it "Agents". One name
  now.

## 9. Processes

* **Purpose:** the long-running project processes described by `devd.yml`.
* **Shows:** the plan, each process with its pid, health and log.
* **Actions:** choose a project; approve the commands; start, stop, restart;
  start all, stop all; read the log; filter the log.
* **States:** loading; "No manifest to run", with an example manifest.
* **Checked:** the example manifest was rendered in the same block as real
  output, so it looked like output. It is now labelled as an example.

## 10. Server → Overview

* **Purpose:** what is running on this machine.
* **Shows:** the services with their state, the sites, the hosts entries, the
  certificates, the authority's trust state and the runtimes.
* **Actions:** start all, stop all; start, stop or restart a service; open a
  site; trust the authority; create a site; install a service.
* **States:** loading; "Nothing installed yet"; a notice per problem.
* **Checked and corrected:** the site count was captioned "served" whether or
  not the server was running; the same panel shows the real state a row below.
  The caption is gone. The certificate count was captioned "issued by devd"
  next to a zero.

## 11. Server → Services

* **Purpose:** the services devd manages, and the ones already on the machine.
* **Shows:** the installed services with their type, version, ports, state and
  pid; and an inventory of what was found on this machine.
* **Actions:** install; start; stop; restart; read the log; remove; and, for
  anything found: use it, only watch it, use your own instance, install, start,
  read the log, remove, reset the decision.
* **States:** loading; "No services"; a notice per problem.
* **Checked and corrected:** each service now carries **its owner's mark**, and
  so does everything found on the machine. The two lists are visually
  distinguishable, which is the point of the inventory.

## 12. Server → Sites

* **Purpose:** the local hostnames.
* **Shows:** each site's name, URL, PHP version, TLS, document root and whether
  the directory is still there.
* **Actions:** new site; create; open; remove.
* **States:** loading; "No sites yet"; "directory missing" on the row.
* **Checked:** a site whose directory was deleted says so in the row, rather
  than looking like a working site.

## 13. Server → Tools

* **Purpose:** the languages, package managers and databases devd can install.
* **Shows:** the tools by category, their version, their state, and the health
  of the services among them.
* **Actions:** plan; install; remove; start; stop; restart; read the log;
  re-check.
* **States:** loading; an empty panel when the catalogue is empty.
* **Checked:** the empty panel said nothing. It now explains what the screen
  is for and what to do.

## 14. Server → Runtimes

* **Purpose:** PHP, Node and Go.
* **Shows:** the installed versions with their paths, and which one is primary.
* **Actions:** install the latest; use for new sites; remove.
* **States:** loading; "Not installed."
* **Checked and corrected:** each runtime now carries its owner's mark.

## 15. Server → Preview

* **Purpose:** see a site in the browser.
* **Shows:** a sandboxed frame of the chosen site.
* **Actions:** choose a site; reload; open it in the real browser; trust the
  authority.
* **States:** loading; "No sites to preview".
* **Checked:** the frame's height was a fixed `100vh - 270px`, which was wrong
  on every screen but a large one. It now follows the space the page has.

## 16. Server → Certificates

* **Purpose:** local HTTPS.
* **Shows:** the authority's certificate and where it is, whether it is trusted,
  and every issued certificate.
* **Actions:** issue for a hostname; trust for this user; trust for all users;
  remove the trust.
* **States:** loading; "No certificates yet."; "No certificate authority yet."

## 17. Server → Hosts

* **Purpose:** the names devd publishes.
* **Shows:** the hosts file, the names in it, and what is pending.
* **Actions:** sync the hosts file.
* **States:** loading; "No names published."

## 18. Models

* **Purpose:** the providers and the models.
* **Shows:** each provider with **its owner's mark**, its kind, its base URL,
  whether it has a key, its default model, how many models it knows, and how it
  discovers them; the official catalogue; the roles and the models assigned to
  them.
* **Actions:** test; fetch the models; verify the model; edit; make default;
  remove; add a provider; find the local servers; refresh the official list;
  assign a model to a role.
* **States:** loading; "No model provider yet".
* **Checked and corrected:**
  * **The 15 providers now carry the owners' own artwork** — Anthropic, OpenAI,
    DeepSeek, Kimi, GLM, Gemini, xAI, Mistral, Groq, Together, Fireworks,
    OpenRouter, Ollama and LM Studio. The mark is the owner's, uncropped and
    un-recoloured, served from the binary so the interface never depends on a
    third party being reachable.
  * A provider whose id is not a preset id could not be edited. Now it can.

## 19. Activity

* **Purpose:** every capability attempt, successful or not.
* **Shows:** when, who, which capability, its tier, the result, the arguments
  (redacted) and the message.
* **Actions:** refresh; page through.
* **States:** loading; "The activity log could not be read" with the reason;
  "Nothing has happened yet".
* **Checked and corrected:** the table was unbounded — 500 rows, 13,000
  characters. It is now paged, 50 at a time, and the pager says exactly what
  is showing. A failed request used to look like an empty history; it now says
  it could not be read.

## 20. Diagnostics

* **Purpose:** what devd checked, what it means, and how to fix it.
* **Shows:** the component checks with their state and their fix; the
  environment mode; the paths; the settings; the registry; the raw environment
  checks.
* **Actions:** re-check; quit devd; switch between local and production mode.
* **States:** loading; a table of checks; the environment panel.
* **Checked and corrected:** a panel claimed that "Services, sites, runtimes,
  DNS and certificates have no capabilities registered yet" — on the same
  screen that renders all five. It was removed. The page no longer shows
  literal backticks around a command name.

## 21. Capabilities

* **Purpose:** the whole registry, for scripts and agents.
* **Shows:** every capability with its tier; the arguments, the plan and the
  schema of the one that is selected.
* **Actions:** filter; page; open one; dry run; run it.
* **States:** loading; "Pick a capability"; "No capability matches that filter."
* **Checked and corrected:** the list was 225 rows in one screen. It is now
  paged, 50 at a time, with a real count.

## 22. The folder picker

* **Entry:** **Add existing project** on the Projects screen; **Choose folder…**
  on the Develop tab; **Choose a folder…** in the working-directory lists; and
  **Choose folder…** beside the clone destination.
* **Purpose:** choose a folder by looking at your device.
* **Shows:** the places work is kept (Home, Desktop, Documents, Downloads) and
  the filesystem roots (one per drive on Windows); the path as a trail you can
  step back through; the folders and files in the current directory, with
  symbolic links marked; and what devd found in the highlighted folder — the
  detected stack, the package manager, the runtimes it asks for, whether it is
  a Git repository, and anything missing.
* **Actions:** go into a folder; go up; jump to a place; use the operating
  system's own dialog where the shell can show one; use this folder; cancel.
* **States:** loading; "Choose one of the places above to begin."; "This folder
  is empty."; a warning when part of a directory could not be read; a note when
  the listing was cut short.
* **Measured:** the native dialog is offered only where the shell can show one
  — in the desktop window, and not in a browser, where the button is absent
  rather than broken.
* **Checked and corrected:** the picker was written against an element that did
  not exist, which took the list down with it. A contract test now checks every
  id the script looks up.

## 23. The overlays

| Overlay | What it is | Checked |
|---|---|---|
| Command palette | Search anything, or go anywhere | The keyboard shortcut, the arrow keys and the focus trap work. Its input had no focus indicator; it has one now. |
| Approval | The plan for an operation that needs a person | Shows the plan, the steps, the arguments, the tier, and a live expiry. The danger notice is only shown for a destructive plan. |
| Detail drawer | More about one thing | It is a `role="dialog"` with no focus management, and it is not in the focus trap. **Known gap.** |
| Toasts | A short confirmation or a failure | An error is announced politely. **Known gap.** |
| Folder picker | See above | — |

---

## Cross-cutting findings and what was done

| Finding | Action |
|---|---|
| A malformed CSS rule left a brace open, so the browser ignored the bottom half of the stylesheet, **including every responsive breakpoint**. The application was unbreakable below 960 px and nobody had noticed, because the bug was in the file and not in the browser | Removed. A contract test now checks that the stylesheet is balanced. **This was the single most damaging defect found.** |
| `hidden` did not hide anything that had a `display` rule | `[hidden] { display: none !important; }` |
| The same screen had two names in two places | One name per destination, in the rail, the tabs, the palette and the breadcrumb |
| "Nothing" was spelled eight different ways | Each empty state now says which kind of nothing it is |
| The approval button was called four different things | "Approve and run" everywhere |
| Two collection screens were unbounded | Both are paged, with a real count |
| A missing element id took the whole screen down | A contract test checks every id the script looks up |
| An empty action handler swallowed the click and did nothing | A contract test checks that no handler is empty |
| 64 inline `style` attributes with literal pixel values | The new code uses the token system; the remainder is listed as remaining work |
| 387 `px` literals in the stylesheet after the token block | Listed as remaining work — the tokens exist and are used, but the values have not all been folded in |
