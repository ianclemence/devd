---
name: agent-browser
description: Drive devd's shared browser to see a page, form or flow actually work
kind: check
roles: implementer,frontend,qa,reviewer,designer
---

# Agent browser

Loads a page in devd's own browser, acts on it and reports what it showed. Reading code is not proof that a page works; loading it is. This is the same browser the person sees in the Browser pane: private, its own profile, able to reach `localhost` and `.test` sites, and nothing to install.

## When to use

- A page should render, a form should submit, a flow should complete.
- You need a screenshot for the report, or the console output of a page.
- Not for a desktop app (`agent-desktop`) and not when you only need to read a page's text (`web.open`).

## Procedure

1. `browser.open` once, then `browser.navigate` to the project's own address from `site.list`, never a guess. If it reports no Chromium-based browser, see the last section.
2. `browser.snapshot`: the page as a tree, with a ref (`e1`, `e2`) on everything you can act on.
3. Act on a ref with `browser.click`, `browser.type`, `browser.key` or `browser.scroll`. Use `browser.mouse` only for a target with no ref, such as a canvas.
4. `browser.snapshot` again. The second snapshot is the proof the action did what you meant.
5. `browser.console` after anything that should have worked: it lists the errors and failed requests the page hid.
6. `browser.screenshot` for the report. It is written to a file and shown in the Browser pane; name the path.

## Pitfalls

- A ref belongs to the snapshot that gave it, and any navigation voids it. Snapshot again before acting on a changed page.
- Passing no tab id. Use one tab id per task, on every call.
- Sleeping instead of waiting. Snapshot until the element you expect is there.
- `browser.evaluate` to click or fill. It skips what a real click goes through. Use it to read a value the tree does not show.
- Testing only the happy path. Check empty, error, loading, a long name and a narrow window.
- Typing real credentials, or finishing a sign-in the task did not name. Use the project's seed accounts and ask for anything else.

## Verification

The report names the page, the action, what the second snapshot showed and the screenshot path. A failed step is reported as it happened, with the console line that explains it. "It works" with no snapshot behind it is not a result.

## No built-in browser

If the machine has no Chromium-based browser, the `agent-browser` CLI does the same loop from a shell. Check `agent-browser --version` with `project.exec`; if missing, ask for it with `toolchain.install`, then run `agent-browser install` once. Its own guide is the source of truth: `agent-browser skills get core`.
