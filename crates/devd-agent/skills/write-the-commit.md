---
name: write-the-commit
description: Commit or open a pull request with a specific, why-first message
kind: ship
roles: implementer,qa,reviewer,docs
---

# Write the commit

Commits what was asked, in the project's own style, and never resolves a conflict on its own. Approval to commit once is not approval to push, merge, or commit again later.

## When to use

Only when asked to commit or open a pull request.

## Procedure

1. Read what you are committing: `git.status`, `git.diff` (staged and unstaged), and the last ten messages in `git.log`. The project's style beats any default here.
2. Stage what the task changed and nothing else. Leave out anything that looks like a secret, a database, a build output or a log.
3. Write the subject as `type(scope): summary` with the project's types (commonly `feat`, `fix`, `docs`, `chore`, `refactor`, `test`). Plain language, present tense, under about seventy characters.
4. Say why, from the point of view of the person who uses the thing. "New threads no longer spike CPU" is a commit; "improved agent experience" is not.
5. The body: the problem in a sentence or two, how it was fixed, and anything a reviewer would get wrong without being told. Not a list of files.
6. If the project asks for a trailer, add it exactly as asked.
7. For a pull request: one request is one pull request, with the problem, the fix and how you checked it, honest about what you did not check. For a visible change, attach before and after pictures.

## Pitfalls

- A conflict: stop. Do not resolve it or pick a side. Say which files and what each side did.
- A failing hook: fix what it found. Never skip it to get the commit through.
- Auto-merge, merge, or pushing, unless asked.

## Verification

Report the hash, the subject, what you left out of the commit on purpose, and whether anything was pushed.
