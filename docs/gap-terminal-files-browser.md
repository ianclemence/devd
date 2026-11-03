# Terminal, file viewer and browser: what devd has, and what to build

An audit of three features devd is assumed to lack — an integrated terminal, a
project file viewer, and a browser tool — measured against DeepSeek Harness (the
app this was written in) and against what devd's own documentation claims.

Findings first, then the plan. Everything below was checked against the code in
this repository and against the DSH 0.2.1-rc.5 build installed on this machine,
not recalled.

## Summary

| Feature | devd today | Verdict |
|---|---|---|
| **Integrated terminal** | `project.terminal` opens the *operating system's* terminal window. No embedded terminal, no PTY, no xterm in the window. | **Missing.** The README claims the opposite. |
| **Project file viewer** | Files are read by the *agent* (`fs.read`) and shown in two places (a search result drawer, a diff). There is no file tree, no browsing of the project in the window. | **Missing.** |
| **Browser tool** | `docs.fetch` reads a public web page as text. `site` **Preview** is an `<iframe>` of a site devd already serves. No way to open a URL in a real browser and read the result. | **Missing** for an agent; **partial** for a person. |

Two documentation errors came out of this and are worth fixing whatever else
happens: the README states the window vendors xterm.js, and
`crates/devd-server/ui/vendor/` does not exist.

## How DeepSeek Harness does it

DSH is built as cordis plugins, each a package with a `dsh` field naming what it
injects. The split that matters here is **seam / backend / tool / UI**: a seam
declares a capability, backends implement it, tools expose it to the model, and UI
packages render it. That split is why a terminal can exist for the agent and for
the person from one implementation.

### Terminal

Four packages, each with one job:

| Package | What it is |
|---|---|
| `dsh-terminal` | The seam. "Persistent PTY session seam — owner-scoped ids, backend registry, interactive sends, reads, signals, and awaited cleanup". A session survives between calls and belongs to an owner. |
| `dsh-terminal-bash` | The backend. "Persistent shell PTY backend", using `@xterm/headless` to keep a real terminal state on the server. |
| `dsh-tool-terminal` | "Six model-facing persistent PTY tools": create, send, read, signal, list, close. The agent types into a shell and reads the screen back. |
| `dsh-client-ui-sidebar-terminal` | "Interactive shell tabs for the right Sidebar", using `@xterm/xterm` and `@xterm/addon-fit` in the window. |

The detail worth copying: **the terminal state lives on the server** (headless
xterm), and the browser is a view of it. An agent reads the same buffer the person
sees. `node-pty` and conpty.dll are in `app.asar.unpacked`, so the PTY is native.

### Files

| Package | What it is |
|---|---|
| `dsh-api-workspace-files` | "Workspace file service and Client resource provider: bounded reads, directory listing, and live metadata". |
| `dsh-client-ui-sidebar-files` | "Workspace file tree tab type for the right Sidebar: lazy directory listing, opening files into the Sidebar." |

Bounded reads, lazy listing, live metadata — the same ideas as devd's `fs.read`
(`offset`/`limit`), but exposed both to the model and to a tree in the window.

### Browser

**There is none.** DSH's web packages are `web-fetch-http` ("anonymous public
HTTP(S) fetch provider") and `web-search-*`, surfaced as `web_search` and
`web_fetch`. No Playwright, Puppeteer or CDP anywhere in the image. So the bar to
clear here is not Claude Code's browser either — it is "fetch a page and make its
content usable".

## What devd has

Measured, not remembered:

- **`project.terminal`** (`crates/devd-*/src/caps.rs`): `Exec`, `human_only`,
  calls `crate::terminal::open`, whose description is *"Opens the operating
  system's own terminal window in the project folder"*. It hands a person an
  external shell. Nothing streams back.
- **No PTY.** `crates/devd-core/src/process.rs` spawns with `stdin(Stdio::null())`
  and offers `kill_tree`, `pid_alive`, `process_image` — process supervision, not
  a terminal. No `portable-pty` or equivalent in any manifest.
- **No WebSocket route.** `devd-server` has axum and tokio; the API is request/
  response plus one SSE stream (`/api/events`).
