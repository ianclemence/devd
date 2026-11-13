# Interface rules

These are the rules every screen of the devd window follows. They exist because the opposite of a designed interface is
not an ugly one; it is one made of many small decisions that were each reasonable and were never held against each other.
A change that breaks a rule either fixes the rule here, in the same change, or does not land.

## Hierarchy

- **One thing wins in each region.** A region is a heading, a list, a panel or a dialog. Its most important element is
  the largest or the heaviest, and only one element is.
- **A heading is a title, one line of context, and at most two actions.** Context is plain words in the secondary text
  colour, separated by a middle dot: `cadence · 1.1M tokens`. Not badges, not icons with numbers. Further actions go
  behind a "more" menu.
- **Raw error text never goes in a heading.** An error is a short phrase ("Can't reach DeepSeek"), on a line of its own
  under the heading, with the detail on hover and the action that resolves it (Retry, Open settings, Ask the assistant).
- **A list row is one line of what it is and at most one line of facts.** The facts are words, not pills, unless the fact
  is a state that needs to be found at a glance.

## Colour

- **Colour carries state, and nothing else.** Accent is selected or primary. Green is finished. Amber needs attention.
  Red failed or removes something. Everything else is the text colours of the theme.
- **No decoration in colour**: no coloured stripes on cards, no tinted badges for categories, no gradients.
- **One accent per view**, used sparingly: the primary button, the selected row, the thing happening now.

## Type and space

- **Four sizes**: extra small (facts), small (secondary text), base (body), large (headings). The display size is for the
  greeting on Home only.
- **Three weights**: regular, medium (titles of rows and cards), semibold (page headings).
- **Tabular numbers** wherever a value changes while it is on screen: counts, durations, tokens, money.
- **Spacing is a scale**: 2, 4, 6, 8, 10, 12, 16, 20, 24, 32. A value between two steps is a mistake.
- **A mark sits level with the first line of its text.** A dot, a glyph or an icon beside a sentence is centred in a box
  as tall as one line of that sentence, so it stays level whether the sentence takes one line or three.

## Components

- One button, one icon button, one row, one chip, one pill, one menu, one tooltip. Screen code composes them and does not
  restyle them.
- **Every icon-only control has a tooltip** that says what it does, and a visible focus ring.
- **Every interactive element shows hover, press and focus.** A hover changes the background at once; nothing waits.

## Motion

- Something that arrives settles into place (a fade with a few pixels of rise, `ease-out`, 140 ms; a panel or dialog,
  200 ms). Something positioned from a corner or an anchor fades without moving.
- Nothing animates that a person does a hundred times a day: typing, switching tabs, scrolling a list.
- Motion never delays a result. If the data is there, it is shown.

## Behaviour

- **Every popover and menu closes on Escape and on a press outside it**, and focus returns to what opened it.
- **Destructive actions confirm.** The confirmation says what goes and what stays. Reversible actions do not confirm.
- **Errors say what to do next**, with a button that does it.
- **Every place has an address** (`devd://…`) and every action is in the command menu, under the same name the
  assistant and the command line use.
- **The assistant can do what the screen does.** If a person can press it, there is a capability for it, described the
  same way.

## Words

- Plain words, in sentences, in the second person where a person is addressed. No jargon where a word will do.
- Name what happened and what to do, not the mechanism: "Can't reach DeepSeek. Retry" rather than "os error 10060".
- A count says what it counts: "3 of 11 tasks done", not "3/11".
