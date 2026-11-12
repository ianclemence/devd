---
name: stageshow
description: Compose product stills, short UI animations and bento layouts
kind: build
roles: designer,frontend,docs,implementer,researcher
---

# Stageshow

A stage is a frame that explains one product moment. It composes the real interface, not a mock of it, and frames it so one idea is legible.

## When to use

A walkthrough, a launch still, a bento of features, a short animation of one change.

## Procedure

1. Pick the moment: the decision, the proof, the handoff. One per frame.
2. Build the frame from the product's own surfaces, spacing and type. Use its components; do not redraw a button.
3. Give the frame a focal point: one panel, row or number. Crop rather than shrink.
4. For a bento, each cell answers a different question, on the same gap as the app.
5. Animate only the change the frame is about: a row arriving, a value settling. Nothing loops.
6. Check at 1600 and 800 wide (`browser.screenshot`). Text no smaller than 12 px; nothing overflows its cell.

## Pitfalls

- A number that looks measured and is invented. Use real data or clearly marked sample data.
- Unrelated logos, gradients, glass or stock imagery.
- An animation that is needed to understand the still.

## Verification

Each still reads without its animation, and every number can be traced to a source.
