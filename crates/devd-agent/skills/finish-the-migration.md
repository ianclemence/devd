---
name: finish-the-migration
description: Move every caller to the new way, then delete the old one
kind: build
roles: implementer,architect
---

Introduce the new API, move every caller, delete the old one. A half-migrated system keeps both and teaches the next contributor, human or agent, to copy the wrong one.

The failure this prevents: two ways to do the same thing, both still used.
