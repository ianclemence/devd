---
name: principle-foundational-thinking
description: Understand the real structure before changing it.
kind: principle
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Understand the real structure before changing it

Read the real code and the real data before designing. Find the invariants the system relies on, what owns each piece of state, and where the boundaries are. A change made without that map fixes the visible symptom and breaks something quiet.
Smell: editing a file you have not read; guessing at how a function is called.
