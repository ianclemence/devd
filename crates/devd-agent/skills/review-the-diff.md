---
name: review-the-diff
description: Review a change for bugs that would hurt, and prove each finding
kind: review
roles: reviewer,qa,security,architect
---

# Review the diff

A review is useful in proportion to how many of its findings are true. Ten maybes cost the author an hour each; three proven defects cost minutes and get fixed. For a high-risk change use `interrogate`.

## When to use

"Review this", "look over my change", or before merging anything.

## Procedure

1. Read the change as it is: `git.diff` for what changed, `git.log` for why, then open the files around each hunk, not only the hunk. The bug is usually in how the hunk meets what was already there.
2. Look for, in order: **it does not do what it says** (run it in your head with a real, an empty, a huge and a hostile input); **it breaks something else** (`blast-radius`); **it fails badly** (a swallowed error, a retry hiding a fault, a leaked resource, a partial write, state wrong after a restart); **it is unsafe** (`secure-coding-review`); **it is incomplete** (`finish-it-everywhere`); **it is not tested where it can break**.
3. Style and naming come last, and only when they hide a defect. Do not review what a formatter or the compiler enforces.
4. For every finding, show the input that triggers it and what goes wrong. Run it if you can. If you suspect something and cannot show it, label it a question.

## Pitfalls

- Inventing a finding to look thorough.
- Findings with no case that shows them.

## Verification

Report most serious first: file and line, what is wrong, the case, the smallest fix. Then what you checked and found fine, and what you did not check. If nothing is wrong, say so in a sentence.
