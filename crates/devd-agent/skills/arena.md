---
name: arena
description: Let several independent attempts compete, pick the best base, graft in the rest's strengths.
kind: technique
roles: planner,architect
---

# Arena

Use when a single attempt risks the wrong design.

1. State the artifact every candidate produces and write 3 to 6 gradeable criteria.
2. In a devd team plan: two or three implementer tasks with the same brief, each in its own isolated copy, and a reviewer task that depends on all of them and scores each against the criteria. They must not depend on each other.
3. Choose the base a future maintainer can extend most easily without breaking invariants; on ties choose the smaller interface.
4. Port one or two real strengths from the others into the base by hand. Keep one coherent design.
5. Verify the result like any other output. If candidates disagree widely, the problem statement was unclear: reframe and run again.
