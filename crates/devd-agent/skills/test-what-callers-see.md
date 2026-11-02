---
name: test-what-callers-see
description: Write tests against observable behaviour, so a refactor does not break them and a bug does
kind: check
roles: implementer,qa,reviewer
---

Two checks for every test: break the behaviour and it must fail; refactor the internals and it must still pass. Derive expected values from the requirement, not from running the code. Pair every "never happens" assertion with a case where it does happen. Assert on the error types callers branch on, not on message wording.

The failure this prevents: tests that mock internals and mirror the code line for line.
