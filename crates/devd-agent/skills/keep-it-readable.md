---
name: keep-it-readable
description: Make code, diffs and reports cheap to read
kind: check
roles: implementer,reviewer,docs
---

# Keep it readable

Every reader pays attention for every line. This keeps the cost low.

## When to use

Before handing back code, a diff or a report.

## Procedure

1. Keep functions short and names exact.
2. Keep unrelated changes out of the diff (`git.diff` before you finish).
3. Keep reports short, with the answer first.
4. Order the work so a reviewer can follow it step by step.

## Pitfalls

- A 40-file diff with one idea in it.
- A report as long as the work.

## Verification

Read your own diff as a stranger would; every hunk belongs to the stated change.
