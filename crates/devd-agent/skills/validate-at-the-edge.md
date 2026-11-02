---
name: validate-at-the-edge
description: Check untrusted input once, where it enters, and keep the logic behind it pure
kind: design
roles: architect,implementer,reviewer
---

Parse and validate untrusted input once, where it enters (arguments, files, network, APIs), into domain types. Inside, trust the types and pass errors up. Keep business logic in pure functions and make the framework wiring thin. Do not leak wire or storage types through your public surface.

The failure this prevents: the same null check at every layer; business logic inside a request handler.
