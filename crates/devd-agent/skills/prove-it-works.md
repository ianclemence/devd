---
name: prove-it-works
description: Check the result against the real artifact before saying it is done
kind: check
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Prove it works

"It compiles" and "the tests pass" are not "it works", unless the tests exercise the behaviour. This is the habit of looking at the real thing before saying so.

## When to use

Before any statement that something is done, fixed, running or working.

## Procedure

1. Run the feature. Read the actual value, not a cached copy.
2. Check process state directly (`service.list`, `task.list`), and inspect the diff (`git.diff`).
3. Prefer a script that repeats the check, and keep its output visible.
4. If verification fails, check your method first: a stale cache, the wrong file, the wrong environment. Then the system.
5. Never report another agent's success without observing the result yourself.

## Pitfalls

- A completion message with no command output behind it.
- Verifying the thing you changed and not the thing the person uses.

## Verification

The report quotes what you ran and what it printed.
