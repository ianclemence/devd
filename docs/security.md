# Security

A tool that manages databases, web servers and certificates, and that an AI agent
can drive, is a tool that can destroy work and publish secrets. This document is
the threat model and the specific protections, including the places where `devd`
deliberately does *not* go.

## The model

The thing being defended against is not a stranger on the network. It is:

1. **An agent that is confident and wrong** — instructed to "clean up the
   repository" and deciding that discarding uncommitted work is a reasonable
   reading of that.
2. **An agent that has been manipulated** — by a file, an issue, a commit
   message or a web page it read, into treating text as an instruction.
3. **A user in a hurry** — who runs a destructive command from a script without
   reading it, and finds out afterwards.
4. **A tool that leaks** — credentials into a config file, a log, a snapshot, an
   audit row, or a message shown on screen.

The design answer to all four is the same: make the dangerous thing require a
decision that only a human can make, make that decision visible before it is
taken, and record what happened.

## Enforced in code, not in a prompt

The single most important property is that **none of these protections is a
sentence in a system prompt**. A prompt is a request. The registry is a gate.

| Protection | Enforced by |
| --- | --- |
| A destructive action needs approval | `Tier::requires_approval()` checked in `Registry::dispatch_inner`, before the handler is called |
| A remote action needs approval | `Tier::Remote`, same check |
| An agent cannot approve its own action | The approval check refuses when `Actor::is_agent()`; no interface can pass an approval on an agent's behalf, because the dispatcher is the only entry point |
| Some operations are human-only | `human_only` is checked *before* the approval check, so an approval cannot unlock it |
| A refusal is not local to a surface | Every surface calls the same dispatcher; there is no second path |
| A declined action does not run | `Approval::Denied` returns before the handler, and the attempt is still audited |
| A dry run changes nothing | The plan calls the capability's *planner*, never its handler |
| A capability that needs approval can always explain itself | A test asserts `requires_approval` implies `supports_dry_run` for every capability |
| A plan cannot contradict itself | `Plan::destructive(true)` also sets `requires_approval` |

An agent's request for a destructive operation is therefore a well-formed
request that fails. That failure is the feature.

## Approvals

Approval is a value the operator produced — `Approval::Granted`, usually after
reading a plan — not a value a caller can assert. `devd --yes` is the CLI's
deliberate, explicit form. `devd --plan` prints the plan and exits without
asking, because a plan is not an approval.

Every approval shows, before the decision: the repository, the branch, the
target, and the affected files. For a hard reset, the plan names the mode and
says the working tree is discarded. For a force push, it names the remote and
the branch. For a conflict resolution, it says the other side is discarded.

## The local HTTP surface

The application window speaks to a loopback HTTP API, which adds a threat this
product would not otherwise have: **a web page the user happens to have open can
send requests to `http://127.0.0.1:<port>`.** It cannot read our responses
without our cooperation, but it can send. Four defences are enforced in the
server, in this order:

1. **`Host` allow-list.** Only `127.0.0.1:<port>`, `localhost:<port>` and
   `[::1]:<port>` are accepted. This is the defence against DNS rebinding: a
   hostile page can point its own hostname at `127.0.0.1` and reach us, but it
   cannot make the `Host` header it sends be a loopback name. A mismatch is
   `403 forbidden_host`.
2. **`OPTIONS` is refused with `405`, before anything else.** There is no CORS
   negotiation, so no cross-origin caller can complete a preflight. Reordering
   this ahead of the origin check makes the answer deterministic regardless of
   what `Origin` says.
3. **Foreign `Origin` is refused with `403 forbidden_origin`.**
4. **A 256-bit bearer token** is required on every `/api` request, compared in
   constant time. The token is generated at startup, injected into the served
   HTML, never written to disk, and returned by no endpoint. A page from another
   origin cannot read our HTML and therefore cannot learn the token.

