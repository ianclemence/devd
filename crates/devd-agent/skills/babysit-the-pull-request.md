---
name: babysit-the-pull-request
description: Use after opening a pull request: when checks, comments or conflicts arrive, verify each against the source, fix the real ones, dismiss false ones with a reason, and stay quiet when nothing is new
kind: ship
roles: implementer,reviewer,infra
---

# Babysit the pull request

Once a pull request is open, your job is to get it to green on the latest commit without waking the person for noise.

## Do not poll

Do not loop, sleep or re-check on a timer. When checks finish, someone comments, or a conflict appears, devd tells you. Between those events, end your turn. A turn spent to say "still waiting" costs the person's attention and a model call for nothing.

## When something arrives

**A failing check.** Read the failing job's log, not only its name. Reproduce it locally with the same command. Fix the cause. If the failure is not yours (a flaky test, an outage), prove it: show the same test passing on the base branch or on a re-run, and say that, rather than re-running until green.

**A review comment or a bot finding.** Verify it against the source before acting. Read the line it points at and decide whether it is true.
- True: fix it, in a new commit, and reply with what changed.
- False or already handled: reply with the reason in a sentence and the line that shows it. Do not silently ignore it, and do not change code to appease a finding you believe is wrong.
- A matter of taste, or a decision for the author or the person: say so and leave it to them.

**A conflict.** Do not resolve it by choosing a side. Say which files, what each side did, and ask the person. Never force-push over someone else's work.

## Rules

- A new commit for each fix, not an amended one, unless the person asks: the reviewer should see what changed.
- Do not enable auto-merge, merge, or mark it ready unless asked.
- Never skip a hook or weaken a test to turn a check green. A check that fails is information.
- Stay quiet when nothing is new. Say something when a check turns red or green, a person asks something, or you need a decision.

## Done

The latest commit is green, every comment has an answer, and there are no conflicts. Then say so in two sentences, with the link, and stop.
