---
name: swarm
description: Fan work out to parallel workers and fold the results into one report.
kind: technique
roles: planner
---

# Swarm

1. Frame: what is "done", what the combined report must contain, and whether the work is split into slices, raced, or both.
2. In a devd team plan, give each independent slice its own task with disjoint `scope` paths so they can run side by side in isolated copies. Tasks that touch the same files must depend on each other.
3. Every brief must stand alone: goal, exact slice, how to verify, what to report.
4. Aggregate from evidence. A slice with no passing verification is a gap, not a pass. Never count a missing result as success.
5. Report once, consolidated: a table of slices and results, issues in one line each, gaps stated.
