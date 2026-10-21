# devd

> **Your development environment, on your machine, with your rules.**
> *One program for the services, the sites, the runtimes and the agents — and
> nothing it does happens without your say-so.*

---

## What devd does for you

devd is the development environment you would otherwise assemble by hand, in one
place that knows what is installed, what is running, and what each project needs.

- **It finds your project.** You browse to the folder; devd reads what is
  inside it — the stack, the package manager, the runtimes it asks for, the
  services it talks to — and sets it up from there.
- **It runs the services.** PHP, Node.js, Go, nginx and MariaDB are installed
  from their publishers, started, health-checked and kept running. A site is a
  directory, a hostname and a certificate, and it answers over HTTPS.
- **It runs your agents.** Claude Code, OpenCode or a shell, in a real terminal
  that survives you changing screens. Give an agent a goal and a team splits it:
  a planner proposes the work, you approve it, specialists do it in isolated
  copies, and a task is only done when devd can see the files that changed and
  the check that passed.
- **It keeps the record.** Every operation is written down: what was asked,
  what ran, what it changed, and who approved it.

You say what you want. devd does the rest, and it asks before it does anything
consequential.

---

## Two rules

Everything in devd follows from these, and they are why it can be trusted with
your machine.

- **One implementation.** Every action is declared once, in a capability
  registry. The command line, the terminal dashboard, the application window and
  the MCP server are projections of that registry; none of them holds business
  logic of its own, so they cannot disagree about what is possible.
- **Approval is enforced, not suggested.** Destructive, privileged and remote
  operations return a plan and require an explicit human decision before they
  run. An agent cannot approve its own action, and a declined operation never
  executes — from any interface.

## Projects for any stack

devd creates, imports and runs projects across languages, driven by structured stack definitions with official documentation links:
`devd new` (guided), `devd project detect|import|tasks|run|exec|browse`, `devd tools list|install|start`, and the same operations in the window (Create, Develop, Tools), the terminal dashboard and MCP. See [`docs/stacks-engine.md`](docs/stacks-engine.md) and the generated [compatibility matrix](docs/stacks.md).

## An agent team that works and proves it

Describe what you want built (`devd team start "..." --project .`, or Assistant > Team
runs in the window). A planner proposes tasks; you approve the plan; specialists work
in parallel in isolated copies of the project; a task counts only when devd can see
the files changed and a check that passed. Configure providers and which model does
which work under **Models**, see what is on your machine under **Server > Services**,
and read how it all fits together in [docs/agents.md](docs/agents.md).

## Requirements

- **Windows 10/11 x64** or **Linux x64**.
- The Windows installer needs nothing else: no Rust, no administrator rights.
  The native window uses the WebView2 runtime that ships with current Edge.
- Building from source needs Rust 1.77 or newer and a C compiler (`cc`) for the
  bundled SQLite.
- Git must be on `PATH` (or named in `git_executable`) for the Git capabilities.

Service management is available on Windows. On Linux the core, Git, the CLI,
the dashboard, the application window, MCP, and the Node.js and Go runtimes work;
nginx and MariaDB installation is not available yet because devd manages the
vendors' Windows archives. See [`docs/limitations.md`](docs/limitations.md).

## Install

### Windows

