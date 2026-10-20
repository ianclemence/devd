---
name: architect
description: Sketch types, signatures and boundaries before writing implementation.
kind: technique
roles: architect,planner,implementer
---

# Architect

1. **Ground.** Understand every system the change touches with `how`; if ownership or layering changes, apply `why` to the current design.
2. **Sketch.** Write the public shapes first: types, function signatures, module boundaries, with placeholder bodies and the caller's usage example at the top. Produce at least two structurally different shapes for anything non-trivial, and prefer the one that hides more complexity behind a smaller interface (`principle-boundary-discipline`).
3. **Check.** Look for red flags: shallow modules that only forward calls, the same decision made in several places, callers needing internal knowledge, leaked transport types.
4. **Implement** against the sketch. A deviation is a signal to bring back to the design, not to absorb quietly.
5. **Scrap** the design when the work keeps needing workarounds. Re-ground and make the new design smaller than the old.
