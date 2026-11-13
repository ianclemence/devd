---
name: decide-who-owns-the-state
description: Give each piece of shared state one owner before reaching for a lock
kind: house
roles: architect,implementer
---

# Decide who owns the state

When parts collide on shared state, the first question is whether they need to share it at all. A lock around everything is what you get when nobody knows who owns what.

## When to use

Two parts of the system write the same thing, a race appears, or someone proposes a lock or a queue.

## Procedure

1. List the pieces of state and who writes each.
2. Give each piece exactly one owner. Everyone else asks the owner.
3. Serialise access only to what must stay shared.

## Pitfalls

- A cache with two writers.
- State kept in memory that a restart makes wrong.

## Verification

Each piece of state has one named writer, and a restart leaves it correct.
