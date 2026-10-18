# The Git command line

`devd git` keeps Git's vocabulary, because the people who use it already know
Git and should not have to translate. Every command below is a thin shell over a
capability — the same capability the TUI, the desktop app, the HTTP API and an
MCP server will call.

Nothing in this surface does work of its own. Each command shapes typed
arguments and dispatches a capability, which means the CLI cannot do anything
another surface cannot do, and cannot skip an approval the registry demands.

## Global options

| Option | Effect |
| --- | --- |
| `--json` | Machine-readable output. |
| `--plan` | Show what would happen and change nothing. |
| `-y`, `--yes` | Approve a destructive or remote operation deliberately, for non-interactive use. |

An operation that needs approval without `--yes` prints its plan and exits `3`.
A plan is never a substitute for an approval and an approval is never implied by
a plan.

## Inspection

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git status` | `git.status` | Branch, upstream, ahead/behind, staged, unstaged, untracked, conflicted, and whether an operation is in progress. |
| `devd git diff` | `git.diff` | Working tree by default; `--staged` for the index, `--stat` for a diffstat, `--rev <rev>` to compare revisions, and a positional path to narrow it. |
| `devd git log` | `git.log` | `-n <count>`, newest first. |
| `devd git reflog` | `git.reflog` | `-n <count>`. This is what makes a bad reset recoverable. |
| `devd git conflicts` | `git.conflicts.list` | Paths Git could not merge. |
| `devd describe git.diff` | — | Summary, tier, argument schema. |

## Changes

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git stage <path>…` | `git.stage` | `.` stages everything. |
| `devd git unstage <path>…` | `git.unstage` | Leaves the working tree alone. |
| `devd git discard <path>…` | `git.discard` | **Destructive.** Loses uncommitted work. |
| `devd git commit -m <msg>` | `git.commit` | `--amend` replaces the previous commit, `--allow-empty` permits an empty one. Never opens an editor or a signing prompt. |
| `devd call git.stage.patch --args '{"repo":".","patch":"…"}'` | `git.stage.patch` | Hunk-level staging and reversing. Not exposed as a subcommand because a patch is not pleasant to type; the GUI and an agent use it directly. |

## Branches

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git branch list [--all]` | `git.branch.list` | `--all` includes remotes. |
| `devd git branch create <name> [--start-point <rev>] [-c]` | `git.branch.create` | `-c` creates and switches. |
| `devd git branch rename <from> <to>` | `git.branch.rename` | |
| `devd git branch delete <name> [--force] [--remote]` | `git.branch.delete` | An unmerged branch needs `--force`, which is the same gate as `git.discard`. |
| `devd git branch check <target>` | `git.switch.check` | Lists the files a switch would overwrite, so a switch is never a surprise. |
| `devd git branch prune [--base <rev>] [--dry-run]` | `git.repo.prune` | Deletes only branches already merged into the base. |
| `devd git switch <name> [-c] [--force]` | `git.switch` | `--force` discards uncommitted changes. |

## Remote synchronisation

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git fetch [--remote <name>] [--prune]` | `git.fetch` | |
| `devd git pull [--remote <name>] [--branch <name>] [--strategy <s>]` | `git.pull` | Defaults to fast-forward-only, so a pull never creates a merge commit by surprise. |
| `devd git push [-u] [--tags]` | `git.push` | **Remote tier: needs approval.** `-u` records the upstream. |
| `devd git push --force` | `git.push.force` | **A different capability with its own approval.** Prefers `--force-with-lease`. |
| `devd git remote list` | `git.remote.list` | |
| `devd git remote add <name> <url>` | `git.remote.add` | A URL that embeds a password is refused. Only `https`, `ssh`, `git`, `file` and scp-like forms are accepted. |
| `devd git remote rename <from> <to>` | `git.remote.rename` | |
| `devd git remote remove <name>` | `git.remote.remove` | |
| `devd git clone <url> [dest]` | `git.clone` | `--depth`, `--branch`, `--recurse-submodules`. |

```bash
devd call git.sync.preview --arg repo=.
devd call git.outgoing --arg repo=. --arg remote=origin --arg branch=main
devd call git.incoming --arg repo=. --arg remote=origin --arg branch=main
```

`sync.preview` reports incoming and outgoing commits separately and says whether
the branch has diverged, so a pull can be decided with the facts in hand.

