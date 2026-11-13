---
name: work-on-a-branch
description: Give each parallel piece of work its own branch, address and database
kind: build
roles: planner,architect,implementer,backend,frontend,qa,reviewer
---

# Work on a branch

When two pieces of work must run at once, or a change should be seen before it is merged, gives each its own worktree instead of a shared checkout, served at its own address.

## When to use

Parallel work on one site, or a change a person wants to look at before merging.

## Procedure

1. `worktree.sites` lists the site's worktrees and the address each is served at. Use the one you need if it exists.
2. Otherwise `worktree.create` with the site and a branch name (or none for a made-up one). It checks the branch out under devd's own folder, serves it at `<branch>.<site>.test`, writes a `.env` pointing at that address, and says what is missing. A new branch starts from the site's current branch unless you pass `from`.
3. Choose the database on purpose. `share` (the default) uses the main one, right for read-only work. `clone` makes a copy named for the branch, right when the branch runs migrations: a migration on a branch must not touch the main database.
4. The result's `todo` names what a fresh checkout lacks, usually `composer install` and `npm install`. Run them in the worktree's folder before you test.
5. Work in the worktree's folder, check the result at its address, and report that address with what you saw.

## Pitfalls

- Sharing the main database on a branch that migrates.
- Expecting the hosts file to change. If the name does not resolve, `hosts.sync` adds it, and asks first.
- Deleting a checkout. `worktree.unserve` removes the site and keeps the files; `git.worktree.remove` deletes work and asks first.

## Verification

You loaded the branch's address and saw the change there.
