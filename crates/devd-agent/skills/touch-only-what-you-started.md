---
name: touch-only-what-you-started
description: Use before stopping a process, deleting a file or changing shared state: act only on what you created, identified by an id you captured, never by a name or pattern
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Touch only what you started

This machine runs other people's work beside yours: dev servers, other agents, the person's real databases and sites. Three mistakes do damage that cannot be undone.

**Ending processes by pattern.** Never stop something because its name, path or command line matches. Your own process has your working folder in its arguments, so a pattern that finds the server also finds you, and it finds the person's other servers too. Keep the process id when you start a process, and stop that id. If you did not start it, you do not stop it; tell the person what is holding the port.

**Writing to live state.** The person's running install, database and sites are in use while you work. Read them, copy from them, run against a copy. Do not write to them, point a second server at them, or tidy them. A cleanup script you wrote an hour ago is not authority to delete things you did not make.

**Baking in addresses.** Do not write `localhost`, a port or a hostname into a bundle, a config or a fixture that other machines will load. Read the real address at run time, or let the environment give it.

## The test

Before an action that cannot be undone, ask: did I create this, in this task, and can I name it by an id I recorded? If yes, go ahead. If you are matching it by what it looks like, stop. A thing that looks like yours and is not is exactly what this prevents.

Reversible work does not need asking: a file under version control, a branch you made, a scratch folder. What needs the person is what cannot be brought back, and what belongs to someone or something else.
