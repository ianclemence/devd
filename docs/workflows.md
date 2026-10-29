# Reviews and agent memory

Two things that happen around a change belong in the same capability surface as
everything else in devd: having it reviewed, and keeping what was learned where
every agent will read it. A project's **Reviews** and **Agent memory** tabs, the CLI (`devd call ...`), the
HTTP API and MCP reach the same capabilities.

| Group | Capabilities |
|---|---|
| Review | `review.run`, `review.start`, `review.list`, `review.get`, `review.delete` |
| Memory | `memory.propose`, `memory.apply`, `memory.list` |
| Instruction files | `compat.audit`, `compat.write` |
| Search | `search.query`, `search.index`, `search.indexed` |
| Conflicts | `git.conflicts.list`, `git.conflicts.show`, `git.conflicts.resolve`, `git.conflicts.resolve_with` |

**Background reviews.** `review.run` waits for the reviewer model. `review.start`
returns at once with an id and runs the same review in the devd process; the
record shows `state: running`, then `done` or `failed`, and a message is left in
the inbox (and a `review.done` event is emitted) when it finishes. Closing the
window does not cancel it; quitting devd does, and it is not restarted.

**Memory proposals merge.** A proposal is made against the text of `AGENTS.md` as
it was. When the file has since changed, `memory.apply` merges three ways - the
text the proposal was made against, the file now, and the proposed text. Changes
to different lines combine, two proposals that each add a list item both survive,
and a collision on the same lines is refused with both versions shown. Nothing is
guessed.

**Searching by relevance.** `search.query` reads files at query time. `search.index`
builds a full-text index of a project's text files (SQLite FTS5, a cache under
`cache/search.db` that can be deleted at any time) and `search.indexed` queries it,
best match first with a fragment around each. Only files that changed are read
again. Binaries, files over 1 MB and secrets files are never indexed, and the
result counts what was skipped.

**Resolving a merge conflict.** `git.conflicts.resolve` takes one side whole.
`git.conflicts.resolve_with` writes text an agent merged from both. It is refused
while conflict markers remain, and its result says how the text relates to the
real stages of the conflict: `ours`, `theirs`, `merge` (exactly what a three-way
merge of the common ancestor and both sides produces) or `custom`. A `custom`
resolution is a change nobody has checked: a goal that mentions a conflict needs
a passing build or test as evidence before the run counts as completed.

Records are JSON files under `<devd home>/workflows/<kind>/<id>.json`. They are
readable and deletable by a person and never live inside a project.

## The rule that shapes them

A result is either something devd observed or something a model said, and the
record says which. A review is the reviewer model's answer about **a real diff**,
stored with it. A compatibility audit lists **the files it read**. When a model is
needed and none is configured, the capability fails with what is missing rather
than returning an empty, plausible-looking result.

## Review

```bash
devd call review.run --arg project=D:\work\shop
devd call review.run --arg project=D:\work\shop --arg base=main
devd call review.list --arg limit=10
```

`review.run` reads `git diff` (uncommitted changes by default, `base:<rev>` for a
comparison), sends it to the model assigned to the `reviewer` role, and stores the
answer as a summary and findings with a severity, a file, a line, a detail and a
suggestion. A diff with no changes is reported as such and is not sent to a model.
Deleting a review removes the record, not anything in the project.

## Agent memory

`memory.propose` turns one lesson into a **diff for `AGENTS.md`**, stored as a
proposal. It writes nothing. `memory.apply` is the only step that writes, and it
is human-only:

* it refuses if `AGENTS.md` changed since the proposal was made, so the text a
  person reviewed is the text that lands;
* it keeps a timestamped copy under `.devd/backups/`;
* the line goes in a managed `Project memory (devd)` section and every other line
  stays byte for byte as it was;
* the same line is not added twice.

An agent may propose. Only a person applies.

## Instruction files

`compat.audit` finds the instruction files agents read in a project (`AGENTS.md`,
`CLAUDE.md`, `.cursorrules`, `.cursor/rules/*`, `.github/copilot-instructions.md`,
`.windsurfrules`, `.clinerules`, `GEMINI.md` and others) and reports each file, who
reads it, contradictions between them with the line that shows each, and a proposed
`AGENTS.md` built from the project's real check command and top-level folders. It
writes nothing. `compat.write` creates `AGENTS.md` only when there is none; with one
present it refuses and points at `memory.propose`. Human only.

## Limits

* A review is a model's opinion about a diff, not a proof; the team's evidence
  rules (a check that really passed) are what accept work.
* `memory.apply` does not merge concurrent proposals: a second proposal against the
  same base must be proposed again.
* Contradictions are found from command-shaped lines and explicit statements; a
  conflict in prose that shares no subject is not detected, and only root-level
  instruction files are read.
* A review blocks the request until the model answers; closing the window does not
  cancel it.
