---
name: test-the-app
description: Use before saying a visible change works: run the real app against a copy of real data, never the live install, and watch the affected flow end to end
kind: check
roles: implementer,frontend,qa,reviewer,designer
---

# Test the app

A change that type-checks and passes its tests can still be wrong on screen. Once, after the tests pass, look at the real thing.

## Never test on the live install

The person's running devd, database and sites are theirs, and in use while you work. Reading or copying from them is fine and is the best source of test data. Writing to them, starting a second server against them, or cleaning them up is not. Run your copy with its own data folder: `DEVD_HOME` pointing at a copy of `state.db` (copy `state.db`, `state.db-wal` and `state.db-shm` together, with the live server idle or read-only). Copy in; never symlink, and never copy back.

An empty database is a bad test. A conversation with 800 events, a paused run, a provider with no key: these are what real use looks like, and they are what breaks.

## Ports and processes

- Use a port you chose, and read the real one from the server's own output.
- Stop only what you started, by the process id you captured when you started it. Never end a process by matching a name or path: your own agent has that path in its command line, and this machine is running other servers.
- An assistant turn ending is not teardown. Leave the server and the page up while the person looks, and say what is running.

## Do not spend what you do not need to

Real model turns cost money. To watch a turn render, use a recorded conversation, or the cheapest model that exercises the behaviour. Use a provider's real behaviour only when the change depends on it.

## What to look at

Exercise the flow the change touched, then the neighbours: the empty state, the error state, a long name, a narrow window, light and dark. Open it from every way in (a link, the command menu, a key), because a change that works from one entry point and not another is the commonest defect there is.

Capture proof that survives: a screenshot saved in the project, the console and the network log clean or explained. For motion or timing, say what you measured, not that it "feels fine".

Report what you ran, what you saw, and what you did not check.
