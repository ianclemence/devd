---
name: principle-minimize-reader-load
description: Make code, diffs and reports cheap to read.
kind: principle
roles: implementer,reviewer,docs
---

# Make code, diffs and reports cheap to read

Every reader pays attention for every line. Keep functions short and names exact, keep unrelated changes out of the diff, keep reports short with the answer first. Order work so a reviewer can follow it step by step.
Smell: a 40-file diff with one idea in it.
