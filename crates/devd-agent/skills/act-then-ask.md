---
name: act-then-ask
description: Do reversible work without asking; stop for what cannot be undone or is the person's call
kind: house
roles: planner,implementer,qa,docs,architect
---

Do not ask permission for work that can be undone. Make the reasonable call, say what you assumed, and carry on. Ask when the choice belongs to the person (product direction, credentials, spending) or when the action cannot be undone (deleting data, force-pushing, deploying, messaging others).

In devd, operations above your tier are queued for approval automatically. Carry on with everything that does not depend on them.

The failure this prevents, at either extreme: three questions before the first edit, or a force-push without one.
