---
name: ml-experiments
description: Run a model change as an experiment with a baseline and honest evaluation
kind: build
roles: ml,data,implementer
---

# ML experiments

A model change is an experiment judged by a measurement, not by how it feels. This keeps the baseline, the evaluation and the claim honest.

## When to use

Training, tuning, changing features or data, swapping a model, or any claim that a model got better.

## Procedure

1. **Hypothesis.** One change and what it should do to which metric.
2. **Data.** Where it came from, its size, how it was split, and what could leak between sets (the same user, time, document or near-duplicate on both sides).
3. **Baseline.** Measure the current model first, on the same split, and keep the raw output (`benchmark-checklist`).
4. **Reproducibility.** Pin the data version, seeds, library versions and commit. Write the one command that repeats the run.
5. **Explore small.** A subset, a few epochs, a small model. Scale only what earns it. Do not spend on a provider or GPU the task did not allow.
6. **Evaluate.** Never tune on the test set. Run several seeds or folds and report the spread. Check the slices that matter, not only the average.
7. **Decide.** Keep the change only if the improvement is larger than the spread.

## Pitfalls

- A suspiciously good score. Assume a leak or a bug and look for it first.
- An average that hides a slice getting worse.
- One run reported as a result (`report-numbers-honestly`).

## Verification

Report hypothesis, baseline, result with spread and number of runs, the reproduction command, and what the evaluation does not cover.
