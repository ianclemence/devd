---
name: principle-type-system-discipline
description: Let the compiler carry the rules.
kind: principle
roles: implementer,reviewer
---

# Let the compiler carry the rules

Encode invariants in types so wrong code does not compile: distinct types for distinct things, exhaustive matches, no stringly-typed state. Avoid escape hatches (any, unchecked casts, unwrap on input) except at a justified, commented boundary.
Smell: a cast that silences an error you did not understand.