Download and run the installer from the
[latest release](https://github.com/ianclemence/devd/releases/latest):

```powershell
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.9.0/devd-setup-0.8.0.exe
.\devd-setup-0.8.0.exe
```

It installs for the current user only - no administrator prompt - adds `devd`
to `PATH`, creates Start Menu entries and installs the native `devd-desktop`
window. On the last page, **Set up devd and open it** downloads PHP, nginx and
MariaDB (about 140 MB), starts them, creates a site and opens the desktop
application.

Uninstalling keeps your sites and data in `%LOCALAPPDATA%\devd`.

The installer is not code-signed, so Windows SmartScreen warns once. The
published `SHA256SUMS.txt` lets you check what you downloaded.

### Portable binary

The same binary is attached as a standalone executable if you would rather not
install anything. Download it, verify it, and put it on `PATH`:

```powershell
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.9.0/devd-v0.9.0-x86_64-pc-windows-gnu.exe
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.9.0/SHA256SUMS.txt
certutil -hashfile devd-v0.9.0-x86_64-pc-windows-gnu.exe SHA256

Rename-Item devd-v0.9.0-x86_64-pc-windows-gnu.exe devd.exe
devd setup --yes
devd ui
```

It is self-contained: it links only Windows system libraries, so it needs no
Rust toolchain, MinGW or Visual C++ runtime.

The native window is also published as a portable zip:

```powershell
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.9.0/devd-desktop-v0.9.0-x86_64-pc-windows-gnu.zip
Expand-Archive devd-desktop-v0.9.0-x86_64-pc-windows-gnu.zip -DestinationPath devd-desktop
.\devd-desktop\devd-desktop.exe
```

The zip contains `devd-desktop.exe` and `WebView2Loader.dll`; keep them together.
It starts its own backend and needs WebView2 (present with current Edge).

### From source

```bash
git clone https://github.com/ianclemence/devd.git
cd devd
cargo build --release
./target/release/devd doctor      # devd.exe on Windows
```

The binary is `target/release/devd`. Put it on `PATH`.

No Linux binary is attached to the current release: an artifact is published
only after it has been executed on its target platform, and the release is built
on Windows. To produce one, run `scripts/release-linux.sh` on Linux, or
`scripts/release-linux.ps1` from Windows through WSL or Docker. Both are
described in [`docs/releasing.md`](docs/releasing.md).

### Linux

```bash
git clone https://github.com/ianclemence/devd.git
cd devd
cargo build --release -p devd-cli
./scripts/install-linux.sh --binary target/release/devd
```

The installer is per-user and needs no administrator rights: it puts the
binary in `~/.local/bin`, adds a desktop entry, and runs the first-time setup.
`./scripts/install-linux.sh --uninstall` removes the program and keeps your
data; `--uninstall --purge` removes both.

## Quick start

The installer runs this for you on first start. From a portable build or a
source checkout, the same step is one command:

```bash
devd setup --yes
```

It installs PHP, nginx and MariaDB if they are missing, starts them, and creates
a site at `https://localhost:<https-port>`. Then open the desktop application with
the Start Menu entry, or from source:

```bash
devd-desktop          # the native window (target/release/devd-desktop)
devd ui               # the same interface in an installed browser
```

From there, a normal project site is three commands:

```bash
devd site create blog            # directory, starter page, vhost, certificate
devd hosts sync -y               # publish blog.test (elevated shell on Windows)
devd tls trust -y                # add devd's CA to the OS trust store
```

`devd site create` prints the URL. On the defaults that is
`https://blog.test`, served by nginx, executed by PHP over FastCGI, with a
certificate issued by devd's own authority.

Two of those commands change machine-level state and are therefore separate,
explicitly approved operations rather than side effects of creating a site:
writing the hosts file (elevation required on Windows) and adding the CA to the
trust store.

## Command reference

`devd <command> --help` documents every flag. The typed commands map one-to-one
onto capabilities; `devd call <id>` reaches any capability directly.

### Runtimes

```bash
devd runtime available php        # what the vendor publishes, and its digest
devd runtime install php@8.3
devd runtime install node@lts
devd runtime install go@latest
devd runtime list
devd runtime verify php
devd runtime use php@8.4          # the version new sites use
devd runtime remove php@8.2       # destructive: asks for approval
```

### Services

```bash
devd service available nginx
devd service install nginx
devd service install mariadb --port 3306
devd service start nginx
devd service status nginx
devd service logs nginx
devd service stop nginx
devd service remove mariadb       # destructive: keeps the data directory
```

`service start` does not report success until the port is open. A service that
fails to start is killed, its run is recorded as failed, and the error includes
the tail of its own log.

### Sites, hosts and the web server

```bash
devd site create blog             # --host and --root override the defaults
devd site list
devd site remove blog             # destructive; keeps files unless --delete-files

devd hosts show                   # what a sync would change
devd hosts sync                   # admin: writes the managed block

devd web render                   # rebuild nginx.conf from the site table
devd web test                     # nginx -t
devd web reload
```

`nginx.conf` is generated. The site table is the source of truth, so edits to
the generated file are overwritten rather than merged.

### Stack

```bash
devd stack up                     # start every enabled service, in order
devd stack down                   # stop every running service
```

### TLS

```bash
devd tls ca                       # create the local authority if missing
devd tls show                     # the CA certificate, for a browser
devd tls trust                    # admin: add it to the system trust store
devd tls untrust
devd tls issue blog.test
devd tls list
```

Certificates are issued in-process: an ECDSA P-256 authority, and per-host leaf
certificates written as a full chain for nginx. The CA key never leaves
`certs/devd-ca.key` and is never returned by a capability.

### Databases

```bash
devd db list
devd db create blog
devd db drop blog                 # destructive: everything in it is deleted
```

### Git

The Git commands keep Git's own vocabulary.

```bash
devd git status
devd git stage .
devd git diff --staged
devd git commit -m "message"
devd git log -n 5

devd git branch list
devd git switch feature
devd git fetch && devd git pull
devd git push

devd git stash push -m "wip"
devd git worktree add ../wt feature
devd git rebase --onto main
devd git bisect start HEAD v1.0.0
devd git undo                     # recover a bad reset through the reflog
```

Every Git command is one of 64 capabilities grouped under `git.repo`,
`git.changes`, `git.commits`, `git.history`, `git.branches`, `git.tags`,
`git.remotes`, `git.merge`, `git.stashes`, `git.worktrees`, `git.config` and
`git.recovery`. See [`docs/git-cli.md`](docs/git-cli.md) for the full mapping.

### Interfaces

```bash
devd ui            # the application window
devd serve         # the loopback API only, for scripts and tests
devd tui           # the terminal dashboard
devd mcp           # MCP on stdin/stdout, for agents

devd capabilities  # the whole registry
devd describe git.reset
devd doctor
devd audit -n 20
```

The graphical surface is one application, not a website: the released
`devd-desktop` window starts the same loopback server in-process and loads the
same HTML, CSS and JavaScript. `devd ui` opens the same interface through an
installed browser for development and for systems without WebView2; `devd serve`
exposes the API without a window for scripts and tests. The server binds
`127.0.0.1` only and requires a per-process bearer token. There is no second
graphical toolkit to keep in step. See [`docs/desktop.md`](docs/desktop.md).

### Agents

The window runs the coding-agent CLI you already use inside the project. It is a
real terminal, not a chat facade: the harness keeps its own model, credentials
and permission prompts.

* **Agents** lists the harnesses found on this machine - Claude Code, OpenCode
  and a plain shell - and starts the one you choose in the selected project
  directory.
* Every session is its own **real PTY**, with a stable id, a durable metadata row
  and a bounded transcript. Switching to Preview, opening another tab, or
  reloading the whole page does not stop it; the window reattaches to the same
  process.
* Multiple sessions run at once, with tabs and a session list. Completed and
  failed sessions stay readable after their process ends.
* **Delegate a task** starts a second agent as a genuine child process and waits
  with a timeout. The child has its own working directory, instructions, exit
  code and result. Limits stop runaway spawning (8 unfinished tasks, depth 3).
* devd does not proxy the model, store prompts or read the conversation. The
  agent's own tools act on the project with the permissions that harness asks
  for; terminal input is human-only, so an agent cannot type into your session.
* See [`docs/sessions.md`](docs/sessions.md).

### Project processes

A project can declare its dev processes in `devd.yml`. devd validates the file,
shows the exact commands, and runs nothing until you approve that exact manifest.

```yaml
version: 1
project:
  name: example-app
processes:
  web:
    command: ["npm", "run", "dev"]
    working_dir: "."
    health:
      url: "http://127.0.0.1:3000/"
  worker:
    command: ["npm", "run", "worker"]
    depends_on: ["web"]
```

* **Processes** shows status, health, pid, restarts and the last failure; start,
  stop, restart and follow logs without leaving the window.
* Commands are argv arrays, never shell strings. Working directories must stay
  inside the project. Health checks are loopback-only. The process environment is
  explicit and does not inherit the daemon's credentials.
* Editing the manifest invalidates the approval. A process will not restart under
  commands nobody reviewed.
* See [`docs/devd-yml.md`](docs/devd-yml.md).

### Shared project state

**Shared** holds project-scoped scratchpads and structured todos. They are not
chat history: they survive sessions, reloads and application restarts, and both
people and agents use the same capability API.

* Scratchpads have authors, timestamps and revision numbers. A stale edit is a
  conflict, not a silent overwrite.
* Todos have status, priority, assignee, task association and transition history.
* An agent can append a finding, create a todo for another agent, spawn it, wait
  for the result and update the todo. Deleting a note or todo needs human
  approval.
* See [`docs/shared-state.md`](docs/shared-state.md).

### Preview

**Preview** embeds the running site in the window: pick a site, see it render,
reload it, or open it in the system browser. The certificate authority has to be
trusted for an HTTPS site to render without a warning, and the page says so when
it is not.

## Configuration

State lives in `%LOCALAPPDATA%\devd` on Windows and `$XDG_DATA_HOME/devd`
(usually `~/.local/share/devd`) on Linux. `DEVD_HOME` overrides the location,
which is useful for a portable installation.

```text
settings.toml     preferences
state.db          sites, services, runtimes and the audit trail
www/              projects
runtimes/         PHP, Node.js and Go
services/         nginx and MariaDB
etc/              generated configuration and MariaDB data
certs/            the authority and per-site certificates
logs/             service logs
sessions/         bounded agent-session transcripts, one per session
projects/         per-project process logs
snapshots/        copies taken before a file devd did not write is changed
cache/            verified downloads
```

`settings.toml` lives in the data directory and is read on every run. It is
written when `devd` changes a setting; missing keys take the default below.

| Key | Default | Meaning |
| --- | --- | --- |
| `tld` | `test` | Suffix for new site hostnames. |
| `http_port` | `80` | Port for plain HTTP, which redirects to HTTPS for TLS sites. |
| `https_port` | `443` | Port for TLS. |
| `www_root` | `<data>/www` | Where new projects are created. |
| `web_server` | `nginx` | Web server driver. |
| `primary_php` | newest installed | PHP version new sites use. |
| `git_executable` | `git` on `PATH` | Git binary to drive. |
| `git_protect_uncommitted` | `true` | Refuse branch switches that would overwrite local work. |
| `git_default_branch` | `main` | Name given to a new repository's first branch. |

Environment variables:

| Variable | Purpose |
| --- | --- |
| `DEVD_HOME` | Data directory, instead of the default. |
| `DEVD_HOSTS_FILE` | The file `hosts sync` manages. Useful for testing without elevation. |
| `DEVD_MYSQL_ROOT_PASSWORD` | Password given to MariaDB `root` when its data directory is first created. |

## How it works

Every operation is a capability: an identifier, a summary, a permission tier, a
JSON Schema for its arguments, and a handler. The handler is the only code that
touches the outside world, and it is reachable only through the dispatcher.

| Guarantee | How it is enforced |
| --- | --- |
| A destructive action cannot run without approval | `Tier::Destructive` and `Tier::Remote` are checked in `Registry::dispatch`, before the handler |
| An agent cannot approve its own action | `Actor::is_agent()` fails the approval check unconditionally; there is no agent-supplied approval path |
| Some operations are reserved for humans | `human_only` is checked before approval is considered |
| A declined action does not run | `Approval::Denied` short-circuits the dispatcher, and the attempt is still audited |
| A refusal cannot be bypassed from another interface | Every interface calls the same dispatcher |
| Input never becomes a shell string | Arguments are typed structs turned into `argv` arrays; branches, paths and hostnames are validated against option injection |
| Secrets do not reach the audit trail | Argument objects are redacted before being recorded |

Approval is two-step. A privileged operation returns its plan and a single-use
token and changes nothing; the plan is what the human approves, and the
operation runs only when that token is presented back. The CLI calls this
`--plan` and `--yes`, the dashboard shows the plan and waits for `y`, and the
HTTP API returns `approval_required` with a token. In every case the refusal is
recorded.

For the threat model, the loopback API's defences, and the Git-specific
protections, see [`docs/security.md`](docs/security.md).

## Documentation

- [Agents, the team, skills and models](docs/agents.md) and the [product audit](docs/audit.md)

| Document | Contents |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | The registry, the dispatcher, the state store, and the skill crates. |
| [`docs/security.md`](docs/security.md) | The threat model and the specific protections. |
| [`docs/app.md`](docs/app.md) | The application window, the loopback API and the approval flow. |
| [`docs/tui.md`](docs/tui.md) | The terminal dashboard. |
| [`docs/mcp.md`](docs/mcp.md) | The agent surface and its five discovery tools. |
| [`docs/git-cli.md`](docs/git-cli.md) | Every Git command and the capability it calls. |
| [`docs/limitations.md`](docs/limitations.md) | What is not implemented, and what is weaker than intended. |
| [`docs/releasing.md`](docs/releasing.md) | How a release is built, verified and published. |
| [`docs/research/00-comment-debt.md`](docs/research/00-comment-debt.md) | The comment audit: what to delete, what to refactor, and what to enforce. |
| [`docs/research/01-herd-investigation.md`](docs/research/01-herd-investigation.md) | What Laravel Herd 2.0.2 actually does, and the evidence for it. |
| [`docs/research/02-feature-matrix.md`](docs/research/02-feature-matrix.md) | Every Herd capability mapped to its devd equivalent, with its state. |
| [`docs/research/03-screen-inventory.md`](docs/research/03-screen-inventory.md) | Every screen, what it shows, and what was corrected. |
| [`CHANGELOG.md`](CHANGELOG.md) | Release history. |

## License

MIT. See [`LICENSE`](LICENSE).

devd bundles no third-party binaries. PHP, Node.js, Go, nginx and MariaDB are
downloaded from their vendors on first use and verified against the digest the
vendor publishes; where a vendor publishes no digest, the install says so.

The application window vendors xterm.js and its fit addon (both MIT) for the
agent terminal; their license files are in
[`crates/devd-server/ui/vendor/`](crates/devd-server/ui/vendor/).
