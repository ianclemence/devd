---
name: no-comments
description: Remove comments that restate code; encode real constraints in the code
kind: build
roles: implementer,reviewer,docs
---

# No comments

A comment is a smell that the code did not say it. This removes what is noise and moves what matters into code.

## When to use

Cleaning a file or a diff of comments, or when asked to remove AI-style commentary.

## Procedure

For each comment in scope:
1. If it restates the code, delete it.
2. If it explains a workaround, fix the cause (`fix-the-root-cause`) and delete both.
3. If it states a constraint ("do not remove", "must run before X"), keep it only when the thing cannot be changed. Otherwise encode it as a type, a runtime check, a test or a lint rule, then delete the comment.
4. Keep comments that carry what code cannot: why a surprising choice was made, a link to a specification, a safety warning.

## Pitfalls

- Deleting a comment that carried a real constraint before encoding it.
- Rewording a comment instead of deleting it.

## Verification

The report lists what was deleted, what was encoded, and what remains unenforced.
