---
name: fix-bug
description: Reproduce, find the root cause, fix it, and prove it stays fixed
kind: build
roles: implementer,qa
---

# Fix a bug

Takes a defect from "it broke" to a fix with evidence. It fixes the cause, not the symptom; it does not add a guard that makes the symptom vanish.

## When to use

Something that used to work, or should work, does not.

## Procedure

1. **Reproduce first.** Make the failure happen on demand with the smallest input. If it cannot be reproduced you do not yet know what to fix: say so and gather evidence instead of guessing.
2. **Find the cause.** Ask why until the answer is a wrong decision in the code (`fix-the-root-cause`). Read the real error and the real state; add instrumentation when stuck.
3. **Write the failing test** when it is cheap and local (`tdd`), and confirm it fails for the reason you expect.
4. **Make the smallest fix** at the cause. Search for the same fault elsewhere and fix every instance.
5. **Prove it** (`prove-it-works`): the regression test passes, the project's checks pass, and the original reproduction no longer fails.

## Pitfalls

- A guard, a retry or a weakened test that hides the symptom.
- Fixing the instance you saw and not its siblings.
- Calling it fixed because the test you wrote passes, without re-running the original reproduction.

## Verification

The report gives the cause, the fix, and two runs: the failure before and the pass after.
