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
devd runtime pin python@3.12     # this project (current folder) runs with Python 3.12.x; --clear removes it
devd runtime pins
devd runtime remove php@8.2       # destructive: asks for approval
```

## PHP, operations and deploys

```bash
devd php settings                         # limits, profile, workers
devd php configure --profile production   # 128M / 5M / 30 s, like a typical server
devd php configure --workers 8 --set memory_limit=512M
devd php extensions
devd php ext gd on
devd setup --services mariadb,redis --php 8.4 --profile production
devd call env.diff --arg site=shop --arg against_file=production.env
devd call cron.laravel --arg site=shop
devd call queue.add --arg site=shop --arg processes=2
devd call redirects.add --arg site=shop --arg from=/old --arg to=/new
devd call deploy.run --arg site=shop
```

Environment files, cron jobs, workers, redirects, logs, deploys and alerts are in [`operations.md`](operations.md).

## Services

```bash
devd service available nginx
devd service install nginx
devd service install mariadb --port 3306
devd service install apache       # Apache Lounge, checked against its published SHA-256 (Windows)
devd service find apache          # Apache or Redis folders already on this machine
devd service adopt apache 'C:\Apache24'   # use one as it is
devd service adopt redis 'C:\Redis' --port 6379
devd service install mailpit      # mail catcher; checked against the digest GitHub records
devd service install redis        # Windows: Redis 8 from redis-windows/redis-windows, checked the same way
devd service install memcached    # Windows: memcached 1.6.8 from jefyt/memcached-windows, checked the same way
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
devd site discover                # what is in the parked folders and which could be sites; changes nothing
devd site park D:\www            # look in this folder too; creates no site
devd site sync                    # make sites of the projects that have a web entry point
devd site sync blog shop          # only these
devd site serving blog static     # what a directory serves: php or static (omit to read it from the folder)
devd site parked
devd site unpark D:\www           # its sites stay
devd site fix-htaccess blog       # guard php_value lines so Apache accepts the .htaccess
devd site share blog --minutes 60 # public https link through a Cloudflare quick tunnel; closes by itself
devd site shares
devd site unshare blog
devd site remove blog             # destructive; keeps files unless --delete-files

devd hosts show                   # what a sync would change
devd hosts sync                   # admin: writes the managed block
devd hosts sync --elevate         # Windows: ask for permission with the usual prompt

devd web use apache               # or nginx; applies when devd next starts
devd web render                   # rebuild nginx.conf or httpd.conf from the site table
devd web test                     # nginx -t or httpd -t
devd web reload                   # nginx reloads; Apache restarts
devd web ports --http 8080 --https 8443  # settings.toml, config and a restart

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

Every Git command is one of 65 capabilities grouped under `git.repo`,
`git.changes`, `git.commits`, `git.history`, `git.branches`, `git.tags`,
`git.remotes`, `git.merge`, `git.stashes`, `git.worktrees`, `git.config` and
`git.recovery`. See [`git-cli.md`](git-cli.md) for the full mapping.

## Interfaces

```bash
devd ui            # the application window
devd serve         # the loopback API only, for scripts and tests
devd tui           # the terminal dashboard
devd mcp           # MCP on stdin/stdout, for agents

devd daemon start  # the resident process: scheduled work, the local API, DNS
devd daemon status
devd update check  # is there a newer, signed release
devd update apply

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
graphical toolkit to keep in step. See [`desktop.md`](desktop.md).

## Agents

devd has its own agents: an assistant for one-off questions and a team for whole
goals, both running through the providers you configure. See
[`agents.md`](agents.md).

```bash
devd chat                                      # the assistant, in the terminal
devd team start "add password reset" --project .
devd team list
devd team resume <run>                         # after a pause
```

Reviews, agent memory and instruction-file audits (`review.*`, `memory.*`, `compat.*`) are
capabilities too, reachable from `devd call`, the HTTP API, MCP and a project's **Reviews** and **Agent memory** tabs. See
[`workflows.md`](workflows.md).

### When a run needs you

A run that cannot continue says one thing. `team.get` returns `needs`: the task,
the evidence devd looked for and did not find, and the actions that resolve it.
The Inbox and the run view show the same object.

```bash
devd call team.get --arg run=<run>          # "needs": null when nothing is waiting
```

### Proposals

An agent can propose work instead of doing it; nothing runs until a person accepts.

```bash
devd call team.suggest  --arg project=. --arg title="Split the parser" --arg why="it is 900 lines and changed four times this month"
devd call team.suggestions                 # what devd would do, and what a person decided
devd call team.accept_suggestion --arg id=<id>    # starts an ordinary run; the plan still waits for you
devd call team.dismiss_suggestion --arg id=<id>
```

### Rules for starting work on its own

A project writes down which event should make devd think about starting a run.
A new rule starts in `observe`, which records a proposal and runs nothing.

```bash
devd call intake.set --arg project=. --arg name=nightly --arg trigger=check.failed \
  --arg goal="Make the nightly check pass again and add a regression test" \
  --arg 'condition={"min_failures":3,"cooldown_minutes":60}'
