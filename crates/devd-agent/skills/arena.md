---
name: arena
description: Race several independent attempts and keep the best base
kind: design
roles: planner,architect
---

# Arena

Runs several independent attempts at the same problem and picks the strongest, because one attempt at a hard design risks the wrong shape. It costs several attempts, so it is for decisions that are expensive to reverse.

## When to use

A design with real alternatives and no obvious winner. Not for routine work.

## Procedure

1. State the artifact every candidate produces and write 3 to 6 criteria that can be graded.
2. In a devd team plan, create two or three implementer tasks with the same brief, each in its own isolated copy, plus a reviewer task that depends on all of them and scores each against the criteria. The candidates must not depend on each other.
3. Choose the base a future maintainer can extend most easily without breaking invariants; on a tie, the smaller interface.
4. Port one or two real strengths from the others into the base by hand. Keep one coherent design.
5. Verify the result like any other output.

## Pitfalls

- Candidates that saw each other's work. They are then one attempt copied.
- Merging everything. A design with every candidate's best idea is usually none of their designs.
- Wide disagreement among candidates usually means the problem statement was unclear. Reframe and run again.

## Verification

The scores, the chosen base, what was ported and what was left out are in the report, and the final design passes its own checks.
