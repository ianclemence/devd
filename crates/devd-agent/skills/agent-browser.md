---
name: agent-browser
description: Drive a real browser to check a web app works - open pages, click, fill forms, read the page, take screenshots - with the agent-browser CLI
kind: check
roles: implementer,frontend,qa,reviewer,designer
---

# Agent browser

Use this when the work has to be seen in a browser to be believed: a page that should render, a form that should submit, a flow a person will click through, a screenshot for the report. Reading the code is not proof that a page works; loading it is.

Adapted for devd from the agent-browser skill by Vercel Labs (github.com/vercel-labs/agent-browser, Apache License 2.0).

## Set up once

1. Check it is there: run `agent-browser --version` with `project.exec`.
2. If it is missing, ask for it with `toolchain.install` and the tool `agent-browser` (a person approves installs). After it installs, run `agent-browser install` once to fetch its browser.

## Load the real guide from the CLI

This file only points the way. The CLI serves the usage guide that matches its own version, so load it before the first command:

- `agent-browser skills get core` - the loop, waiting, common tasks and troubleshooting. Start here.
- `agent-browser skills get dogfood` - exploratory testing and bug hunts.
- `agent-browser skills get electron` - Electron desktop apps.

## How to use it in devd

- Open the project's own address (the site's `https://<name>.test` from `site.list`), not a guess.
- Work in the loop the guide describes: snapshot the page, act on an element by its ref, snapshot again, and check the result you expected is on the page.
- Save screenshots inside the project (for example `storage/screenshots/` or the project's test output folder) and name the path in your report, so the person can open it.
- Report what you saw: the page, the action, the result, and the screenshot path. A failed step is evidence too; report it as it happened.
- Never type real credentials. Use the project's seed or test accounts, and stop to ask when a page wants anything else.
