---
name: principle-migrate-callers-then-delete-legacy-apis
description: Finish migrations: move every caller, then remove the old way.
kind: principle
roles: implementer,architect
---

# Finish migrations: move every caller, then remove the old way

Introduce the new API, move every caller, delete the old one. A half-migrated system keeps both and teaches the next contributor (human or agent) to copy the wrong one.
Smell: two ways to do the same thing, both still used.
