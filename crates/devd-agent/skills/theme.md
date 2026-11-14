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
- The person asks to change how the window looks: a colour, the contrast, the picture, the name.
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
   - a `scene` for the picture: `ripples` (rings on still water), `pasture` (hills under a low sun), `silk`
     (folds catching light), `contours` (a hand-drawn map), `rays` (light fanning from a corner) or `none`.
     `ink` and `glint` colour its dots; leave them out to use the text and the accent. `strength` from 0 to 2.
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
