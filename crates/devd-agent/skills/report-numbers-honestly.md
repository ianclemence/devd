---
name: report-numbers-honestly
description: A measurement is not a result until it says how it was taken
kind: check
roles: implementer,qa,reviewer
---

# Report numbers honestly

Reports what was measured, how, how many samples and how much it varied, and explains why the number is what it is.

## When to use

Any figure in a report: timing, size, count, percentage, cost.

## Procedure

1. State what was measured and on what input.
2. State how, how many samples, and the variation.
3. Give the baseline beside the result.
4. Explain why the number is what it is, not only what it is.

## Pitfalls

- "Twice as fast" with no baseline.
- A suspiciously good number. Suspect the method first (`benchmark-checklist`).

## Verification

A reader can tell what to run to reproduce each number.
