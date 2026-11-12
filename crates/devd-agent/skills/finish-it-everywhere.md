---
name: finish-it-everywhere
description: Walk every way in, surface and state before calling a feature done
kind: check
roles: planner,architect,implementer,qa,reviewer,frontend,docs
---

# Finish it everywhere

The usual unfinished change works on the path you tried and is missing everywhere else. This is the walk that finds the gaps.

## When to use

Before calling any feature, fix or rename done, and when reviewing one.

## Procedure

1. **Every way in.** A behaviour reachable from one screen is usually also reachable from the command menu, a key, a link, the CLI and an agent. If a person can trigger it, an agent should reach it through a capability, running the same code, not a copy.
2. **Every surface.** devd has a native window, a browser interface, a terminal interface and an HTTP API. Say which you changed and whether the change reaches the others. A new status, name or phrase belongs in the one shared table, not in each surface.
3. **Every provider and platform.** Anything that depends on a model, an operating system or a runtime needs a decision for each, even if the decision is "not supported here, and it says so".
4. **The contract.** Anything crossing a boundary has one definition both sides read. Change it and let the compiler and tests show the rest.
5. **The way back.** If you added a way in, add the way out and a way to see it: pause needs resume, add needs remove.
6. **Every state.** Empty, loading, failed, partial, very large, half-configured (a provider with no key, a project with no runtime).
7. **Restarts and reconnects.** What does the screen show after the server restarts or the window opens late? State only in memory is wrong after a restart.
8. **The words.** A label, a tooltip, a menu entry and an agent's description of one action say the same thing about the same object. Show people words, not identifiers.
9. **Docs and skills.** If the change makes guidance wrong, rewrite the sentence; do not append a second account of the new behaviour.

## Pitfalls

- Testing the path you built and only that one.
- One-way doors: a switch with no way to switch back.

## Verification

The report lists which of the nine you checked and which you chose not to, with the reason for each skipped one.
