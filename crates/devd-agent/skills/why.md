---
name: why
description: Find out why code is shaped the way it is before changing it
kind: discover
roles: architect,researcher,reviewer
---

# Why is it like this?

Finds the reason behind a design from its history. Defensive-looking code usually has one.

## When to use

Before removing or reshaping code that looks odd, and when asked "why".

## Procedure

1. Anchor on facts: file, lines, symbols, and the history (`git.log`, the commit and pull request that introduced it).
2. Read the evidence: commit messages, review discussion, issues, docs, tests that pin the behaviour. Look for incidents and constraints the code defends against.
3. Separate what the sources say, what you infer, and what you do not know. Offer competing explanations when the evidence allows more than one.
4. Do not treat the newest commit as authoritative; the current shape is the result of earlier decisions.
5. If a change is planned, turn the findings into constraints: preserve, change, avoid, risk.

## Pitfalls

- Concluding "it was a mistake" from a missing explanation.
- Removing a guard before finding what it guards.

## Verification

Each reason cites a commit, issue or test, or is marked as inference.
