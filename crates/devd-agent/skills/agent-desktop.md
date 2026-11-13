---
name: agent-desktop
description: Operate a desktop app, installer or dialog through its accessibility tree
kind: check
roles: implementer,qa,reviewer,designer,mobile
---

# Agent desktop

Reads and operates the windows of a desktop application with the `agent-desktop` CLI, on Windows, macOS and Linux. This is the person's real screen, so it is the narrowest tool in the box. It is not for web pages: use `agent-browser`, which is faster and touches nothing of theirs.

## When to use

A Tauri or Electron build, a native app, an installer or a dialog that the task is about. Not for anything with a URL.

## Prerequisites

1. `agent-desktop status` through `project.exec` reports the platform, the permissions granted and what can be operated.
2. If it is missing, ask for it with `toolchain.install` and the tool `agent-desktop`. A person approves installs.
3. Read the guide the CLI serves for its own version: `agent-desktop skills get platform` (on Windows, the PowerShell forms) and `agent-desktop <command> --help`. This file does not repeat flags.

## Procedure

1. Snapshot the window to get refs.
2. Act on one ref.
3. Snapshot again and check the change you expected is there. Every command prints one JSON result (`ok`, `data` or `error`); read it rather than assuming.
4. Close what you opened.

## Pitfalls

- Touching anything the task did not name: a password prompt, system settings, another person's app.
- Working around a missing permission (accessibility, screen recording). Stop and ask.
- A bare element ref in PowerShell. Quote it (`'@abc:e1'`): `@name` is PowerShell syntax.
- Typing secrets. If an app asks for a sign-in, stop and ask.

## Verification

Screenshots are saved inside the project and their paths are in the report, with what you did, what the window showed afterwards, and what you did not check.
