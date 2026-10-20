---
name: investigate
description: Answer a question about code or behaviour from evidence, changing nothing.
kind: playbook
roles: architect,researcher,reviewer,planner
---

# Investigate

Goal: an answer a maintainer can rely on, with the evidence beside it.

1. Restate the question in one sentence. If it is ambiguous, state the reading you chose and go on.
2. Collect evidence from the sources, not from memory: `project.detect`, `fs.search`, `fs.read`, `git.log`, `git.blame`-style history, the project's own docs, and for outside facts `docs.fetch` on official documentation.
3. Separate three things in every claim: what you saw (file and line, command output), what you infer from it, and what you could not determine.
4. Run the code when running it is cheaper than arguing about it. A ten-line script that calls the real function beats a paragraph of reasoning.
5. Report: the answer first, then the evidence, then the open questions. No evidence, no claim.

Never fix anything during an investigation. If you find a defect, report it and stop.
