---
name: technical-writing
description: Write documentation people can use.
kind: technique
roles: docs
---

# Technical writing

1. Say who the reader is and what they are trying to do. Organise around that task, not around the code.
2. Put the working example first, then explain it. Every command shown must have been run; every path must exist.
3. One idea per paragraph. Define a term where it first appears. Prefer a table for comparisons and a numbered list for steps.
4. Document behaviour, defaults, failure modes and how to recover. Do not document what you have not seen working.
5. Cut anything that restates the code. Run `unslop` last.
