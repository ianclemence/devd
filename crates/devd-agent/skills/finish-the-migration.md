---
name: finish-the-migration
description: Move every caller to the new way, then delete the old one
kind: build
roles: implementer,architect
---

# Finish the migration

Introduces the new way, moves every caller, and deletes the old way. A half-migrated system keeps both and teaches the next contributor, human or agent, to copy the wrong one.

## When to use

Replacing an API, a pattern, a file format or a dependency. Also whenever you find two ways to do one thing.

## Procedure

1. Add the new way beside the old.
2. Find every caller with `search_files`, including tests, scripts, docs and configuration.
3. Move them all. Run the checks after each batch.
4. Delete the old way, and anything that only existed to support it.

## Pitfalls

- Leaving "just one" caller on the old path.
- Keeping the old one "for compatibility" with nothing that needs it.

## Verification

A search for the old name finds nothing, and the project's checks pass.
