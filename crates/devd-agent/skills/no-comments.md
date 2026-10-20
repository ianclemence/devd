---
name: no-comments
description: Remove comments that restate code; encode real constraints in the code.
kind: technique
roles: implementer,reviewer,docs
---

# No comments

A comment is a smell that the code did not say it. For each comment in scope:

1. If it restates the code, delete it.
2. If it explains a workaround, fix the cause (`principle-fix-root-causes`) and delete both.
3. If it states a constraint ("do not remove", "must run before X"), keep it only when the thing cannot be changed. Otherwise encode it: a type, a runtime check, a test or a lint rule, then delete the comment.
4. Keep comments that carry what code cannot: why a surprising choice was made, a link to a specification, a safety warning.
Report what was deleted, what was encoded and what remains unenforced.
