---
name: validate-at-the-edge
description: Check untrusted input once, where it enters, and keep the logic behind it pure
kind: design
roles: architect,implementer,reviewer
---

# Validate at the edge

Parses and validates untrusted input once, where it enters, into domain types. Inside, trust the types and pass errors up.

## When to use

Anything that reads arguments, files, the network or an API.

## Procedure

1. Find the one place input enters.
2. Parse it there into domain types, rejecting what does not fit.
3. Keep business logic in pure functions and the framework wiring thin.
4. Do not leak wire or storage types through your public surface.

## Pitfalls

- The same null check at every layer.
- Business logic inside a request handler.

## Verification

A bad input is refused at the edge with a clear error, and nothing deeper checks it again.
