---
name: build-the-tool-if-it-repeats
description: Script work that will recur instead of repeating it by hand
kind: build
roles: implementer,planner
---

# Build the tool if it repeats

When the same work will happen again (a check, a migration step, a measurement), a script that does it beats doing it by hand: repeatable and reviewable beats clever and manual.

## When to use

The second time you are about to do the same multi-step thing, or when you notice the same hand-edited command in more than one place.

## Procedure

1. Write the smallest script or capability that does the whole job and prints its result.
2. Make it safe to run twice (`make-it-safe-to-rerun`).
3. Run it instead of the manual steps, and keep it where the project keeps its tasks.
4. Mention it in the report so the next person finds it.

## Pitfalls

- A script for something done once.
- A script that needs a paragraph of instructions. If it cannot be run with one command, it is not finished.

## Verification

You ran the script, not the manual steps, and its output is in the report.
