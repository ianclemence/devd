---
name: defects-that-keep-coming-back
description: Check a change against the ten defects reviewers find over and over
kind: review
roles: reviewer,qa,implementer,architect
---

# Defects that keep coming back

Most of what a review finds is not new. Across hundreds of reviewed changes the same few defects return, and authors agree with them on sight. Check for these before writing, and again before saying a change is done. This is a checklist of causes, not a style guide; for the whole review use `review-the-diff`.

## When to use

Before you write code that retries, caches, runs something in the background, saves state, or takes input. Again when reviewing such a change. Not for naming or formatting.

## Procedure

For each rule, find the place in the change it applies to and show the case, or say it does not apply.

1. **A caught failure is kept or passed on.** Never catch an error, log it and return success. Never turn a failure into an empty result ("no rows" when the read failed). Never delete the record an operation depends on before its retry worked. Look for every `Err(_)`, `.ok()`, `unwrap_or_default()` and empty `catch`; each needs a reason in the code.
2. **A late answer is checked against the newest question.** A fetch or probe that finishes after the thing it was for changed must not write its result. Give each a generation or a token and compare before writing. Also clear derived state on every path that removes the original: deleted, renamed, refreshed, restarted.
3. **A test fails when the guard is removed.** Delete the fix, run the test, and see it fail; if it passes it protects nothing. Bind it to the real code path, not a copy written for the test. Give a pure rule a table of every combination of inputs.
4. **Everything has a bound.** Captured output, loops, retries, queues, and whole process trees. A retry needs a growing wait and an end state, and a failure to contain a process is an error, not a warning. Check what happens at zero and at the limit.
5. **One action is one write.** If a single user action becomes several saves, a failure halfway leaves torn state. Write one snapshot in one transaction, or order the writes so every prefix is valid and the rest is retried.
6. **A guard never removes the only way back.** Before hiding a button or blocking a state, ask what the person does if the state it assumed is wrong. If the answer is nothing, the guard is the bug.
7. **Every way in is tested.** Keyboard, pointer, a shortcut, an agent's call, a paste. Modifier keys and releasing focus are where the paths drift apart. New interface also needs a label a screen reader can speak, one owner for each action.
8. **Many at once.** After a restart or a network drop every client reconnects in the same seconds. Count the reads each makes, jitter the start, back off with a cap, and prefer a push with a slow backstop to a fast poll. Three agents on one project are a crowd.
9. **Unknown is not a verdict.** A failed read is not "offline", "empty" or "none". Show it as unknown and say what was tried, and never let a stale good value hide a settled failure.
10. **The claim matches the evidence.** "Fixed", "passing" and "safe" each need a result you can point to. Report what you did not check.

## Pitfalls

- Treating a rule as met because the code looks careful. Run the case.
- Fixing the instance and not the class: when one swallowed error is found, search for its siblings.
- Adding a retry to hide a fault. A retry is for a failure that can pass, with a limit.
- Reporting all ten. Report the ones that apply, with the case that shows each.

## Verification

For each rule that applies: the file and line, the input that triggers it, what goes wrong, and the smallest fix. For each that does not apply, one line saying why. Name the rules you could not check.
