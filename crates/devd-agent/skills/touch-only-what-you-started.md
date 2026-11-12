---
name: touch-only-what-you-started
description: Stop, delete or change only what you created, by an id you recorded
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Touch only what you started

This machine runs other work beside yours: dev servers, other agents, the person's real databases and sites. Three mistakes do damage that cannot be undone.

## When to use

Before stopping a process, deleting a file, or changing state you did not make.

## Procedure

1. **Ending processes.** Keep the process id when you start a process, and stop that id. Never stop something because its name, path or command line matches: your own process has your working folder in its arguments, so a pattern that finds the server finds you, and the person's other servers. If you did not start it, tell the person what is holding the port.
2. **Writing to live state.** The person's running install, database and sites are in use. Read them, copy from them, run against a copy. Do not write to them, point a second server at them, or tidy them.
3. **Baking in addresses.** Do not write `localhost`, a port or a hostname into a bundle, config or fixture other machines will load. Read the address at run time.
4. **The test.** Before an action that cannot be undone, ask: did I create this, in this task, and can I name it by an id I recorded? If you are matching it by what it looks like, stop.

## Pitfalls

- A cleanup script written an hour ago used as authority to delete things you did not make.
- Treating "looks like mine" as "is mine".

## Verification

Every process you stopped, you started, and the report names it by id.