The bind is loopback only, with no configuration that serves the interface to a
network. The server also **forces the actor** of every request to `human:http`,
so a caller cannot claim to be an agent or a daemon to escape a restriction, and
cannot claim to be a human to unlock an agent-only path.

Approvals over HTTP are two-step, and this is a security property rather than a
convenience: `POST /api/invoke` returns a plan and a token and changes nothing;
`POST /api/approve { token }` replays **the server's own stored arguments**. A
client cannot assert `approve: true`, and what was displayed cannot differ from
what executes. A token is single-use, expires after 120 seconds, is bounded to a
fixed number of pending decisions, and is bound to the capability, the arguments
and the actor. A declined operation is dispatched with `Approval::Denied`, so it
reaches the audit trail rather than vanishing.

A test suite drives the real router for each defence, and the same checks were
run against the release binary on a socket; the observed results are recorded in
[`app.md`](app.md#what-was-actually-observed).

## The terminal dashboard

A terminal is an output device that executes control sequences, which makes it a
place repository content must not be written raw. A branch name or a path
containing `ESC [ 2 J` is not text at that point; it is a command to clear the
reader's screen. Every string the dashboard draws passes through one function
that replaces control characters, and a test renders a branch named
`evil\u{1b}[2Jbranch` and asserts that no escape reached the buffer.

The dashboard is a client of the dispatcher and nothing else, so the tiers,
the approvals and the audit apply unchanged. Two details of its own:

* **A modal owns every key while it is open**, so a keystroke cannot both answer
  a prompt and act on the view behind it, and `Esc` on an approval means *no*
  rather than *dismiss the question*.
* **A decline is dispatched, not dismissed.** `n` calls the dispatcher with
  `Approval::Denied`, so the refusal is a row in the audit trail instead of a
  keystroke that vanished.

It refuses to start without an interactive terminal rather than attempting raw
mode on a handle that cannot support it — an attempt that took the process down
with `0xc0000005` and no output on Windows before the guard existed.

## The agent surface

An agent connecting over MCP is treated as what it is: a caller that is
untrusted, not because it is hostile but because it can be *confident and wrong*.

* **The actor is forced to an agent.** The MCP server discards whatever actor it
  is handed and installs its own, for the same reason the HTTP server does: a
  claim is not evidence, and the tier model must not rest on one.
* **A refusal is recorded and explained.** An approval-gated operation is
  refused and written to the audit trail. The agent is handed the plan plus an
  explicit instruction not to report the operation as having happened, because
  the realistic failure is an agent that describes work it was not permitted to
  do as though it were done.
* **`human_only` cannot be unlocked by an approval an agent holds**, since
  `Approval::Granted` is only produced by a human decision and the agent surface
  has no way to produce one.
* **Tool results are data.** Repository content — commit messages, file names,
  diff text — is never turned into a capability call, and never echoed back as an
  instruction. The only instructions a model receives are written by the
  registry or by the MCP crate.

## Protected Git operations

These cannot execute without a human approval:

| Operation | Why |
| --- | --- |
| `git.discard` | Destroys uncommitted work, which is the only copy. |
| `git.reset` (`hard`) | Destroys uncommitted work and moves the branch. |
| `git.rebase` | Rewrites commit identity. |
| `git.squash` | Rewrites commit identity. |
| `git.cherry.pick`, `git.revert` | Can conflict, can rewrite the tree. |
| `git.branch.delete` | A branch can be the only reference to a commit. Force is required for an unmerged one. |
| `git.tag.delete` | A published tag may be someone's release. |
| `git.stash.drop` | The last reference to shelved work. |
| `git.worktree.remove` | Refuses a worktree with uncommitted work unless forced. |
| `git.conflicts.resolve` | Taking one side discards the other from the working tree. |
| `git.push` | Publishes to a machine `devd` does not own. |
| `git.push.force` | Overwrites history on a machine `devd` does not own. |
| `git.clean`-like effects | There is no capability that removes ignored or untracked files in bulk; the pattern is never written into a `git clean` invocation. |

Two decisions are worth stating plainly.

**Force push is a separate capability, not a flag.** `Agit push --force` and a
force push being reachable as a boolean on the ordinary push path are very
different. Here there is no `force` property in `git.push`'s schema at all, so
no code path — not the CLI, not the GUI, not an agent through MCP — can construct
a forced update without going through a capability whose tier demands approval.
A test asserts that the ordinary push schema has no `force` field.

**A denial cannot be routed around.** If `git.discard` is denied, the workaround
would be a hard reset, which is a *different* capability that demands its own
approval and is refused too. There is no general-purpose "run a command"
capability anywhere in the registry, so there is no escape hatch to lower into.

## Input never becomes a shell string

Every external command is an `argv` array. There is no `format!`-into-a-shell
anywhere in the codebase, and `git` is invoked with `Command::new(path).args(…)`.

On top of that, the pieces that could be mistaken for options are validated:

* **Revisions and refs** are refused if they start with `-` or contain control
  characters or whitespace, so a branch named `--upload-pack=…` is an error
  rather than an option.
* **Repository-relative paths** are refused if they are absolute, contain a null
  byte, or escape the repository with `..`.
* **Config keys** must look like `section.name` and may not start with `-`.
* **Config values** may not start with `--`, so a value cannot become a flag.
* **Remote URLs** must use a scheme `devd` understands, and are refused if they
  carry a password or begin with `-`.

The `--` separator is used before positionals in clone and apply invocations so
that a path or URL cannot be read as an option.

## Credentials and secrets

These rules are absolute:

* **A remote URL that embeds a password is refused.** `https://user:pw@host/x`
  would be written to `.git/config` in plain text, and Git would echo it in
  error messages. A bare username is allowed, because that is how SSH and many
  hosts address a repository and it carries no secret.
* **Configuration keys that exist to hold a credential are refused**:
  anything containing `extraheader`, and any key whose last segment is
  `password`, `passwd`, `token`, `secret`, `apikey`, `api_key` or `credential`.
  The error says what to use instead: a credential helper, an SSH key, or the OS
  credential store.
* **Certificate verification cannot be turned off.** `http.sslVerify` (including
  the per-URL form) may not be set to a false value. `devd` never weakens
  transport security to make a connection succeed.
* **Messages are redacted.** Any URL in an error or a log line is passed through
  a redactor that replaces userinfo with `[redacted]` before it is returned or
  recorded.
* **Audit arguments are redacted.** A secret-looking field in an invocation's
  arguments is replaced before the row is written, so an interface that leaks a
  secret into an argument object cannot leak it into the audit file.
* **Git cannot prompt.** `GIT_TERMINAL_PROMPT=0` is set for every invocation, so
  a command fails rather than blocking on a credential prompt nobody can answer.

Credentials are never stored by `devd` in a configuration file, a log, an audit
row, or an MCP response. Authentication is Git's own: credential helpers, the OS
credential store, and the SSH agent.

## Output handling

* `LC_ALL=C` is forced, so messages and dates have one shape.
* The pager is disabled (`GIT_PAGER`, `PAGER`), so a command cannot block
  waiting for a reader.
* Output is size-capped, so a repository with an enormous diff cannot exhaust
  memory.
* Every external call takes a timeout, so a wedged process cannot hang a
  request.
* The process tree is killed on stop — `taskkill /T /F` on Windows, a process
  group on Linux — so an interrupted operation does not leave a server running.
* `GIT_ADVICE=0` and `GIT_OPTIONAL_LOCKS=0` keep output and locking behaviour
  stable and non-interactive.

## Interrupted operations

An interrupted merge, rebase, cherry-pick or revert is a state, not a failure.
`git.status` reports it, `git.op.continue` resumes it and `git.op.abort` restores
the previous state.

The property that matters is tested explicitly: **aborting must not remove work
the user had not committed in files the operation did not touch.** The lifecycle
suite makes an uncommitted change, forces a conflict, aborts, and asserts both
that the conflict is gone and that the unrelated uncommitted file is byte-for-byte
intact. `devd git undo` prefers to abort rather than to reset, because aborting
is the reversible option.

## Git hooks

`devd` does not install hooks, does not create a `hooksPath`, and does not run
anything from a repository merely because it opened it. Opening, inspecting and
diffing a repository are `Tier::Read` and execute no repository content.

Hooks that already exist in the repository a user is working in *do* run when
Git runs them during a commit, because that is Git's own semantics and a user's
commits are expected to respect their repository's hooks. `devd` disables
Git's *editors* (`GIT_EDITOR`, `GIT_SEQUENCE_EDITOR`) so an unattended command
cannot hang, but it does not silently bypass verification a repository asks for.
If you are inspecting a repository you do not trust, use the read-tier
operations — `status`, `diff`, `log`, `show`, `blame`, `conflicts` — none of
which execute repository content.

An explicit switch for this, so a user can choose a strict no-hooks mode, is a
candidate for a later milestone rather than a silent default either way.

## Git configuration as a code path

A Git config file can name a program to run: `core.sshCommand`, `core.pager`,
`core.editor`, `core.hooksPath`, `core.fsmonitor`, and aliases. Writing Git
configuration is therefore not a benign operation, which is why
`git.config.set` is `human_only`: an agent cannot call it, with or without an
approval. A human editing their own repository's configuration is choosing
their own tooling, and `devd` records the change.

## Services, TLS and the database

A service runs as the invoking user, so what it is allowed to do is the user's
own authority. The protections are about what `devd` writes and who can reach it.

* **Nothing is bundled and everything is fetched over HTTPS.** Runtimes and
  services are downloaded from the vendor when installed, checked against the
  digest the vendor publishes, extracted in-process, and moved into place only
  after the digest matches. nginx publishes only a PGP signature, so its install
  reports that it could verify length and not content — the gap is in
  [`limitations.md`](limitations.md) and in the install output itself.
* **Hostnames are validated before they reach a file.** `util::validate_hostname`
  rejects anything that is not a lowercase DNS label sequence, so a site name
  cannot inject a directive into `nginx.conf` or a path into `certs/`.
* **Configuration is generated, not executed.** `nginx.conf` is rendered from the
  site table, and a service is started with an `argv` array. There is no path by
  which a site name, a path or a model's output becomes a shell string.
* **Listeners.** nginx listens on the configured HTTP and HTTPS ports on all
  interfaces, because a site may need to be reachable from another device on the
  LAN; MariaDB binds `127.0.0.1` only. `php-cgi` is a FastCGI listener on
  `127.0.0.1:9000` with **no authentication of its own** — any local user who can
  connect to that port can execute PHP as the user running `devd`. That is
  inherent to FastCGI, not something `devd` introduces, and it is why the
  listener is bound to loopback.
* **The CA key never leaves the machine and never enters a response.** `devd`
  writes `certs/devd-ca.key` when it creates the authority; `tls.ca.show` returns
  the certificate, never the key; the key is not logged and no capability returns
  it. Leaf keys are written with owner-only permissions on Unix.
* **Trusting the CA is a separate, human-approved operation.** `tls.trust` is
  `Tier::Admin`, prints the exact `certutil`/`trust` command's effect before it
  runs, and changes the machine's trust store. `devd` never disables certificate
  verification anywhere.
* **Hosts writes are `Tier::Admin` and snapshotted.** The managed block preserves
  every line `devd` did not write, the file is copied to `snapshots/` before the
  write, and the operation is separate from creating a site.
* **The database password is not stored.** `root` is passwordless and bound to
  loopback by default, and the install says so. If
  `DEVD_MYSQL_ROOT_PASSWORD` is set it is passed to the one-time bootstrap on its
  command line and to clients through `MYSQL_PWD`, never written to `my.ini`,
  `settings.toml`, the audit row or a log. A generated password held in the
  operating system's credential store is the next step, not a claim of this one.
* **A fresh MariaDB data directory has its `test` schema removed.** MariaDB's
  bootstrap creates a world-writable database; `devd` deletes it while the server
  is still stopped, and never touches an existing data directory.

## Windows and Linux

Differences are stated rather than hidden:

* Process trees are killed with `taskkill /T /F` on Windows and a process group
  on Linux. A Windows Job Object would be stronger and is not implemented.
* Certificates are trusted with `certutil -addstore` on Windows and `trust
  anchor` on Linux; on Linux without p11-kit the command prints the manual
  `update-ca-certificates` recipe instead of pretending it worked.
* **Only Windows has a service provider in this release.** nginx publishes a
  Windows zip and no Linux binary; MariaDB publishes `winx64` archives. On Linux
  the service catalogues are therefore empty rather than guessing at a
  distribution's package manager. See [`limitations.md`](limitations.md).
* Locations differ (`%LOCALAPPDATA%` versus `~/.local/share`), resolved through
  one `Paths` type rather than scattered string building.
* Line endings are the repository's business, not `devd`'s. `devd` never changes
  a user's `core.autocrlf`. Parsers trim line-ending artefacts so that output
  from either platform is read the same way.
* Paths containing spaces are covered by test, because passing each argument as
  an `argv` element is the whole point.
* There is no macOS support in this version.

## Audit

Every attempt is recorded: the actor (human, agent or daemon), the surface (CLI,
TUI, GUI, HTTP, MCP or daemon), the capability, the tier, the redacted arguments,
whether it succeeded, and the message. **A refused operation is recorded too** —
a refusal is a fact, and a trail that only records successes cannot answer the
question "did anything try?".

`devd audit -n 20` reads the trail, and the Activity page in the application
window shows the same rows. A row is marked as a *plan* when it was a preview
that changed nothing, and as *refused* when it was declined or blocked, so a
preview is never mistaken for an applied operation.

The store is SQLite with a versioned schema and an integrity check, so a
corrupted database is reported rather than silently trusted.

## Snapshots

`hosts.sync` copies the hosts file to `snapshots/` before it writes, and records
the copy in the store, so the one file `devd` changes outside its own directory
can be recovered. Nothing else is snapshotted yet: there is no capability that
restores a snapshot, and changing a site, a certificate or a service
configuration is not reversible through `devd`.

## What is not protected

The following are outside the model, stated so that no protection is assumed
that does not exist.

* **A human who passes `--yes` without reading** is trusted. The plan is printed;
  whether it is read is the operator's responsibility.
* **A human who is deceived into approving** is out of scope. The gate ensures a
  human decided, not that the human was right.
* **A compromised `git` executable** on `PATH` defeats every argument validation.
  `devd` runs the tool the user has installed, deliberately, and reports which
  one via `devd doctor`.
* **The state database is not encrypted.** It holds audit rows and an index, not
  credentials. If a token somehow reached an argument object, redaction keeps it
  out of the row, but a filesystem-level attacker with read access is out of
  scope.
* **No sandbox.** `devd` runs services as the invoking user, on the machine,
  without containers. This is a deliberate design decision — containers are not
  the substrate — and it means `devd` inherits that user's privileges.
* **Another process on the machine that can read the browser's memory or the
  served page** could learn the session token. The defences above stop a *web
  page*; they do not stop a local attacker with the user's own privileges, and
  nothing on a single-user machine can.
* **A malicious browser extension** in the window that has been granted access to
  the page can act with the token. The window is opened in application mode
  without extensions in the recommended launcher, but the fallback is the user's
  ordinary browser.
