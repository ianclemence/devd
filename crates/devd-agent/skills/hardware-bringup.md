---
name: hardware-bringup
description: Get firmware running on a real board in layers, proving each one
kind: build
roles: hardware,implementer,qa
---

# Hardware bring-up

Takes firmware from "it compiles" to "it works on the device" in layers, so a failure points at one layer. It does not flash or change a device the task did not name.

## When to use

New firmware, a new board, a driver or peripheral, or a bug that only shows on the device.

## Procedure

1. **Identify the target.** The chip and board, the toolchain, the pin and memory maps, and the datasheet or manual that governs them. Read the attached board's identity before you rely on it.
2. **Build.** The project's own build task (`project.run`, or `project.exec` for the vendor tool), for the right target. Fix warnings that mention types, alignment or stack.
3. **Test the logic on the host.** Put hardware access behind a thin interface and test everything above it without a device.
4. **Define the safe state** for anything that drives power, motors, heaters or radios, and make it reachable at every step. Change one thing at a time.
5. **Flash** only the named device, then watch it: a serial log, a logic-analyser capture, a measured value. Compare to what the datasheet says it should be.
6. **Soak.** Run it long enough to meet the slow faults: a watchdog, a leak, a brown-out, a clock drift.

## Pitfalls

- Guessing a pin, a clock or a register value. Quote the datasheet.
- Flashing whatever is attached.
- "Works on my bench" with no log. Keep the capture.
- Blaming the hardware before the build target, the voltage and the ground.

## Verification

Report the layer you reached (builds, host tests pass, flashes, behaves correctly on the device), the commands and their output, and what needs a real device to check.
