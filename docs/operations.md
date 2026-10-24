# Operations: what a server panel shows, for the machine you develop on

A server panel such as Ploi gives each site an environment editor, cron jobs, queue workers, redirects, logs, deploys and alerts, and gives the server its PHP settings and services. devd has the same things for a local site, so a project behaves here the way it does on the server, and an agent can look after all of it through the same capabilities.

Open **Server > Sites**, choose **Manage** on a site, and use its tabs. The whole-machine screens are **Server > Databases** (users and backups), **Server > Remote** (servers, SSH keys and Ploi) and **Server > Runtimes** (PHP settings). Everything on these screens is a capability, so it is also `devd call <id>` and an MCP tool, with the same approvals. The old **Server > Operations** screen is still reachable by its route for one release but is no longer in the strip.

| Where | What it does | Capabilities |
| --- | --- | --- |
| Site: Environment | The site's `.env`, secrets hidden; change or remove a variable; compare with another environment | `env.get`, `env.reveal`, `env.set`, `env.unset`, `env.diff` |
| Site: Cron | Cron jobs, with the Laravel scheduler as one click | `cron.add`, `cron.laravel`, `cron.list`, `cron.run`, `cron.log`, `cron.enable`, `cron.remove` |
| Site: Queue | `php artisan queue:work` (or any program), supervised and restarted | `queue.add`, `queue.list`, `queue.start`, `queue.stop`, `queue.restart`, `queue.logs`, `queue.remove` |
| Site: Redirects | A path to another path or an https address, written into the web server | `redirects.add`, `redirects.list`, `redirects.remove` |
| Site: Logs | The Laravel log as entries; web server and PHP logs | `logs.laravel`, `logs.site`, `logs.laravel_clear` |
| Site: Deploy | A script per site, history, rollback, repository link, push-to-deploy | `deploy.script`, `deploy.script_set`, `deploy.run`, `deploy.history`, `deploy.status`, `deploy.log`, `deploy.check`, `deploy.rollback`, `repo.status`, `repo.link`, `deploy.hook` |
| Site: Laravel | Maintenance mode, storage link, caches and Artisan commands | `laravel.status`, `laravel.artisan`, `laravel.down`, `laravel.up`, `laravel.storage_link`, `laravel.clear` |
| Site: Web server | Per-site nginx directives, validated before they are applied | `nginx.site_set`, `nginx.site_show`, `nginx.site_clear` |
| Site: Database | The database this site uses and whether a backup is scheduled; read-only | `backup.list` (the full users and backup screens are under Server > Databases) |
| Site: Preview | Embed the running site, reload it, or open it in the system browser | none (the site's own URL) |
| Server > Databases | Database users, scheduled dumps and files for the whole machine | `db.users`, `db.user_create`, `db.user_drop`, `backup.add`, `backup.run`, `backup.list`, `backup.files`, `backup.remove`, `db.restore` |
| Server > Remote | Remote servers, SSH keys and Ploi | `server.*`, `ssh.*`, `ploi.*` |
| Server > Runtimes | PHP limits profile, worker pool and extensions; site defaults and runtime versions | `php.settings`, `php.configure`, `php.extensions`, `php.extension_set`, `php.pool`, `runtime.*` |
| Server > Overview | Machine CPU, memory and disks; services, sites and pending hostnames or certificates | `monitor.stats`, `service.list`, `site.list`, `hosts.show`, `tls.list` |
| Server > Diagnostics | Alerts, health checks and the machine's state | `notify.add`, `notify.list`, `notify.test`, `notify.remove`, `health.*`, `monitor.stats` |
| Site: Overview | The site's PHP version and per-site limits profile | `php.site_profile`, `site.set_php` |

## Who may do what

Reads are free. Changing a project's own files or devd's records is `write`. Anything that **runs a command** (a cron job, a worker, a deploy script, saving a deploy script) is `exec`, so an agent can propose it and only you approve it. Restoring an earlier commit and emptying logs are `destructive`. Anything that changes the Ploi server is `remote`.

Secrets get extra care:

* `env.get` never shows a secret: any name containing `PASSWORD`, `SECRET`, `TOKEN`, `KEY` and similar is shown as hidden with its length. `env.reveal` shows them and is for a person only; an agent never reads secrets.
* The audit trail records the names of the variables `env.set` changes, and hides the values of secret-looking ones.
* The Ploi API token is kept in a file only you can read, never in the database, settings or audit trail. `ploi.connect` is for a person only. Give the token only the scopes you want agents to be able to use; Ploi enforces them.

## The `.env` file

Changes keep every comment, the order, and every line they do not touch. The previous file stays beside it as `.env.devd-backup-<time>`; the last five are kept. A running app may cache its configuration (`php artisan config:clear`).

`env.diff` compares with a file (a copy of the production `.env`) or with the `.env` of the linked Ploi site. It lists what is only here, only there, and different; secrets are reported as differing without their values. It warns about `APP_DEBUG` on in production, a local `APP_URL` there, and a missing `APP_KEY`.

## Cron jobs

A job is a five-field cron expression (`*/5 * * * *`, `0 3 * * 1`, `@daily`) and one command, run in the site's folder with the site's PHP first on `PATH`, so `php artisan schedule:run` uses the PHP the site is served with. `composer` works too.

Windows has no cron and a laptop sleeps, so devd runs the jobs itself while it is open. A job that was due while devd was closed is skipped, not run late in a burst. One devd process runs the jobs: the first long-lived one (the window, `devd ui`, `devd serve`). Output goes to `logs/cron-<site>-<id>.log`; a failing job tells the alert targets.

## Queue workers

A worker is a devd service named `worker-<site>-<name>`: started with the rest, restarted when it exits, with its output in `logs/`. For a Laravel site it is `php artisan queue:work` with `--tries`, `--sleep` and `--max-time` (the worker restarts hourly by default so it picks up new code). Give a `command` to supervise any other program. Restart workers after a deploy.

## Redirects

`/old` redirects that path. `/blog/*` redirects everything under it, and `$1` in the target is what the star matched: `/blog/*` to `/news/$1` sends `/blog/a/b` to `/news/a/b`. Targets are a path on the site or an `https://` address. The redirect is written into the web server configuration (nginx and Apache), which is reloaded.

## Deploys

The script is one command per line, run in the site's folder, stopping at the first failure. `{BRANCH}` and `{SITE_DIRECTORY}` are filled in. Lines that only make sense on a server (`cd` into the site folder, `sudo`, reloading PHP-FPM) are skipped and the log says so, so a script copied from Ploi runs here unchanged. With no saved script devd uses one chosen from the project: pull, `composer install`, `migrate --force`, `npm ci` and `npm run build`, `optimize`.

`deploy.run` returns at once with a deployment; `deploy.status` and `deploy.log` follow it. It records the commit before and after and tells the alert targets when it ends. `deploy.rollback` checks out the commit a deployment ended on (or the commit before the newest deployment) and runs the script without pulling; it refuses while there are uncommitted changes and leaves a detached `HEAD` (`git switch <branch>` goes back).

### Push to deploy

`repo.link` records the branch a site follows. `deploy.hook` turns on a secret address on the devd server, `POST /hooks/deploy/<site>/<secret>`, that deploys the site. Turning it on makes a new secret; the old one stops working. The address is the only authorisation, so treat it like a password. To let GitHub call it, open a public link to the devd server and give GitHub that address as a webhook. Every refusal is the same `404`.

## PHP

`php.configure` sets a **profile** for every installed PHP:

| | `local` | `production` |
| --- | --- | --- |
| `memory_limit` | 2G | 128M |
| `upload_max_filesize` | 2G | 5M |
| `post_max_size` | 2G | 8M |
| `max_execution_time` | 36000 | 30 |

`production` is what a typical Ploi server runs, so a limit that would bite there bites here first. Single values (`memory_limit`, `upload_max_filesize`, `post_max_size`, `max_execution_time`, `max_input_time`, `date.timezone`, `display_errors`, `opcache.enable`) can be set on top. The values live in one marked block at the end of `php.ini`, which wins over the rest and is replaced, not stacked.

**Workers.** `php-cgi` answers one request at a time, so a page that makes many requests at once would queue behind itself. devd runs a pool of PHP processes per version (4 by default, 1 to 16 with `php_workers` or `php.configure --workers`) and nginx spreads requests across them. Worker 1 is on the version's own port (9084 for 8.4) and the others are 100 apart. Apache uses a single process per version.

With the Apache driver the workers are balanced through `mod_proxy_balancer` when its modules are in the Apache build; without them Apache uses a single process per version.

**A profile for one site.** `php.site_profile` serves a site with the other profile whatever the machine's is, so you can try a site against your server's limits while everything else stays generous. It is set on the site's **Overview** tab; the machine-wide profile and extension list are under **Server > Runtimes**. A site whose profile differs from the machine's is served by a second pool of PHP processes started with the other profile's ini (`php.<profile>.ini` beside `php.ini`), 50 ports above the main pool; the pool exists only while some site needs it.

**Your server's limits.** Ploi's API does not expose a server's PHP limits, so tell devd what yours has (read them from the PHP tab of your server in Ploi) with `ploi.limits_set`. `deploy.check` then lists every limit that is looser here than there, and `deploy.run` shows the same notes in its plan and in the deployment's log. Until you do, devd assumes a default Ploi server (the `production` profile) and says it is assuming.

**Extensions.** `php.extensions` lists what the build ships and which are on; `php.extension_set` switches one.

## Services

`devd service install redis` and `devd service install memcached` download Windows builds and check them against the SHA-256 GitHub records for each file (Redis 8 from `redis-windows/redis-windows`, memcached 1.6.8 from `jefyt/memcached-windows`). Neither project is the vendor, so devd says where the build comes from. MariaDB is the MySQL-compatible server; devd does not download MySQL itself because MySQL publishes no SHA-256 for its Windows archives.

On a new machine **Server → Overview → Set up devd** shows what a first run installs, with what each thing is and its size before anything downloads: PHP and nginx always, then MariaDB, Redis, memcached and Mailpit as you choose, the PHP series, and the limits profile (generous or like a typical server). Each step is a capability, so each is audited and asks for approval where it needs it, and something already installed is skipped.

`devd setup --services mariadb,redis,memcached,mailpit` (or `--services none`) chooses what a first run installs besides PHP and the web server; `--php 8.4` and `--profile production` set the PHP series and the limits profile.

## The Laravel tab

`laravel.status` reads the site's state without running PHP: maintenance mode (and its bypass path), whether `public/storage` is linked, `APP_ENV` and `APP_DEBUG` (it flags debug on in production), and whether configuration and routes are cached. `laravel.artisan` runs `php artisan <command>` with the site's own PHP in its folder, as an argument list and never through a shell (so `;` and `|` are plain text), and returns the exit code and output. `laravel.down` and `laravel.up` switch maintenance mode (with an optional bypass secret), `laravel.storage_link` links storage, and `laravel.clear` clears views, configuration, routes, events, or all four; the application cache is cleared only when named, because it empties whatever the cache driver holds.

## Databases

`db.users`, `db.user_create` and `db.user_drop` manage the users of devd's MariaDB. A user can be given every privilege on one database; a password is generated when none is given, returned once and never stored. `backup.add` dumps a database on a cron schedule while devd is open, into `<devd home>/backups/<database>/`, keeps the newest `keep` (default 7) and removes only files it named itself; a failed backup tells the alert targets (`backup.failed`). `backup.run`, `backup.list`, `backup.files` and `backup.remove` complete it. Restore with `db.restore` and a file's path.

## Per-site nginx configuration

`nginx.site_set` writes directives and presets (`upload-100m`, `gzip`, `cache-static`, `cors`, `no-index`, `security-headers`, `long-requests`) into a site's `server` block, checks the whole configuration with `nginx -t`, reloads nginx, and, if nginx rejects them, puts the previous directives back. `listen`, `server_name`, certificates, `include` and anything that runs code inside nginx are refused. `nginx.site_show` shows yours next to the block devd generated; `nginx.site_clear` removes them. With the Apache driver they are saved and not used.

## Deploy variables, scheduled deploys, staging

Variables (`deploy.vars_set`, `deploy.vars`) are `{NAME}` in the deploy script and environment variables to every command in it; secret-looking values are masked when shown. `deploy.schedule` runs a deploy at a time (`in 2h`, or an RFC 3339 time) while devd is open; one due while devd was closed is marked missed and the alert targets are told, never run at a time nobody chose. `deploy.staging_create` makes a second working folder of the same Git repository (a worktree) on its own branch, served as `<site>-staging` with its own certificate, the production `.env` with `APP_ENV=staging` and its own `APP_URL`, and a deploy script without the pull. `deploy.staging_promote` merges that branch into the production site's branch (fast-forward only) and deploys; `deploy.staging_remove` removes the site and its folder (refused while tracked files have uncommitted changes).

## Health and the machine

`health.add` asks a site for its front page every minute while devd is open (devd's own web server by default, without following the redirect to HTTPS; or a URL you give, such as your public address) and records the status and time. Below 500 is up. The alert targets hear when a site goes down and when it is back (`health.down`, `health.up`). `monitor.stats` returns the machine's CPU, memory, disks and uptime, and how many of devd's services are running.

## SSH keys

`ssh.keys` lists the keys in `~/.ssh` (never the private halves), `ssh.public` shows a public key, `ssh.keygen` makes an ed25519 key, and `ssh.adopt` copies a private key you already have into `~/.ssh` under a name, locks its permissions and writes the public half (a person only). The original is left where it is.

## Servers: deploy to any machine you can SSH into

A server is a name, an address, a user, a port and a key from `~/.ssh`; the provider (`digitalocean`, `hetzner`, `vultr`, `linode`, `aws`, `ploi`, `forge`, `other`) is only a label. devd reaches it with the system's OpenSSH client, one non-interactive command at a time, and **pins the host key in its own known-hosts file** (`<devd home>/secrets/ssh/known_hosts`), never yours: the first `server.connect` records it and returns the fingerprint to compare with your provider's console, and a later connection to a server whose key changed is refused.

| Capability | What it does |
| --- | --- |
| `server.add`, `server.list`, `server.remove` | The registry. Nothing is contacted until `server.connect`. |
| `server.connect`, `server.info`, `server.run` | Pin and test a connection; read OS, load, memory, disk, what is installed and whether sudo is passwordless; run one command. |
| `server.provision` | Set a fresh Ubuntu or Debian server up for PHP sites: nginx, PHP-FPM, Composer (checked against its published signature), Node, MariaDB or MySQL, optionally Redis, supervisor, certbot, and the firewall with SSH, HTTP and HTTPS open. The plan shows the whole script. |
| `remote.link`, `remote.unlink`, `remote.links` | Where a local site is deployed: server, folder, mode, branch, domain, build commands, steps. A site can be linked to several servers. |
| `remote.deploy`, `remote.releases`, `remote.rollback` | Deploy, list the releases, go back. |
| `remote.env_push`, `remote.site_create`, `remote.ssl`, `remote.steps_set` | Send the `.env`, write the nginx block, get a Let's Encrypt certificate, change what runs in a release. |

Two modes, chosen per link:

* **`releases`** (the default). devd runs the local build (`npm ci`, `npm run build` for a project that builds), packages the project, uploads it over SSH, unpacks it as `releases/<time>`, links the shared `.env` and `storage`, runs the steps on the server (`composer install --no-dev`, `migrate --force` and the caches for a Laravel project), and only then points `current` at the new release with **one atomic rename** and reloads PHP-FPM. A failure before that leaves the running site untouched. Rollback points `current` back at an earlier release (they stay on disk; the newest `keep` are kept), so it is instant; migrations are not undone. The archive never contains `.env`, `.git`, `vendor`, `node_modules` or logs. The web root on the server is `current/<web dir>`.
* **`git`**, for machines a panel such as Ploi or Forge manages. The server pulls the branch and runs the site's deploy script exactly as written, `cd`, `sudo` and PHP-FPM reloads included.

A deploy to a server is recorded in the same history as a local one (`deploy.history`, `deploy.status`, `deploy.log`), carries the same limits warnings, and tells the same alert targets. Every capability that touches a server is `remote` tier, so an agent can ask and only you approve.

**What has and has not been run.** The generated scripts, the archive contents, the SSH arguments and the Git-based staging flow are covered by tests that run everywhere. The whole `releases` flow (ship, switch, prune, rollback) is covered by an end-to-end test that drives the real SSH layer through a stand-in `ssh` command, and it runs on Linux only (the script uses GNU `mv -T` and symlinks). It had not been run against a real server when this was written: connect one, run `server.info`, and read the plan of `server.provision` before approving it.
