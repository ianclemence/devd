# DEVD design system

The shared visual language for the window, the Assistant, agent conversations
and any interface an agent produces.

## Principles

1. **Graphite and one blue.** Surfaces are true neutrals; blue marks the one
   action that matters, where you are, and live work. Status colour appears only
   beside a word or an icon.
2. **Density with air.** A row is 32-60 px and a section is separated by space
   and a hairline, not by another box. Do not wrap a box in a box.
3. **One control vocabulary.** Every action is a `btn` variant. Every field is
   `input`, `select`, `textarea` or a shared choice control. Every collapsed
   thing is a `ui-fold`.
4. **Say the state in words.** Colour is never the only signal: "Healthy",
   "Needs you", "3 queued", "failed" accompany their dot.
5. **Motion explains, never decorates.** 120-220 ms, ease-out, only on arrival,
   press, or a change of place. Everything stops under reduced motion.
6. **Dark only.** There is no theme switch. The page background is
   `--color-base-100` and is a deliberate part of the product.

## Token architecture

Tokens are defined in one authoritative block near the end of
`crates/devd-server/ui/style.css`. Components use the semantic names; the raw
palette is only the source of their values.

### Colour

`primary` / `primary-content`, `secondary` / `secondary-content`, `accent` /
`accent-content`, `neutral` / `neutral-content`, `base-100`, `base-200`,
`base-300`, `base-content`, `info` / `info-content`, `success` /
`success-content`, `warning` / `warning-content`, `error` / `error-content`.

All twenty variables exist. Renderings that predate the system map onto them;
the contract test `components_use_the_semantic_colour_names` fails if a
component rule reaches for the old palette variables directly.

### Shape, size and depth

| Token | Value | Use |
|---|---|---|
| `--radius-selector` | 6 px | checkbox, radio, toggle, small badge |
| `--radius-field` | 8 px | input, select, tab, button |
| `--radius-box` | 12 px | panel, modal, alert, picker |
| `--size-selector` | 14 px | the square of a checkbox or radio |
| `--size-field` | 28 px | the height of a field or button (the shared control height) |
| `--border` | 1 px | every hairline and control edge |
| `--depth` | 0 | no drop shadows by default; 1 is reserved for floating surfaces |
| `--noise` | 0 | no texture; kept so a future surface can opt in |

The existing `--radius` and `--radius-lg` are aliases of field and box so no
rendered radius changed during the migration.

### Type

| Token | Size | Use |
|---|---|---|
| `--text-2xs` | 11 px | metadata, status bar |
| `--text-xs` | 12 px | labels, row sub-lines |
| `--text-sm` | 13 px | body in dense surfaces |
| `--text-md` | 14 px | default body |
| `--text-lg` | 16 px | section lead, notices |
| `--text-xl` | 18 px | page titles |
| `--text-2xl` | 22 px | KPI values |
| `--text-display` | 28 px | Home greeting |

Families: `--sans` for prose and UI, `--mono` for paths, ids, code and numbers.
Weight carries hierarchy before size does: 400 body, 500 controls, 600 headings,
650 only for a KPI value.

### Space and layout

`--s1` 4, `--s2` 8, `--s3` 12, `--s4` 16, `--s5` 24, `--s6` 32, `--s7` 48.
Page edge `--edge`, section gap from `--s5`, row padding `--s3`/`--s4`.

### Motion

`--fast` 120 ms, `--base` 180 ms, `--slow` 220 ms. Easings from the existing
motion block. No animation on a keyboard-driven action or on a high-frequency
surface. `prefers-reduced-motion` removes movement and keeps the fade.

### Layering

`--z-sticky` 10, `--z-popover` 40, `--z-drawer` 60, `--z-modal` 70,
`--z-toast` 80, `--z-palette` 90. A surface never invents a z-index.

## Component architecture

- **Foundation**: tokens, reset, typography, icons, focus ring, scrollbars.
- **Primitives**: `btn`, `input`, `select`, `textarea`, `ds-check`, `ds-radio`,
  `ds-toggle`, `ds-range`, `ds-progress`, `ds-radial`, `ds-kbd`, `viz-steps`,
  `ds-timeline`, `ds-validator`, `pill`, `dot`, `face`, `skel`.
- **Composed**: `panel`, `ui-row`, `summary-band`, `kpi`, `notice`, `toast`,
  `drawer`, `modal`, `picker`, `palette`, `subnav`, `table`.
- **Domain**: `rail`, `ctx-panel`, `statusbar`, `env-cell`, `ops-*`, `site-*`,
  `run-*`, `decision`, `team-*`.
- **Conversation**: `turn`, `turn-who`, `turn-body`, `tool`, `worked`,
  `chat-plan`, `team-note`, `md`, `md-code`, `diff`.
- **Agent content**: the safe subset in `agent-ui.md` and the machine manifest.

A component exists because the same problem is solved in more than one place.
Screen-specific layouts keep their own class and use the foundation.

## Adding a component

1. Add the token or use an existing one. No raw hex, px radius or duration.
2. Add the CSS next to its category, with hover/focus/pressed/disabled states.
3. Apply it on the screen that needs it, and verify it there at the supported widths.
4. Add it to `docs/design/manifest.json` with its props and variants.
5. Add a contract assertion if the rule can drift.
6. Verify at 1600, 1280, 1024, 800 and 390 px, and with reduced motion.

## Agent-generated UI

Agents do not emit arbitrary HTML, CSS, JavaScript, iframes or external widgets.
They emit Markdown and a small, declared set of blocks. The renderer validates
and styles them with this system. The contract and the allowed component list
are in `docs/design/agent-ui.md` and `docs/design/manifest.json`; the running
capability is `design.manifest`.
