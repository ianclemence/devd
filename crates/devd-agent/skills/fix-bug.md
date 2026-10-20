---
name: fix-bug
description: Reproduce, find the root cause, fix it, and prove it stays fixed.
kind: playbook
roles: implementer,qa
---

# Fix a bug

1. **Reproduce first.** Get the failure to happen on demand with the smallest input. If it cannot be reproduced, you do not yet know what to fix; say so and gather more evidence instead of guessing.
2. **Find the cause, not the symptom.** Ask why until the answer is a wrong decision in the code (`principle-fix-root-causes`). Read the real error and the real state; instrument when stuck.
3. **Write the failing test** when the test is cheap and local (`tdd`). Confirm it fails for the reason you expect.
4. **Make the smallest fix** at the cause. Search for the same fault elsewhere and fix every instance.
5. **Prove it** (`principle-prove-it-works`): the regression test now passes, the project's own checks pass, and the original reproduction no longer fails.
6. **Report** the cause, the fix, and the evidence: the failing run before and the passing run after.

Do not add a guard that makes the symptom disappear, retries that hide failures, or a test weakened to pass.
