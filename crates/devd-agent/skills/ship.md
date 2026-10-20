---
name: ship
description: Verify the whole change on the real thing before it is committed or merged.
kind: playbook
roles: implementer,reviewer,qa
---

# Ship

1. Review your own diff as a stranger would (`git.diff`). Remove debugging leftovers and anything outside the task.
2. Run every project check that exists: test, lint, typecheck, build. All must pass on the final state, not an earlier one.
3. Exercise the changed behaviour end to end once (`principle-prove-it-works`).
4. Commit in an order that tells the story: failing test, then fix; removal, then reshaping. One concern per commit, a message that says why.
5. Pushing, deploying and anything on a production system are the person's decisions; propose them, never perform them unasked.
