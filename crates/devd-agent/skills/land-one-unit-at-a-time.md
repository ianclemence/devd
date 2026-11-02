---
name: land-one-unit-at-a-time
description: Split work into small steps that each end in a state you can check
kind: build
roles: planner,implementer
---

Do not start the next unit until the current one passes. A failure caught where it was introduced is cheap; one found after a large batch is buried. Order commits so that the sequence shows the work: failing test then fix; baseline then change.

The failure this prevents: ten edits, then the first test run.
