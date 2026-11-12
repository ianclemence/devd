---
name: keep-your-context-lean
description: Every call re-sends the whole conversation, so search before reading, read only what you need, batch edits, and keep conclusions instead of output
kind: house
roles: planner,architect,researcher,implementer,qa,reviewer,infra,docs
---

Every model call sends the whole conversation again. Ten thousand tokens of output you did not need is paid for on every call after it, and a model reading a long, noisy history reasons worse, not better. devd puts old tool output away for you once a conversation grows, but only after the fact; the habits below keep it from growing in the first place.

**Find, then read.**
- `search_files` for the symbol or phrase first. Then `read_file` with `offset` and `limit` for the part that matters. Read a whole file only when you will change most of it.
- `list_files` one folder at a time. Never list a tree you could search.
- For a command that prints a lot, narrow it at the source: a filter flag, a quiet flag, `tail`, a single test file. Do not run the whole suite to learn about one test, and do not paste a log to describe a failure; quote the failing line and its cause.

**Do not buy the same thing twice.**
- A file you just read or wrote is already in front of you. Read it again only if something else may have changed it, such as a formatter or a command you ran.
- Several changes to one file are one `edit_file` call with `edits`, not several calls. They apply in order and are written only if all of them apply.
- Independent reads belong in the same turn.
- Do not call `devd_describe` for a capability whose shape you already have in your instructions.

**Keep conclusions, not output.**
- When you have learned something, write it in one line (`scratch.write`, or your next message) and move on. The line is what you will need; the output it came from is not.
- Send bulk exploration to a separate task and take back its summary.
- Say a result once. A report that restates the plan, the diff and the logs makes the next turn cost more and tells the person less.

**Do not write what is on disk.** A file you created is saved. Do not paste its contents into a message or a note afterwards; name it.

The failures this prevents: a 2,000-line log pasted into the conversation; the same file read four times; five calls to edit one file; a final report as long as the work.
