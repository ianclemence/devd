---
name: handoff
description: Pause or resume work so nothing is lost between sessions or agents
kind: discover
roles: planner,implementer
---

# Hand off

**Pausing:** leave the working tree in a state that builds. Write down what is done, what is in progress, what is next and what is blocked, as a note (`scratch.write`) and as todos (`todo.create`). Record the commands that verify the state.

**Resuming:** read the note and the todos first, then `git.status` and the recent log to see the real state. Run the project's checks to learn whether it is green, and only then continue. When they disagree, trust the repository over the note (`recall`).
