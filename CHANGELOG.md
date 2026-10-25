# Changelog

All notable changes to `devd` are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Because `devd` is pre-1.0, each `0.x` release is a milestone, and a milestone
may change capability identifiers where the design was wrong.

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
  `deploy.script_set`, `deploy.vars_set`, `laravel.artisan` and `ploi.pull_script` take a command line or
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
- **Server -> Remote** holds remote servers, SSH keys and Ploi. Deploy links and remote servers are
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

- **Operations**, what a server panel such as Ploi shows for a site, as capabilities and a **Server → Operations** screen: the
  `.env` file with secrets hidden and a diff against another environment (`env.*`), cron jobs and the Laravel scheduler
  (`cron.*`), queue workers (`queue.*`), redirects written into nginx and Apache (`redirects.*`), the Laravel log as entries
  (`logs.*`), a deploy script per site with history, rollback, a repository link and a push-to-deploy hook (`deploy.*`,
  `repo.*`), alerts to the inbox, Slack, Discord or a webhook (`notify.*`), and Ploi's own API (`ploi.*`: servers, sites,
  environment, deploy). See [`docs/operations.md`](docs/operations.md).
- **PHP workers**: a pool of PHP processes per version (`php_workers`, default 4) behind an nginx upstream, so a page that makes
  many requests no longer queues behind itself.
- **PHP settings** (`php.settings`, `php.configure`, `php.extensions`, `php.extension_set`): a `local` or `production` limits
  profile (128M / 5M / 30 s, like a Ploi server), single values, and an extension manager.
- **Redis and memcached are downloaded** on Windows and checked against the SHA-256 GitHub records, like Mailpit.
- **`devd setup --services`, `--php`, `--profile`** choose what a first run installs.
- `php` command group; `devd call` reaches every new capability.
- **A PHP limits profile per site** (`php.site_profile`): a site can run with the other profile, served by a second pool of PHP
  processes started with that profile's ini.
- **Deploy warnings** (`deploy.check`, `ploi.limits_set`): every PHP limit that is looser here than on your server, in the
  deploy plan and the deployment's log. Ploi's API does not expose a server's limits, so devd is told them.
- **Apache balances PHP workers** through `mod_proxy_balancer` when the build has its modules.
- **Set up devd** on the Overview screen: a first run shows what it will install, with sizes, and lets you choose.
- **Deploy to any server over SSH** (`server.*`, `remote.*`): register DigitalOcean, Hetzner, Vultr, Ploi or Forge machines; devd pins
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

See the line below; the numbers are from the run recorded in
[`docs/releases/v0.5.0.md`](docs/releases/v0.5.0.md). The suite covers the
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
