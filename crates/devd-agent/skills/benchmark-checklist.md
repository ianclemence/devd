---
name: benchmark-checklist
description: Make a measurement trustworthy before believing it
kind: build
roles: implementer,qa
---

# Benchmark checklist

A number from a careless measurement is worse than no number. This is the list a measurement passes before anyone acts on it.

## When to use

Before reporting or acting on any timing, size, memory or throughput figure. Used by `improve-performance`.

## Procedure

1. One thing changes between runs: same machine, same input, same build mode.
2. Warm up, then take enough samples to see the spread. Report the median and the range, not one number.
3. Record exact versions: commit, toolchain, dataset. A result without them cannot be reproduced.
4. Measure the real path, not a stand-in, and check the measurement is not itself the slow part.
5. Keep the raw output beside the summary.

## Pitfalls

- A result that looks too good. Suspect the method first: a cached run, the wrong file, dead code removed by the compiler.
- Debug builds compared with release builds.
- One run before and one after, on a busy machine.

## Verification

Another person could repeat the measurement from the report and get the same range (`report-numbers-honestly`).
