---
name: figure-it-out
description: Design an auditable workflow for large or unusual work no playbook covers
kind: design
roles: planner,architect
---

# Figure it out

For large, unusual or unattended work where no other skill fits. It sets what "done" means before starting, runs the work as a series of checked experiments, and keeps a trail a reviewer can read.

## When to use

The task is big, new to you, or will run without anyone watching. For ordinary work use `work` to pick the right skill instead.

## Procedure

1. **Frame.** Define done as a falsifiable predicate. Quantify the scope. Set the rigour by the stakes: higher for one-way doors and a wide blast radius (`blast-radius`).
2. **Design.** Break the work into atomic units that can land separately, riskiest unknown first. Build the verification harness and record a baseline before changing anything. Decide what can run in parallel (only across clean seams, each in its own copy; `swarm`).
3. **Run.** Treat each unit as an experiment: hypothesis, smallest change, measure against the predicate, keep or revert. Verify each unit before the next. A verdict is verified, not verified, or inconclusive; inconclusive is never a pass.
4. **Keep the trail** with `decision-log`.
5. **Verify** the whole against the original predicate on the real product, and turn recurring corrections into checks (`correct`).

## Pitfalls

- A predicate that cannot fail ("it works well").
- Starting without a baseline, so improvement cannot be shown.
- Counting an inconclusive result as a pass.

## Verification

The original predicate is checked on the real product, and the trail shows each unit's verdict with its evidence.
