---
name: how
description: Explain how a system works well enough to change it safely.
kind: technique
roles: architect,researcher,reviewer,implementer
---

# How does this work?

Build a working model, not a tour of files.

1. Decide the scope. A single module needs one pass; a subsystem needs several angles (entry points, data flow, state, failure paths).
2. For each angle read the real code and follow one real request or event all the way through.
3. Write the explanation the way you would for a senior engineer joining the team: overview, the key concepts, how it works step by step, where things live, and the gotchas.
4. Naming a file is not understanding it. Say what it owns and what depends on it. For motivation use `why`.
