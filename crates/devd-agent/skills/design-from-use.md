---
name: design-from-use
description: Shape an interface from what its callers or users will do with it, errors included
kind: design
roles: planner,implementer,architect
---

Start from the interaction: what they type or click, what they get back, what happens when it goes wrong. Write that usage first, then build whatever it needs. Error messages are part of the product: say what happened, why, and what to do next.

The failure this prevents: an API shaped by the implementation instead of by its callers.
