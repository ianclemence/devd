---
name: incident-response
description: Diagnose something down or wrong, restore it safely, and prevent a repeat
kind: check
roles: sre,infra,implementer,qa
---

# Incident response

Finds what is wrong, restores service with the smallest reversible action, then fixes the cause. Reading comes before touching, and shared things are never changed to hide a symptom.

## When to use

A service is down, slow or returning wrong answers; a site will not load; something that worked stopped.

## Procedure

1. **Read, do not touch.** `service.list`, `health.status`, the failing thing's logs (`project.logs`, `task.logs`), the recent change (`git.log`) and the record of what was done (`activity.list`). Check what shares its port, disk or memory.
2. **Timeline.** When it last worked, what changed, when it failed. The cause is the earliest thing you can show is wrong, not the loudest error; later errors are usually its echo.
3. **Restore first.** The smallest reversible action that brings it back (restart one service, roll back one change). Confirm with a check you can repeat.
4. **Fix the cause** in code or configuration, and add the check that would have caught it: a health check, a test, a threshold.
5. **Write it up** as a blameless note: affected, for how long, timeline, cause with evidence, what you did, what you changed so it does not recur, what is open.

## Pitfalls

- Killing a process you did not start (`touch-only-what-you-started`).
- Deleting data or changing a shared service to make the symptom go away. Say what and why, and ask.
- Declaring it fixed because the error stopped. Run the check.

## Verification

The check that failed now passes and you ran it twice; the note names the evidence for the cause.