## History rewriting

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git rebase --onto <rev>` | `git.rebase` | **Destructive.** Replays local commits with new hashes. |
| `devd git rebase --root --autosquash` | `git.rebase` | Replays the whole history; folds `fixup!`/`squash!` commits. Driven by a no-op sequence editor, so it cannot hang waiting for a human. |
| `devd git squash <count> -m <msg>` | `git.squash` | **Destructive.** Combines the most recent commits; `--root` semantics are handled internally when the whole branch is squashed. |
| `devd git cherry-pick <rev>` | `git.cherry.pick` | |
| `devd git revert <rev> [--no-commit]` | `git.revert` | Undoes a commit with a new commit — the safe way to undo shared history. |
| `devd git reset <rev> [--mode soft\|mixed\|hard]` | `git.reset` | **Destructive.** The plan states exactly what each mode destroys. |

## Stashes and worktrees

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git stash list` | `git.stash.list` | |
| `devd git stash push [-m <msg>] [--include-untracked]` | `git.stash.push` | |
| `devd git stash apply [ref] [--index]` | `git.stash.apply` | Restores without dropping. |
| `devd git stash pop [ref]` | `git.stash.apply` | `pop: true`. |
| `devd git stash drop <ref>` | `git.stash.drop` | **Destructive.** |
| `devd git stash show <ref>` | `git.stash.show` | Read the patch before applying it. |
| `devd git worktree list` | `git.worktree.list` | |
| `devd git worktree add <path> <branch> [-b]` | `git.worktree.add` | |
| `devd git worktree remove <path> [--force]` | `git.worktree.remove` | Refuses a worktree with uncommitted work unless `--force`. |
| `devd git worktree prune` | `git.worktree.prune` | |

A worktree is how a substantial task is done without disturbing the working
tree the user is in — the right default for an agent asked to make a risky
change.

## Tags

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git tag list` | `git.tag.list` | |
| `devd git tag create <name> [-m <msg>] [--rev <rev>] [--force]` | `git.tag.create` | A message makes it annotated. |
| `devd git tag delete <name> [--remote <name>]` | `git.tag.delete` | Deleting a published tag needs approval. |

## Merging and recovery

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git merge <branch> [--no-ff] [--squash]` | `git.merge` | A conflict is a state, not a failure: the command reports it and exits `5`. |
| `devd git continue` | `git.op.continue` | Continues a merge, rebase, cherry-pick or revert. |
| `devd git abort` | `git.op.abort` | Restores the previous state. Uncommitted work the operation did not touch is preserved. |
| `devd git undo` | `git.op.abort` / `git.reflog` | Aborts whatever is in progress; otherwise prints the reflog and the reset commands that go back. |
| `devd git bisect start <bad> <good>` | `git.bisect.start` | |
| `devd git bisect mark <good\|bad\|skip>` | `git.bisect.mark` | |
| `devd git bisect reset` | `git.bisect.reset` | Returns to the branch the bisect started from. |
| `devd call git.conflicts.show --arg repo=. --arg path=<p>` | `git.conflicts.show` | The file with its conflict markers intact. |
| `devd call git.conflicts.resolve --arg repo=. --arg path=<p> --arg side=ours` | `git.conflicts.resolve` | **Destructive.** Taking one side discards the other, so the plan says so first. |

## Repositories

| Command | Capability | Notes |
| --- | --- | --- |
| `devd git repo init <path> [--initial-branch <name>]` | `git.repo.init` | Creates and indexes. |
| `devd git repo open <path>` | `git.repo.open` | Read-only; describes a repository without indexing it. |
| `devd git repo track <path>` | `git.repo.track` | Adds it to the index. |
| `devd git repo untrack <path>` | `git.repo.untrack` | Removes it from the index. Nothing on disk is touched. |
| `devd git repo list` | `git.repo.list` | Most recently used first. |

## Configuration

```bash
devd call git.config.get --args '{"repo":".","key":"user.email"}'
devd call git.config.set --args '{"repo":".","key":"user.email","value":"me@example.test"}'
```

`git.config.set` is **human-only**: an agent cannot call it even with an
approval. A key that exists to hold a credential — `user.password`, `*.token`,
`*.secret`, `*extraheader` and similar — is refused outright, because a token in
`.git/config` is a plaintext token on disk and in every backup.

## Reading the exit code

| Code | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | The operation failed. |
| 2 | Invalid arguments, or unsupported. |
| 3 | Approval required, or the operation was denied. |
| 4 | Not found. |
| 5 | A conflict. The operation reached a state you must resolve. |

A script that wants to distinguish "the user said no" from "it broke" branches on
`3` versus `1`.
