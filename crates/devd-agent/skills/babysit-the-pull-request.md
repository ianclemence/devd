---
name: babysit-the-pull-request
description: Get an open pull request to green without waking the person for noise
kind: ship
roles: implementer,reviewer,infra
---

# Babysit the pull request

After a pull request is open, brings it to green on its latest commit: fixes what is true, answers what is not, and stays quiet otherwise.

## When to use

Right after opening a pull request, and whenever checks, comments or a conflict arrive on one you opened.

## Procedure

1. **Do not poll.** Do not loop, sleep or re-check on a timer. devd tells you when checks finish, someone comments or a conflict appears. Between events, end your turn.
2. **A failing check.** Read the failing job's log, not only its name, and reproduce it locally with the same command. Fix the cause. If the failure is not yours (a flaky test, an outage), prove it with the same test passing on the base branch or on a re-run, and say so.
3. **A comment or bot finding.** Read the line it points at and decide if it is true. True: fix it in a new commit and reply with what changed. False or already handled: reply with the reason and the line that shows it. Taste or a decision for the author: say so and leave it.
4. **A conflict.** Do not choose a side. Say which files and what each side did, and ask. Never force-push over someone else's work.

## Pitfalls

- Amending when the reviewer needs to see what changed. A new commit per fix, unless asked.
- Weakening a test or skipping a hook to turn a check green.
- Enabling auto-merge, merging, or marking ready without being asked.
- Re-running until green and calling it fixed.

## Verification

Done means: the latest commit is green, every comment has an answer, and there is no conflict. Say so in two sentences with the link, and stop.
