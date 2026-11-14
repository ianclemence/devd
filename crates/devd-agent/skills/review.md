---
name: review
description: Review uncommitted changes, a commit, a branch or a pull request
kind: review
roles: reviewer,qa,security,architect
---

# Review

Reviews what the person points at: their uncommitted changes by default, or a commit, a branch or a pull
request they name. It reports findings with the proof for each; it does not change files.

## When to use

- The person types `/review`, optionally followed by a commit, a branch name or a pull request number.
- Before something is committed, merged or released, when the person asks for a second look.

## Procedure

1. **Find the change.** Nothing named: read `git_status` and `git_diff` for the project, staged and unstaged.
   A commit or branch: `git_log` to find it, then `git_diff` against its base. A pull request: read its diff
   from the host the project's remote points at. Say in one line what is being reviewed.
2. **Read what the diff touches,** not only the diff: the callers of a changed function and the tests that
   cover it. Use `explore` when that means looking across many files, so the reading does not fill this
   conversation.
3. **Look for what would hurt:** wrong results, unhandled errors, data loss, security holes, broken callers,
   and tests that do not test the change. Leave style to the formatter.
4. **Prove each finding.** Run the test or the command that shows it, or quote the lines that make it true.
   A finding with no proof is a question, and is labelled as one.
5. **Report.** The verdict first (nothing found, or the number of findings), then each finding: where it is,
   what goes wrong and when, and the smallest fix. Order by how much it would hurt.

## Pitfalls

- Reviewing the whole file when only a few lines changed.
- Listing every nit. A report with twenty small points hides the one that matters.
- Saying "looks good" without having read the callers.
- Changing files to fix what was found. The person decides which findings to act on.

## Verification

- The first line says what was reviewed.
- Every finding names a file and line, and either shows its proof or says it could not be proved.
- No file was changed.
