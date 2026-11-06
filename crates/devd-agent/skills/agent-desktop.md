---
name: agent-desktop
description: Operate desktop apps through the operating system's accessibility tree - read a window, click, type, take a screenshot - with the agent-desktop CLI
kind: check
roles: implementer,qa,reviewer,designer,mobile
---

# Agent desktop

Use this when the thing to check is a desktop application, not a web page: a Tauri or Electron build of the project, a native app, an installer, a dialog. It reads and operates windows through the accessibility tree on Windows, macOS and Linux.

Adapted for devd from the agent-desktop skill by Vercel Labs (github.com/vercel-labs/agent-desktop, Apache License 2.0).

## Set up once

1. Check it is there: run `agent-desktop status` with `project.exec`. It reports the platform, the permissions it has and what it can operate.
2. If it is missing, ask for it with `toolchain.install` and the tool `agent-desktop` (a person approves installs).

## Load the real guide from the CLI

- `agent-desktop skills get platform` - the guide for the operating system it runs on (on Windows, the PowerShell forms).
- `agent-desktop <command> --help` - flags and syntax. This file does not repeat them.

Every command prints one JSON result (`ok`, `data` or `error`); read it rather than assuming the action worked.

## How to use it in devd

- Operate only the application the task is about, started from the project. Never touch a password prompt, a system settings window, another person's app or anything the task did not name.
- Observe, act, then observe again: the second snapshot is the proof the action did what you meant.
- In PowerShell, quote every element ref (`'@abc:e1'`), because a bare `@name` is PowerShell syntax.
- Save screenshots inside the project and name the path in your report.
- If the CLI reports a missing permission, stop and ask the person; do not try to work around it.
