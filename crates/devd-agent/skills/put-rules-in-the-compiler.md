---
name: put-rules-in-the-compiler
description: Encode invariants in types so wrong code does not compile
kind: design
roles: implementer,reviewer
---

# Put rules in the compiler

Encodes invariants in the type system so a mistake is a build error, not a bug report.

## When to use

Designing types, reviewing a cast, or when a rule exists only in a comment.

## Procedure

1. Use distinct types for distinct things (an id for a site is not an id for a user).
2. Use exhaustive matches, and no stringly-typed state.
3. Avoid escape hatches (`any`, unchecked casts, `unwrap` on input) except at one justified boundary.

## Pitfalls

- A cast that silences an error you did not understand.
- A type alias that looks like a distinct type and is not.

## Verification

The wrong usage fails to compile, and you have seen the compiler refuse it.
