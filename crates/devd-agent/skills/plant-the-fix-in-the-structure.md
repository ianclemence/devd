---
name: plant-the-fix-in-the-structure
description: Make the right thing easy and the wrong thing impossible
kind: build
roles: architect,reviewer,implementer
---

# Plant the fix in the structure

A lesson written in a document gets ignored. This puts it where it cannot be: a type, a lint, a test, a single supported path, or the deletion of the example that gets copied.

## When to use

A rule keeps being broken, or a review keeps repeating a comment (`correct`).

## Procedure

1. Ask what would make the wrong thing impossible: a type, a lint, a test, one supported path.
2. Build that.
3. Delete the example or the old path that agents copy.
4. Write prose only for judgement calls nothing can enforce.

## Pitfalls

- A README rule broken twice this month.
- A lint with no message saying what to do instead.

## Verification

The wrong thing now fails a check, and you have seen it fail.
