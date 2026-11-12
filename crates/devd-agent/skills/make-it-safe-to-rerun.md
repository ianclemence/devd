---
name: make-it-safe-to-rerun
description: Design each step so running it twice, or after a crash, ends the same
kind: house
roles: implementer,infra
---

# Make it safe to rerun

Derives each step from the state that exists, not from what the previous run meant to do, so a crash or a second run does no harm.

## When to use

Installers, migrations, scripts, jobs, anything that changes files or services.

## Procedure

1. Ask two questions of every operation: what if this runs twice, and what if it crashed at this point?
2. Derive each step from current state ("is it already there?"), not from a record of what the last run did.
3. Make leftovers (locks, temporary files, claims) recognisable as stale wherever the last run stopped.
4. Write to a temporary name and rename it into place.

## Pitfalls

- An install script that fails the second time.
- A lock file with no way to tell whether its owner is alive.

## Verification

Run it twice in a row and once with a forced stop in the middle; the end state is the same each time.
