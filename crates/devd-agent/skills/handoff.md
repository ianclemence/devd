---
name: handoff
description: Pause or resume work so nothing is lost between sessions or agents.
kind: playbook
roles: planner,implementer
---

# Hand off

**Pausing:** leave the working tree in a state that builds. Write what is done, what is in progress, what is next and what is blocked in a note (`scratch.write`) and as todos (`todo.create`). Record the commands that verify the state.

**Resuming:** read the note and todos first, then `git.status` and the recent log to see the real state, run the project's checks to learn whether it is green, and only then continue. Trust the repository over the note when they disagree (`recall`).
