---
name: benchmark-checklist
description: Make a measurement trustworthy before believing it.
kind: technique
roles: implementer,qa
---

# Benchmark checklist

- One thing changes between runs. Same machine, same input, same build mode.
- Warm up, then take enough samples to see the spread; report median and range, not one number.
- Record exact versions: commit, toolchain, dataset. A result without them cannot be reproduced.
- Measure the real path, not a stand-in. Check that the measurement itself is not the slow part.
- If a result looks too good, suspect the method first (cached, wrong file, dead code eliminated).
- Keep the raw output beside the summary.
