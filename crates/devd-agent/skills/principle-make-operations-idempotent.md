---
name: principle-make-operations-idempotent
description: Running it twice, or after a crash, reaches the same end state.
kind: principle
roles: implementer,infra
---

# Running it twice, or after a crash, reaches the same end state

Derive each step from the state that exists, not from what the previous run meant to do. Make leftovers (locks, temp files, claims) recognisable as stale wherever the last run stopped. Write to a temporary name and rename. Ask: what if this runs twice, and what if it crashed at each point?
Smell: an install script that fails the second time.
