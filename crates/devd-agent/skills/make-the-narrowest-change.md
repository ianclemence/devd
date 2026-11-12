---
name: make-the-narrowest-change
description: Delete before you add, and change the least that solves the problem
kind: build
roles: implementer,architect,reviewer
---

# Make the narrowest change

Removes before it adds, and when a change is needed makes the narrowest one that solves the problem.

## When to use

Before every edit, and when reviewing a diff that looks bigger than its problem.

## Procedure

1. Ask what can be deleted to solve this.
2. Put a decision in one place and pass a simple flag, instead of repeating it at every call site.
3. Choose the shorter code over elegant boilerplate.
4. Do not thread a signal through five layers; centralise the decision.

## Pitfalls

- A new abstraction used once.
- A parameter added to ten functions.
- A diff larger than the problem.

## Verification

Every hunk in `git.diff` is needed for the stated problem, and you can say why for each.
