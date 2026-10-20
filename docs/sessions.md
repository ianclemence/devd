# Agent sessions

An agent session is a **real process with a real PTY**, owned by the devd backend.
The window is a client of that process, not its parent. Closing a view, switching
to Preview, or reloading the whole page disposes the terminal *component*; it
does not stop the harness.

## What survives what

| Event | The process | The metadata | The output |
| --- | --- | --- | --- |
| Switch view or tab | Keeps running | Kept | Kept; polling resumes |
| Reload the window | Keeps running | Kept | Replayed from the bounded buffer |
| Close the window, server still running | Keeps running | Kept | Kept; reattach on the next launch |
| Leave `devd ui` running and open a second window | Keeps running | Kept | Both clients can attach |
| devd exits normally | Stopped (cancelled) | Kept | Transcript kept |
| devd crashes or is killed | Orphaned or stopped by the OS | Kept | Transcript tail kept |
| Start a new devd process | **Cannot be reattached** | Reconciled to `stale` | Transcript still readable |

A PTY handle belongs to the process that opened it. No amount of SQLite can
re-adopt a handle owned by a dead process, so devd does not pretend it can. On
startup, rows whose owning devd process is gone are marked `stale`; rows whose
owner is still alive are shown as `unavailable` in another process, not as
running. A stale row is never presented as a running process.

## Lifecycle states

| State | Meaning |
| --- | --- |
| `starting` | The row exists; the process is being created. |
| `running` | The PTY is live and attachable from the owner process. |
| `completed` | The process exited with code 0. Output is replayable. |
| `failed` | It exited non-zero, or ended without a status. Output is replayable. |
| `cancelled` | A person or a cancellation request stopped it. |
| `stale` | The owning devd process is gone. Metadata and transcript remain. |
| `unavailable` | Another live devd process owns the PTY; attach there. |

The API reports both the durable `state` and a reconciled `live_state`, plus
`attachable`, `unattached_reason` and `replayable`. The reconciled value is what
the interface displays.

## Ownership and concurrency

Each session records its `owner_pid` and the owner's start time. Only the owner
may write, resize or stop it. A second process can list it, read its transcript,
and say honestly that it cannot attach. This prevents two devd processes from
writing to one PTY or racing a stop.

Within one process, each session has its own `portable-pty` pair, reader, writer,
child and output buffer. There is no shared PTY. Terminal input is serialised by
the writer mutex; output is appended by one reader thread. A watcher thread waits
on the child independently, because on Windows a ConPTY reader does not always
reach end-of-file when the child exits. Waiting on the child is the authoritative
exit signal.

## Output recovery

* The live buffer retains the most recent **2 MiB** per session.
* The transcript on disk is capped at **8 MiB**, keeping the newest half when it
  is trimmed.
* `POST /api/sessions/:id/output {cursor}` returns base64 bytes since a byte
  cursor. If the cursor predates the retained window, `reset: true` tells the
  client to rebuild its screen.
* A session with no live PTY replays the last 256 KiB of its transcript.
* A task result stores the last 16 KiB when the child ends, so a coordinating
  agent can read the outcome after the terminal is gone.

## Limits

* **12 live sessions** per devd process.
* **8 unfinished delegated tasks** per installation.
* **3 levels** of delegation.
* A wait may hold for at most **900 s**; a task's declared timeout is at most
  **3600 s**. A wait timeout does not kill the process.

The limits are enforced before a process is created, not after.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/sessions?project=` | List sessions with live state |
| `POST` | `/api/sessions` | Start `{kind,cwd,project?,label?,cols?,rows?,command?}` |
| `GET` | `/api/sessions/:id` | One session, reconciled |
| `POST` | `/api/sessions/:id/attach` | Session metadata plus replay and cursor |
| `POST` | `/api/sessions/:id/output` | `{cursor}` → base64 bytes |
| `POST` | `/api/sessions/:id/input` | `{data}` |
| `POST` | `/api/sessions/:id/resize` | `{cols,rows}` |
| `POST` | `/api/sessions/:id/stop` | Kill the tree, keep the transcript |
| `POST` | `/api/sessions/:id/rename` | `{label}` |
| `DELETE` | `/api/sessions/:id` | Dismiss a finished session |

The v0.7 `/api/terminal/*` routes remain as compatibility aliases over the same
session manager.

The same operations are capabilities (`session.start`, `session.list`,
`session.get`, `session.attach`, `session.output`, `session.input`,
`session.resize`, `session.rename`, `session.stop`, `session.dismiss`), so an
agent reaches them through MCP. `session.input`, `session.resize`,
`session.rename` and `session.dismiss` are reserved for humans: an agent typing
into a person's terminal is indistinguishable from prompt injection. An agent
may stop a session it delegated, but not a person's interactive one.
