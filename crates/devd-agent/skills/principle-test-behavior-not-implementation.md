---
name: principle-test-behavior-not-implementation
description: Tests assert what callers can observe.
kind: principle
roles: implementer,qa,reviewer
---

# Tests assert what callers can observe

Two checks for every test: break the behaviour and it must fail; refactor the internals and it must still pass. Derive expected values from the requirement, not from running the code. Pair "never happens" assertions with a case where it does happen. Assert on error types callers branch on, not message wording.
Smell: tests that mock internals and mirror the code line for line.
