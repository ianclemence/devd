---
name: investigate
description: Answer a question about code or behaviour from evidence, changing nothing
kind: discover
roles: architect,researcher,reviewer,planner
---

# Investigate

Produces an answer a maintainer can rely on, with the evidence beside it. It never fixes anything: if it finds a defect it reports it and stops.

## When to use

"Why does X happen", "is Y true", "where does Z come from".

## Procedure

1. Restate the question in one sentence. If it is ambiguous, state the reading you chose and go on.
2. Collect evidence from the sources, not memory: `project.detect`, `search_files`, `read_file`, `git.log`, the project's docs, and for outside facts `docs.fetch` on official documentation.
3. Separate three things in every claim: what you saw (file and line, command output), what you infer, and what you could not determine.
4. Run the code when running it is cheaper than arguing. A ten-line script that calls the real function beats a paragraph of reasoning.
5. Report the answer first, then the evidence, then the open questions.

## Pitfalls

- Answering from how similar systems usually work.
- Fixing as you go. That is a different task.

## Verification

No evidence, no claim: each statement in the answer has its source, or is marked as inference or unknown.
