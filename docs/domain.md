# The domain: projects, sites, runtimes, services, and who owns what

This is the reference for what devd's words mean and which part of the code is allowed to decide each
thing. When a screen, a command and an agent disagree, this page says which of them is wrong.

## The four nouns

| Noun | What it is | Owner | Stored? |
| --- | --- | --- |
| **Project** | A codebase: a folder somebody is building. A Laravel app, a Go service, a Node app, a library, a monorepo. It may have no website at all. | `project.list`; the projects table and indexed repositories | Yes, when registered. Its identity is its folder. |
| **Site** | A hostname, a certificate and a way of answering requests, pointed at a project's folder or at a running program. | `devd-sites`; the `sites` table | Yes. Its identity is its name (unique) and id. |
| **Runtime** | An interpreter devd installs or finds: PHP, Node, Go, Python, Ruby. | `devd-runtime`; the `runtimes` table | Yes (installed versions), plus pins. |
| **Service** | A process devd supervises: web server, database, cache, mail catcher, a PHP FastCGI pool. | `devd-services`; the `services` table | Desired state is stored. Whether it is running is **observed**, never stored. |

A project has **zero, one or several** sites (`Site::project_dir()` is the one definition of which project a
site belongs to: its folder, or the folder above a `public/`). A site can exist for a project devd has not
registered; `site.list` says which (`project_known`). `project.list` lists the sites that serve each project.
Removing a site never removes a project, and removing a project from devd never touches its files.

There is no "workspace" noun in the product. The word in the code (`devd-workspace`) is the isolated Git
worktree an agent works in.

## What a site serves

A directory is not a web site. What a site is *served as* is one answer, computed in one place
(`devd-core::serving::effective`) and used by both web-server renderers and every screen:

| Serving | When | What the web server does |
| --- | --- | --- |
| `proxy` | the site has an `upstream` | forwards every request to a running program; no PHP, no directory |
| `php` | the directory has an `index.php` (in `public/` if that holds the entry, else the folder) or PHP files at its web root, or a person chose `php` | PHP through FastCGI |
| `static` | an `index.html`/`index.htm` and no PHP entry, or a person chose `static` | files as they are; **no PHP in the configuration** |
| `no_entry` | nothing to serve | **no document root**; every request answers with an explanation of what the directory is and what to do. The folder is never exposed. |

Precedence: an upstream, then the person's explicit choice (`site.set_serving`), then what the files say.
Detection is evidence, never stored, and re-read whenever the configuration is rendered, so it stays true as a
project changes. A Laravel project also has a `package.json`; both are reported as evidence, neither hides the other.

A **PHP version** means something only for a site that serves PHP. It is either *pinned* (a person ran
`site.set_php`) or *inherited* (the site follows the default, `primary_php`). **Creating a site never pins**
the default into it; earlier versions did, which froze whatever the default was that day into every site made
then. A pin on a site that does not serve PHP is reported as `php_unused_pin`, shown as unused, and can be cleared.

### Which PHP runs, where

| Context | PHP |
| --- | --- |
| A request to a site | its pin, else the default |
| `php`/`composer` in a terminal, inside a site's project folder | that site's PHP (same rule) |
| ... inside a folder with a `devd runtime pin` | the pin |
| ... anywhere else | the default |
| A cron job, worker or deploy command for a site | that site's PHP, first on `PATH` |

The default is `primary_php`, else the newest installed. `site.list` reports `php_effective` and
`php_source` (`pinned`/`inherited`); the web server's upstream is chosen by the same function, so the screen and
the process cannot disagree.

## Discovery, registration, configuration, activation

These are four steps with different consequences, and no step performs the next one.

| Step | Capability | Changes | Approval |
| --- | --- | --- | --- |
| Discovery | `site.discover` | nothing | none |
| Folder to look in | `site.park` | `settings.toml` only; **creates no site** | human only |
| Registration | `site.sync [names]` | a site record and a certificate per eligible project; the web server is re-rendered once | its plan lists exactly which |
| Configuration | `site.set_serving`, `site.set_php`, `site.set_upstream` | one site | none (reversible) |
| Activation | `hosts.sync`, `stack up` | the hosts file; running processes | admin / exec |

