---
name: theme
description: Make a theme of your own, or change one, from a description of how it should look
kind: design
roles: designer,frontend
---

# Theme

Makes or changes a theme for the devd window from what the person says: "cream paper and brown ink",
"make the accent greener", "a darker background", "use the contour picture". A theme is values, not
code: three colours (background, text, accent), any others the person asks for, and a picture. devd
works out every other colour and refuses a theme that cannot be read, so the job is to choose well and
to listen to the check.

## When to use

- The person asks for a theme, a colour scheme, a look, or a mood for the window.
- The person asks to change how the window looks: a colour, the contrast, the picture, the name, or the
  background ("make the background a night sky", "a sunrise gradient", "move the picture to the left").
- A theme in Settings cannot be used and the person asks to fix it.

## Procedure

1. **Find out what is there.** Call `theme.list`. It names every theme in the mods folder, which can be
   used, what is wrong with the rest, and which theme the window is wearing (`in_use`).
2. **Changing a theme?** "This theme", "the accent", "make it warmer" mean the theme in use when that is
   `mod:<folder>`. Call `theme.check` with that `id` and work from its `values`: change only what was asked
   for, keep every other value as it is, and save with `replace: true`. When the window wears a built-in
   theme (sling, dawn, psalm, sharon, daybreak), those cannot be edited: make a new theme that starts from
   its colours and say so.
3. **Making a theme?** Turn the description into:
   - `background`, `text` and `accent`, as `#rrggbb`. One accent only: it marks where you are and what
     you are about to do, so pick a colour with character that is not used for anything else.
   - `muted`, `faint`, `surface`, `raised`, `ok`, `warn`, `danger` or `info` only when the person asked for
     them or the description needs them. Left out, devd derives them in the proportions its own themes use.
   - a `scene` for the picture. These are all there are; any other name is refused. Each is drawn smoothly across
     the whole window: `ripples` (thin rings over a soft light, upper right), `pasture` (a low sun over layered
     dunes, along the bottom), `silk` (aurora ribbons of light across the top), `contours` (a map's fine contour
     lines), `rays` (beams of light from a corner), `glow` (a few soft pools of light), `mesh` (colour blended
     across the whole window) or `none`. Pick by the picture the person described: a dusk, desert or hills is
     `pasture`; night sky or northern lights is `silk`; water is `ripples`; terrain or paper is `contours`; sunlight
     or neon is `rays`. `glow` and `mesh` are for a person who asks for plain soft colour and nothing else.
     `ink` and `glint` colour the picture; `strength` runs from 0 to 2.
   - the placement, only when the person asks: `origin` ("top-right", "top-left", "bottom-right", "bottom-left") and
     `seed` (any whole number, for another arrangement of the same scene).
   - a `feel` of a few words ("Cream and umber"), a `name` of a word or two, and a one-sentence
     `description` of how it looks. Never use "light" or "dark" in a name: a theme is one look.
   - an `id` for its folder: lowercase letters, digits and hyphens, from the name ("paper", "night-shift").
4. **Save it.** Call `theme.save` with `{ id, mod: { name, version, description }, theme: { feel, colours:
   { background, text, accent, ... }, backdrop: { scene, ... } } }`. Nothing is written unless the theme
   passes the check.
5. **When it is refused,** read every problem. Each names two colours, the contrast they reach, the
   contrast they need and which way to move. Move the colour the person cared least about, keep the
   character they asked for, and save again. Do not stop at the first problem: fix them all in one pass.
6. **Show it.** Call `theme.use` with the `id`. Tell the person the theme is on, what it is called, and
   that it is in Settings under Your themes, where they can switch back. Say in a sentence why it reads
   well (the contrast `theme.save` reports), not a list of hex values.

## Make it good, not just readable

The check only proves the text can be read. These are what make a theme look designed, and every one of them is
required unless the person asks otherwise.

- **One hue for the room.** `background`, `surface`, `raised`, `muted`, `faint`, `ink` and the picture are all one
  hue family. Give only `background`, `text` and `accent` and devd derives the rest in step; do not hand-pick
  nine greens.
- **One accent, used little.** The accent is the only contrasting colour. It marks buttons and where you are. It is
  never the `ink`, never a wash, and never spread across the background.
- **Never mix opposites in the background.** Amber, orange or red light laid over a green or blue ground turns to
  olive or brown mud. `ink` is a lighter step of the background's own hue (a pale sage on a forest green, a
  periwinkle on a navy). Only `glint` may carry the accent, because it is drawn as thin highlights.
- **Dark means deep.** A dark theme's background has very low lightness (roughly 6 to 12 percent) and modest
  saturation. A light theme's is near white (95 to 98 percent) with a faint tint.
- **Text is off-white or near-black** with a touch of the room's hue, never a saturated colour.
- **No `wash` unless the person asks for a gradient.** It covers every screen and is the quickest way to make a
  window look muddy. If asked: both ends within a few lightness points of the background, same hue.
- **Quiet picture.** Leave `strength` at 1. Lower it to 0.7 for `glow` and `mesh`. Never raise it above 1.2.

Worked examples (background, text, accent, scene, ink):

| Asked for | Values |
| --- | --- |
| calm forest at dusk, amber accent | `#0d1712`, `#e4ece4`, `#e0a458`, `pasture`, ink `#5f8a70`, glint `#e0a458` |
| deep ocean | `#08131f`, `#e3edf5`, `#4fc3d9`, `ripples`, ink `#4a7fa8` |
| warm paper and umber ink | `#f6f0e4`, `#2c241b`, `#9a4f17`, `contours`, ink `#8c7355` |
| neon night | `#0f0b1c`, `#ece8fb`, `#ff4fa3`, `rays`, ink `#6d5bd0` |

Before saving, read your values back against these rules and change what breaks one.

## Verification

- `theme.save` answered with the theme's `contrast`, not a refusal, and `theme.list` shows it as usable.
- `theme.use` answered that the window was asked to wear it.
- For a change: `theme.check` on the folder shows the new value, and every value not asked about is as it was.

## Pitfalls

- Overwriting a theme the person did not mean to change. Without `replace: true`, `theme.save` refuses an
  existing folder: that is the moment to check which theme they meant.
- Repairing a refusal by draining all the colour out. A failing accent usually needs to be a little deeper
  or brighter, not grey.
- Writing the files by hand with file tools. Only `theme.save` checks a theme before it is written.
- Changing several things when one was asked for. "Make the accent greener" changes the accent and
  nothing else.
