---
name: put-rules-in-the-compiler
description: Encode invariants in the type system so that wrong code does not compile
kind: design
roles: implementer,reviewer
---

Encode invariants in types so that wrong code does not compile: distinct types for distinct things, exhaustive matches, no stringly typed state. Avoid escape hatches (any, unchecked casts, unwrap on input) except at a justified, commented boundary.

The failure this prevents: a cast that silences an error you did not understand.
