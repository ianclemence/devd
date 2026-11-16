---
name: ship-what-you-can-prove
description: Finish a recreation on evidence, and say plainly what is missing
kind: ship
roles: recreator,qa,reviewer,release
---

# Ship what you can prove

A recreation is a promise about behaviour. The finish has to be a set of things that were
seen to work, plus an honest list of the things that were not.

## When to use

When the build runs and the check report exists, before reporting the work as done.

## Procedure

1. Read the check report you wrote. For every gap, decide one of three things and write it
   down: fixed and re-checked, accepted and why, or missing. Nothing else is allowed.
2. Name the evidence for each behaviour you claim: the flow you completed on the rebuild and
   the picture that shows it. `recreate.report` gathers them beside the run.
3. Run the project's checks again with `project.run` or `project.verify` and keep the output.
   A passing recreation with a failing suite is not done.
4. State the clean-room limit in the report: what was observed, what was never read, and what
   the person should check themselves.
5. Ask the reviewer to read it before you call it done. Their verdict is the last step, and a
   similarity figure does not replace it.

## Pitfalls

- Calling a gap "polish" to avoid naming it.
- Reporting a feature as working because the screen exists.
- Leaving the mode and the right-to-recreate declaration out of the report; both are part of
  the claim, not paperwork.

## Verification

`recreate.report` names every behaviour with its evidence, the check output is attached, the
unfinished list is present, and a reviewer's verdict is on the run.
