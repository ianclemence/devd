---
name: hyperframes
description: Make a video from HTML and CSS, preview it, and render an MP4
kind: build
roles: planner,designer,frontend,docs,implementer,researcher
---

# HyperFrames

A film is a page on a timeline. Write HTML and CSS, preview it in a browser, then render frames. For a launch film, a pull-request walkthrough, captions or a short chart sequence; not for a screen recording of a window.

## When to use

A short explanatory video built from the product's own surfaces.

## Procedure

1. Decide the running time and the beats before writing markup. One idea per beat; the cut is the sentence break.
2. Write one HTML file with a fixed frame size (1600x1000 for desktop). Use the product's real surfaces and type; do not invent a brand.
3. Keep transitions in CSS, and drive frame-dependent movement from the timeline the tool provides.
4. Preview at the real size with `browser.navigate` and `browser.screenshot`. Check the first frame, the middle and the last. Nothing may be left hovered or focused in a still.
5. Render the MP4 and open it once.

## Pitfalls

- Third-party music, footage or logos. Only the product's own screens.
- A faked cursor, unless the beat is about clicking, and then it shows the real state change.
- A number on screen that came from an estimate. It must come from the run's record.

## Verification

Report the duration, the file size and every line of text that appears. Do not say a render passed without opening the file.
