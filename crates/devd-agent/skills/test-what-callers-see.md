---
name: test-what-callers-see
description: Test observable behaviour so a refactor does not break tests and a bug does
kind: check
roles: implementer,qa,reviewer
---

# Test what callers see

A good test fails when behaviour breaks and passes when only the internals change.

## When to use

Writing or reviewing any test.

## Procedure

1. Break the behaviour: the test must fail. Refactor the internals: it must still pass.
2. Derive expected values from the requirement, not from running the code.
3. Pair every "never happens" assertion with a case where it does happen.
4. Assert on the error types callers branch on, not on message wording.

## Pitfalls

- Tests that mock internals and mirror the code line for line.
- An expected value copied from the output of the code under test.

## Verification

You have seen the test fail against a deliberately broken version of the behaviour.
