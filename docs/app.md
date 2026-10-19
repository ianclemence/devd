# The app: one window, one loopback API

This document records the interface decision and the reasoning, because the
choice constrains everything built afterwards.

## The decision

`devd` gets **one application**: a local web app served by the `devd` binary on
loopback, opened as a chrome-less application window.

```
devd ui
  → starts an HTTP API on 127.0.0.1:<port>
  → serves the UI from inside the binary
  → opens it in an application window
```

Inside the same binary, three surfaces over one core:

| Surface | Transport | Who it is for |
| --- | --- | --- |
| `devd` | argv | Scripts, terminals, CI |
| `devd ui` | HTTP on loopback + a window | A human managing projects |
| MCP | stdio/HTTP | Agents |

All three call the same `Registry::dispatch`. None of them has a rule the others
lack.

## Why not Tauri first

Tauri is what the reference implementation uses, and it is the eventual shell.
It is not the *first* shell, for one concrete reason and one design reason.

**The concrete reason.** This machine builds for `x86_64-pc-windows-gnu` with a
Zig-based C toolchain and no MSVC. Tauri's build step embeds a Windows resource
manifest, which wants `rc.exe` or a working `windres`, and pulls in
WebView2/COM bindings that assume the MSVC ABI. That is a real chance of spending
hours on a toolchain failure instead of on the product.

**The design reason.** A Tauri app whose UI is a webview cannot be inspected or
driven by anything except a human with a mouse. An HTTP API over the same core
can be tested in-process, called by `curl`, called by an agent, and wrapped in a
Tauri window later without touching a line of UI code. Build the API and the UI;
add the window when the window is the only thing missing.

The gap between the two is smaller than it looks. `devd ui` opens the app in a
chrome-less window using the browser's own application mode, so there is no
address bar, no tabs and no browser chrome — it reads as an application, not as a
page. What Tauri adds later is a tray icon, native notifications, and a bundle
with an installer.

## Why loopback HTTP is safe here, and what makes it safe

A local HTTP server is reachable by every process on the machine, and — this is
the part people miss — by any web page the user has open, because a browser will
happily `POST` to `http://127.0.0.1:PORT/` from a page the user is visiting.

Four defences, all enforced in the server, not in the UI:

1. **A bearer token.** Generated fresh at startup, 256 bits from the operating
   system CSPRNG, injected into the served HTML and required on every `/api`
   request. A page from another origin cannot read our HTML, so it cannot learn
   the token. Nothing about it is written to disk, and no read endpoint returns
   it.
2. **A loopback bind.** `127.0.0.1` only. There is no configuration that serves
   the UI to the network.
3. **A `Host` and `Origin` check.** Only `127.0.0.1:<port>`, `localhost:<port>`
   and `[::1]:<port>` are accepted as the request's host. That is what defuses
   DNS rebinding: a page can make its own hostname resolve to `127.0.0.1` and
   reach us, but it cannot make the `Host` header it sends be a loopback name.
   A request whose `Origin` is not ours is refused as well.
4. **No CORS at all.** We never send `Access-Control-Allow-Origin`, and `OPTIONS`
   is answered with `405` before anything else. A cross-origin caller cannot
   complete a preflight, so it cannot send a request that a browser would let it
   read the response to.

A request without a valid token gets `401` and changes nothing. Every one of
these four is covered by a test in `crates/devd-server/tests/http.rs`, three of
them against a real socket.

## The API surface

| Endpoint | Purpose |
| --- | --- |
| `GET /` | The application shell, with the session token injected |
| `GET /app.js`, `GET /style.css`, `GET /favicon.svg` | The UI, compiled into the binary |
| `GET /api/session` | Version, actor, paths, settings, registry summary |
| `GET /api/capabilities` | Ranked search: `?q=&group=&limit=&offset=` |
| `GET /api/capabilities/{id}` | The full declaration, including its argument schema |
| `GET /api/groups` | Group names with counts |
| `POST /api/invoke` | Run a capability, or open an approval for one |
| `POST /api/approve` | Apply the invocation a token stands for |
| `POST /api/deny` | Decline it, and record the refusal |
| `GET /api/audit` | Recent capability attempts, arguments redacted |
| `GET /api/doctor` | The environment check, identical to `devd doctor` |
| `POST /api/shutdown` | Stop the server |

There is no endpoint that does anything on its own. Every one of them either
reads the registry or calls it, which is why the API cannot drift from the CLI.

## Approvals in a browser

This is the interesting part, and the naive design is wrong.

The naive design is `POST /api/invoke { "approve": true }`. That makes the
approval a boolean a *client* asserts, which means anything that can reach the
API can approve its own destructive operation — exactly the property the whole
product exists to prevent.

`devd` uses a two-step flow instead:

```
POST /api/invoke { id, args }
  → { approval_required: true, plan: {…}, token: "…", expires_in_ms: 120000 }

POST /api/approve { token: "…" }
  → { outcome: {…} }
```

