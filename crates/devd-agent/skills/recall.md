---
name: recall
description: Rebuild context before resuming work
kind: discover
roles: planner,implementer
---

# Recall

Rebuilds what happened on a topic before continuing it. The repository outranks any summary.

## When to use

Resuming earlier work, or "what did we decide about X".

## Procedure

1. Set the scope: the topic, the time window (default a week), this project only.
2. Read recent conversations and run history (`chat.list`, `team.list`, `stacks.runs`); note decisions, open threads, and what was tried and failed.
3. Verify against live state: `git.status`, branches, the project's checks.
4. Report a brief: where things stand, one line per thread with a status (merged, in flight, verified but uncommitted, reverted, planned), recurring problems, and the one next move.

## Pitfalls

- Trusting a summary over the code.
- A brief longer than the work it summarises.

## Verification

Every status in the brief was checked against the repository in this turn.
