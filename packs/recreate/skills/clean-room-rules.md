---
name: clean-room-rules
description: What a clean-room recreation may look at, and what it must leave alone
kind: house
roles: recreator,reviewer
---

# Clean room rules

A recreation is built from what a user of the original can see. This is a rule about which
tools you use, not a fencing of the machine: it holds because you keep it, and a refusal from
a tool is the signal that you have stopped keeping it.

## When to use

Before the first study step of a recreation, and again when you are tempted to look at source.

## Procedure

1. Confirm the mode from the project record: `recreate.mode` is `clean_room` unless a person
   switched it. If it says `deep`, the person approved reading code and files; if it does not,
   treat every read of source as refused.
2. Study only through what a visitor meets: `browser.open`, `browser.snapshot` for the
   accessibility tree, `browser.screenshot` at each width, `browser.click` and `browser.type`
   to reach the states, and `recreate.capture` to record them together.
3. Read visible text, measured layout, colours, fonts, spacing and the order of controls.
   These are observations, and each one carries the tool that produced it.
4. If a tool refuses because it would look inside, record the refusal and continue. Do not
   fetch the same thing another way, and do not ask an outside service for the original's code.
5. Declare the right to recreate before you start: `recreate.begin` requires it, and it is the
   person's statement, not yours.

## Pitfalls

- Treating "I found it in the page source" as visible. It is not, in clean room.
- Copying a logo, an icon, a photograph or the original's text because it was on screen.
- Skipping the study pass and building from what similar apps usually do.

## Verification

The project record names the mode, the tool behind every observation, and the right-to-recreate
declaration. A reviewer can read the capture folder and see no source inside it.
