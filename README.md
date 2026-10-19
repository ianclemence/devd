# devd

A local development environment manager, written in Rust. It installs and runs
the services a PHP or Node project needs — nginx, MariaDB, PHP, Node.js, Go —
creates virtual hosts served over HTTPS under a local domain, and exposes every
operation as a typed, permissioned capability reachable from the CLI, a terminal
dashboard, a web interface, an HTTP API and an MCP server for agents.

devd replaces Windows-only stacks such as Laragon and XAMPP. It is built on two
rules:

- **One implementation.** Every action is declared once, in a capability
  registry. The interfaces are projections of that registry; none of them holds
  business logic of its own.
- **Approval is enforced, not suggested.** Destructive, privileged and remote
  operations return a plan and require an explicit human decision before they
  run. An agent cannot approve its own action, and a declined operation never
  executes — from any interface.

## Requirements

- **Windows 10/11 x64** or **Linux x64**.
- The prebuilt Windows binary needs nothing else.
- Building from source needs Rust 1.99 or newer and a C compiler (`cc`) for the
  bundled SQLite.
- Git must be on `PATH` (or named in `git_executable`) for the Git capabilities.

Service management is available on Windows. On Linux the core, Git, the CLI,
the dashboard, the web application, MCP, and the Node.js and Go runtimes work;
nginx and MariaDB installation is not available yet because devd manages the
vendors' Windows archives. See [`docs/limitations.md`](docs/limitations.md).

## Install

### Windows

Download the binary and its checksum from the
[latest release](https://github.com/ianclemence/devd/releases/latest):

```powershell
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.5.0/devd-v0.5.0-x86_64-pc-windows-gnu.exe
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.5.0/SHA256SUMS.txt
certutil -hashfile devd-v0.5.0-x86_64-pc-windows-gnu.exe SHA256
```

Compare the result with the line in `SHA256SUMS.txt`, rename the file to
`devd.exe`, put it on `PATH`, and run:

```powershell
devd doctor
```

The binary is self-contained: it links only Windows system libraries, so it
needs no Rust toolchain, MinGW or Visual C++ runtime. It is not code-signed, so
SmartScreen warns once on first run; that is why the checksum is published.

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

## Quick start

```bash
devd runtime install php          # newest stable PHP, digest-verified
devd service install nginx
devd service install mariadb
devd stack up                     # php-cgi, then MariaDB, then nginx

devd site create blog             # directory, starter page, vhost, certificate
devd hosts sync -y                # publish blog.test in the hosts file
devd tls trust -y                 # add devd's CA to the OS trust store
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
devd ui            # the web interface, opened as a window
devd serve         # the HTTP API only, on loopback
devd tui           # the terminal dashboard
devd mcp           # MCP on stdin/stdout, for agents

devd capabilities  # the whole registry
devd describe git.reset
devd doctor
devd audit -n 20
```

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

| Document | Contents |
| --- | --- |
| [`docs/architecture.md`](docs/architecture.md) | The registry, the dispatcher, the state store, and the skill crates. |
| [`docs/security.md`](docs/security.md) | The threat model and the specific protections. |
| [`docs/app.md`](docs/app.md) | The web interface, the loopback API and the approval flow. |
| [`docs/tui.md`](docs/tui.md) | The terminal dashboard. |
| [`docs/mcp.md`](docs/mcp.md) | The agent surface and its five discovery tools. |
| [`docs/git-cli.md`](docs/git-cli.md) | Every Git command and the capability it calls. |
| [`docs/limitations.md`](docs/limitations.md) | What is not implemented, and what is weaker than intended. |
| [`docs/releasing.md`](docs/releasing.md) | How a release is built, verified and published. |
| [`CHANGELOG.md`](CHANGELOG.md) | Release history. |

## License

MIT. See [`LICENSE`](LICENSE).

devd bundles no third-party binaries. PHP, Node.js, Go, nginx and MariaDB are
downloaded from their vendors on first use and verified against the digest the
vendor publishes; where a vendor publishes no digest, the install says so.
