---
name: build-feature
description: Build new behaviour from the data it works on, in small verified steps
kind: build
roles: planner,implementer
---

# Build a feature

Turns a request for new behaviour into working, checked code by starting from the data and the outcome, and landing it in steps that each pass.

## When to use

New behaviour that touches more than one file. For a defect use `fix-bug`; for restructuring without behaviour change use `refactor`.

## Procedure

1. Start from the data shape and the outcome a person sees: what comes in, what goes out (`model-the-domain`, `design-from-use`).
2. Read how the project already does something similar and follow its conventions. Check whether the feature is mostly deleting or reusing before you add (`make-the-narrowest-change`).
3. Cut the work into units that can each be checked alone (`land-one-unit-at-a-time`). Do the riskiest unknown first.
4. For each unit: make the change, run the project's check for it, fix until it passes, then continue.
5. Test behaviour through the public interface (`test-what-callers-see`).
6. Finish by using the feature itself, not only its tests, and walk the other ways in (`finish-it-everywhere`).

## Pitfalls

- Building the interface before the data. The shape of the data decides the rest.
- A new abstraction used once.
- "Done" with the tests passing and the feature never run.

## Verification

You ran the feature and can say what you did and saw; the project's own checks pass on the final state.
