---
name: design-from-use
description: Shape an interface from what its callers will do with it, errors included
kind: design
roles: planner,implementer,architect
---

# Design from use

Starts from the interaction: what callers type or click, what they get back, and what happens when it goes wrong. Write that usage first, then build whatever it needs.

## When to use

Before any new interface: a function, a command, an endpoint, a screen.

## Procedure

1. Write the calls as the caller would write them, including a failing one.
2. Write the error messages: what happened, why, and what to do next. They are part of the product.
3. Build the least that makes those calls work.

## Pitfalls

- An API shaped by the implementation instead of by its callers.
- An error message that names an internal function.

## Verification

The usage you wrote first runs unchanged against the finished code.
