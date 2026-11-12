---
name: keep-your-context-lean
description: Search before reading, batch edits, and keep conclusions instead of output
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

# Keep your context lean

Every model call sends the whole conversation again. Ten thousand tokens of output you did not need is paid for on every later call, and a model reading a long, noisy history reasons worse. devd puts old tool output away once a conversation grows, but only after the fact; these habits keep it from growing.

## When to use

Always. It is most useful before reading a large file, running a command that prints a lot, or editing a file more than once.

## Procedure

1. **Find, then read.** `search_files` for the symbol first, then `read_file` with `offset` and `limit` for the part that matters. `list_files` one folder at a time.
2. **Narrow commands at the source:** a filter flag, a quiet flag, the last lines only, a single test file. Quote the failing line and its cause, not the log.
3. **Do not buy the same thing twice.** A file you just read or wrote is in front of you. Several changes to one file are one `edit_file` call with `edits`. Independent reads go in the same turn. Do not call `devd_describe` for a shape you already have.
4. **Keep conclusions, not output.** Write what you learned in one line (`scratch.write` or your next message) and move on.
5. **Send bulk exploration to a separate task** and take back its summary.

## Pitfalls

- Pasting a 2,000-line log into the conversation.
- Reading the same file four times.
- Writing a file and then pasting its contents into a message. Name it.
- A final report as long as the work.

## Verification

Your reads were ranges, your edits per file were one call, and your report says each result once.
