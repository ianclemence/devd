# devd

> **Your development environment, on your machine, with your rules.**
> *One program for your sites, your services, your runtimes and your agents — and nothing it does happens without your say-so.*

---

## What devd does for you

devd is the local development server you would otherwise assemble by hand: a web server, a database, a few runtimes, a hosts file and a pile of terminal tabs. It knows what is installed, what is running and what each project needs, and it shows all of it in one place.

- **It finds your project.** Browse to the folder with your operating system's own dialog. devd reads what is inside — the stack, the package manager, the runtimes it asks for, the services it talks to — and sets it up from there.
- **It gives every project a name.** `blog.test`, over HTTPS, with a certificate from devd's own local authority. A Laravel app, a Next.js site, a Django API and a Rails app all sit in the same list.
- **It serves your dev servers too.** Point a hostname at a program that is already running — `devd site create api --upstream 3000` — and a Node, Python or Rails dev server gets the same `.test` name and HTTPS.
- **It runs the services.** PHP, nginx and MariaDB are installed, started, health-checked and kept running. A service is "Ready" only after it has answered, not because its port is open.
- **It sees what you already have.** MySQL, Redis, PostgreSQL and Docker are detected, never assumed healthy and never replaced. You decide whether devd uses them, watches them or leaves them alone.
- **It has its own agents.** An assistant, and a team of fifteen specialists (planner, product, designer, architect, engineers, QA, security, infrastructure, docs). Give the team a goal and a definition of done: a planner proposes the work, you approve it, and specialists do it in isolated copies of the project until the evidence shows it is done.
- **It keeps the record.** What was asked, what ran, what it changed and who approved it.

You say what you want. devd does the rest, and it asks before it does anything consequential.

---

## Two rules

Everything in devd follows from these, and they are why it can be trusted with your machine.

- **One implementation.** Every action is declared once, in a capability registry. The command line, the terminal dashboard, the application window and the MCP server are views of that registry. None of them holds business logic of its own, so they cannot disagree about what is possible.
- **Approval is enforced, not suggested.** Destructive, privileged and remote operations return a plan and wait for an explicit human decision. An agent cannot approve its own action, and a declined operation never runs, from any interface.

```
you or an agent → capability registry → plan, tier, approval, audit → the work
```

Machine-level changes are always their own approved step. Creating a site never edits your hosts file; `devd hosts sync` does, and asks first.

---

## Projects for any stack

devd creates, imports and runs projects across languages, driven by structured stack definitions with links to each framework's official documentation: Next.js, Nuxt, Astro, SvelteKit, Vite, Vue, Express, Laravel, Slim, Django, Flask, FastAPI, Rails, Go, Rust, Spring Boot, ASP.NET, Expo and React Native.

A project can declare its dev processes in `devd.yml`. devd shows the exact commands and runs nothing until you approve that exact file; edit it and the approval is withdrawn. See [`docs/devd-yml.md`](docs/devd-yml.md) and [`docs/stacks.md`](docs/stacks.md).

## An agent team that works and proves it

Describe what you want built (`devd team start "…" --project .`, or Assistant → Team runs in the window). A planner proposes tasks; you approve the plan; specialists work in parallel in isolated copies of the project. A task counts only when devd can see the files that changed and a check that passed. Configure providers, and which model does which work, under **Models**. See [`docs/agents.md`](docs/agents.md).

---

## Platforms

| | Windows | Linux | macOS |
|---|---|---|---|
| Status | Supported and tested | Written and compiled, not yet run on a real machine | Written and compiled, not yet run on a real machine |
| nginx, MariaDB, PHP | Downloaded from the publisher and checked against its digest | Adopted from your package manager after it confirms the file is intact | Adopted from Homebrew |
| Trust the local CA | `certutil` | p11-kit `trust` | `security` |
| Default ports | 80 / 443 | 8080 / 8443 | 8080 / 8443 |

On Linux and macOS devd does not download a second copy of nginx or MariaDB. It asks the package manager who owns the installed file and whether it is still what was installed, and it refuses a file that differs. [`docs/platforms.md`](docs/platforms.md) says how that works and exactly what has and has not been verified.

---

## Requirements

