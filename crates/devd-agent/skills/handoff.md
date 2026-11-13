---
name: handoff
description: Pause or resume work so nothing is lost between sessions or agents
kind: discover
roles: planner,implementer
---

# Hand off

Leaves work in a state someone else, or you tomorrow, can pick up cold, and picks it up safely.

## When to use

Stopping before the work is done, passing it to another agent, or resuming after a gap.

## Procedure

**Pausing**
1. Leave the working tree in a state that builds.
2. Write down what is done, in progress, next and blocked, as a note (`scratch.write`) and as todos (`todo.create`).
3. Record the commands that verify the state.

**Resuming**
1. Read the note and the todos.
2. Check the real state with `git.status` and the recent `git.log`.
3. Run the project's checks to learn whether it is green, and only then continue.

## Pitfalls

- A note that says "mostly working". Say which check passes and which does not.
- Trusting the note over the repository. When they disagree, the repository wins (`recall`).

## Verification

A reader with no context could continue from the note alone; on resuming, you ran the checks before changing anything.
