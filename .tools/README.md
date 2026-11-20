# .tools — watching devd work

These are the probes and monitors used to observe four team runs end to end and to check the result
from outside the harness. They are not part of the product; they are the instruments that made the
product's claims testable.

Everything here talks to the running daemon over HTTP (`POST /api/invoke`) with the session token
read from the page, so a probe needs only `devd serve` and nothing else.

## Watching a run

| Tool | What it does |
|---|---|
| `monitor.mjs <run> --every 8 --out run.log` | follows a run: state, tasks, and every tool call and result, one line each |
| `runstate.mjs <run>` | the state line: run state, cost, turns, each task with role and attempts, verdict, reasons |
| `runwhy.mjs <run>` | why a run ended: its error, verdict, and the report of the task that judged it |
| `events.mjs <run> [kind]` | the run's own event log, one line each (`accepted`, `retrying`, `budget_extended`, `read_enough`, …) |
| `task.mjs <run> <key> [width]` | one task in full: workdir, branch, verdict, criteria, report, evidence |
| `taskid.mjs <run> <key>` | just the task id, for capabilities that want one |
| `thread.mjs <id>` | one conversation: state, effort, tokens, cost and the messages |

## Driving it

| Tool | What it does |
|---|---|
| `probe.mjs invoke <capability> '<json>'` | one capability call, with `@file.json` to read the arguments from a file (a JSON object cannot survive a Windows command line otherwise) |
| `devdcall.mjs` | the same through the `devd` CLI, for when a one-shot process is what is being tested |
| `deepseek-probe.mjs` | one provider request on the wire, to see whether the model answers and whether it reasons with thinking off |

`args-*.json` are the exact payloads used: the run goals, the retries, and the defect report handed to
a live run mid-flight. They are kept because they are the reproduction steps, not noise.

## Checking a project from outside

| Tool | What it proves |
|---|---|
| `verify-pulse.mjs` | a status board: measured checks, counters that include the reading request, one log line per request |
| `verify-ledger.mjs` | double-entry bookkeeping: an unbalanced transaction writes nothing, balances add up, idempotency is decided by the database |
| `verify-courier.mjs` | a job queue: ten concurrent claims never share a job, a `SIGKILL`ed lease is swept and re-claimed, a job survives the process dying and restarting on the same file |
| `verify-quorum.mjs` | agreement: three nodes, one killed mid-flight, a write still succeeds on two acks, two killed and it is refused with 503, and a node that was behind is brought current by a read with `repaired: true` |

Each starts real processes, asserts on what a client sees, and checks that nothing is left listening.

**Ports matter.** `verify-courier.mjs` uses 4390–4391 and `verify-quorum.mjs` uses 4521–4523. The
first version of the quorum probe used 4421–4423 and collided with a reproduction the team's reviewer
was running on the same ports at the same time; the resulting 401 looked like a defect in their code
for about ten minutes. Isolate a probe's ports before believing its failures.

## Configuring

`DEVD_URL` overrides the daemon address (default `http://127.0.0.1:8787`). The token is read from the
served page each time, so it survives a daemon restart.
