---
name: finish-it-everywhere
description: Use before calling a feature done: a change that works on the path you tested and is missing elsewhere is the commonest defect, so walk every way in, every surface, every state and the way out
kind: check
roles: planner,architect,implementer,qa,reviewer,frontend,docs
---

# Finish it everywhere

The usual unfinished change works on the path you tried. Before you call something done, walk this list and say which entries applied.

- **Every way in.** A behaviour reachable from one screen is usually also reachable from the command menu, a key, a link, the CLI and an agent. Fixing one is not fixing the feature. If a person can trigger it, an agent should be able to reach it through a capability, and it should be the same code, not a copy.
- **Every surface.** devd has a native window, a browser interface, a terminal interface and an HTTP API. Say which you changed, and for the others, whether the change reaches them or why not. A new status, name or phrase belongs in the one shared table, not in each surface.
- **Every provider and platform.** Anything that depends on a model, a provider, an operating system or a runtime needs a decision for each one, even when the decision is "not supported here, and it says so".
- **The contract.** Anything that crosses a boundary has one definition both sides read. Change the definition and let the compiler and the tests show you the rest.
- **The way back.** If you added a way in, add the way out and a way to see it: pause needs resume, close needs reopen, add needs remove. A one-way door is a bug.
- **Every state.** Empty, loading, failed, partial, very large, and half-configured (a provider with no key, a project with no runtime).
- **Reconnects and restarts.** What does the screen show if the server restarts, or the window opens late? State that exists only in memory is a lie after a restart.
- **The words.** A label, a tooltip, a menu entry and an agent's description of the same action should say the same thing about the same object. Show a person words, not identifiers.
- **The docs and skills.** If the change makes existing guidance wrong, fix the guidance, and rewrite the sentence rather than appending a second account of the new behaviour.

Then say, in your report, which of these you checked and which you chose not to, and why.