- **Windows 10/11 x64** (supported), **Linux x64** or **macOS** (see above).
- The Windows installer needs nothing else: no Rust and no administrator rights. The window uses the WebView2 runtime that ships with current Edge.
- Git on your `PATH`, for the Git features.
- To build from source: Rust 1.77 or newer and a C compiler for the bundled SQLite.

---

## Install

### Windows

Download and run the installer from the [latest release](https://github.com/ianclemence/devd/releases/latest):

```powershell
curl.exe -LO https://github.com/ianclemence/devd/releases/download/v0.9.0/devd-setup-0.9.0.exe
.\devd-setup-0.9.0.exe
```

It installs for the current user only, adds `devd` to `PATH` and creates Start Menu entries. Uninstalling keeps your sites and data in `%LOCALAPPDATA%\devd`. The installer is not code-signed, so SmartScreen warns once; `SHA256SUMS.txt` on the release lets you check what you downloaded. A portable `devd.exe` and a portable window are attached to each release as well.

### From source

```bash
git clone https://github.com/ianclemence/devd.git
cd devd
cargo build --release
./target/release/devd doctor
```

On Windows, if your `cargo` is the MSVC toolchain, run `. .\scripts\dev-env.ps1` first. It puts the GNU toolchain this project targets first on `PATH` for the session. On Linux, `./scripts/install-linux.sh --binary target/release/devd` installs per user.

---

## Quick start

```bash
devd setup --yes          # install what is missing, start it, create a first site
devd ui                   # open the application
```

After that, a normal project site is three commands:

```bash
devd site create blog             # directory, starter page, certificate, web server block
devd hosts sync -y                # publish blog.test (needs an elevated shell on Windows)
devd tls trust -y                 # add devd's certificate authority to your trust store
```

A few more:

```bash
devd site create api --upstream 3000   # serve a running dev server at api.test
devd runtime install php@8.4           # install a runtime
devd service start nginx               # waits until the port answers
devd capabilities                      # everything devd can do
devd doctor                            # what is wrong with this machine, and the fix
```

`devd <command> --help` documents every flag. Every command, the settings table and how approval works are in [`docs/reference.md`](docs/reference.md).

---

## Where your data lives

State is in `%LOCALAPPDATA%\devd` on Windows and `~/.local/share/devd` on Linux; `DEVD_HOME` overrides it.

```text
settings.toml     preferences
state.db          sites, services, runtimes and the audit trail
www/              projects
runtimes/         PHP, Node.js and Go
services/         nginx and MariaDB
certs/            the authority and per-site certificates
logs/             service and session logs
```

---

## Documentation

| Document | Contents |
|---|---|
| [`docs/reference.md`](docs/reference.md) | Every command, the settings table and how approval works. |
| [`docs/platforms.md`](docs/platforms.md) | Windows, Linux and macOS: what differs and what is verified. |
| [`docs/architecture.md`](docs/architecture.md) | The registry, the dispatcher, the state store, the crates. |
| [`docs/security.md`](docs/security.md) | The threat model and the specific protections. |
| [`docs/app.md`](docs/app.md), [`docs/tui.md`](docs/tui.md), [`docs/mcp.md`](docs/mcp.md) | The window and its API, the terminal dashboard, the agent surface. |
| [`docs/agents.md`](docs/agents.md) | The assistant, the team, skills and models. |
| [`docs/stacks.md`](docs/stacks.md), [`docs/stacks-engine.md`](docs/stacks-engine.md) | Supported stacks and how they are defined. |
| [`docs/devd-yml.md`](docs/devd-yml.md) | Declaring a project's processes. |
| [`docs/limitations.md`](docs/limitations.md) | What is not implemented, and what is weaker than intended. |
| [`CHANGELOG.md`](CHANGELOG.md) | Release history. |

---

## Development

```bash
cargo test --workspace            # everything
.\scripts\cross-check.ps1         # type-check for Linux and macOS from Windows
```

`TODO.md` is the live list of what is being worked on.

---

## License

MIT. See [`LICENSE`](LICENSE).

devd bundles no third-party binaries. PHP, Node.js, Go, nginx and MariaDB are downloaded from their publishers on first use and verified against the digest the publisher gives; where a publisher gives none, the install says so. The window vendors xterm.js and its fit addon (both MIT); their licences are in [`crates/devd-server/ui/vendor/`](crates/devd-server/ui/vendor/).
