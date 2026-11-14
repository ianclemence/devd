---
name: init
description: Set up AGENTS.md for a project from what its code and tasks show
kind: build
roles: architect,docs,implementer
---

# Init

Writes the AGENTS.md that tells every agent that works in a project how it is built, run and tested. It
records what the project shows, never what is merely usual for its stack.

## When to use

- The person types `/init`, or asks for project instructions for agents.
- A project has no AGENTS.md, or the one it has is out of date or disagrees with the code.

## Procedure

1. **Read what exists.** `project_info` and `project_tasks` give the stack and the tasks the project defines.
   Read any README, existing AGENTS.md or CLAUDE.md, and the manifest files. Use `explore` for the layout of a
   large project.
2. **Run the checks once,** so the file names commands that work: the test task, the lint task and the build
   task, each with how long it took and whether it passed.
3. **Ask only what the code cannot say:** conventions that are not written down, things agents must never
   touch, and how releases are made. Use one question with a few options, not a form.
4. **Write the file,** short and specific, in this order: what the project is (one paragraph), how to build,
   test and lint (exact commands), how the code is laid out, conventions agents follow, and what they must not
   do. Keep it under 100 lines. Do not copy the README.
5. **Show the person the file** and what it was based on, and leave any older file as it was until they agree.

## Pitfalls

- Writing commands that were not run. A wrong command in this file is run by every agent after you.
- Describing the stack in general terms. "Uses Laravel" tells an agent nothing it does not know; the folder
  where jobs live and the command that runs them does.
- Overwriting an AGENTS.md the person wrote.

## Verification

- Every command in the file was run, and the file says which of them pass.
- The file is under 100 lines and has no section copied from the README.
- An existing AGENTS.md was merged with or left, never replaced without the person's say.
