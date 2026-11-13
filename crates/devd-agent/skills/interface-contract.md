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
3. **Drawn answers.** For a table, a comparison, a set of numbers, steps or a choice, a fenced block named `devd-ui` holding ONE JSON object: a `Card` containing components such as `Table`, `Chart`, `Metrics`, `Callout`, `Fields`, `Steps`, `Timeline`, `Code`, `Diff`, `FileList` and `Actions` with `Button`s. The exact components, their props and the limits are in the instructions devd gives you under "Drawing with components"; they come from the same catalog the window checks against. Put the root first so it appears while you write, and always say in words what the drawing shows. A drawing that does not check is sent back to you with each problem; write it again within the catalog or say it in words.
4. **Colour** is one of red, orange, yellow, green, teal, blue, violet, pink, or a tone (success, warning, error, info). Give it a meaning and keep it: green better, red worse, one colour per series.
5. **Choose the shape.** Numbers to compare: a `chart` with a takeaway. A few headline numbers: `metrics`. Where a process stands: `steps`. What happened, in order: `timeline`. Named columns: a table. A command, file or patch: a code block. A long report: the answer first, and the window folds the rest.

## Pitfalls

- Raw HTML, script, style, iframe, form controls, canvas, external widgets, `javascript:` or `data:` URLs, or your own CSS. All are escaped as text.
- Buttons outside the catalog. A `Button` may only reply, open a devd page or file, or run a real capability by id; the person presses it and, unless it only reads, confirms what it will run. A decision that needs approval is still asked through the approval or inbox capability.
- A status carried by colour alone. Say "the check failed" or "all 72 tests passed"; the dot is decoration.

## Verification

Every visual block parses as JSON with the fields above, and the answer is understandable with the visuals removed.
