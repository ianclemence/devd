---
name: review-the-diff
description: Use to review a change, yours or someone else's: read the real diff, find the bugs that would hurt, prove each one, and report few findings you are sure of instead of many you are not
kind: review
roles: reviewer,qa,security,architect
---

# Review the diff

A review is useful in proportion to how many of its findings are true. Ten maybes cost the author an hour each; three proven defects cost a few minutes and get fixed.

## Read the change as it is

`git.diff` for what changed, `git.log` for why, then open the files around each hunk, not only the hunk. A diff shows what was added; the bug is usually in how it meets what was already there: a caller that passes something the new code does not expect, a second copy of the logic that was not updated, a test that still asserts the old behaviour.

## What to look for, in this order

1. **It does not do what it says.** Run the path in your head with a real input, then with an empty one, a huge one and a hostile one. Off-by-one, wrong operator, a condition inverted, a value read before it is set.
2. **It breaks something else.** Who calls this? What did they rely on? Use `blast-radius` when the change touches a shared name, a stored format or a public contract.
3. **It fails badly.** An error swallowed, a retry that hides a real fault, a resource not released, a partial write left behind, state that is wrong after a restart.
4. **It is unsafe.** Input trusted without a check, a secret logged, a path escaping its folder, a permission loosened. Use `secure-coding-review` for anything that crosses a trust boundary.
5. **It is incomplete.** One entry point fixed and the others not, an add with no remove, the docs left saying the old thing (`finish-it-everywhere`).
6. **It is not tested where it can break.** The test that would fail without the change, for the case the change was about. A test that only mirrors the implementation is not one.

Style, naming and taste come last and only when they hide a defect or will mislead the next reader. Do not review what a formatter or the compiler already enforces. Remove noise you would not have written yourself (`unslop`).

## Prove it, or say you could not

For every finding, show the input that triggers it and what goes wrong, or the line that contradicts another. Run it if you can: a failing test or a command's output is better than an argument. If you suspect something and cannot show it, say that plainly and label it a question, not a defect.

## Report

Most serious first. Each finding: where (file and line), what is wrong, the case that shows it, and the smallest fix. Then what you checked and found fine, and what you did not check. If there is nothing wrong, say so in a sentence; do not invent a finding to look thorough.
