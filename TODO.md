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
| ~~A2~~ | ~~Home card says "PHP — not installed"~~ **fixed** | `runtime.list` now reports a PHP found on the machine, marked as not managed by devd | `devd-runtime/src/lib.rs` |
| ~~A3~~ | ~~MariaDB reported installed when only MySQL is~~ **fixed** | A tool may now say what its output must contain and must not contain | `tools.yaml`, `tools.rs` |
| ~~A4~~ | ~~ImageMagick probes the Windows disk converter~~ **fixed** | Same mechanism: only a program that says "ImageMagick" counts | `tools.yaml` |
| ~~A5~~ | ~~A failed probe leaves the path pointing at the last candidate~~ **fixed** | The path is only recorded when the program actually ran | `tools.rs` `probe` |

### B. devd's own team of agents

| # | What | Why | Where |
|---|------|-----|-------|
| B1 | devd needs its own agents — a startup team, not a launcher for other people's | The person should name a member of the team and the work should go to that member, with its own brief, its own skills and its own copy of the work | new: `devd-agent` roster + `agent.spawn` |
| B2 | Each agent has a role, a brief and a loadout of skills | Anthropic's pattern: a description says when to use it, a prompt says how it behaves, tools say what it may touch | `devd-agent/src/roles.rs` |
| B3 | The team works on different projects at the same time, each in its own copy | "A team of engineers working on different projects being given to them" | `devd-workspace/src/orchestrate.rs` |
| B4 | Agents report progress and ask questions through the inbox | The person should not have to watch | `devd-agent/src/team.rs`, `inbox.*` |
| B5 | The assistant's own prompt must follow Anthropic's engineering guidance | Orchestrator-worker, explicit delegation, context isolation, bounded depth | `devd-agent/src/chat.rs` `PROMPT` |

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
| D1 | A **Skills** tab in the Agents screen | Skills are part of the team's loadout and need a home | `ui/app.js`, `devd-agent/src/skills.rs` |
| D2 | Install a skill from a GitHub repository | Skills are shared as repositories | new capability |
| D3 | Install a skill from skills.sh | The catalogue people already use | new capability |
| D4 | Import a skill from another agent — OpenCode, Cursor, Codex | Their skills are already written | new capability |
| D5 | Everything that is installed must load correctly | An installed skill that does not load is worse than one that is not there | new + tests |

### E. The interface

| # | What | Why | Where |
|---|------|-----|-------|
| E1 | A route for every page, tab and section | A link to a screen should work, and the back button should mean something | `devd-server/src/api.rs`, `ui/app.js` |
| E2 | Every button does what it says; the ones that are not needed are gone | A button that does nothing is a lie | all screens |
| E3 | EAS for mobile development, like ADB for Android | Mobile is half the stacks and one of the two screens had a devices button | `toolchain.eas_projects`, done |
| E4 | The composer at the bottom, the microphone in the send slot, `@` for context | asked for | done |
| E5 | The "Plan" button beside Install was saying what the approval already says | Removed: one button per action | `ui/app.js`, done |

### F. The command line

| # | What | Why | Where |
|---|------|-----|-------|
| F1 | The CLI is audited command by command | It is the oldest surface and the least looked at | `devd-cli/src` |

### G. Research

| # | What | Why | Where |
|---|------|-----|-------|
| G1 | How Herd fetches PHP versions and installs its services, and what devd should do differently | The vendoring, digesting and switching model is worth copying | `docs/research/` |

### H. Housekeeping

| # | What | Why |
|---|------|-----|
| H1 | Commit and push in one batch | Nothing is saved until it is committed |

## Done

- The assistant follows OpenCode's shape: the mark before a conversation, the
  box with its controls in one row, the state of the session underneath.
- A conversation says what it used — "Used 3 Edit, Shell" — before the words.
- The microphone holds the send slot until there is something to send.
- `@` in the composer lists capabilities, skills, projects and agents.
- Symfony, CodeIgniter 4 and CakePHP are gone from the stacks, and from the
  text that describes them.
- The Tools tab, its page and its breadcrumb are all called Tools, and it
  behaves like the other Server tabs: it keeps its heading while it probes.
- Services, Sites, Tools and Runtimes can be searched.
- The README talks about devd and not about other products.
- The tool steps in a conversation were being squashed to two pixels by a
  flex column; the user's own message was being squeezed to one word.
- Brand marks for the toolchain and the stacks, from the owners' own artwork.
- **npm, Composer, Yarn and the Expo CLI are found and run again.**
- **MySQL is MySQL and MariaDB is MariaDB; ImageMagick is ImageMagick.**
- **The Home screen names the PHP it found and says why devd wants its own.**
