---
name: swarm
description: Fan work out to parallel workers and fold results into one report
kind: build
roles: planner
---

# Swarm

Splits independent work across parallel workers and folds the results into one report.

## When to use

Many independent pieces of work: files to check, modules to migrate, questions to answer.

## Procedure

1. Frame it: what "done" is, what the combined report must contain, and whether the work is split into slices, raced, or both.
2. In a devd team plan, give each independent slice its own task with disjoint `scope` paths so they run side by side in isolated copies. Tasks that touch the same files depend on each other.
3. Make every brief stand alone: goal, exact slice, how to verify, what to report.
4. Aggregate from evidence. A slice with no passing verification is a gap, not a pass.
5. Report once: a table of slices and results, issues in one line each, gaps stated.

## Pitfalls

- Overlapping scopes that conflict at merge.
- Counting a missing result as success.

## Verification

Every slice is in the table with its evidence or marked as a gap.
