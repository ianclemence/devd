---
name: figure-it-out
description: Design an auditable workflow for work no playbook covers.
kind: technique
roles: planner,architect
---

# Figure it out

For large, unusual or unattended work.

1. **Frame.** Define done as a falsifiable predicate. Quantify the scope. Set rigour by stakes: higher for one-way doors and wide blast radius.
2. **Design.** Break the work into atomic units that can land separately, riskiest unknown first. Build the verification harness and record a baseline before changing anything. Decide what can run in parallel (only across clean seams, each in its own copy).
3. **Run.** Treat each unit as an experiment: hypothesis, smallest change, measure against the predicate, keep or revert. Verify each unit before the next. Verdicts are verified, not verified, or inconclusive; inconclusive is never a pass.
4. **Keep the trail** (`decision-log`).
5. **Verify** the whole against the original predicate on the real product, and turn recurring corrections into checks (`correct`).
