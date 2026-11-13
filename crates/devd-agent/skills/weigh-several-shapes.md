---
name: weigh-several-shapes
description: Sketch at least two structurally different designs before choosing one
kind: design
roles: architect,planner
---

# Weigh several shapes

The first idea is usually the one that came to mind first, not the best. Comparing on paper is cheap; changing in code is not.

## When to use

Any non-trivial design, before committing to it.

## Procedure

1. Produce at least two structurally different options (different ownership or layering, not two spellings of one).
2. Compare them on what the caller sees, what changes later, and what breaks.
3. Say why you picked one.

## Pitfalls

- Two options that differ only in naming.
- A straw alternative.

## Verification

The report names the rejected shape and the reason.
