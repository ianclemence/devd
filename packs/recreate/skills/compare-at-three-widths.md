---
name: compare-at-three-widths
description: Measure a rebuild against the original and list what does not match yet
kind: check
roles: recreator,qa,frontend,mobile,reviewer
---

# Compare at three widths

A recreation is finished when the differences are known and acceptable, not when it looks
similar on one screen at one size. This is the pass that turns "close" into a list.

## When to use

After the build runs and its pages open, and again after every round of fixes.

## Procedure

1. Open the rebuild at the same three widths the study used. If a width is not reachable in
   the build, that is the first finding: the layout does not exist yet.
2. Compare the pair screen by screen: order of content, what is present, what is missing, what
   moved, what changed size or colour. Take both pictures and put them side by side in the
   report through `recreate.check`.
3. Try the flows the person named on the rebuild, not only the first screen. A flow that
   cannot be completed is a gap, whatever the first screen looks like.
4. Sort the gaps by what a person would notice first, and separate the ones that break use
   from the ones that are cosmetic.
5. Run the project's own checks with `project.verify`. A page that matches and a suite that
   fails is not a finished recreation.

## Pitfalls

- Honouring a similarity figure as if it were a test result. It is a measurement; the checks
  and a reviewer's reading are the verdict.
- Comparing only the happy path.
- Fixing the comparison by cropping the screenshot instead of the layout.

## Verification

`recreate.check` has written a report naming every screen at every width, with the gap list
sorted and the project's checks beside it, and the reviewer has read that report.
