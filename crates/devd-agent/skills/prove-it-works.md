---
name: prove-it-works
description: Check the result against the real artifact before you say it is done
kind: check
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

Run the feature. Read the actual value, not a cached copy. Check the process state directly. Inspect the diff. Prefer a script that repeats the check and keep its output visible. If verification fails, check your observation method (stale cache, wrong file, wrong environment) before blaming the system.

"It compiles" and "the tests pass" are not "it works", unless the tests exercise the behaviour. Never report another agent's success without observing the result yourself.

The failure this prevents: a completion message with no command output behind it.
