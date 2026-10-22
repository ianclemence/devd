---
name: api-design
description: Design an interface other code will depend on: resources, errors, validation, idempotency and compatibility.
kind: technique
roles: backend,architect
---

# API design

An interface is easy to add to and hard to take back. Decide these before writing it.

1. **Nouns and verbs.** What are the things, and what can be done to them? Name them as the callers think of them.
2. **Inputs.** Validate every field at the boundary: type, range, length, allowed values. Reject unknown fields rather than ignoring them silently.
3. **Errors.** One shape for all of them: a stable code, a human message, and the field at fault. The status says who is at fault (caller or server). Never leak internals, paths or stack traces.
4. **Idempotency.** Say which operations are safe to repeat. Anything a client may retry (a network timeout) needs a key or a natural identity, so doing it twice has the effect of doing it once.
5. **Authorisation.** Each operation states who may call it, and the server checks it every time.
6. **Pagination and limits.** Lists have a maximum size and a way to continue. Nothing returns "everything".
7. **Compatibility.** Adding an optional field is safe; removing, renaming or changing a meaning is not. If it must change, add the new form first and keep the old one until callers have moved.
8. **Documentation by example.** One request and response for each operation, including a failure.
