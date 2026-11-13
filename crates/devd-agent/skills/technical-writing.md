---
name: technical-writing
description: Write documentation people can use
kind: words
roles: docs
---

# Technical writing

Writes documentation organised around what the reader is trying to do, with examples that have been run.

## When to use

READMEs, guides, how-tos, API docs, release notes.

## Procedure

1. Say who the reader is and what they are trying to do. Organise around that task, not around the code.
2. Put the working example first, then explain it. Every command shown has been run; every path exists.
3. One idea per paragraph. Define a term where it first appears. A table for comparisons, a numbered list for steps.
4. Document behaviour, defaults, failure modes and how to recover.
5. Cut anything that restates the code. Run `unslop` last.

## Pitfalls

- Documenting what you have not seen working.
- A tour of the files in place of a task.

## Verification

You followed your own steps from a clean state and they worked.
