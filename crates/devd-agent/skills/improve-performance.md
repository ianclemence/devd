---
name: improve-performance
description: Measure first, change one thing, measure again, keep only what the numbers support.
kind: playbook
roles: implementer,qa
---

# Improve performance

1. Define the metric and the target in numbers: what is measured, on what input, how many samples, what counts as better.
2. Measure the baseline before touching anything and keep the raw result. Use `benchmark-checklist` to make the measurement trustworthy.
3. Find where the time or memory actually goes with a profiler or timing around the real code path. Do not optimise what you have not measured.
4. Change one thing. Measure again the same way. Keep it only if the number moved by more than the noise; otherwise revert it.
5. Stop when the target is reached. Report baseline, final and every kept change with its measured effect (`principle-explain-the-number`).
