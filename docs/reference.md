# Command reference, configuration and internals

Everything the README summarises, in full. `devd <command> --help` documents every flag; the typed commands map one-to-one onto capabilities, and `devd call <id>` reaches any capability directly.

## Runtimes

```bash
devd runtime available php        # what the vendor publishes, and its digest
devd runtime install php@8.3
devd runtime install node@lts
devd runtime install go@latest
devd runtime install python@3.12   # python-build-standalone, checked against its SHA256SUMS
devd runtime install ruby@3.3      # Windows only (RubyInstaller); other platforms use the system Ruby
devd runtime list
devd runtime verify php
devd runtime use php@8.4          # the version new sites use
devd runtime remove php@8.2       # destructive: asks for approval
```

## Services

```bash
devd service available nginx
devd service install nginx
devd service install mariadb --port 3306
devd service install apache       # Apache Lounge, checked against its published SHA-256 (Windows)
devd service find apache          # Apache or Redis folders already on this machine (XAMPP, WampServer...)
devd service adopt apache 'C:\Apache24'   # use one as it is
devd service adopt redis 'C:\Redis' --port 6379
devd service install mailpit      # mail catcher; checked against the digest GitHub records
devd service adopt postgres 'C:\pgsql'    # also mongodb and memcached: run one you already have
devd db list --engine postgres
devd service start nginx
devd service status nginx
devd service logs nginx
devd service stop nginx
devd service remove mariadb       # destructive: keeps the data directory
```

`service start` does not report success until the port is open. A service that
fails to start is killed, its run is recorded as failed, and the error includes
the tail of its own log.

## Sites, hosts and the web server

```bash
devd site create blog             # --host and --root override the defaults
devd site create api --upstream 3000   # serve a running dev server at api.test
devd site upstream api 5173       # point it at another port; omit to serve a directory again
devd site list
devd site park D:\www            # a site for every project directory in it
devd site sync                    # pick up folders added since
devd site parked
devd site unpark D:\www           # its sites stay
devd site remove blog             # destructive; keeps files unless --delete-files

devd hosts show                   # what a sync would change
devd hosts sync                   # admin: writes the managed block
devd hosts sync --elevate         # Windows: ask for permission with the usual prompt

devd web use apache               # or nginx; applies when devd next starts
devd web render                   # rebuild nginx.conf or httpd.conf from the site table
devd web test                     # nginx -t or httpd -t
devd web reload                   # nginx reloads; Apache restarts

devd db dump blog                 # a SQL file under <devd home>/backups
devd db copy blog --from-port 3306   # from a MySQL on this machine into devd's MariaDB
devd db restore blog ./blog.sql

devd autostart enable             # run devd stack up when you sign in
```

`nginx.conf` and `httpd.conf` are generated. The site table is the source of truth,
so edits to the generated file are overwritten rather than merged.

## Stack

```bash
devd stack up                     # start every enabled service, in order
devd stack down                   # stop every running service
```

## TLS

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

## Databases

```bash
devd db list
devd db create blog
devd db drop blog                 # destructive: everything in it is deleted
```

## Git

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

## Interfaces

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

## Agents

devd has its own agents: an assistant for one-off questions and a team for whole
goals, both running through the providers you configure. See
[`docs/agents.md`](docs/agents.md).

Reviews, agent memory and instruction-file audits (`review.*`, `memory.*`, `compat.*`) are
capabilities too, reachable from `devd call`, the HTTP API, MCP and the window's **Studio**. See
[`docs/workflows.md`](docs/workflows.md).

## Project processes

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

## Shared project state

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

## Preview

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
runtimes/         PHP, Node.js, Go, Python and Ruby
services/         nginx and MariaDB
etc/              generated configuration and MariaDB data
certs/            the authority and per-site certificates
logs/             service logs
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
