---
name: decision-log
description: Keep a reviewable trail of decisions for long or unattended work
kind: ship
roles: planner,implementer,qa,reviewer
---

# Decision log

For long or multi-step work, keeps an append-only log a reviewer can read cold, so the reasoning survives the session.

## When to use

Work that spans many steps, runs unattended, or involves forks the person will want to audit.

## Procedure

1. One row per decision: time, phase, what was decided, why, where the evidence is (commit, file and line, command output), and the result (tests green, reverted, inconclusive).
2. Log forks, finished units with their verification, pivots and reverts with the trigger, and blockers. Not every action.
3. In devd the team run's event log already records starts, reports, evidence and decisions; add your own rows with `scratch.write` when the reasoning would otherwise be lost.
4. Before handing back, check each row against what happened and that its evidence exists.

## Pitfalls

- Editing a row. A wrong entry is superseded by a new row.
- Logging every command. A log nobody reads is no log.

## Verification

Each row's evidence exists and matches, and nothing the report claims is missing from the log.
