# The terminal dashboard

`devd tui` is the third projection of the registry, after the CLI and the
application window. It exists for the two situations the other two do not cover:
a terminal where no browser can be opened, and a keyboard where reaching for a
mouse would be slower than reading. It is also the only surface that works over a
plain SSH session without forwarding anything.

It is not a second implementation of anything. Every row on screen came out of a
capability, and every key that changes something goes back through
`Registry::dispatch` — the same checks, the same tiers, the same audit rows as
`devd git discard` on the command line.

```bash
devd tui
```

## It refuses to start without a terminal

```console
$ devd tui < /dev/null
devd: unsupported: the dashboard needs an interactive terminal (use `devd ui` for
the window, `devd serve` for the API, or `devd capabilities` for the list)
$ echo $?
2
```

That sentence is not decoration. Entering raw mode on a stdout that is a pipe is
not a condition crossterm can report on Windows — there is no console handle to
switch, and the call took the process down with `0xc0000005` and no output at
all. A refusal with a reason and a suggestion is the correct behaviour for a
surface that cannot do its job, and it is asserted by test.

## The views

`1`–`9` jump straight to one; `Tab` and `Shift+Tab` cycle. The digit is shown in
the tab strip, so the strip is also the key map.

| Key | View | What it shows |
| --- | --- | --- |
| 1 | Dashboard | The environment checks, the selected repository and its state, the project count. |
| 2 | Projects | Every repository `devd` knows about: name, path, default branch, remote. |
| 3 | Changes | The working tree, one row per file, grouped by staged, unstaged, untracked, conflicted. |
| 4 | History | The log: short hash, subject, author. |
| 5 | Branches | Local branches, the current one marked, upstream and subject. |
| 6 | Capabilities | The whole registry, filterable, with each capability's tier. |
| 7 | Activity | The audit trail, newest first. |
| 8 | Doctor | The same checks as `devd doctor`. |
| 9 | Help | The key map and the rule about destructive actions. |

## What a frame looks like

This is a real frame, captured from the dashboard pointed at a real repository
with a staged file, two unstaged files and one untracked file. Trailing padding
is trimmed.

```text
 devd 0.4.0  C:\devd  git 2.46.0.windows.1
 1 Dashboard  2 Projects  3 Changes  4 History  5 Branches  6 Capabilities  7 Activity  8 Doctor  9 Help
 ┌ Changes - main: 1 staged, 2 unstaged, 1 untracked in C:/code/devd ──────────────┐
 │M staged     README.md                                                           │
 │M unstaged   README.md                                                           │
 │M unstaged   crates/devd-core/src/lib.rs                                         │
 │? untracked  NOTES.md                                                            │
 │                                                                                 │
 │                                                                                 │
 └─────────────────────────────────────────────────────────────────────────────────┘
  Enter stage/unstage   s stage   u unstage   d discard   a stage all   c commit   r refresh   ? help   q quit
```

The first line is the version, the home directory and the discovered Git. The
second is the tab strip. The title of the body carries the branch and the counts,
so the state of the repository is readable without moving the cursor. The footer
is one line for the last message and one line for the keys that apply right here.

## Approvals

`d` discards the selected file. Discarding is `Tier::Destructive`, so the
dashboard does not discard it. It asks the registry for the plan and shows it:

```text
 ┌ Confirm a destructive operation ────────────────────────────────────────────────┐
 │ git.discard needs your decision before it runs                                  │
 │                                                                                 │
 │ discard uncommitted changes in 1 path(s)                                        │
 │   - files: README.md                                                            │
 │   - 1 of these currently differ from HEAD                                       │
 │   - this content is NOT recoverable from Git; it was never committed             │
 │                                                                                 │
 │ arguments                                                                       │
 │ {"paths":["README.md"],"repo":"C:/code/devd"}                                   │
 │                                                                                 │
 │ y approve and run       n decline, and record the refusal                        │
 └─────────────────────────────────────────────────────────────────────────────────┘
```

Four things about that prompt are load-bearing:

* **The plan is the registry's.** Those three lines are the ones `git.discard`
  wrote when asked to describe itself. The dashboard does not summarise them,
  reorder them or decide which matter, so the text cannot drift from what the
  operation actually does.
* **The request on screen is the request that runs.** The controller keeps the
  `InvokeRequest` it built, shows the plan, and replays *that* on `y`. It is never
  rebuilt from the screen, which is the difference between "what you read is what
  runs" and "what you read is what it looked like at the time".
* **`n` is recorded.** Declining does not close the prompt and return to the
  view; it dispatches the same request with `Approval::Denied`, which the
  registry refuses and audits. A refused `git.discard` is a row in the Activity
  view, labelled `refused`, exactly like the one the CLI produces.
