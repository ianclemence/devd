---
name: write-the-commit
description: Use when asked to commit or open a pull request: a specific, why-first message in the project's own style, committing only what was asked, and never resolving a conflict on your own
kind: ship
roles: implementer,qa,reviewer,docs
---

# Write the commit

Commit or open a pull request only when asked. Approval to commit once is not approval to push, to merge, or to commit again later.

## Before you write anything

Run `git.status` and `git.diff` (staged and unstaged) and read them, and read `git.log` for the last ten messages: the project's own style beats any default here. Stage what the task changed and nothing else. A file you did not touch stays out, and so does anything that looks like a secret, a database, a build output or a log.

## The message

- A subject of the form `type(scope): summary`, with the project's types (commonly `feat`, `fix`, `docs`, `chore`, `refactor`, `test`). The scope is optional. Plain language, present tense, under about seventy characters.
- Say why, from the point of view of the person who uses the thing. "New threads no longer spike CPU" is a commit. "Improved agent experience" is not: name the specific change a user would notice.
- The body is the problem in a sentence or two, then how it was fixed, then anything a reviewer would get wrong without being told. Not a list of files: the diff has that.
- If the project asks for a trailer (who or what helped), add it exactly as asked.

## Conflicts and surprises

If a merge, rebase or pull reports a conflict, stop. Do not resolve it, and do not choose a side: say which files, and what each side did, and let the person decide. The same goes for a hook that fails: fix what it found, and never skip it to get the commit through.

## Pull requests

One request is one pull request. Title in the commit style. The body says the problem, the fix and how you checked it, and is honest about what you did not check. For a visible change, attach before and after pictures. Do not enable auto-merge unless asked. After opening it, do not poll for results: when checks finish or someone comments, you will be told.

## Report

The hash, the subject, what was left out of the commit on purpose, and whether anything was pushed.
