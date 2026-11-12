---
name: hyperframes
description: Make a film from HTML and CSS with devd's built-in renderer
kind: build
roles: motion,planner,designer,frontend,docs,implementer,researcher,content
---

# HyperFrames

A film is a page that is a pure function of time. devd carries the runtime, the scaffold and the renderer, so nothing is installed: `motion.new` starts a composition and `motion.render` photographs it frame by frame in devd's own browser. For React, 3D, maps or captions use `remotion-film`. For what makes a film good use `motion-design`.

## When to use

A launch film, a walkthrough, a chart sequence or a UI animation that can be written as markup and CSS.

## Procedure

1. `motion.new` with an absolute folder, the title, the size and length. It writes `index.html` and `devd-motion.js` and never overwrites a film.
2. Write the scene as a function of time: `Motion.draw(t => ...)` sets the page to exactly its state at second `t`. Helpers: `Motion.progress(t, from, to, easing)`, `Motion.stagger`, `Motion.typed`, `Motion.count`, `Motion.ease.settle`, `Motion.seeded` for randomness that is the same every render.
3. Preview by serving the folder: it loops, Space pauses, the arrows step a frame.
4. Look before you render: `motion.render` with `at: [0, 1.5, ...]` makes stills in `out/stills`. Check the first frame, each scene, the last, and every cue.
5. Render the film: `motion.render` writes `out/frames` and, with ffmpeg on the machine, `out/film.mp4`. Pass `audio` to put a sound file under it.

## Pitfalls

- CSS animations, timers, `requestAnimationFrame` loops or `Math.random` in the composition. Anything that runs on its own clock is caught half way. Compute it from `t`.
- Remote fonts and images. Put them in the folder: the render has no reason to wait for a network.
- A video that is not made because ffmpeg is missing. The frames are still delivered: tell the person, and do not claim a video.
- Rendering the whole film to find a typo. Render stills first.

## Verification

Two renders of the same composition give identical frames. Report the frame count, size, frame rate, the video path or the reason there is none, and every line of text that appears.