* **A switch that would lose work becomes a confirm, not a refusal.** Before
  offering a branch switch, the dashboard asks `git.switch.check` — the product's
  own overwrite check, not a second opinion invented for the TUI. If files are at
  risk, the list of them is turned into a destructive plan and the same prompt
  appears.

Nothing in the TUI can reach an operation that needs approval without going
through this prompt, because the prompt *is* how it gets called: the controller
calls `dispatch` with no approval and reacts to `approval_required`.

## Keys

| Key | Action |
| --- | --- |
| `1`–`9` | Jump to a view |
| `Tab` / `Shift+Tab` | Next / previous view |
| `j` `k` `↑` `↓` | Move the cursor |
| `PageUp` `PageDown` `Home` `End` | Move faster, or to the ends |
| `Enter` | Open the row: choose a project, toggle a file in or out of the index, show a commit, switch a branch, describe a capability |
| `s` / `u` | Stage / unstage the selected file |
| `d` | Discard the selected file — asks first |
| `a` | Stage everything |
| `c` | Commit — asks for the message |
| `n` | New branch (in Branches) |
| `t` | Track a repository by path (in Projects) |
| `/` | Search capabilities |
| `x` | Run the selected capability, with arguments you type as JSON |
| `y` / `n` | Approve / decline the operation being asked about |
| `r` | Reload the current view |
| `?` | Help |
| `q`, `Ctrl+C` | Quit |

A modal owns every key while it is open, so a keystroke cannot both answer a
prompt and act on the view behind it. `Esc` cancels a prompt without acting, and
`Esc` on an approval means *no* — the safe reading, not the convenient one.

## What it will never do

* **It never builds a shell string.** The dashboard calls capabilities, and the
  capabilities are typed structs turned into `argv` arrays. However many rows it
  shows, it has no path to a shell.
* **It never writes repository content to the terminal raw.** A branch name or a
  path with a control character in it is not text at that point, it is a command
  being sent to the reader's terminal — `ESC [ 2 J` clears the screen. Every
  string that reaches the screen passes through one function that replaces
  control characters, and a test renders a branch named `evil\u{1b}[2Jbranch` and
  asserts that no escape reached the buffer.
* **It never invents data.** A view with nothing to show says which nothing it
  is: "No repository selected - press 2 to choose one, or t to add a path" is a
  different sentence from "The working tree is clean", because they are different
  facts.
* **It never counts a plan as an action.** The Activity view labels a dry run
  `plan`, an application `applied`, and a refusal `refused`.

## How it is tested

The dashboard is split so that nothing important needs a terminal: plain state,
a controller over a `Backend` trait, and a render function that is a pure
function of that state. That makes the parts that matter testable, and the
tests are the reason to believe the claims above.

* **52 unit tests.** The controller is driven with real keystrokes against a
  fake backend that counts what actually ran. They assert, among other things,
  that a destructive key runs nothing until `y`, that `n` runs nothing *and*
  sends the refusal, that `Esc` on a prompt does not leak the keystroke to the
  view, that an empty commit message never reaches Git, that bad JSON arguments
  are reported rather than attempted, that a `human_only` capability surfaces the
  refusal, and that an unsafe branch switch becomes a confirm.
* **5 integration tests against the real registry and a real repository.** They
  create a throwaway repository, track it through `git.repo.track`, and then
  drive the dashboard through the same code path the binary uses — including
  `s`/`Enter` actually changing the index, and `d` followed by `n` leaving the
  file on disk and a refusal in the audit table.
* **Every frame is rendered, not described.** The view tests render into
  `ratatui`'s `TestBackend` and assert on the resulting buffer, so a frame that
  stops showing the branch, the file list, the plan or the choice fails a test
  rather than reaching a person. This is also how the two transcripts above were
  produced.

## Honest limits

* **The interactive path is not covered by an automated test.** The tests cover
  the state, the controller, the frames and the refusal without a terminal. What
  they cannot cover is crossterm's raw mode, the event loop and the actual
  drawing to a real console, because this project has no way to attach a pty in
  its test environment. It was run by hand in a real terminal; that is a weaker
  claim than the rest of this document and is stated as such in
  [`platforms.md`](platforms.md).
* **No mouse.** Keyboard only, deliberately: the palette and the digits are
  faster, and a mouse adds a code path with no safety value.
* **No horizontal scrolling.** A very long path is truncated with an ellipsis
  rather than pushing the rest of the row off screen. The full value is visible
  in the detail overlay.
* **No interactive rebase.** Rebase is driven by a no-op sequence editor, so
  `--autosquash` works but commits cannot be reordered or dropped yet. A sequence
  editor belongs in a surface that can show a menu, which is the next milestone
  rather than this one.
* **Detail overlays do not wrap long JSON.** A wide declaration scrolls
  vertically and clips horizontally. The application window's inspector is the
  better place to read one.
