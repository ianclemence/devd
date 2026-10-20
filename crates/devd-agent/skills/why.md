---
name: why
description: Find out why code is shaped the way it is before you change it.
kind: technique
roles: architect,researcher,reviewer
---

# Why is it like this?

1. Anchor on facts: file, lines, symbols, and the history (`git.log`, the commit and pull request that introduced it).
2. Read the evidence: commit messages, review discussion, issues, docs, tests that pin the behaviour. Look for incidents and constraints that the code defends against; defensive-looking code usually has a reason.
3. Separate what the sources say, what you infer, and what you do not know. Offer competing explanations when the evidence allows more than one.
4. Do not treat the newest commit as authoritative; the current shape is the result of earlier decisions.
5. If a change is planned, turn the findings into constraints: preserve, change, avoid, risk.
