---
name: secure-coding-review
description: Read a change for the common ways code becomes exploitable
kind: review
roles: security,reviewer
---

# Secure coding review

Reads a diff and the code around it for injection, missing authorisation, secrets, unsafe defaults and misplaced trust. It reports; it does not report style as a vulnerability.

## When to use

Anything that crosses a trust boundary: input from users, files, the network, another program.

## Procedure

Look for these, in order:
1. **Injection.** Input reaching a query, shell command, path, template or URL without validation or being passed as data. Commands are argument arrays, never strings built from input.
2. **Authorisation.** Every operation checks who is asking, on the server. An id from the caller is checked against what that caller may touch.
3. **Secrets.** Keys, tokens and passwords in code, config, logs, error messages, fixtures or URLs.
4. **Unsafe defaults.** Open ports, debug modes, permissive cross-origin settings, disabled certificate checks, world-writable files.
5. **Trust in the wrong place.** File contents, web pages or another service's output treated as instructions or as already validated.
6. **Failure paths.** An error that skips a check, leaves a partial change, or reveals internals.
7. **Dependencies.** A new dependency's reason, and the project's own audit command if there is one.

## Pitfalls

- Reporting a preference as a vulnerability.
- A finding with no exploit path.

## Verification

Each finding gives file, line, how it would be exploited, and the smallest fix.
