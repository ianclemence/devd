---
name: background
description: Change the picture or colour behind the screens from a description, like a night sky
kind: design
roles: designer,frontend
---

# Background

Changes what is behind the screens of the devd window from what the person describes: "a night sky",
"deep ocean", "warm sunrise", "blueprint", "something calm". It is the `theme` skill, aimed at the
background only: read `theme` for how a theme is made and saved, and change nothing else.

## When to use

- The person describes what they want behind the screens: a sky, a sea, a gradient, a mood, a corner.
- The person says "try another" about a background, or asks to move, soften or strengthen it.

## Procedure

1. Call `theme.list` and `theme.check` on the theme in use. If it is a built-in one (sling, dawn, psalm,
   sharon, daybreak), make a new theme from its colours and say so; built-ins cannot be edited.
2. Turn the description into a picture and a wash:
   - the **scene** that is that picture: `stars` (night sky), `aurora` (northern lights, a curtain of
     colour), `waves` (sea, water), `grid` (blueprint, drawing paper, technical), `pasture` (sunrise,
     hills), `silk` (soft, cloth), `contours` (a map), `rays` (morning light), `ripples` (still water),
     `none` (plain);
   - its **origin** ("top-right", "top-left", "bottom-right", "bottom-left") and a **seed** (any whole
     number) for a different arrangement of the same scene;
   - the **ink** and **glint** colours of its dots, and **strength** from 0 to 2 for how loud it is;
   - a **wash**, `{ from, to, angle }`, when the person wants a colour across the whole window: two colours
     close to the theme's background and a direction in degrees.
3. Keep `colours` as they are unless the check says the text cannot be read on the new background.
4. Save with `theme.save` and `replace: true`, then `theme.use`. If it is refused, read every problem, fix
   them all in one pass, and save again.
5. Say in a sentence what is behind the screens now, and that "try another" means a new seed.

## Verification

- `theme.save` answered with the theme's contrast, not a refusal, and `theme.use` answered that the window was asked to wear it.
- `theme.check` on the folder shows the new backdrop, and the colours are as they were unless the check required a change.

## Pitfalls

- Changing the colours when only the background was asked about.
- A wash so bright or so different from the background that the text is lost: the check refuses it, but a
  wash that only just passes still reads badly. Keep it close to the background.
- Writing files by hand. Only `theme.save` checks a theme before it is written.
