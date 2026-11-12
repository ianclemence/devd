---
name: work
description: Start here for any request: decide what kind of work it is, then load that skill
kind: discover
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Choose how to work

Reads the request, decides what kind of work it is, and loads the skill for it with `skills.get`. Do not improvise a process when one exists.

## When to use

At the start of any request that is more than a one-line answer.

## Procedure

Match the request:

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
| many independent pieces at once | `swarm` |
| a finished change that needs reviewing | `review-the-diff`, or `interrogate` if risky |
| a page, flow or visible change to check | `agent-browser`, `test-the-app` |
| landing a change | `ship`, then `write-the-commit` |
| an open pull request | `babysit-the-pull-request` |
| work that is large, unusual or unattended | `figure-it-out` |
| coming back to earlier work | `recall`, `handoff` |
| a project that has no guide for agents | `write-the-project-guide` |

## Pitfalls

- Loading several skills "to be safe". Load the one that matches, and the others when the work changes.

## Verification

Whatever you load, three rules hold: understand before you change (`uncover-the-real-structure`), prove the result against the real thing (`prove-it-works`), and say plainly what you did and did not verify (`plain-speech`).
