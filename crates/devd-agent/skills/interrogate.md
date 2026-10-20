---
name: interrogate
description: Review a change adversarially: independent reviewers, then a verdict.
kind: technique
roles: reviewer
---

# Interrogate

1. Scope: the diff against the base branch (`git.diff`) or the named files.
2. State the intent of the change in two sentences from the request, commits and code. If intent is unclear, ask.
3. Review as an adversary: correctness, edge cases, error paths, security (secrets, injection, unsafe defaults, authorisation), concurrency, data loss, consistency with the codebase, missing tests. Where different models are configured, use a second reviewer from a different provider for an independent view.
4. Merge duplicate findings. A problem found independently twice is the strongest signal.
5. Sort every finding: **Act on**, **Consider**, **Noted**, **Dismissed**, each with its reason. Cite file and line. Do not apply fixes yourself.
