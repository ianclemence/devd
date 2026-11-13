---
name: improve-performance
description: Measure first, change one thing, measure again, keep what the numbers support
kind: build
roles: implementer,qa
---

# Improve performance

Makes something faster or smaller with evidence at each step. It does not optimise what it has not measured.

## When to use

"It is slow", "reduce memory", "make startup faster", or any change justified by performance.

## Procedure

1. Define the metric and the target in numbers: what is measured, on what input, how many samples, what counts as better.
2. Measure the baseline before touching anything and keep the raw result (`benchmark-checklist`).
3. Find where the time or memory actually goes, with a profiler or timing around the real code path.
4. Change one thing. Measure again the same way. Keep it only if the number moved by more than the noise; otherwise revert it.
5. Stop when the target is reached.

## Pitfalls

- Optimising what looked slow instead of what measured slow.
- Several changes at once, so none can be credited.
- Comparing a debug build with a release build.

## Verification

The report gives baseline, final, and every kept change with its measured effect (`report-numbers-honestly`).
