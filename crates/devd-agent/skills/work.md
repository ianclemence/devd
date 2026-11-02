---
name: work
description: Start here for any request: decide what kind of work it is, then load the skill for it
kind: discover
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Choose how to work

Read the request, decide what kind of work it is, load the matching skill with `skills.get`, and follow it. Do not improvise a process when one exists.

| The request is... | Load |
|---|---|
| a question about code, a system or a decision | `investigate`, with `how` and `why` |
| a defect, a failing test, a wrong result | `fix-bug` |
| something slow, large or using too much memory | `improve-performance` |
| new or changed behaviour | `build-feature` |
| restructuring with no behaviour change | `refactor` |
| a design question you can settle by trying | `prototype` |
| "will this break anything?" | `blast-radius` |
| a design with several plausible shapes | `architect`, then `arena` |
| many independent pieces of work at once | `swarm`; in devd, a team run with parallel tasks |
| a finished change that needs reviewing | `interrogate` |
| landing a change | `ship` |
| work that is large, unusual or unattended | `figure-it-out` |
| coming back to earlier work | `recall`, `handoff` |

Whatever skill you load, three rules hold: understand before you change (`uncover-the-real-structure`), prove the result against the real thing (`prove-it-works`), and say plainly what you did and did not verify (`plain-speech`).
