---
name: architect
description: Sketch types, signatures and boundaries before writing the implementation
kind: design
roles: architect,planner,implementer
---

# Architect

Settles the shape of a change (public types, function signatures, module boundaries) before the bodies exist, so the implementation has something to be checked against.

## When to use

A change that adds a module, moves ownership, or touches more than one layer. Not for a one-function fix.

## Procedure

1. **Ground.** Understand every system the change touches with `how`. If ownership or layering changes, apply `why` to the current design.
2. **Sketch.** Write the public shapes first, with placeholder bodies and the caller's usage example at the top. For anything non-trivial produce two structurally different shapes, and prefer the one that hides more behind a smaller interface (`validate-at-the-edge`).
3. **Check.** Look for red flags: modules that only forward calls, one decision made in several places, callers needing internal knowledge, transport types leaking inward.
4. **Implement** against the sketch. A deviation goes back to the design; it is not absorbed quietly.
5. **Scrap** the design when the work keeps needing workarounds. Re-ground, and make the new design smaller than the old one.

## Pitfalls

- Drawing boxes with no signatures. A sketch that does not compile in your head is not one.
- Designing for requirements nobody has.

## Verification

The sketch type-checks (stubs compile), the caller's example reads naturally, and the final code matches it or the report says where and why it moved.
