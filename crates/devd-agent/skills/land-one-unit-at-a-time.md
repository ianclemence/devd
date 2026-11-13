---
name: land-one-unit-at-a-time
description: Split work into small steps that each end in a state you can check
kind: build
roles: planner,implementer
---

# Land one unit at a time

Does not start the next unit until the current one passes. A failure caught where it was introduced is cheap; one found after a large batch is buried.

## When to use

Any work larger than one edit.

## Procedure

1. Cut the work into units that each end in a checkable state.
2. Do the riskiest unknown first.
3. For each unit: change, run the check, fix until it passes, then continue.
4. Order commits so the sequence shows the work: failing test then fix; baseline then change.

## Pitfalls

- Ten edits, then the first test run.
- A unit whose only check is "it compiles".

## Verification

After each unit you can name the check that passed.
