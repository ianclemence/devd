---
name: write-the-project-guide
description: Write a project's AGENTS.md: only what a newcomer would get wrong
kind: discover
roles: planner,architect,researcher,docs,implementer
---

# Write the project guide

A project's guide (`AGENTS.md`) is read at the start of every conversation about the project, so every line costs tokens on every turn and is obeyed as if the person wrote it. Write the lines that earn that.

## When to use

A project with no guide, a guide that is stale or bloated, or when agents keep making the same project-specific mistake.

## Procedure

1. **Find out from the project, not from habit.** `project.detect` and `project.tasks` for the real commands; run the test command once so the guide says what passes. `git.log` for the last 30 commits: the commit style, what breaks, what is reverted. Read the existing guide and rules files (`.cursorrules`, `.devd/instructions.md`) and fix and trim them rather than stacking a second copy.
2. **Include:** how to run it, and the one command that proves a change works; constraints invisible in the code (a generated folder never edited by hand, a rule about the test database, the reason for a strange-looking pattern); traps (the silent failure, the files that change together); conventions that differ from the language default; what not to do, with the reason in a clause.
3. **Leave out:** a description of each folder, a framework summary, anything a compiler, linter or formatter already enforces, advice that fits any project, status or plans, and a paragraph where a line would do.
4. **Shape:** a screen or two. Put what people get wrong most first. Commands in code.
5. **Check:** read it as a stranger; cut every line that would not change what they do. Then run its commands as written.

## Pitfalls

- A command in the guide that does not work. That is worse than no guide.
- A tour of the repository.

## Verification

You ran every command in the guide, and each line passes the test: would it change what a newcomer does?
