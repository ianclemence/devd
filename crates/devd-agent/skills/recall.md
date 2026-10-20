---
name: recall
description: Rebuild context before resuming work.
kind: technique
roles: planner,implementer
---

# Recall

1. Set the scope: the topic, the time window (default a week), this project only.
2. Read the recent conversations and the run history (`chat.list`, `team.list`, `stacks.runs`) for that topic; note decisions, open threads and what was tried and failed.
3. Verify against live state: `git.status`, branches, the project's checks. The repository outranks any summary.
4. Report a brief: a few bullets on where things stand, one line per thread with a status (merged, in flight, verified but uncommitted, reverted, planned), recurring problems, and the one next move.
