---
name: hyperframes
description: Make a video by writing HTML and CSS, preview it, and render a deterministic MP4
kind: build
roles: planner,designer,frontend,docs,implementer,researcher
---

# HyperFrames

A film is a page on a timeline. Write HTML and CSS, preview it in a browser, then
render frames. This is for a launch film, a pull-request walkthrough, captions or
a short chart sequence - not for a screen recording of the window.

## Work

1. Decide the running time and the beats before writing markup. One idea per
   beat; the cut is the sentence break.
2. Write one HTML file with a fixed frame size (1600x1000 for desktop). Use the
   product's real surfaces and type; do not invent a brand.
3. Keep animation in CSS where it is a transition, and drive frame-dependent
   movement from the timeline the tool provides.
4. Preview in the browser at the real size. Check the first frame, the middle and
   the last. No control may be left hovered or focused in a still.
5. Render the MP4 and watch it once. Report the duration, size and every line of
   text that appears.

## Rules

- No third-party music, footage or logos. Only the product's own screens.
- The pointer is not in frame; do not fake a cursor unless the beat is about
  clicking something, and then show the real state change.
- If a frame says a number, it must come from the run's record, not an estimate.
- Do not claim a render passed without opening the file.
