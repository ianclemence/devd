---
name: blast-radius
description: Find what a change could break beyond its callers, and prove the one fact that makes it safe.
kind: technique
roles: reviewer,architect,qa
---

# Blast radius

A plausible write-up is not evidence. Find the one or two facts the change's safety depends on and prove them by running something.

1. Read the change: symbols added, changed, deleted, and behaviour the diff implies but does not state.
2. Name the safety fact ("this only removes entries nothing reads"). If it holds, most risks fall away.
3. Look past grep: library behaviour at the pinned version, timing and ordering, data that crosses boundaries (database columns, JSON shapes, files other tools read), configuration and flags.
4. Rank evidence: the author says so < a cited line of source < a walked-through path < a script or test that calls the real code < the behaviour reproduced in the running app. Aim for the last two.
5. Report: what it does, the safety fact and the evidence level reached, each risk with where and how likely, what is cleared, and the cheapest test to run before merging.
