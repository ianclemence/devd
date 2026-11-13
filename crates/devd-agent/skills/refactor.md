---
name: refactor
description: Change structure without changing behaviour, proving it did not change
kind: build
roles: implementer,architect
---

# Refactor

Improves structure and leaves behaviour alone, with proof.

## When to use

Restructuring, renaming or moving code with no intended behaviour change. For anything else use `build-feature` or `fix-bug`.

## Procedure

1. Pin the current behaviour first: run the existing tests. If coverage is thin, write characterisation tests that record what the code does today.
2. Work in steps that each leave everything green. Never mix a refactor with a behaviour change in one step.
3. Move callers to the new shape, then delete the old one (`finish-the-migration`).
4. Re-run the same checks after every step. A red step is reverted or fixed before the next.

## Pitfalls

- "While I'm here" behaviour changes.
- Leaving both the old and the new shape.

## Verification

The same checks pass before and after, and the finished diff has fewer concepts than the start.
