# devd — what is being worked on

A live list. Every item is either **done** or **open**; nothing sits in
between. Update it when you finish something, and add anything new the person
asks for before you start on it.

Ordered by what makes devd *wrong* today, not by what is easiest.

## Open

### A. False statements — the interface says things that are not true

| # | What | Why it matters | Where |
|---|------|----------------|-------|
| ~~A1~~ | ~~npm, Composer, Yarn and EAS reported "missing"~~ **fixed** | `util::which` now tries the `PATHEXT` candidates first, and `util::run` starts a batch file through the command interpreter | `devd-core/src/util.rs` |
| ~~A2~~ | ~~Home card says "PHP — not installed"~~ **fixed** | `runtime.list` now reports the interpreters the machine already has, marked as not managed by devd | `devd-runtime/src/lib.rs` |
| ~~A3~~ | ~~MariaDB reported installed when only MySQL is~~ **fixed** | A tool may now say what its output must contain and must not contain | `tools.yaml`, `tools.rs` |
| ~~A4~~ | ~~ImageMagick probes the Windows disk converter~~ **fixed** | Same mechanism: only a program that says "ImageMagick" counts | `tools.yaml` |
| ~~A5~~ | ~~A failed probe leaves the path pointing at the last candidate~~ **fixed** | The path is only recorded when the program actually ran | `tools.rs` `probe` |
| ~~A6~~ | ~~The Runtimes tab said Node.js was not installed~~ **fixed** | Node and Go are found the same way PHP is; Go is asked with `go version`, which is the only thing it answers | `devd-runtime/src/lib.rs` |
| A7 | A runtime the host's package manager still lists but whose files are gone | Bun on this machine: the registry entry is left, the files are not | the toolchain's "missing" note |

### B. devd's own team of agents

| # | What | Why | Where |
|---|------|-----|-------|
| B1 | devd needs its own agents — a startup team, not a launcher for other people's | **done**: eight specialists, named in the message, given the work | `devd-agent/src/roles.rs`, `team.assign` |
| B2 | Each agent has a role, a brief and a loadout of skills | **done**: the roster already had one job, one permission set and one proof each; the assistant's prompt now describes them | `devd-agent/src/chat.rs` |
| B3 | The team works on different projects at the same time, each in its own copy | The implementer role works in its own copy; the rest do not need one | `devd-workspace/src/orchestrate.rs` |
| B4 | Agents report progress and ask questions through the inbox | **done**: the team already reports through the inbox and the run | `devd-agent/src/team.rs` |
| B5 | The assistant's own prompt must follow Anthropic's engineering guidance | **done**: explicit delegation, context isolation, bounded depth, evidence over assertion | `devd-agent/src/chat.rs` `PROMPT` |
| B6 | Show the roster on a screen, not only in the mention list | A team nobody can see is not a team | the Team runs screen |

### C. Memory

| # | What | Why | Where |
|---|------|-----|-------|
| C1 | Memory that grows in layers: L0 conversation, L1 atoms, L2 scenarios, L3 persona | So the next agent starts where the last one stopped | new: `devd-core` memory tables |
| C2 | Memory assets — chat memory, skills, wiki, code graph — with an owner, a version and a visibility | Sharing is an explicit act, never a leak | new |
| C3 | Memory is a loadout, not a prompt: the agent is equipped with the assets it needs | "Give each agent the memory assets it actually needs" | new |
| C4 | Memory is retrieved on demand, not injected wholesale | The context window is a budget | new |
| C5 | A place to see, review and revoke memory | The person owns it | new: a Memory screen |

### D. Skills

| # | What | Why | Where |
|---|------|-----|-------|
| D1 | A **Skills** tab in the Agents screen | **done** | `ui/app.js` |
| D2 | Install a skill from a GitHub repository | **done**: the repository archive, read as text | `skills.install` |
| D3 | Install a skill from skills.sh | **done, differently**: skills.sh's API needs a Vercel token and allows sixty requests an hour from an address; the files come from GitHub, where they live. The screen links to the catalogue | `skills.install`, the screen's "Browse the catalogue" |
| D4 | Import a skill from another agent — Claude Code, Cursor, Codex, OpenCode, Windsurf, Cline, Gemini, Amp, Zed | **done** | `skills.import`, `skills.sources` |
| D5 | Everything that is installed must load correctly | **done**: the front matter is parsed the way it is written, including a description folded across lines | `skills.rs` |

