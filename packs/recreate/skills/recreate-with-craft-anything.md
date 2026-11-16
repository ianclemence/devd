---
name: recreate-with-craft-anything
description: Drive the craft-anything engine to study a target and check a rebuild
kind: discover
roles: recreator
---

# Recreate with craft-anything

The craft-anything engine does the inspection and the comparison; you do the deciding. It is a
tool server named `craft-anything`, saved disabled when this pack is installed, and it offers
every one of its tools over MCP once a person turns it on.

## When to use

When a recreation needs a measurement devd cannot take itself: file formats, binaries, game
data, a whole site crawled at once, or a comparison report the engine already knows how to make.

## Procedure

1. Check the tool server is on before planning around it: `mcp.servers` names it and its state.
   If it is off, say so and continue with devd's own tools rather than waiting.
2. Ask the engine to study the target, then read the result as evidence: every record carries
   confidence, coverage and its limitations. Copy the record into the run with `recreate.check`
   so the claim and its basis sit together.
3. Keep clean room: the engine refuses tools that look inside a target, and a refusal is
   information, not an obstacle. Do not switch it to Deep mode yourself.
4. Build with devd's own specialists and tools. The engine is for measuring, not for writing the
   app; a project built by the engine alone has no run, no evidence and no verdict here.
5. Compare with `recreate.check`, which records the engine's report beside devd's own captures.

## Pitfalls

- Treating a similarity or confidence figure as proof. It is a measurement.
- Installing the engine's own project into the person's folder and calling that the delivery.
- Leaving the tool server on after the run because turning it off felt like extra work.

## Verification

The run has the engine's evidence records, devd's three-width captures, the check report, and
the project's own checks passing; a reviewer has read all four.
