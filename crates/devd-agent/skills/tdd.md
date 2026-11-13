---
name: tdd
description: Write the failing test first when a cheap, local test exists
kind: build
roles: implementer,qa
---

# Test first

Writes the test that fails for the right reason, then the smallest fix. Skip it when the test would be expensive, integration-heavy or unclear.

## When to use

A bug with a clear test path, or when asked.

## Procedure

1. State the intended and current behaviour and the smallest reproduction.
2. Pick the narrowest executable check, following the project's existing test patterns (`test-what-callers-see`).
3. Write the test for the intended behaviour, run it, and confirm it fails for the right reason.
4. Make the smallest production fix and re-run: it passes.

## Pitfalls

- Editing a test to match a wrong implementation, or weakening an assertion without a stated reason.
- A test that fails for a reason other than the bug.
- If a failing test is impractical, use the closest real check (a script, a manual reproduction) and say which.

## Verification

You saw the test fail before the fix and pass after, and the report shows both runs.
