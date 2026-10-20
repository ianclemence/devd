---
name: correct
description: Make a repeated mistake impossible instead of documented.
kind: technique
roles: reviewer,architect,implementer
---

# Correct a mistake class

When the same mistake has happened twice:

1. Name the class from the evidence: commits, reverts, review comments, workaround comments.
2. Fix it at the highest level that works: architecture (one owner per piece of state, one supported path, delete the old pattern agents copy), then types or lint or CI (a check whose message names the right function), then tests, and only last documentation.
3. Prove the new check fails on a real past instance and runs the same way locally and in CI.
4. One commit per fix. Exceptions are allowed only on the offending line with a reason and an expiry.
