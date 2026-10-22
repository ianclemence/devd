---
name: secure-coding-review
description: Read a change for the common ways code becomes exploitable: injection, authorisation, secrets, unsafe defaults.
kind: technique
roles: security,reviewer
---

# Secure coding review

Read the diff and the code around it, and look for these, in this order.

- **Injection.** Input reaching a query, a shell command, a path, a template or a URL without being validated or passed as data. Commands are argument arrays, never strings built from input.
- **Authorisation.** Every operation checks who is asking on the server. An id from the caller is checked against what that caller may touch.
- **Secrets.** Keys, tokens and passwords in code, config, logs, error messages, test fixtures or URLs.
- **Unsafe defaults.** Open ports, debug modes, permissive cross-origin settings, disabled certificate checks, world-writable files.
- **Trust in the wrong place.** Treating file contents, web pages or another service's output as instructions or as already validated.
- **Failure paths.** An error that skips a check, leaves a partial change, or reveals internals.
- **Dependencies.** A new dependency's reason, and the project's own audit command if there is one.

Report each finding with file, line, how it would be exploited, and the smallest fix. Do not report a style preference as a vulnerability.
