# Themes of your own

devd ships five themes: Sling, Dawn, Psalm, Sharon and Daybreak. Each is a whole look, with its own
colours and its own picture behind every screen. This document describes how a person adds a sixth
look from a file, without rebuilding devd. It is the first part of devd's mods: a mod is a folder
devd reads, and a theme is the first thing a mod can provide.

What this part covers:

* A theme is a folder with two small files. devd reads it, checks it, and shows it in Settings
  beside the built-in themes.
* devd works out the full set of colours from three: the background, the text and the accent.
  Every other colour can be given too, and is then used as written.
* devd refuses a theme it cannot paint readably and says what to change, in numbers and in words.
* The picture behind the screens is one of devd's own scenes, drawn in the theme's colours.
* Saving a file shows the change at once. A theme in use is repainted in place.
* `/theme` in the message box asks the assistant to make one. The assistant writes the theme
  through a capability that checks it first, so it never writes a theme that cannot be used.

What it does not cover yet: mods that run code, places a mod can draw, and restyling what devd
already draws. Those build on this folder and its checks, and are described in a later part.

## A theme is a folder

Themes live in the `mods` folder of devd's home, one folder per theme:

```text
<devd home>/mods/
  paper/
    mod.toml      what the mod is: its name, version and a line about it
    theme.toml    the theme: its colours and its picture
```

The folder's name is the theme's id. It is lowercase letters, digits and hyphens, 1 to 40
characters, such as `paper` or `night-shift`. The saved choice of theme is `mod:paper`, so a mod
can never be mistaken for a built-in theme of the same name.

### `mod.toml`

```toml
name = "Paper"
version = "0.1.0"
description = "Cream paper and umber ink, like a printed manual."
```

| Key | Required | What it is |
| --- | --- | --- |
| `name` | yes | What a person calls the theme. 1 to 32 characters. |
| `version` | no | The mod's own version, shown in Settings. |
| `description` | yes | One sentence about how it looks. Shown under the name on its card. |

### `theme.toml`

```toml
feel = "Cream and umber"

[colours]
background = "#efe6d3"
text = "#2a241d"
accent = "#8a4b14"

[backdrop]
scene = "contours"
```

| Key | Required | What it is |
| --- | --- | --- |
| `feel` | yes | A few words for its colours, the line in bold on its card. |
| `colours.background` | yes | The window behind everything. |
| `colours.text` | yes | Body text. |
| `colours.accent` | yes | The one signal colour: where you are, or what you are about to do. |
| `colours.muted`, `colours.faint` | no | Supporting text, and the faintest text that is still read. |
| `colours.surface`, `colours.raised` | no | Lists and side panels, and cards, inputs and menus. |
| `colours.ok`, `warn`, `danger`, `info` | no | The four state colours. |
| `backdrop.scene` | no | `ripples`, `pasture`, `silk`, `contours`, `rays` or `none`. The default is `ripples`. |
| `backdrop.ink`, `backdrop.glint` | no | The picture's dots, and its brighter dots. The defaults are the text and the accent. |
| `backdrop.strength` | no | How strongly the picture shows, from `0` to `2`. `1` is the scene's own strength. |

Colours are written `#rrggbb` or `#rgb`. `colors` is accepted for `colours`. A key devd does not
know is refused by name, so a spelling mistake is found rather than ignored.

## How the colours are worked out

devd's tokens are the same for every theme: 24 colours, from `bg` to `diff_removed_word`, listed
in [`design.md`](design.md). A theme file gives three and devd derives the rest, following the
same proportions as the built-in themes:

* **Light or dark** is read from the background. A theme is one look, not a pair.
* **Surfaces** are steps from the background towards the text: a lighter step for cards on a pale
  theme, a quieter one on a dark theme.
* **Lines, hover and pressed** are the text colour at a low strength, so they take the theme's tint.
* **Muted and faint text** sit between the text and the background, at the distance the built-in
  themes use.
* **On the accent**, text is white or near-black, whichever reads better.
* **States** use devd's own state colours for a light or a dark theme unless the file gives them.
* **Selection, the soft accent and diffs** are the accent and state colours at fixed strengths.

## The readability check

devd measures contrast the way the Web Content Accessibility Guidelines do, as a ratio from 1:1 to
21:1, and refuses a theme that falls short anywhere a person has to read:

| What | Against | At least |
| --- | --- | --- |
| Text | the background, surfaces and cards | 4.5:1 |
| Muted text | the background | 3:1 |
| Faint text | the background | 2:1 |
| The accent | the background | 2.5:1 |
| Text on the accent | the accent | 4.5:1 |
| Each state colour | the background | 2.5:1 |

The five built-in themes pass the same check; a test keeps it that way. Each problem names the two
colours, the ratio they reach, the ratio they need, and which way to move: for example, "The muted
text is too close to the background: 2.4:1, and it needs at least 3:1. Make the muted text lighter
or the background darker."

## Where a theme appears

* **Settings.** Below the built-in themes, under *Your themes*, each theme from a file has the same
  card: a small window drawn in its colours with its picture behind it, its name, its feel, its
  description and its colours. Choosing it works as choosing a built-in theme does.
* **A theme that cannot be used** is listed there too, with what is wrong with it and a button that
  asks the assistant to fix it. It is never applied.
* **Saving a file** is seen within about a second. A theme in use is repainted in place; a change
  that breaks it keeps the last good version on screen and says why.
* **Removing a folder** that is in use returns the window to Sling.

## Making one with the assistant

`/theme` in the message box loads the theme skill. The assistant turns the description into the
three colours, any others the description asks for, and a scene, then calls `theme.save`:

| Capability | What it does |
| --- | --- |
| `theme.list` | Every theme in the mods folder, with whether it can be used and why not. |
| `theme.check` | Check a theme folder, or a theme given as values, without writing anything. |
| `theme.save` | Check a theme and, only if it passes, write its folder. |
| `theme.use` | Ask the window to wear a theme. |
| `theme.remove` | Remove a theme's folder. Asks first. |

`theme.save` takes values, not file text, and writes the two files itself, so a theme written by
the assistant is always well formed. When the check refuses it, the assistant gets every problem at
once and adjusts the colours before trying again.

## Decisions

* **Data before code.** A theme is values. It cannot run anything, so it needs no permission and
  no approval to be used, and it can be read and checked anywhere devd runs.
* **Three colours in, twenty-four out.** Asking for every token invites a theme that is almost
  right everywhere. Deriving them keeps a file theme in the same proportions as the built-in ones,
  while every token can still be set by hand.
* **Refuse, do not repair.** devd does not quietly lighten a colour that fails the check: the person
  chose it. It says what is wrong and lets the person or the assistant decide.
* **devd's own scenes.** The picture behind a screen is drawn by devd in the theme's colours. A
  picture supplied as a file, and pictures that move, come with the later part of mods.
