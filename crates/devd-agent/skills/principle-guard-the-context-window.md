---
name: principle-guard-the-context-window
description: Keep the working context lean.
kind: principle
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Keep the working context lean

Long outputs and many files degrade reasoning. Search before you read; read the part you need (`fs.read` with offset and limit); keep conclusions, not raw output; delegate bulk exploration to a separate task and take back its summary. Re-reading the same file is a sign you are not keeping notes (`scratch.write`).
Smell: pasting a 2,000-line log into the conversation.
