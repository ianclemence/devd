---
name: blast-radius
description: Find what a change could break beyond its callers, and prove why it is safe
kind: review
roles: reviewer,architect,qa
---

# Blast radius

A plausible write-up is not evidence. This finds the one or two facts a change's safety depends on and proves them by running something.

## When to use

A change to a shared name, a stored format, a configuration key, a dependency or a public contract. Before merging anything that is hard to roll back.

## Procedure

1. Read the change: symbols added, changed and deleted, and the behaviour the diff implies but does not state.
2. Name the safety fact ("this only removes entries nothing reads"). If it holds, most risks fall away.
3. Look past `search_files`: library behaviour at the pinned version, timing and ordering, data that crosses boundaries (database columns, JSON shapes, files other tools read), configuration and flags.
4. Rank the evidence: the author says so, then a cited line of source, then a walked-through path, then a script or test that calls the real code, then the behaviour reproduced in the running app. Aim for the last two.
5. Report what it does, the safety fact and the level of evidence reached, each risk with where and how likely, what is cleared, and the cheapest test to run before merging.

## Pitfalls

- Stopping at the callers you can find with a search.
- Believing a comment or a commit message about what the code does.

## Verification

The safety fact has evidence at the "script or test" level or better, and the report says which level it reached.
