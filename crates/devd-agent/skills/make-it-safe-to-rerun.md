---
name: make-it-safe-to-rerun
description: Design each step so that running it twice, or after a crash, reaches the same end state
kind: house
roles: implementer,infra
---

Derive each step from the state that exists, not from what the previous run meant to do. Make leftovers (locks, temporary files, claims) recognisable as stale wherever the last run stopped. Write to a temporary name and rename it into place. Ask two questions of every operation: what if this runs twice, and what if it crashed at each point?

The failure this prevents: an install script that fails the second time.
