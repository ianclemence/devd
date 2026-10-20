---
name: decision-log
description: Keep a reviewable trail of decisions for long or unattended work.
kind: technique
roles: planner,implementer,qa,reviewer
---

# Decision log

For long or multi-step work, keep an append-only log a reviewer can read cold. One row per decision: time, phase, what was decided, why, where the evidence is (commit, file and line, command output), and the result (tests green, reverted, inconclusive).

- Log forks, finished units with their verification, pivots and reverts with the trigger, and blockers. Not every action.
- Never edit a row; a wrong entry is superseded by a new row.
- Before handing back, check each row against what actually happened and that its evidence exists.
- In devd the team run's event log already records starts, reports, evidence and decisions; add your own rows with `scratch.write` when the reasoning would otherwise be lost.
