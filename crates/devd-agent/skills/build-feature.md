---
name: build-feature
description: Build new behaviour from the data it works on, in small verified steps
kind: build
roles: planner,implementer
---

# Build a feature

1. Start from the data shape and the user-visible outcome: what comes in, what goes out, what the person sees (`model-the-domain`, `design-from-use`).
2. Read how the project already does something similar and follow its conventions. Check whether the feature is mostly deleting or reusing before you add (`make-the-narrowest-change`).
3. Cut the work into units that can each be checked on their own (`land-one-unit-at-a-time`). Do the riskiest unknown first.
4. For each unit: make the change, run the project's check for it, fix until it passes, then continue.
5. Test behaviour through the public interface (`test-what-callers-see`).
6. Finish by exercising the feature itself, not only its tests, and report what you ran.
