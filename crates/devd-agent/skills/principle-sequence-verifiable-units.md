---
name: principle-sequence-verifiable-units
description: Work in small steps that each end in a checkable state.
kind: principle
roles: planner,implementer
---

# Work in small steps that each end in a checkable state

Do not start the next unit until the current one passes. A failure caught where it was introduced is cheap; one found after a large batch is buried. Order commits so the sequence demonstrates the work: failing test, then fix; baseline, then change.
Smell: ten edits, then the first test run.
