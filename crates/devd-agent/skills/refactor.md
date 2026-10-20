---
name: refactor
description: Change structure without changing behaviour, proving behaviour did not change.
kind: playbook
roles: implementer,architect
---

# Refactor

1. Pin the current behaviour first: run the existing tests; if coverage is thin, write characterisation tests that record what the code does today.
2. Work in steps that each leave everything green. Never mix a refactor with a behaviour change in the same step.
3. Move callers to the new shape, then delete the old one (`principle-migrate-callers-then-delete-legacy-apis`). Do not leave both.
4. Re-run the same checks after every step. A red step is reverted or fixed before the next begins.
5. The finished diff should have fewer concepts than before, not more.
