---
name: how
description: Explain how a system works well enough to change it safely
kind: discover
roles: architect,researcher,reviewer,implementer
---

# How does this work?

Builds a working model of a system, not a tour of files. Read-only.

## When to use

Before changing code you do not know, or when asked "how does X work".

## Procedure

1. Decide the scope. One module needs one pass; a subsystem needs several angles (entry points, data flow, state, failure paths).
2. For each angle, read the real code and follow one real request or event all the way through (`search_files`, then `read_file` for the part that matters).
3. Write it as you would for a senior engineer joining the team: overview, key concepts, how it works step by step, where things live, the gotchas.
4. For the motivation behind a design, use `why`.

## Pitfalls

- Naming a file is not understanding it. Say what it owns and what depends on it.
- Explaining from memory of similar systems instead of this one.

## Verification

Every claim names the file and line it came from, and the walk-through follows a real input end to end.
