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

Everything else is escaped as text. There is no HTML path.

## Choose the shape

| You have | Write |
|---|---|
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
