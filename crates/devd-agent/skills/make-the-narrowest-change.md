---
name: make-the-narrowest-change
description: Delete before you add, and change the least that solves the problem
kind: build
roles: implementer,architect,reviewer
---

Remove before you add. When a change is needed, make the narrowest one that solves it.

- Put a decision in one place and pass a simple flag, instead of repeating it at every call site.
- Choose the shorter code over the more elegant boilerplate.
- Do not thread a signal through five layers; centralise the decision.

The failure this prevents: a diff larger than the problem, a new abstraction used once, a parameter added to ten functions.
