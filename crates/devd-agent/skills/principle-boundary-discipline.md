---
name: principle-boundary-discipline
description: Validate at the edge; trust the inside; keep logic pure.
kind: principle
roles: architect,implementer,reviewer
---

# Validate at the edge; trust the inside; keep logic pure

Parse and validate untrusted input once, where it enters (CLI args, files, network, APIs), into domain types. Inside, trust the types and pass errors up. Keep business logic in pure functions and the framework wiring thin. Do not leak wire or storage types through your public surface.
Smell: the same null check at every layer; business logic inside a request handler.
