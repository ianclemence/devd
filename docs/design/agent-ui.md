# Agent-generated interface

An agent writes a conversation turn, not a web page. The window renders the
turn; the agent chooses content and the renderer chooses the form.

## What an agent may emit

Markdown, and only these blocks:

- headings, paragraphs, ordered and unordered lists
- tables (a header row; numeric columns align end)
- fenced code with a language name
- a fenced `chart` block holding JSON: `{"title":"...","unit":"...","bars":[{"label":"...","value":12,"tone":"success"}]}`. The application validates it (at most 12 bars, finite non-negative values) and draws the shared bar chart. Invalid data stays a code block.
- blockquotes
- links to public `http` and `https` addresses
- inline code, strong and emphasis

Everything is sanitised by `md()` and styled by the shared classes. Long code
folds; tables become the shared grid; there is no other path into the DOM.

## What an agent must not emit

Raw HTML, `<script>`, `<style>`, `<iframe>`, `<object>`, `<embed>`, `<form>`,
`<input>`, `<canvas>`, `<video>`, inline event handlers, `javascript:` or
`data:` URLs, external stylesheets, fonts or widgets. There is no component
syntax that executes.

## Choosing a shape

| The agent has | It writes |
|---|---|
| A result with named columns | a Markdown table |
| A command or a file | a fenced code block with its language |
| A comparison | a table, or two short lists |
| A sequence | an ordered list |
| A warning that changes the decision | a paragraph beginning with the word, then the detail |
| A patch it applied | a fenced `diff` block |
| A long report | a short answer first, then the detail, because the turn folds long work |

Do not invent status glyphs or colour words. Say "the check failed" or "all 72
tests passed"; the renderer carries meaning that a colour alone cannot.

## Components the application owns

Buttons, fields, toggles, drawers, modals, progress, steps and timelines are
application components. An agent does not generate them. An agent that needs a
decision writes the question and calls the approval or inbox capability; the
application renders the real control. This is deliberate: generated controls
would bypass the permission, evidence and audit model.

The machine-readable list of what is allowed is `docs/design/manifest.json`,
served to agents as `design.manifest`.