- **File viewing:** `fs.read` is used in `app.js` at one place — a search result
  drawer that shows an 80-line window around a hit — and in the diff view. There
  is no file tree and no way to browse a project's folders in the window.
- **Preview:** `sitePreview()` renders `<iframe src="{site url}">` with a sandbox
  and a "Reload" button. It previews a site devd serves; it cannot open
  `https://example.com` or a page of a project that is not served.
- **`docs.fetch`:** fetches a public https page and returns text, with private and
  loopback addresses refused. That is a fetcher, not a browser.

## The plan

Ordered by value and by how much of devd's existing architecture they reuse. Each
part is a capability in the registry first, so the CLI, the window, MCP and the
agent reach it the same way — devd's rule, and the reason these are additive.

### 1. A project file viewer (smallest, no new dependencies)

**Why first:** highest ratio of usefulness to risk. No PTY, no native code, no new
protocol. It reuses `fs.list` and `fs.read`, which already exist and already
refuse secrets files.

- `fs.tree` (planned) — one lazy directory listing: `{ path, depth: 1 }` returns entries with
  `name`, `kind`, `size`, `modified`, and nothing else. Reuses
  `fs::authorize` and `fs::is_secret_file`, so the fence is the existing one.
- **Files tab** on a project: a lazy tree on the left, the selected file on the
  right with line numbers, gutter and syntax tokens.
- **Syntax highlighting without a dependency:** ship a small tokenizer for the
  languages devd's stacks use (PHP, JS/TS, JSON, YAML, TOML, Rust, SQL, HTML, CSS,
  shell, Markdown), ~400 lines of JS in the UI. A highlighter library would be a
  third dependency in a window that deliberately has two (xterm, its addon) and
  currently neither. Write the tokenizer, or vendor one under the same MIT
  licence and record it in `ui/vendor/`.
- **The agent's view and the person's view of a file must agree:** the same
  `fs.read` bounds, the same secret refusal, the same "this file is too large,
  here is the first 1 MB" behaviour.

Estimate: **2–3 days.** Files: `devd-agent/src/fs.rs` (add `tree`), `caps.rs`
(registration), one capability in `docs/reference.md`, `ui/app.js` (a view),
`ui/style.css`, a test that the secret fence holds in `fs.tree` (planned).

### 2. An integrated terminal

**Why this is the big one:** it is the one feature where devd being a *server* pays
off, because the same session can be read by a person and by an agent.

The mistake to avoid is building a terminal only for the person, which is what
every "open a terminal" button does. The design that makes it worth doing is:

```
  shell process  ──►  session buffer  ──►  xterm.js in the window   (a person)
  (portable-pty)      (server, owned)  ──►  terminal.read tool      (an agent)
                                      ──►  the run's evidence        (the record)
```

- **A session, not a spawn.** `terminal.open { project, shell? }` starts a PTY,
  keeps it in a registry keyed by id, and returns the id. `terminal.write { id,
  data }`, `terminal.read { id, since }`, `terminal.resize { id, cols, rows }`,
  `terminal.signal { id, signal }`, `terminal.close { id }`, `terminal.list` (planned).
  This is DSH's six-tool shape, and it is the right one: an agent that can only
  `exec` a command in one shot cannot debug interactively, and a person who has to
  leave the window to run a command loses the context the window exists to keep.
- **Who may drive it.** A person drives their own sessions. An **agent cannot open
  a shell**: that would be `project.exec` with extra steps and no argument to
  review. An agent may *read* a session a person opened, and write to it only
  through an approval that names the exact bytes — the same rule as every other
  destructive operation. This keeps the "an agent cannot approve its own action"
  guarantee intact, which a naive agent-terminal would break.
- **Transport.** A WebSocket per session at `/api/terminal/:id`, authorised by the
  same loopback token as every other route, or — if a WebSocket is too much for
  the current axum wiring — SSE down (`/api/events` already exists) and `POST`
  up. Decide this first; it sets the size of the work.
- **Xterm.** Vendor `@xterm/xterm` and `@xterm/addon-fit` under
  `crates/devd-server/ui/vendor/` with their MIT licences, which is what the
  README already (wrongly) says is there. ~300 KB, no build step.
