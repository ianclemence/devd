# Component reference review

This records the decision for every component in the reference inventory. The
official documentation pages were fetched and their examples, variants, sizes
and states were read before deciding. Adopted components were then inspected in
the running application at desktop, tablet and phone widths. Nothing
here is claimed as visually inspected when it was only read.

Decision words:

- **Adopt** - the component belongs in DEVD and uses the shared tokens.
- **Adapt** - the pattern is useful but DEVD needs a different shape.
- **Pattern** - take the composition idea, not a new component.
- **Existing** - DEVD already has an equivalent; consolidate rather than add.
- **Exclude** - not useful in a local development environment; the reason is
  recorded so the decision can be revisited.

## The 68 references

| # | Component | Decision | DEVD component and reason |
|---:|---|---|---|
| 1 | Accordion | Existing | `ui-fold` / native `<details>`. Several can be open; no radio-group exclusivity. |
| 2 | Alert | Existing | `notice` with `notice-ok`, `notice-warn`, `notice-bad`, `notice-info`. Tone is a tint and an icon, never colour alone. |
| 3 | Aura | Exclude | Decorative glow. Contradicts the graphite, no-glow direction. |
| 4 | Avatar | Existing | `face` for agents, `brand-dot` for providers, `req-icon` for status. Sizes `xs`/`sm`. |
| 5 | Badge | Existing | `pill`, `chip`, `fact`, `tag`. Tone variants; no colour-only meaning. |
| 6 | Breadcrumbs | Existing | `crumbs` in the top of a page and `picker-crumbs` in the folder picker. |
| 7 | Button | Existing | `btn` plus `btn-primary`, `btn-go`, `btn-ghost`, `btn-mini`, `btn-icon`, `btn-danger`; pressed, disabled and loading states. |
| 8 | Calendar | Adapt | Native `input[type=date]` for fields; no calendar widget. A full date picker is not needed in the current screens. |
| 9 | Card | Existing | `panel` (a section with a hairline) and `ui-row-accent` for a raised row. Cards are not used as a default container. |
| 10 | Carousel | Exclude | No screen needs swipeable marketing slides. Previews are single frames. |
| 11 | Chat | Existing | `turn`, `turn-who`, `turn-body`, `msg-agent`, `msg-user`, `team-note`, `worked`. This is the conversation language. |
| 12 | Checkbox | Adopt | `ds-check` (native input styled with the tokens); `check` for label rows. |
| 13 | Collapse | Existing | `ui-fold`, `run-fold`, `worked`, `brief-body`, `team-note-body`. One arrow language. |
| 14 | Countdown | Exclude | Schedules show the next time as a date sentence, not a ticking clock. |
| 15 | Diff | Existing | `diff`, `diff-line`, `diff-add`, `diff-del`, `diff-badge`; hunks and per-file labels. |
| 16 | Divider | Existing | `hr`, `section-title`, panel hairlines, `sb-sep`. A divider is a line, not a labelled ornament. |
| 17 | Dock | Adapt | The phone rail is a bottom bar with five destinations and a More sheet. Not a macOS-style dock. |
| 18 | Drawer | Existing | `drawer` (right-side sheet) and the phone context panel. One close/escape behaviour. |
| 19 | Dropdown | Existing | `composer-popover`, `more-pop`, `popover`, `select` custom list. Opens from its trigger and flips when space is short. |
| 20 | FAB | Exclude | The composer is the one local action; a floating button would compete with it. |
| 21 | Fieldset | Existing | `field`, `field-label`, `form-grid`; `stack` for vertical groups. |
| 22 | File Input | Existing | `composer-add`, hidden file input, `ATT` chips, upload progress and retry states. |
| 23 | Filter | Existing | `ui-controls` with `ui-search` and `ui-sort`; site/team/activity filters. |
| 24 | Footer | Existing | `statusbar`: health, context and environment, one line. |
| 25 | Hero | Exclude | No marketing hero inside the product. Home leads with work, not a banner. |
| 26 | Hover 3D | Exclude | Decorative tilt; does not help information density. |
| 27 | Hover Gallery | Exclude | No gallery of products or images is part of the workflow. |
| 28 | Indicator | Existing | `rail-badge`, `rail-live`, `rail-dot`, `count`, `kpi` dots. |
| 29 | Input | Existing | `input`, `ui-search`, `mono`; disabled, invalid and focus-visible states. |
| 30 | Join | Existing | `inline`, `segmented`, `btn-group`; inputs and buttons on one line. |
| 31 | KBD | Adopt | `ds-kbd` for shortcuts (Ctrl K, Ctrl F, Enter); also in the palette footer. |
| 32 | Label | Existing | `field-label`, `check`, `sr-only`; label click targets the control. |
| 33 | Link | Existing | `link` for external links, `text-link` for a link-shaped button. |
| 34 | List | Existing | `ui-row`, `uiList`, `panel-body flush`, `rows`; one row height and one action area. |
| 35 | Loading | Existing | `loading`, `skelRows`, `composer-off`, the "Checking..." status. |
| 36 | Mask | Exclude | Decorative shapes for avatars. Agent faces are square-ish marks with a letter. |
| 37 | Megamenu | Exclude | The command palette (Ctrl K) is the large navigation surface; a mega menu would duplicate it. |
| 38 | Menu | Existing | `rail`, `panel-item`, `.more-pop` menu; grouped destinations and active state. |
| 39 | Mockup Browser | Adapt | The Preview tab is a real frame, not a decorative browser chrome. |
| 40 | Mockup Code | Existing | `md-code`, `log-block`, `term`, line prefixes and folding. |
| 41 | Mockup Phone | Exclude | No device mockups in the product. |
| 42 | Mockup Window | Exclude | The desktop window is the real window. |
| 43 | Modal | Existing | `modal` for decisions, `drawer` for detail sheets, `picker` for the folder browser. One overlay and Escape rule. |
| 44 | Navbar | Existing | The rail replaces a top navbar; `page-head` carries the page title and actions. |
| 45 | OTP | Exclude | No multi-factor flow in devd. |
| 46 | Pagination | Existing | `pager` for API and tools, activity and long lists. |
| 47 | Progress | Adopt | `ds-progress` for determinate work; `share` for token shares. |
| 48 | Radial Progress | Adapt | `ds-radial` for a single pass rate or score where a ring is clearer than a bar. |
| 49 | Radio | Adopt | `ds-radio`; used in compact choice rows. |
| 50 | Range | Adopt | `ds-range`; useful for effort/limits where a number field is heavier. |
| 51 | Rating | Exclude | No five-star ratings; quality is shown as verified work, not stars. |
| 52 | Select | Existing | `select` with the app's own popover list; disabled and invalid states. |
| 53 | Skeleton | Existing | `skelRows`, `loading`; shapes match the row they replace. |
| 54 | Stack | Existing | `stack`, `inline`, `composer-dock`; vertical rhythm from the spacing scale. |
| 55 | Stat | Existing | `kpi`, `kpi-row`, `summary-band`, `env-cell`. Label, value, one supporting sentence. |
| 56 | Status | Existing | `uiDot` (`ok`, `warn`, `bad`, `live`, `off`), `health-dot`, `dot`; always beside a word. |
| 57 | Steps | Adopt | `ds-steps` for a short ordered process (setup, run stages); not for every list. |
| 58 | Swap | Exclude | No icon-swap control; the only theme is dark. |
| 59 | Tab | Existing | `subnav`/`subtab` for page tabs, `tabs` for secondary sections. |
| 60 | Table | Existing | `.table`, `.table-wrap`, `.kpi-table`, `.roles-table`, `.mu`; sticky heads and numeric alignment. |
| 61 | Text Rotate | Exclude | Rotating marketing words would be noise in a tool. |
| 62 | Textarea | Existing | `textarea`, `composer-input` with growth and a maximum. |
| 63 | Theme Controller | Exclude | DEVD is dark-only by design; a theme switch is not offered. |
| 64 | Timeline | Existing | `timeline`, `tl-item`, `run-event`, `hrow`; newest first where it is activity. |
| 65 | Toast | Existing | `toast` with `ok`, `warn`, `error`, `info`; one stack, auto-dismiss except errors. |
| 66 | Toggle | Adopt | `ds-toggle` for a real on/off setting; not for a destructive action. |
| 67 | Tooltip | Existing | Native `title` for secondary hints, `aria-label` for icon-only controls. A custom tooltip is not needed. |
| 68 | Validator | Adopt | `ds-validator` hint under a field; inline error text plus `aria-invalid`. |

