---
name: work-on-a-branch
description: Give each parallel piece of work its own branch and its own address, so it can be looked at without disturbing the rest
kind: build
roles: planner,architect,implementer,backend,frontend,qa,reviewer
---

When two pieces of work must run at once, or a change should be looked at before it is merged, give each its own worktree instead of sharing a checkout.

1. `worktree.sites` lists the site's worktrees and the address each is served at. If the one you need exists, use it.
2. Otherwise `worktree.create` with the site and a branch name (or none for a made-up one). It checks the branch out under devd's own folder, serves it at `<branch>.<site>.test`, writes a `.env` pointing at that address, and says what is still missing. A new branch starts from the site's current branch unless you pass `from`.
3. Choose the database on purpose. `share` (the default) uses the main one, which is right for read-only work. `clone` makes a copy named for the branch, which is right when the branch runs migrations: a migration on a branch must not touch the main database.
4. The result's `todo` names what a fresh checkout lacks, usually `composer install` and `npm install`. Run them in the worktree's folder before you test.
5. Work in the worktree's folder, check the result at its address, and report that address with what you saw there.

The hosts file is not changed by any of this; if the new name does not resolve, `hosts.sync` adds it, and asks first. `worktree.unserve` removes the site and keeps the files. Removing the checkout itself is `git.worktree.remove`, which deletes work and asks first.