- **The agent's view.** `terminal.read` (planned) returns the last N lines of the *rendered*
  buffer, not raw escape sequences. That is the whole reason to keep terminal
  state server-side: an agent that receives `\x1b[32m` noise is worse at this than
  one that receives the screen. Either reimplement the grid in Rust or take
  headless xterm's model; a plain 2D char grid plus the last SGR span per cell is
  enough for what a model needs.
- **Cleanup.** A session that outlives the process is a leaked shell. The daemon
  closes sessions whose owner is gone, exactly as it already reconciles services
  by process image and start time.
- **Windows.** devd is Windows-first. A PTY on Windows is a ConPTY, which means
  `portable-pty` or a hand-rolled `CreatePseudoConsole`. This is the real risk in
  the whole plan: it is the first native dependency in a project that has
  deliberately downloaded and verified binaries instead. Decide early whether to
  take `portable-pty` (MIT/ Apache-2.0, used widely) or to shell out to
  `conhost` — and record the decision, because it changes what "no third-party
  binaries" in the README means.

Estimate: **1.5–2 weeks** for a version worth shipping: a day to settle the
transport, two on the PTY and session registry, two on the window, two on the
agent-facing read/write with approval, the rest on tests and the Windows path.

### 3. A browser

Three separable things are called "the browser tool". Take them in this order;
the first is a day, the last is a week and changes the dependencies.

1. **A fetch that keeps the DOM** (1 day). `docs.fetch` already fetches public
   https pages. Add `web.open { url }` returning the title, the visible text, the
   headings, the links and the form fields — what an agent needs to act on a page,
   derived with a small HTML-to-structure pass rather than a renderer. Reuse
   `docs.rs`'s address checks. This makes devd's agent able to read the web as well
   as DSH's can, and better, because it returns structure rather than prose.
2. **A preview that is not a served site** (2–3 days). Today the Preview tab only
   works for sites devd serves. Let it take any loopback URL the machine can
   reach — a `php artisan serve` on 8000, a `next dev` on 3000, a static file — with
   the same `<iframe>`, the same sandbox, and a URL bar. A person can then look at
   a dev server devd did not start, which is a real and common case.
3. **A real headless browser, for a person to look at and for an agent to record**
   (1 week+, opt-in). Not a tool that drives a browser to click things — that is a
   different product, and DSH does not have it either. The valuable version here
   is: *open a page in a real engine and return a screenshot and the console
   errors.* That is what lets an agent say "the page renders, here it is" instead
   of "the server returned 200", and it is the only part of a browser worth the
   cost. It needs a Chromium — years of download and SmartScreen — so it must be
   an install the person opts into, verified the way every other runtime devd
   downloads is, and it must not become a required dependency.

**Do not** build an agent that clicks through a UI. There is no evidence from DSH
that this is needed, and it fights devd's model: an agent that can drive a browser
can drive any page, including ones holding a session.

### 4. Fix the documentation now, whoever does the work

- `README.md` says the window vendors xterm.js and points at
  `crates/devd-server/ui/vendor/`. Neither is true. Either vendor it (part 2) or
  remove the sentence.
- `docs/app.md` and `docs/tui.md` should say plainly that the terminal is the
  operating system's, not devd's, until it is not.
- `scripts/check-docs.ps1` checks capability ids and links; it does not check that
  a claimed file exists. Add that check — the xterm claim would have been caught.

## Recommendation

**Do part 1 now.** A file viewer is 2–3 days, adds no dependency, reuses the
existing fence, and closes the most obvious gap between what devd shows (projects,
sites, services) and what it lets you look at (nothing inside a project).

**Then part 2, but decide the Windows PTY question before writing code.** It is
the only part of the plan with a dependency that changes the product's promise,
and it is the feature that most makes devd feel like an IDE rather than a manager.

**Part 3, item 1 only, until there is a reason for more.** `web.open` (planned) is a day and
makes the agent's web reading as good as DSH's. Parts 3.2 and 3.3 are only worth
it if someone is actually troubled by not being able to preview a dev server devd
did not start, or by an agent saying "it works" with nothing behind it.
