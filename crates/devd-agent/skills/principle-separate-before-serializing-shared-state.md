---
name: principle-separate-before-serializing-shared-state
description: Split shared state apart before you add locks or queues.
kind: principle
roles: architect,implementer
---

# Split shared state apart before you add locks or queues

When parts collide on shared state, first ask whether they need to share it. Give each piece of state one owner and make the rest ask the owner. Serialise access only to what must stay shared.
Smell: a lock around everything because nobody knows who owns what.
