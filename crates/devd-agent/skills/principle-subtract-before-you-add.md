---
name: principle-subtract-before-you-add
description: Remove complexity first, then build on the simpler base.
kind: principle
roles: implementer,architect
---

# Remove complexity first, then build on the simpler base

Delete dead code, redundant validation and stub references before adding the feature. Reach the minimum working version before polishing. Design for how the system is used, not for edge cases nobody has seen. Every change should leave the surface the same or smaller.
Smell: validators for impossible inputs, empty placeholder files, polish before it works.
