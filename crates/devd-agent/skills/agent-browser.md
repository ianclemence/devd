---
name: agent-browser
description: Use when a page, form or flow has to be seen working: drive devd's own shared browser (browser.open, snapshot, click, type, screenshot, console) and report what the page showed
kind: check
roles: implementer,frontend,qa,reviewer,designer
---

# Agent browser

Reading the code is not proof that a page works; loading it is. devd has a browser built in, the same one the person sees in the Browser pane, so use it. It is private (its own profile, never the person's cookies or windows), reaches `localhost` and `.test` sites, and needs nothing installed.

## The loop

1. `browser.open` once, then `browser.navigate` to the project's own address from `site.list` (a `https://<name>.test`), never a guess. If `browser.open` reports no Chromium-based browser, say so and fall back to the CLI section below.
2. `browser.snapshot`: the page as a tree, with a ref (`e1`, `e2`) on everything you can act on.
3. Act on a ref: `browser.click`, `browser.type`, `browser.key`, `browser.scroll`. Use `browser.mouse` only when the target has no ref (a canvas).
4. `browser.snapshot` again. The second snapshot is the proof the action did what you meant; a click you did not look at is a guess.
5. `browser.console` after anything that should have worked: it lists errors and failed requests the page hid from you.
6. `browser.screenshot` for the report. The picture goes in the Browser pane and to a file; name the path.

## Rules that save turns

- Refs belong to the snapshot that gave them, and any navigation voids them. After a page changes, snapshot before you act, or you will click the wrong thing or nothing.
- One tab id per task, passed to every call. Do not rely on a "current page".
- Wait for the thing, not for time: snapshot until the element you expect is there, rather than sleeping.
- `browser.evaluate` is for reading a value the tree does not show (a computed style, a store). Do not use it to click, fill or navigate: that skips the checks a real person's click goes through, and proves nothing.
- Test the states, not only the happy path: empty, error, loading, the long name, a narrow window.
- Never type real credentials. Use the project's seed or test accounts, and stop and ask when a page wants anything else. A sign-in the task did not name is not yours to complete.

## Report what you saw

The page, the action, the result and the screenshot path. A step that failed is evidence, so report it as it happened, with the console line that explains it. "It works" with no snapshot behind it is not a result.

## When there is no built-in browser

If the machine has no Chromium-based browser, the `agent-browser` CLI does the same loop from a shell. Check it with `agent-browser --version` through `project.exec`; if it is missing, ask for it with `toolchain.install`, then run `agent-browser install` once. Its own guide is the source of truth: `agent-browser skills get core`.
