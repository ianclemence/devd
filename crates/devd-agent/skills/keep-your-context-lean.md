---
name: keep-your-context-lean
description: Search before reading, read only the part you need, and keep conclusions rather than raw output
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

Long outputs and many files degrade reasoning. Search before you read. Read only the part you need: `fs.read` takes `offset` and `limit`. Keep conclusions, not raw output. Send bulk exploration to a separate task and take back its summary. Re-reading the same file is a sign you are not keeping notes; write them down with `scratch.write`.

The failure this prevents: pasting a 2,000-line log into the conversation.
