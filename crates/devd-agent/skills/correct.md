---
name: correct
description: Make a repeated mistake impossible instead of documented
kind: review
roles: reviewer,architect,implementer
---

# Correct a mistake class

When the same mistake has happened twice, fixing the instance is not enough. This fixes the class, at the strongest level that works.

## When to use

The same kind of defect appears again, or a review keeps making the same comment.

## Procedure

1. Name the class from the evidence: commits, reverts, review comments, workaround comments.
2. Fix it at the highest level that works: architecture first (one owner per piece of state, one supported path, delete the old pattern agents copy), then types, lint or CI (a check whose message names the right function), then tests, and documentation last (`plant-the-fix-in-the-structure`).
3. Prove the new check fails on a real past instance, and runs the same locally and in CI.
4. One commit per fix. Exceptions only on the offending line, with a reason and an expiry.

## Pitfalls

- A paragraph in a README as the fix.
- A check that has never been seen to fail.

## Verification

The new check fails on the past instance you named, and passes on the fixed code.
