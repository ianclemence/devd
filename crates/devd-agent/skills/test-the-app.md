---
name: test-the-app
description: Run the real app on a copy of real data and watch the changed flow
kind: check
roles: implementer,frontend,qa,reviewer,designer
---

# Test the app

A change that type-checks and passes its tests can still be wrong on screen. Once the tests pass, this looks at the real thing, against a copy of real data and never the live install.

## When to use

Any visible change, and any change to behaviour a person relies on.

## Procedure

1. **Copy, never share.** The person's running devd, database and sites are in use. Point your own copy at its own data folder (`DEVD_HOME`) holding a copy of `state.db`, `state.db-wal` and `state.db-shm` together. Copy in; never symlink, never copy back.
2. **Use meaningful data.** A conversation with 800 events, a paused run, a provider with no key. An empty database is a bad test.
3. **Own your processes** (`touch-only-what-you-started`). Pick a port, read the real one from the server's output, and stop only the process id you captured.
4. **Do not spend what you do not need to.** Use a recorded conversation, or the cheapest model that exercises the behaviour. Use a provider's real behaviour only when the change depends on it.
5. **Exercise the flow** the change touched, then the neighbours: empty, error, a long name, a narrow window, light and dark. Open it from every way in: a link, the command menu, a key (`agent-browser`).
6. Capture proof that survives: a screenshot in the project, and the console and network clean or explained.

## Pitfalls

- Starting a second server against the live database.
- Stopping a process by name or path.
- "It feels fine" for motion or timing. Say what you measured.
- Tearing everything down when your turn ends. Leave the server and page up while the person looks, and say what is running.

## Verification

The report lists what you ran, what you saw, and what you did not check.
