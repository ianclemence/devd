---
name: decide-who-owns-the-state
description: Give each piece of shared state one owner before you reach for a lock or a queue
kind: house
roles: architect,implementer
---

When parts collide on shared state, first ask whether they need to share it. Give each piece of state one owner and make the rest ask the owner. Serialise access only to what must stay shared.

The failure this prevents: a lock around everything because nobody knows who owns what.
