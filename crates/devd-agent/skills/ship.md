---
name: ship
description: Verify the whole change on the real thing before it is committed
kind: ship
roles: implementer,reviewer,qa
---

# Ship

Verifies a finished change end to end and commits it in an order that tells the story. Pushing, deploying and anything on a production system are the person's decisions: propose them, never perform them unasked.

## When to use

The work is done and about to be committed or merged.

## Procedure

1. Review your own diff as a stranger would (`git.diff`). Remove debugging leftovers and anything outside the task.
2. Run every project check that exists: test, lint, typecheck, build. All must pass on the final state, not an earlier one.
3. Use the changed behaviour end to end once (`prove-it-works`, `test-the-app`).
4. Commit (`write-the-commit`): one concern per commit, in an order that shows the work.

## Pitfalls

- Checks run before the last edit.
- A commit that mixes the fix with unrelated cleanup.

## Verification

The report lists the checks run on the final state and what you did not run.
