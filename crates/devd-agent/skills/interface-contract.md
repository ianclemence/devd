---
name: interface-contract
description: Write a result as Markdown the window renders, using the shared tables, code, diffs and status words
kind: build
roles: planner,architect,researcher,implementer,qa,reviewer,frontend,backend,mobile,data,designer,docs,infra,security
---

# The interface contract

A conversation turn is rendered by devd, not by a browser. You choose the
content; the window chooses the form. Call `design.manifest` when you need the
exact list.

## What you may write

- headings, paragraphs, ordered and unordered lists
- Markdown tables with a header row
- fenced code with a language name (`php`, `rust`, `ts`, `sh`, ...)
- fenced `diff` for a patch you applied or read
- blockquotes
- public `http` and `https` links, inline code, strong and emphasis
- maths as `$...$` inline and `$$...$$` on its own lines
- a fenced `mermaid` diagram for a flow or a structure
- declared visuals: a fenced block named `chart`, `metrics`, `steps`, `timeline`
  or `details` holding JSON. The window checks the data and draws it; data that
  does not check stays a code block.

Everything else is escaped as text. There is no HTML path.

## Declared visuals

```chart
{"title":"Time to set up a worktree","subtitle":"median of 42 runs","unit":"s",
 "bars":[{"label":"macOS","note":"before","value":38.6,"color":"red"},
         {"label":"Linux","note":"after the cache fix","value":3.8,"color":"green"}],
 "takeaway":"Linux is 10x faster","source":"pnpm install, fresh worktree"}
```

- `metrics`: `{"items":[{"label","value","unit?","delta?","detail?","color?"}],"lower_is_better?"}`;
  a delta starting with + is green and with - is red (the other way round when
  lower is better). For one number where lower is better in a mixed strip, set
  `"delta_tone": "success"` or `"error"` on that item.
- `steps`: `{"title?","items":["Plan","Build","Test"],"current":1}`.
- `timeline`: `{"items":[{"at?","text","detail?","tone?"}]}`.
- `details`: `{"title","hint?","body":"Markdown"}`, folded until opened.

A colour is one of red, orange, yellow, green, teal, blue, violet, pink, or a
tone: success, warning, error, info. Give it a meaning and keep it: green is
better, red is worse, one colour per series.

## Choose the shape

| You have | Write |
|---|---|
| numbers to compare | a `chart` with a takeaway |
| a few headline numbers | `metrics` |
| where a process stands | `steps` |
| what happened, in order | `timeline` |
| named columns | a table |
| a command, file or patch | a fenced code block |
| a sequence | an ordered list |
| a decision that changes the work | one sentence, then the detail |
| a long report | the answer first; the window folds the rest |

## Never

Do not emit raw HTML, script, style, iframe, form controls, canvas, external
widgets, `javascript:` or `data:` URLs, or your own CSS. Do not invent buttons:
a decision is asked through the approval or inbox capability, and the window
draws the real control.

## Words before colour

Say "the check failed" or "all 72 tests passed". A status is a word; the dot
beside it is decoration. Never make a colour the only signal.

## What this protects

Generated controls would bypass the permission, approval, evidence and audit
model. The renderer keeps an agent's output data, and the application keeps the
authority to act.
