---
name: goal
description: Keep working until an objective is met, with pause, resume and clear
kind: build
roles: implementer,architect,qa
---

# Goal

Sets an objective on this conversation and keeps working until it is met. While a goal is open, devd does not
let a turn end because the model stopped: it asks whether the objective is met and sends the model back to
work if not. The person can pause, resume or clear it.

## When to use

- The person types `/goal` followed by what they want achieved: "make the tests pass", "migrate every route
  to the new router", "get the page under two seconds".
- The person types `/goal pause`, `/goal resume` or `/goal clear`, or asks to stop, continue or drop the goal.

## Procedure

1. **Read what follows `/goal`.**
   - Words other than `pause`, `resume` or `clear`: that is the objective.
   - `pause`, `resume`, `clear`: call `chat.goal` with that `action`, say what it did in one line, and stop.
   - Nothing: call `chat.goal` with `action: "get"` and say what the goal is and its state, or that there is none.
2. **Make the objective checkable.** If it names an outcome you can test, use it as written. If it is a
   direction ("improve the importer"), turn it into the outcome you will check ("the importer handles the three
   sample files in the repository without errors") and tell the person which you chose.
3. **Set it.** Call `chat.goal` with `action: "set"` and the objective. Then start working toward it; setting a
   goal is not the work.
4. **Work in small, checked steps.** After each change run the check that shows progress: the tests, the build, the
   page. A step with no check is a guess.
5. **When it is met, say so with proof.** Call `goal_done` with the evidence: the command you ran and what it
   printed, or the file you read and what it said. Then answer the person in a few sentences.
6. **When you are blocked,** say exactly what you need from the person and stop. Do not ask whether to continue:
   the goal already says to.

## Pitfalls

- Calling `goal_done` because the work looks finished. It is finished when a check shows it.
- Setting the goal and then stopping to confirm it. The person already asked.
- Widening the objective while working. If it turns out to be a different task, say so and ask.
- Looping on the same failing command. Change the approach, or say what blocks you.

## Verification

- `chat.goal` with `get` shows the objective and its state.
- On success the conversation says "Goal met" and the evidence in `goal_done` names a command or a file.
- On a pause or a stop, the conversation says why, and nothing is left running.