The token is bound to a server-side record of the exact invocation: the
capability, the arguments themselves, and the actor who asked. Approving replays
**the server's own copy** of the arguments, never the client's, so what was
displayed and what executes cannot differ even if the client is buggy, stale or
hostile. A token is refused if it is unknown, already used, expired, or if the
approval store is holding more pending decisions than it is willing to.

Declining is a first-class outcome: `POST /api/deny` dispatches the capability
with `Approval::Denied`, so the refusal is written to the audit trail and the
event stream. A declined operation is therefore never indistinguishable from one
that never arrived — which is the whole point of keeping an audit log.

That is the difference between "the human approved this" and "the client typed
`true`".

## Finding the window again

Closing a browser window closes a window, not a process. Two consequences are
handled rather than ignored:

* the served HTML carries an `X-Devd-App` header, and `devd ui` probes the port
  for it before starting anything. A port held by an existing `devd` is
  *reopened*; a port held by an unrelated program is not touched, and the next
  free port is tried instead;
* the interface has a **Quit devd** button, which calls `POST /api/shutdown` and
  stops the server gracefully.

## Command palette

The registry's ranked search is the palette. `Ctrl+K` searches capability ids,
summaries and groups, and every result is a real capability with its real tier —
so the palette cannot offer an action the CLI cannot perform, and it shows the
risk of what you are about to run.

## Navigation

| Area | Source of truth |
| --- | --- |
| Projects | `git.repo.list`, `git.status` |
| Working tree | `git.status`, `git.diff`, `git.stage`, `git.unstage`, `git.commit` |
| History | `git.log`, `git.show`, `git.search`, `git.revert`, `git.cherry.pick` |
| Branches | `git.branch.list`, `git.switch`, `git.switch.check`, `git.merge`, `git.branch.create`, `git.branch.rename`, `git.branch.delete` |
| Tags | `git.tag.list`, `git.tag.create`, `git.tag.delete`, `git.push` |
| Remotes | `git.remote.list`, `git.fetch`, `git.pull`, `git.push`, `git.push.force`, `git.sync.preview`, `git.remote.add`, `git.remote.remove` |
| Stashes | `git.stash.list`, `git.stash.push`, `git.stash.apply`, `git.stash.drop`, `git.stash.show` |
| Worktrees | `git.worktree.list`, `git.worktree.add`, `git.worktree.remove`, `git.worktree.prune` |
| Conflicts and recovery | `git.conflicts.list`, `git.conflicts.show`, `git.conflicts.resolve`, `git.reflog`, `git.reset`, `git.rebase`, `git.squash`, `git.bisect.*`, `git.op.continue`, `git.op.abort` |
| Capabilities | the registry |
| Activity | the audit table |
| System | the shared doctor checks, paths, settings |
| Services, sites, runtimes | reachable through the capability inspector; there are no dedicated panels yet |

Every panel reads a real capability. There are no placeholder controls, because a
switch that does nothing is worse than an honest empty state.

## What was actually observed

The following was run against the release binary on Windows with a real
repository, not asserted from reading the code. `T` is the session token.

| Request | Result |
| --- | --- |
| `GET /api/session` with no token | `401 unauthorized` |
| `GET /api/session` with `Host: evil.example:8790` | `403 forbidden_host` |
| `GET /api/session` with `Origin: https://evil.example` | `403 forbidden_origin` |
| `OPTIONS /api/invoke` with a foreign origin | `405 no_cors` |
| `GET /` | `200`, `X-Devd-App: 0.1.0 (8790)`, token present in the page |
| `POST /api/invoke { id: "git.reset", mode: "hard", rev: "HEAD~1" }` | `approval_required: true`, `HEAD` unchanged |
| `POST /api/approve` with that token | applied; `HEAD` moved |
| `POST /api/approve` with the same token again | `403`, `HEAD` unchanged |
| `POST /api/invoke { id: "git.discard", paths: ["src.js"] }` then `/api/deny` | file still contained the uncommitted work |
| `GET /api/audit` | the refusal recorded as `human:http`, tier `destructive`, `ok: false`, with the reason |

The `git.discard` plan that the human is shown before deciding reads:

```
discard uncommitted changes in 1 path(s)
  files: src.js
  1 of these currently differ from HEAD
  this content is NOT recoverable from Git; it was never committed
```

which is the level of honesty the approval gate is for.

## What is deliberately not here

* **No build step.** Three files — `index.html`, `app.js`, `style.css` — served
  from `include_str!`. No bundler, no `node_modules`, no framework. The interface
  is small enough that a toolchain would cost more than it saves, and a build step
  is a second thing that can be the wrong version.
* **No inline script or style.** The Content-Security-Policy is
  `default-src 'none'` with same-origin script, style and connect. That is a real
  policy, not decoration: a future change that adds an inline handler will fail
  loudly rather than quietly widen what the page can do. A test asserts the shell
  contains none.
* **No server-side business logic.** The server is generic over the registry; the
  dashboard, the working-tree view and the diff viewer are composed in the
  browser from `/api/invoke` calls. There is no bespoke `/api/git/status`
  endpoint, because that would be a second implementation of `git.status` that
  could disagree with the CLI.
