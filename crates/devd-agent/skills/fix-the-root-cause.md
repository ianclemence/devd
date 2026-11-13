---
name: fix-the-root-cause
description: Fix the fault where it lives, not where it showed up
kind: check
roles: implementer,qa
---

# Fix the root cause

A failure shows up in one place and is usually caused in another. This keeps the fix at the cause.

## When to use

Any time a fix is a check, a default, a retry or a special case added where an error appeared.

## Procedure

1. Reproduce first.
2. Ask why until the answer is a wrong decision in the code.
3. If the state that triggered the failure is legitimate, handle it where it broke. If it should never exist, find what produced it, rather than adding a check where it surfaced.
4. Search for every instance of the same decision and fix them together.

## Pitfalls

- Retries, swallowed errors and clamping. They hide the fault; they are not fixes.
- A workaround that needs a paragraph of explanation.

## Verification

You can state the wrong decision in one sentence, and the fix removes it rather than working around it.
