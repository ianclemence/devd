---
name: uncover-the-real-structure
description: Read the code and data that a system really runs on before changing any of it
kind: discover
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

Read the real code and the real data before you design. Find the invariants the system relies on, what owns each piece of state, and where the boundaries are. A change made without that map fixes the visible symptom and breaks something quiet.

Signs you are skipping this: editing a file you have not read; guessing at how a function is called.
