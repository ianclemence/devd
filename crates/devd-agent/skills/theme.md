---
name: theme
description: Make a theme of your own, or change one, from a description of how it should look
kind: design
roles: designer,frontend
---

# Theme

A theme is values, not code: a background, a text colour, an accent and a picture. devd derives every other colour
and refuses a theme that cannot be read.

## When to use

- The person asks for a theme, a colour scheme, a look or a mood for the window, or to change one part of it.
- A theme in Settings cannot be used and the person asks to fix it.

## Procedure

1. Call `theme.list` for the themes there are and the one in use.
2. Changing one: call `theme.check` with its `id`, change only what was asked, save with `replace: true`. Built-in
   themes (sling, dawn, psalm, sharon, daybreak) cannot be edited: make a new one from their colours and say so.
3. Making one, give `background`, `text`, `accent` (`#rrggbb`) and a `scene`. Add other colours only when asked.
   The scenes, and there are no others:
   - `ripples`: fine rolling lines (water); `pasture`: a low sun over dunes (dusk, desert, hills)
   - `silk`: ribbons of light (night sky, northern lights); `contours`: map lines (terrain, paper)
   - `rays`: beams from a corner (sunlight, neon); `none`
   - `glow` and `mesh`: plain soft colour, only when that is what was asked
4. Add `feel` (a few words), a `name` of one or two words (never "light" or "dark"), a one-sentence `description`
   and an `id` in lowercase with hyphens. Set `origin` or `seed` only to move or rearrange the picture.
5. Call `theme.save` with `{ id, mod: { name, version, description }, theme: { feel, colours, backdrop } }`.
6. If refused, fix every problem named in one pass, moving the colour the person cared least about.
7. Call `theme.use`, then say its name, that it is in Settings under Your themes, and why it reads well.

## Make it good, not just readable

Required unless the person asks otherwise.

- **One hue for the room.** Surfaces, quiet text, `ink` and the picture share the background's hue. Give three
  colours and let devd derive the rest.
- **One accent, used little.** Never use it as `ink`, as a wash or across the background.
- **Never lay a warm light over a cool ground.** Amber, orange or red over green or blue turns to mud. `ink` is a
  lighter step of the background; only `glint`, drawn as thin highlights, may carry the accent.
- **Dark means deep:** 6 to 12 percent lightness. Light means near white, 95 to 98 percent, faintly tinted.
- **Text** is off-white or near-black with a touch of the room's hue.
- **No `wash`** unless a gradient is asked for; then both ends stay close to the background.
- **Leave `strength` at 1,** 0.7 for `glow` and `mesh`.

| Asked for | background, text, accent | scene, ink |
| --- | --- | --- |
| forest at dusk, amber | `#0d1712` `#e4ece4` `#e0a458` | `pasture`, `#5f8a70` |
| deep ocean | `#08131f` `#e3edf5` `#4fc3d9` | `ripples`, `#4a7fa8` |
| warm paper | `#f6f0e4` `#2c241b` `#9a4f17` | `contours`, `#8c7355` |
| neon night | `#0f0b1c` `#ece8fb` `#ff4fa3` | `rays`, `#6d5bd0` |

## Verification

- `theme.save` answered with `contrast`, and `theme.use` answered that the window was asked to wear it.
- For a change, `theme.check` shows the new value and every other value as it was.

## Pitfalls

- Overwriting a theme the person did not mean to change: a refusal for an existing folder means check which one.
- Repairing a refusal by draining the colour out: deepen or brighten the accent instead.
- Writing theme files with file tools: only `theme.save` checks them.
- Changing more than was asked.
