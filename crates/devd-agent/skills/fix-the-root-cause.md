---
name: fix-the-root-cause
description: Fix the fault where it lives, not the place where it showed up
kind: check
roles: implementer,qa
---

Reproduce first. Ask why until the answer is a wrong decision in the code. If the state that triggered the failure is legitimate, handle it where it broke. If it should never exist, find what produced it rather than adding a check where it surfaced. Search for every instance.

Retries, swallowed errors and clamping are not fixes; they hide the fault.

The failure this prevents: a workaround that needs a paragraph of explanation.
