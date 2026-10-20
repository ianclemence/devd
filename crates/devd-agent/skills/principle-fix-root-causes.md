---
name: principle-fix-root-causes
description: Fix the fault, not the place it showed up.
kind: principle
roles: implementer,qa
---

# Fix the fault, not the place it showed up

Reproduce first. Ask why until the answer is a wrong decision in the code. If the state that triggered the failure is legitimate, handle it where it broke; if it should never exist, find what produced it, do not add a check where it surfaced. Search for every instance. Hiding mechanisms (retries, swallowed errors, clamping) are not fixes.
Smell: a workaround that needs a paragraph of explanation.
