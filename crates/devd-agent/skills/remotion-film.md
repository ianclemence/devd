---
name: remotion-film
description: Make a video with Remotion: set up, install its official skills, preview, render
kind: build
roles: motion,frontend,designer,content,planner
---

# Remotion film

Remotion makes video from React components. Use it when the brief needs React, 3D, maps, captions, a design system's components, or the project already has it. For a film that markup and CSS can do, `hyperframes` is built in and needs nothing. For what makes a film good use `motion-design`.

## When to use

The project has Remotion, or the film needs what only Remotion has.

## Prerequisites

1. Node.js: `runtime.list` shows what devd manages; ask for it with `runtime.install` if missing.
2. A Remotion project (`package.json` with `remotion`). If none exists, scaffold one in a new folder: `npx create-video@latest --yes --blank --no-tailwind <name>` with `project.exec`, then `npm i`. It downloads packages and a browser, so say so first.
3. **Remotion's official skills** (best practices for markup, captions, maps, rendering, Studio and more) are published at `remotion-dev/skills` and are not shipped inside devd, which respects their repository and keeps them current. A person installs them once with `skills.install` and source `remotion-dev/skills`; the command menu has "Install the Remotion skills". An agent cannot, because a skill changes how every agent works: ask for it. Once installed, `skills.get` reads remotion-best-practices (the router) and then the page it points to with `file`.

## Procedure

1. Read the router, remotion-best-practices, then the page for the task (markup, captions, maps, render).
2. Build the scenes as components driven by the frame. Keep timing in one file read by picture and sound.
3. Preview before rendering: `npx remotion studio <entry> --no-open --port <port>` with `project.exec` in the background, then look with `agent-browser`. Do not render by default.
4. Render stills first: `npx remotion render <entry> <Comp> out/x.png --frames=a,b,c --image-format=png`. Look at them.
5. Render the film: `npx remotion render <entry> <Comp> out/film.mp4 --codec=h264 --crf=16`. It can take many minutes; use a background task.

## Pitfalls

- Remotion's own browser downloads on first use, and its bundled ffmpeg is minimal (some filters are missing). Do not point it at another browser unless asked.
- React ignores a numeric `scale` in a style object (it appends "px"). Use a string.
- A render's success is not the film's. Extract frames from the MP4 with exact seeks (`-ss t -i file -frames:v 1`), not a frame-rate sample, and read them.
- Licensing: Remotion has terms for companies. Read Remotion's license page (its remotion-license skill has the links) before a commercial project, and say so in the report.

## Verification

Stills and the final video were opened and read. Report the composition, size, frame rate, duration, file path, and every line of text on screen.
