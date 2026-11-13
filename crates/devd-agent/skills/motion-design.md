---
name: motion-design
description: Plan, build and check a film, animation or UI motion, from idea to frames
kind: design
roles: motion,designer,frontend,content,planner
---

# Motion design

How a film is made well: the idea and the words first, then the frames, then looking at what was made. It applies to a launch film, a walkthrough, an explainer and a UI transition. It does not choose the tool: use `hyperframes` for devd's built-in renderer or `remotion-film` for Remotion.

## When to use

Any request to make something that moves and will be watched.

## Procedure

1. **One idea.** State what the film argues in a sentence, and the one question each act answers. If a scene does not answer one, cut it.
2. **Write three documents beside the composition** before building: `DESIGN.md` (the idea, a palette taken from the product, the rules), `SCRIPT.md` (every word that appears), `STORYBOARD.md` (acts, scenes, frame ranges, what is on screen). A stranger should be able to say what the film is from them alone.
3. **Fix the clock.** Pick the frame rate and a tempo (120 BPM at 30 fps makes a beat 15 frames). Keep cue frames in one file the picture and the sound both read, so they cannot drift.
4. **Use real things.** Build from the product's own screens, tokens and real data from a real run: `team.trace` gives a run's tool calls in order, by specialist, with timings, ready to draw. A mock that only looks real is the first thing a viewer distrusts.
5. **Restraint.** One subject per frame. Colour means a state (ok, waiting, fault), never decoration. A near-black ground, one soft light, fine grain; no boxes, grids or ghost numbers behind the subject.
6. **Move with purpose.** Ease in and out; stagger arrivals; hold long enough to read (a line is on screen for as long as it takes to read it twice). Animate only the change the frame is about.
7. **Sound.** Soft and tactile: ticks, clicks, a held chord. No risers, whooshes or stabs. Original or none; no third-party music.
8. **Look at it.** Render stills at the first frame, each scene's middle and last, and every cue. Read them: text cut off, overlaps, dead beats, a link on screen for under a second.

## Pitfalls

- Copy that sounds like a generic promo: aphorisms, superlatives, "Your". Plain, specific, observational captions. Never invent a slogan: use the owner's.
- A raw error shown as a headline. Say it in plain words and use amber, not red, unless it is a fault.
- Trusting that a render exited cleanly. Open the frames.
- Drifting from the brief while iterating. Re-read the three documents before each pass.

## Verification

The three documents exist and match the film; the stills for the checkpoints were opened and read; the report lists every line of text on screen and what you could not check, such as the sound on real speakers.
