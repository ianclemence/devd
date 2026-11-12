---
name: interface-contract
description: Write results as Markdown the window renders, using shared tables and visuals
kind: build
roles: planner,architect,researcher,implementer,qa,reviewer,frontend,backend,mobile,data,designer,docs,infra,security
---

# The interface contract

A conversation turn is rendered by devd, not by a browser. You choose the content; the window chooses the form. Call `design.manifest` for the exact list. Nothing here lets an agent draw a control or run script: output is data, and the application keeps the authority to act.

## When to use

Writing any answer, report or summary that a person will read in the window.

## Procedure

1. **Allowed.** Headings, paragraphs, lists; Markdown tables with a header row; fenced code with a language; fenced `diff` for a patch; blockquotes; public `http` and `https` links; inline code, strong, emphasis; maths as `$...$` and `$$...$$`; a fenced `mermaid` diagram.
2. **Declared visuals.** A fenced block named `chart`, `metrics`, `steps`, `timeline` or `details` holding JSON. The window checks the data and draws it; data that does not check stays a code block.
   - `chart`: `{"title","subtitle?","unit?","bars":[{"label","note?","value","color?"}],"takeaway?","source?"}`
   - `metrics`: `{"items":[{"label","value","unit?","delta?","detail?","color?"}],"lower_is_better?"}`; a delta starting with + is green and - is red (reversed when lower is better).
   - `steps`: `{"title?","items":["Plan","Build","Test"],"current":1}`
   - `timeline`: `{"items":[{"at?","text","detail?","tone?"}]}`
   - `details`: `{"title","hint?","body":"Markdown"}`, folded until opened.
3. **Colour** is one of red, orange, yellow, green, teal, blue, violet, pink, or a tone (success, warning, error, info). Give it a meaning and keep it: green better, red worse, one colour per series.
4. **Choose the shape.** Numbers to compare: a `chart` with a takeaway. A few headline numbers: `metrics`. Where a process stands: `steps`. What happened, in order: `timeline`. Named columns: a table. A command, file or patch: a code block. A long report: the answer first, and the window folds the rest.

## Pitfalls

- Raw HTML, script, style, iframe, form controls, canvas, external widgets, `javascript:` or `data:` URLs, or your own CSS. All are escaped as text.
- Inventing buttons. A decision is asked through the approval or inbox capability and the window draws the real control.
- A status carried by colour alone. Say "the check failed" or "all 72 tests passed"; the dot is decoration.

## Verification

Every visual block parses as JSON with the fields above, and the answer is understandable with the visuals removed.