## Existing DEVD component audit

| Component | Current location | Used on | Reference | Decision | Shared component |
|---|---|---|---|---|---|
| Rail navigation | `app.js` `rail()` | Every screen | Menu / Dock | Keep | `rail`, `rail-item`, `rail-badge` |
| Context panel | `app.js` `contextPanel()` | Every screen | Drawer / Menu | Keep | `ctx-panel`, `panel-item` |
| Page head | `page()` | Every screen | Navbar | Keep | `page-head`, `page-actions` |
| Summary band | `summaryBand()` | Home, Projects, Sites, Environment | Stat | Keep | `summary-band`, `summary-item` |
| Row list | `uiRow()` / `uiList()` | Nearly every list | List | Keep | `ui-row`, `ui-row-accent` |
| Panel section | `panel` | Cards, tables, forms | Card | Keep | `panel`, `panel-head`, `panel-body` |
| Table | `.table` | API, Team, Models, Activity | Table | Keep | `table`, `table-wrap`, `kpi-table` |
| KPI tiles | `teamRecordHtml()` | Team, Usage | Stat | Keep | `kpi`, `kpi-row` |
| Callout | `notice` | Health, errors, warnings | Alert | Keep | `notice-ok/warn/bad/info` |
| Toast | `toast()` | Global | Toast | Keep | `toast`, `toast-ok/warn/error/info` |
| Drawer sheet | `openDrawer()` | Approvals, messages, logs | Drawer | Keep | `drawer`, `drawer-head`, `drawer-body` |
| Decision modal | `uiDialog()` / `askApproval()` | Deletes, approvals | Modal | Keep | `modal`, `modal-head/body/foot` |
| Command palette | `openPalette()` | Ctrl K | Menu / Megamenu | Keep | `palette`, `palette-input`, `palette-item` |
| Composer | `composerHtml()` | Home, Assistant | Chat / Textarea | Keep | `composer`, `composer-input`, `composer-bar` |
| Conversation turn | `paintChat()` | Assistant, runs | Chat | Keep | `turn`, `turn-who`, `turn-body` |
| Tool call card | `toolCard()` | Conversations | Collapse | Keep | `tool`, `tool-head`, `tool-body` |
| Diff viewer | `diffHtml()` | Project card, reviews | Diff | Keep | `diff`, `diff-line` |
| Code block | `md()` / `md-code` | Conversations, docs | Mockup Code | Keep | `md-code`, `log-block`, `term` |
| Plan strip | `paintChatPlan()` | Assistant | Steps | Keep | `chat-plan`, `chat-plan-item` |
| Approval row | `approvalRow()` | Home, Inbox | Alert / Card | Keep | `decision`, `ask-approval` |
| Empty state | `uiEmpty()` | Every list | Alert | Keep | `ui-empty` |
| Skeleton | `skelRows()` | Every load | Skeleton | Keep | `skel`, `loading` |
| Status dot | `uiDot()` | Everywhere | Status | Keep | `dot`, `dot-ok/warn/bad/live/off` |
| Pills and tags | `pill`, `fact`, `tag` | Everywhere | Badge | Keep | `pill`, `fact`, `chip` |
| Tabs | `.subnav` / `.subtab` | Project, site, machine | Tab | Keep | `subnav`, `subtab` |
| Form field | `field` | Settings, modals | Fieldset | Keep | `field`, `field-label`, `input`, `select`, `textarea` |
| Folder picker | `openPicker()` | Projects, Clone, Import | Modal / Menu | Keep | `picker`, `picker-item`, `picker-inspect` |
| Status bar | `statusbar()` | Every screen | Footer | Keep | `statusbar`, `sb-left`, `sb-right` |
| Health chip | `healthChip()` | Status bar | Status | Keep | `health`, `health-title`, `health-sub` |
| Face / identity | `faceHtml()` | Team, conversations | Avatar | Keep | `face`, `face-xs/sm` |
| Share bar | `usageHtml()` | Models | Progress | Keep | `share` |

### Consolidations made in this pass

- The environment page repeated the header's primary "Set up devd" button in the
  empty services block; the repeat was removed.
- Projects showed "no address" while its own tiles counted the project as
  exposed; the row now uses the site name from `project.list`.
- Inbox offered a free-text answer box for a "Plan ready" message; it now offers
  Review the plan, the decision the run is actually waiting for.
- The folder picker and Clone/Import used three different names for the same
  action and the picker listed files that could not be chosen.
- Form submit buttons (Clone, Start a member, Save a rule, Create a schedule)
  accepted clicks on empty forms and answered with a warning; they are now
  disabled until their required fields are filled.
- All component colour references moved to the semantic token names.

## What is deliberately not built

Carousel, hero, hover 3D, hover gallery, FAB, mask, megamenu, mockup phone,
mockup window, OTP, rating, swap, text rotate, theme controller, aura and
countdown are excluded above with reasons. They are not half-built; there is no
component and no dead control.
