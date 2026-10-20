---
name: principle-laziness-protocol
description: Prefer deleting code to adding it; make the narrowest change that works.
kind: principle
roles: implementer,architect,reviewer
---

# Prefer deleting code to adding it; make the narrowest change that works

Remove before you add. When a change is needed, make the narrowest one that solves the problem.
- Put a decision in one place and pass a simple flag, instead of repeating it at every call site.
- Choose the shorter code over the more elegant boilerplate.
- Do not thread a signal through five layers; centralise the decision.
Smell: a diff larger than the problem, a new abstraction used once, a parameter added to ten functions.
