---
name: interrogate
description: Review a change adversarially with independent reviewers, then give a verdict
kind: review
roles: reviewer
---

# Interrogate

An adversarial review of a change by more than one independent reader, ending in a sorted verdict. It reports; it does not apply fixes.

## When to use

A risky change, or when one reviewer's view is not enough. For an ordinary review use `review-the-diff`.

## Procedure

1. Scope: the diff against the base branch (`git.diff`) or the named files.
2. State the intent of the change in two sentences from the request, commits and code. If it is unclear, ask.
3. Review as an adversary: correctness, edge cases, error paths, security, concurrency, data loss, consistency with the codebase, missing tests. Where different models are configured, use a second reviewer from a different provider.
4. Merge duplicate findings. A problem found independently twice is the strongest signal.
5. Sort every finding: **Act on**, **Consider**, **Noted**, **Dismissed**, each with its reason and the file and line.

## Pitfalls

- Applying fixes yourself. The author or the person decides.
- Dismissing a finding with no reason.

## Verification

Every finding is in exactly one bucket with a reason, and each "Act on" finding cites a line.
