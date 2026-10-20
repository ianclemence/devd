---
name: work
description: Start here for any request: pick the right playbook, then follow it.
kind: playbook
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Choose the playbook

Read the request, decide what kind of work it is, load the matching skill with `skills.get`, and follow it. Do not improvise a process when one exists.

| The request is... | Load |
|---|---|
| a question about code, a system or a decision | `investigate` (and `how`, `why`) |
| a defect, a failing test, a wrong result | `fix-bug` |
| something is slow, big or uses too much | `improve-performance` |
| new or changed behaviour | `build-feature` |
| restructuring with no behaviour change | `refactor` |
| a design question you can settle by trying | `prototype` |
| "will this break anything?" | `blast-radius` |
| a design with several plausible shapes | `architect`, then `arena` |
| many independent pieces of work at once | `swarm` (in devd: a team run with parallel tasks) |
| reviewing a finished change | `interrogate` |
| landing a change | `ship` |
| work is large, unusual or unattended | `figure-it-out` |
| coming back to earlier work | `recall`, `handoff` |

Whatever the playbook, three rules hold: understand before you change (`principle-foundational-thinking`), prove the result against the real thing (`principle-prove-it-works`), and say plainly what you did and did not verify (`plain-speech`).
