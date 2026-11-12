---
name: agent-desktop
description: Use when the thing to check is a desktop app, installer or dialog, not a web page: read and operate windows through the accessibility tree with the agent-desktop CLI
kind: check
roles: implementer,qa,reviewer,designer,mobile
---

# Agent desktop

For a Tauri or Electron build, a native app, an installer or a dialog. For anything that is a web page, use `agent-browser`: it is faster, it does not touch the person's screen, and it needs nothing installed.

## Set up once

1. `agent-desktop status` through `project.exec` reports the platform, the permissions granted and what can be operated.
2. If it is missing, ask for it with `toolchain.install` and the tool `agent-desktop`. A person approves installs.
3. The CLI serves the guide that matches its own version: `agent-desktop skills get platform` (on Windows, the PowerShell forms) and `agent-desktop <command> --help`. Read it before the first command; this file does not repeat flags.

Every command prints one JSON result (`ok`, `data` or `error`). Read it. Do not assume the action worked.

## The loop

Observe, act, observe. Snapshot the window to get refs, act on one ref, snapshot again, and check the change you expected is there. The second snapshot is the proof.

## Boundaries

- Operate only the application the task is about, started from the project. Not a password prompt, not system settings, not another person's app, not anything the task did not name. This is the person's real screen.
- If the CLI reports a missing permission (accessibility, screen recording), stop and ask. Do not look for a way around it.
- In PowerShell, quote every element ref (`'@abc:e1'`): a bare `@name` is PowerShell syntax.
- Never type secrets. If an app asks for a sign-in, stop and ask.
- Close what you opened. A window left running is the next person's surprise.

## Evidence

Save screenshots inside the project and name the path in your report. Say what you did, what the window showed afterwards, and what you did not check.
