---
name: tdd
description: Write the failing test first when a cheap, local test exists.
kind: technique
roles: implementer,qa
---

# Test first

Use for a bug with a clear test path or when asked. Skip when the test would be expensive, integration-heavy or unclear.

1. State intended versus current behaviour and the smallest reproduction.
2. Pick the narrowest executable check, following the project's existing test patterns.
3. Write the test for the intended behaviour, run it, and confirm it fails for the right reason.
4. Make the smallest production fix. Re-run: it passes.
5. Never edit a test to match a wrong implementation or weaken an assertion without a stated reason. If a failing test is impractical, use the closest real check (a script, a manual reproduction) and say which.
