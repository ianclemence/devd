---
name: uncover-the-real-structure
description: Read the real code and data before changing a system
kind: discover
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Uncover the real structure

Reads what the system actually runs on before designing: the invariants it relies on, what owns each piece of state, and where the boundaries are. A change made without that map fixes the visible symptom and breaks something quiet.

## When to use

Before the first edit to code or data you did not write.

## Procedure

1. Find the entry points and follow one real request through (`how`).
2. Find what owns each piece of state and what depends on it.
3. Look at real data, not the schema alone.
4. Note the invariants the code relies on.

## Pitfalls

- Editing a file you have not read.
- Guessing how a function is called instead of searching for its callers.

## Verification

You can name the owner of each piece of state your change touches, and where you saw it.