A project is **eligible** for registration only if it has something a web server can serve. A Go module, a
Node app without an `index.html`, a PHP library and an empty folder are projects and stay projects:
discovery reports them with the reason, and `site.sync` skips them. A program that listens on a port is a
**proxy** site (`site.create --upstream`), which is how a Node or Go service is served. Repeated discovery gives
the same answer and writes nothing; two folders that slug to the same name are a conflict, not two sites.

`site.create` writes a starter page only into a directory that is empty. An existing project is never given
files it did not have.

## State and configuration: who owns what

| State | Owner | Notes |
| --- | --- | --- |
| Settings (`settings.toml`) | the file | read at start; written only by capabilities |
| Records (sites, runtimes, pins, jobs, audit) | `state.db`, migrated by `Store::open` | migrations are additive, so an older devd opens a newer database |
| Generated configuration (`nginx.conf`, `httpd.conf`, `php.ini`) | **derived** from the records | never edited by hand; rendered from a site's effective serving |
| Whether a service is up, a port is held, a job is due | **observed** | checked when asked, not stored as truth |

### Which home

`DEVD_HOME` selects the data folder: the process value, else `%LOCALAPPDATA%\devd` (Linux: `$XDG_DATA_HOME/devd`).
A process started before the variable was changed keeps its old copy, so one machine can have several homes, each
with its own database. `devd doctor` reports it: `home.conflict` (this process uses X but your account's
`DEVD_HOME` is Y) and `home.other` (another home with data exists and is not in use). It only reports. Nothing
is picked, migrated, moved or deleted for you.

### Looking never writes

`doctor`, `devd shim` (the `php`/`composer` launchers) and `health.status` open the database read-only
(`Store::open_read_only`): they do not create the folder or the file, switch the journal mode or migrate the
schema. A database from before this build's newest migration is reported as waiting for devd to start;
`state` reports the file's own version. When a process has the database open, `health.status` and the window's
Diagnostics check that one rather than re-opening the file.

## Lifecycle: who runs the background work

| Responsibility | Runs in | Survives closing the window? |
| --- | --- | --- |
| Web server, databases, PHP pools, mail | their own processes, started by `stack up` | **Yes** |
| Cron jobs, scheduled database backups, scheduled deploys, health checks | a loop inside the first long-lived devd process: the window, `devd ui`, or `devd serve` (`ops-host.pid` claims it) | **No** |
| Agent conversations and team runs | the same kind of process (`agent-host.pid`) | No |

So services outlive the window and scheduled work does not. This is deliberate (no resident privileged
service) and used to be invisible. Diagnostics now has a **Scheduled jobs** component: healthy when a devd
process holds the claim or nothing is scheduled; degraded, and saying so, when jobs are scheduled and nothing
is running them. To keep scheduled work running without a window, leave `devd serve` running. A hidden
`devd serve` started at sign-in is the missing piece and is recorded in `limitations.md`.

## Authority

Every interface dispatches through `Registry::dispatch`; the checks live there, in this order:

1. a declined call never runs;
2. `human_only` cannot be performed by an agent, and no approval an agent holds unlocks it;
3. a capability not marked `agent_safe` is not available to agents;
4. in production mode an agent needs approval for every change;
5. `Destructive`, `Remote` and `Admin` tiers need a person's approval;
6. **`agent_needs_approval`**: an agent that supplies the code devd will run waits for a person to read it.

The Exec tier means "starts something devd already manages" (start a service, restart a pool) and needs no
approval for anyone. It does **not** cover a caller who writes the code, and before this change several capabilities
that do (`cron.add`, `queue.add`, `deploy.script_set`, `deploy.vars_set`, `laravel.artisan`, `ploi.pull_script`)
were Exec and agent-callable, so an agent could have devd run a command line it wrote, on a schedule, with no
approval. They now carry `agent_needs_approval`, and support dry runs so the approver sees the code in the plan.
A person at the command line or in the window is not asked: they wrote it.

A test (`devd-bootstrap/tests/authority.rs`) walks the whole shipped registry and fails when an agent-callable,
ungated capability takes a `command`, `script`, `steps`, `argv`, `program` or `directives` argument and is not on
the reviewed list, with the reason each is reviewed.

**Open decision.** `project.exec` and `project.run` stay open to agents: they run the stack's allow-listed
programs and the project's own tasks, with no shell, and they are the core of an agent's work loop. The
allow-list includes interpreters (`php`, `node`, `python`), which can execute code given on the command line.
Gating them is a product decision about the agent loop, not a defect fix, and is recorded in `limitations.md`.