devd call intake.list --arg project=.
devd call intake.delete --arg project=. --arg name=nightly
```

The triggers are `health.down`, `check.failed` and `schedule.missed`. `mode` is
`observe` (the default) or `auto`, which starts a run that still waits for plan
approval and never starts one in production. Only a person writes a rule. See
[`agents.md`](agents.md).

### Runs

A conversation, a team run, a scheduled job and an API call write the same record:
steps, capability calls, evidence, approvals and a replayable timeline.

A run has no capability of its own. It is read over the loopback HTTP API, which
is what the window's Run sheet uses, and it can only be retried by a person:

```bash
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/api/runs
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/api/runs/<id>
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:8080/api/runs/<id>/events?after=0"
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/api/runs/<id>/changes
devd call run.retry    --arg run_id=<run>        # human only; a team run is resumed with team.resume
devd call approval.list --arg pending_only=true
```

See [`runs.md`](runs.md) for the lifecycle and the evidence rules.

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
* See [`devd-yml.md`](devd-yml.md).

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
* See [`shared-state.md`](shared-state.md).

## Terminal

**Terminal** is a tab on a project and runs a shell in that project's folder,
inside devd. The tab lists only the shells started in that folder, so a
terminal always belongs to the project on screen.

The shell is a real PTY, and it lives in the devd process rather than in the window. That has three consequences worth knowing:

* **It keeps running** while devd does. Closing the window, reloading the page or moving to another screen does not end it; the session is listed until you close it.
* **An agent can read it.** `terminal.read` returns the same screen the window shows. A person opens the shell; an agent may read a session it was given, and may not open one, because a shell is where anything can happen and an agent typing into one would be `project.exec` with no argument to review.
* **The sessions belong to the process that opened them.** A `devd call` in a separate process cannot reach them: each command is its own process.

| Capability | Who | What |
|---|---|---|
| `terminal.open` | a person | Start a shell in a folder. Returns the session id. |
| `terminal.read` | a person, an agent | The visible screen as text, whether it is still running, and its exit code once it has ended. |
| `terminal.write` | a person | Send exactly the characters given; a command runs when they end with a newline. |
| `terminal.resize` | a person | Columns and rows, so a program that draws a screen draws it at the right size. |
| `terminal.list` | a person, an agent | Every terminal this process has open, with its owner and folder. |
| `terminal.close` | a person, approved | End the shell and the processes it started. |

The folder passes the same fence the file tools use, so a shell cannot be opened somewhere an agent could not read.

### Why it answers the terminal's questions

A program in a terminal does not only write: it asks, and waits for the answer. Windows' console host asks "where is the cursor?" (`ESC[6n`) as soon as it starts, and a shell that gets no reply **blocks forever** - the screen stays empty and no command runs. devd therefore keeps the small part of a terminal emulator a shell needs (cursor position, device status, device attributes) and replies to it. This was found by building it: the first version hung on its first prompt.

The screen is kept as text with the escape sequences removed, so what a person and a model both read is the screen rather than the raw byte stream.

`portable-pty` (MIT, from wezterm) is the only way to get a terminal across Windows, Linux and macOS, and it is the workspace's one native dependency. devd still downloads and verifies every other binary it uses.
## Files

A project's **Files** tab lists its folders and opens its files in the window.

The tree asks devd for **one folder at a time** (`fs.tree`), so a large project costs one request per folder you open rather than one per row you can see. A file is opened with `fs.read`, so a person sees exactly what an agent would see: the same line-numbered text, the same size limit, the same refusals.

What the fence does in the tree, and why:

* a **secrets file** (`.env`, a key, `.npmrc`) is **listed and marked**, not hidden. A person should be able to see that their project has a `.env` and that devd will not open it, rather than wonder why it is missing. Choosing it gives the same refusal `fs.read` gives;
* a **dependency or build folder** (`node_modules`, `vendor`, `target`, `dist`) is listed and marked as skipped, so a folder's real contents are accounted for;
* `.git` is refused: it is managed through the Git capabilities, not browsed.

Folders sort before files, each by name without regard to case. `fs.tree` is `Read`, `agent_safe` and idempotent: it changes nothing.

Open files are syntax-highlighted for the languages devd's own stacks use (PHP, JavaScript and TypeScript, JSON, YAML, TOML, Rust, Python, Ruby, Go, SQL, HTML, CSS, shell and Markdown). Anything else is shown as plain text rather than guessed at. **Copy** puts the file's text on the clipboard without the line numbers, which are a reading aid and would break the file if pasted back.

There is no editing here: an edit is a `Write` capability with an undo snapshot, and a person editing a file is better served by their own editor.

## Preview

**Preview** embeds a page in the window. On a site's page choose the **Preview** tab; the site devd serves is loaded, and you can reload it or open it in the system browser. The certificate authority has to be trusted for an HTTPS site to render without a warning, and the page says so when it is not.

**Any address on this machine**, not only a site devd serves: type it in the address bar above the frame. A `php artisan serve` on 8000, a `next dev` on 3000 or a static file server all preview the same way. The address is checked by the server (`POST /api/preview`) before it reaches the frame, because a frame is a place a page can run script:

* http and https only;
* a host on this machine: `127.0.0.1`, `::1`, `localhost` or a `*.localhost` name. Another machine on the network and a public address are both refused, because a preview shows work happening here;
* no credentials in the address, and the fragment is dropped, since it never reaches the server.

The rule lives in `devd_core::preview` so the window does not carry a second copy of it.

## Reading the web

`docs.fetch` reads a public https page and returns its text. `web.open` reads the same page and returns its **structure**: the title, the headings, the links with their text, and the form controls with their name, type and label, alongside the readable text.

```bash
devd call web.open  --arg url=https://example.com
devd call docs.fetch --arg url=https://example.com
```

Both refuse private, loopback and metadata addresses, and re-check every redirect, so a public host that redirects to `169.254.169.254` is refused at the redirect rather than at the first request. Neither sends credentials, and both say that a page's content is data rather than an instruction.

There is **no browser automation**: nothing clicks, types or renders a page. An agent that needs to know what a form contains reads it with `web.open`; one that needs to know a page renders is asking a question a fetcher cannot answer.

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
| `primary_php` | newest installed | The default PHP: what a site follows until someone pins a version for it with `site.set_php`, and what `php` runs outside any site. |
| `php_workers` | `4` | PHP processes per version (1 to 16); nginx spreads requests across them. |
| `php_profile` | `local` | `local` (generous limits) or `production` (128M memory, 5M uploads, 30 s). |
| `git_executable` | `git` on `PATH` | Git binary to drive. |
| `git_protect_uncommitted` | `true` | Refuse branch switches that would overwrite local work. |
| `git_default_branch` | `main` | Name given to a new repository's first branch. |

Change `http_port` or `https_port` with `devd web ports`, not by editing the
file: it rewrites the generated configuration, moves the web server's recorded
port with it and restarts a running server. The other settings take effect the
next time devd starts.

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
| A destructive action cannot run without approval | `Tier::Destructive` and `Tier::Remote` are checked in `Registry::dispatch`, before the handler. The built-in harness may pre-authorise `fs.delete` only inside the conversation's project; the root, `.git`, secrets and outside paths still need a person |
| An agent cannot approve its own action | `Actor::is_agent()` fails the approval check unconditionally; there is no agent-supplied approval path. The in-project delete policy is the harness's decision, made before dispatch |
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
protections, see [`security.md`](security.md).