### E. The interface

| # | What | Why | Where |
|---|------|-----|-------|
| E1 | A route for every page, tab and section | **done**: every screen has an address; the back button works | `devd-server/src/lib.rs`, `ui/app.js` |
| E2 | Every button does what it says; the ones that are not needed are gone | **done for the toolchain**: the "Plan" button said what the approval already says | `ui/app.js` |
| E3 | EAS for mobile development, like ADB for Android | **done**: `toolchain.eas_projects` and a Projects button | `crates/devd-stacks/src/caps.rs` |
| E4 | The composer at the bottom, the microphone in the send slot, `@` for context | **done** | `ui/app.js` |
| E5 | The "Plan" button beside Install was saying what the approval already says | **done**: removed; one button per action | `ui/app.js` |
| E6 | The dialogs flashed and vanished | **done**: the modal and the drawer had their starting and resting states the wrong way round | `ui/style.css` |
| E7 | The actions in a conversation were boxes with raw JSON in them | **done**: a sequence, joined by a line, with the time it took | `ui/app.js`, `ui/style.css` |
| E8 | Every button on every screen does what it says | A button that does nothing is a lie | an audit of each screen |

### F. The command line

| # | What | Why | Where |
|---|------|-----|-------|
| F1 | The CLI is audited command by command | It is the oldest surface and the least looked at | `devd-cli/src` |

### G. Research

| # | What | Why | Where |
|---|------|-----|-------|
| ~~G1~~ | ~~How Herd fetches PHP versions and installs its services~~ **done** | The evidence and the three things to take from it are written down; nginx is now verified | `docs/research/05-herd-catalogue-and-install.md` |
| G2 | One catalogue for the services, rather than an HTML page and a REST API | A catalogue is data; the installer should not know where it came from | `devd-services/src/catalogue.rs` |
| G3 | nginx from nginx.org's own listing, not from a page of markup | A change in someone's HTML is not a contract | `devd-services/src/catalogue.rs` |

### H. Housekeeping

| # | What | Why |
|---|------|-----|
| H1 | Commit and push in one batch | Nothing is saved until it is committed |

## Done

- The assistant follows OpenCode's shape: the mark before a conversation, the
  box with its controls in one row, the state of the session underneath.
- A conversation says what it used — "Used 3 Edit, Shell" — before the words.
- The actions in a conversation are a sequence, joined by a line, with the time
  each one took; the details open on a click and say what a person would say.
- The microphone holds the send slot until there is something to send.
- `@` in the composer lists capabilities, skills, projects, devd's own team and
  other people's agents.
- devd's team can be named, and the work is handed to that member.
- A Skills tab, with skills installed from GitHub and imported from the
  agents already on this machine.
- The dialogs stay open. The modal and the drawer had their starting and
  resting states the wrong way round, so every one of them flashed and went.
- Symfony, CodeIgniter 4 and CakePHP are gone from the stacks, and from the
  text that describes them.
- The Tools tab, its page and its breadcrumb are all called Tools, and it
  behaves like the other Server tabs: it keeps its heading while it probes.
- Services, Sites, Tools, Runtimes and Skills can be searched.
- The README talks about devd and not about other products.
- Brand marks for the toolchain and the stacks, from the owners' own artwork.
- **npm, Composer, Yarn and the Expo CLI are found and run again.**
- **MySQL is MySQL and MariaDB is MariaDB; ImageMagick is ImageMagick.**
- **PHP, Node.js and Go are reported the same way everywhere: what the
  machine has, and whether devd installed it.**
- **Every screen has an address**, and the back button means something.
- **nginx is verified before it is installed**, with the digests devd
  recorded the first time it fetched each archive.
