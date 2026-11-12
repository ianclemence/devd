---
name: api-design
description: Design an interface other code will depend on, before writing it
kind: design
roles: backend,architect
---

# API design

An interface is easy to add to and hard to take back. This decides the contract (resources, inputs, errors, retries, compatibility) before any implementation exists.

## When to use

Adding or changing an endpoint, a capability, a public function, a file format or a message another program reads.

## Procedure

1. **Nouns and verbs.** What are the things, and what can be done to them? Name them as the callers think of them (`design-from-use`).
2. **Inputs.** Validate every field at the boundary: type, range, length, allowed values. Reject unknown fields rather than ignoring them (`validate-at-the-edge`).
3. **Errors.** One shape: a stable code, a human message, the field at fault. The status says who is at fault. Never leak paths or stack traces.
4. **Idempotency.** Say which operations are safe to repeat. Anything a client may retry needs a key or a natural identity, so twice has the effect of once.
5. **Authorisation.** Each operation states who may call it, and the server checks every time.
6. **Limits.** Lists have a maximum size and a way to continue. Nothing returns "everything".
7. **Compatibility.** Adding an optional field is safe; removing, renaming or changing a meaning is not. Add the new form first and keep the old until callers move (`finish-the-migration`).
8. **Examples.** One request and response per operation, including a failure.

## Pitfalls

- An interface shaped by the implementation instead of by its callers.
- Errors as free text that callers must parse.
- A "temporary" field that every client now depends on.

## Verification

Call each operation for real, with a valid input, an invalid one and a retry, and show the three results.
