---
name: build-the-tool-if-it-repeats
description: When the same work will recur, write the script that does it instead of repeating it by hand
kind: build
roles: implementer,planner
---

If you are going to do something many times (a check, a migration step, a measurement), spend the effort on a script or capability that does it, and run that. Repeatable and reviewable beats clever and manual.

The failure this prevents: the same hand-edited command in three places.
